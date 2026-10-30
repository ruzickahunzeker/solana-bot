# Copy Trading

独立跟单系统，读取 `pumpapi_zmq_bridge` 数据流，通过 Telegram 优先管理，后续再接 Web。

边界：

- 交易由独立的 `trade_service` 执行；钱包余额、Quote 余额、自定义地址和 SOL 转账使用受鉴权的本机内部接口
- 手动实盘下单只通过 `trade_service` HTTP API；跟单执行先走独立 executor
- 钱包私钥只以密文形式落库
- Telegram 小范围白名单使用
- 实盘默认关闭，先 dry-run

## 初始化

```bash
cp .env.example .env
openssl rand -hex 32
```

把生成的 64 位十六进制字符串填入 `MASTER_KEY_HEX`。

```bash
cargo run
```

## 配置参数

配置文件来自 `.env`，可以先从 `.env.example` 复制。

### Telegram

```env
TELEGRAM_BOT_TOKEN=
ALLOWED_TELEGRAM_USER_IDS=123456789
ENABLE_TELEGRAM_BOT=true
RUST_LOG=copy_trading=info
```

- `TELEGRAM_BOT_TOKEN`：Telegram Bot Token。必须填写，否则服务无法启动 Telegram bot。
- `ALLOWED_TELEGRAM_USER_IDS`：允许使用机器人的 Telegram 用户 ID，多个用户用英文逗号分隔，例如 `123,456`。第一版只给小范围用户使用，所有消息和按钮都会先检查这个白名单。
- `ENABLE_TELEGRAM_BOT`：是否启动 Telegram bot。生产环境通常为 `true`；本地只测试数据库或 ZMQ 时可以设为 `false`。
- `RUST_LOG`：日志过滤级别。推荐 `copy_trading=info`，可看到极速跟单的 `event_to_submit_start_ms` 和 `submit_ms`；排查快照刷新时临时使用 `copy_trading=debug`。未配置时程序默认使用 `copy_trading=info`。

### 数据库与私钥加密

```env
DATABASE_URL=sqlite://./data/copy_trading.sqlite
MASTER_KEY_HEX=
```

- `DATABASE_URL`：业务数据库地址。当前推荐 SQLite，例如 `sqlite://./data/copy_trading.sqlite`。首次启动会自动创建数据库文件和表。
- `MASTER_KEY_HEX`：钱包私钥加密主密钥，必须是 64 位十六进制字符串，也就是 32 字节。可用 `openssl rand -hex 32` 生成。

重要：

- `MASTER_KEY_HEX` 丢失后，数据库里的私钥密文无法导出或使用。
- `MASTER_KEY_HEX` 泄露后，数据库里的钱包私钥有被解密风险。
- 不要把 `.env`、数据库文件、备份包发到不可信位置。

### PumpAPI 数据流

```env
PUMPAPI_ZMQ_ENDPOINT=tcp://127.0.0.1:8939
ENABLE_STREAM_CONSUMER=true
```

- `PUMPAPI_ZMQ_ENDPOINT`：连接 `pumpapi_zmq_bridge` 的 ZMQ 地址。当前 bridge 默认发布到 `tcp://127.0.0.1:8939`。
- `ENABLE_STREAM_CONSUMER`：是否启动 PumpAPI ZMQ 消费者。设为 `true` 后，跟单路径订阅 `normalized` 事件，全量 Alpha 观察路径订阅 `observation` 事件并写入 `alpha_observation_events`；`pumpapi_events` 仍只保存命中跟踪钱包或 pending 限价单的事件。消费者每 10 秒更新一次 `pumpapi_stream_last_seen` 心跳，Python ZMQ 子进程异常退出后会自动重启。设为 `false` 时只跑 Telegram/数据库等其他模块。

Alpha paper signal 配置：

```env
ALPHA_PAPER_ENABLED=false
ALPHA_PAPER_MIN_SCORE=0.65
ALPHA_PAPER_MIN_TRADERS=3
```

- `ALPHA_PAPER_ENABLED`：启用内存窗口特征和 `alpha_paper_signals` 写入。默认关闭；开启后仍只生成 `status=paper` 记录，不调用 Trade Service、不签名、不广播。
- `ALPHA_PAPER_MIN_SCORE`：30 秒、2 分钟、10 分钟流量特征的最低评分，范围 `0..1`。
- `ALPHA_PAPER_MIN_TRADERS`：2 分钟窗口要求的最少独立交易者数量。

只读查看最近 paper signal：

```bash
python3 scripts/alpha_paper_report.py data/copy_trading.sqlite --last-hours 24 --limit 50

# 72 小时 shadow 值班门禁：request_id 全覆盖，禁止 unknown/blocked
python3 scripts/alpha_paper_report.py data/copy_trading.sqlite \
  --last-hours 72 --check-shadow --min-request-id-coverage 1.0

# 只读检查 Trade Service readiness，不提交交易
python3 scripts/alpha_shadow_readiness.py --wallet-id "$ALPHA_SHADOW_WALLET_ID"
```

脚本按 observation 数量、signal 状态、Pool、最近信号以及 shadow order 状态和 `request_id` 覆盖率输出 JSON；不会写数据库，也不会提交交易。

值班环境可以参考 `docs/alpha-shadow-audit.service.example` 和 `docs/alpha-shadow-audit.timer.example` 安装只读门禁。示例默认要求最近 72 小时 `request_id` 全覆盖且没有 `unknown`/`blocked`；安装前应确认 shadow 已明确开启并按环境调整阈值。示例不会调用交易提交接口。

`alpha_paper_signals` 还会保留后续订单关联字段（`order_id`、`trade_service_request_id`、`remote_status`、`updated_at_ms`）。当前阶段这些字段保持空值；只有进入 shadow/simulate 并拿到 Trade Service 响应后才允许写入。

### Alpha shadow/simulate

```env
ALPHA_SHADOW_ENABLED=false
ALPHA_SHADOW_WALLET_ID=
ALPHA_SHADOW_BUY_SOL=0.01
ALPHA_SHADOW_SLIPPAGE_BPS=1500
ALPHA_SHADOW_TIP_SOL=0.0001
ALPHA_SHADOW_POLL_ATTEMPTS=5
ALPHA_SHADOW_MAX_INFLIGHT=2
ALPHA_SHADOW_MIN_SCORE=0.90
ALPHA_SHADOW_MAX_PER_MINUTE=10
ALPHA_SHADOW_ALLOWED_POOLS=
ALPHA_SHADOW_ALLOWED_MINTS=
```

开启 shadow 前必须填写 Trade Service 中已存在的钱包 ID。每个 signal 先经过评分门槛和可选的 Pool/Mint 白名单，再经过 `/ready`（`ready`、RPC、blockhash、wallet）和 `/v1/cache/warmup`，最后以固定幂等键调用 `/v1/trades` 且强制 `simulate:true`；响应的 `request_id`、状态和错误写入 `alpha_shadow_orders` 并按原 request_id 回查。`unknown` 不会创建新幂等键。`ALPHA_SHADOW_MAX_INFLIGHT` 限制并发 shadow 请求，默认 2；`ALPHA_SHADOW_MIN_SCORE` 默认 0.90；`ALPHA_SHADOW_MAX_PER_MINUTE` 默认 10。设置 `ALPHA_SHADOW_ALLOWED_POOLS` 和 `ALPHA_SHADOW_ALLOWED_MINTS` 可把试运行限制到单一 Pool/单一 Mint；它们为空时不增加白名单限制。该开关不会打开真实广播。

要求：

- 需要先启动 `/root/http/sol-bot/pumpapi_zmq_bridge/pumpapi_zmq_bridge.py`。
- 事件默认只做入库和 dry-run；只有策略明确切到 Live，且全局开关、readiness、钱包绑定和风控全部通过时，才会自动提交实盘订单。

### trade_service

```env
TRADE_SERVICE_URL=http://127.0.0.1:18787
TRADE_SERVICE_TOKEN=
TRADE_SERVICE_TIMEOUT_SECS=15
POOL_RESOLVE_TIMEOUT_SECS=6
TRADE_SUBMIT_MAX_ATTEMPTS=2
TRADE_SUBMIT_RETRY_DELAY_MS=250
TRADE_RECONCILE_INTERVAL_SECS=2
FAST_FOLLOW_ENABLED=true
FAST_FOLLOW_MAX_INFLIGHT=16
FAST_FOLLOW_CACHE_REFRESH_MS=5000
FAST_FOLLOW_LIMIT_QUEUE_CAPACITY=4096
MIGRATE_LOCAL_WALLETS_TO_TRADE_SERVICE=false
```

- `TRADE_SERVICE_URL`：现有 `trade_service` 的 HTTP 地址。新系统只调用它，不修改它。
- `TRADE_SERVICE_TOKEN`：调用 `trade_service` 的 Bearer Token。如果 `trade_service` 开启鉴权，这里必须与其 `LOCAL_TRADE_API_TOKEN` 一致；如果本地测试未启用鉴权，可以留空。
- `TRADE_SERVICE_TIMEOUT_SECS`：单次 HTTP 调用超时，单位秒，默认 `20`。覆盖 readiness、钱包查询、下单和订单状态查询；应大于 `trade_service` 的串行 RPC 寻池预算。
- `POOL_RESOLVE_TIMEOUT_SECS`：TG 输入新 Mint 时等待 Pool 解析的最大秒数，默认 `18`。路由优先从内存和 `mint_pool_route_candidates` 读取；同一 Mint 可保留多个 Pool，已验证的 WSOL/USDC 路由优先，未命中才调用 `trade_service`；“刷新”按钮会跳过已验证缓存并重新解析。
- `TRADE_SUBMIT_MAX_ATTEMPTS`：Live 下单提交的最大尝试次数，默认 `2`。每次都使用相同幂等键，避免请求已被远端接收但本地超时时重复成交。远端明确返回交易失败时不会盲目重下链上交易。
- `TRADE_SUBMIT_RETRY_DELAY_MS`：两次提交尝试之间的等待时间，单位毫秒，默认 `250`。设为 `0` 表示立即进行下一次有限重试。
- `TRADE_RECONCILE_INTERVAL_SECS`：后台回查 `accepted`、`warming_params`、`building_tx`、`submitting` 订单状态的间隔，单位秒，默认 `2`。
- `FAST_FOLLOW_ENABLED`：是否启用极速跟单路径。开启后，跟踪钱包与策略从内存快照匹配，Live 信号先调用 `trade_service`，再异步补写本地事件、Signal 和订单。关闭后回退到原来的数据库串行路径。
- `FAST_FOLLOW_MAX_INFLIGHT`：极速跟单最大并发任务数，默认 `16`。同一个执行钱包和 Mint 仍会串行，不同 Mint 可以并行。
- `FAST_FOLLOW_CACHE_REFRESH_MS`：跟踪钱包、策略、执行钱包 ID 和用户 MEV 设置的兜底刷新间隔，单位毫秒，默认 `5000`。Telegram 修改相关设置后会立即刷新，不必等待此间隔。
- `FAST_FOLLOW_LIMIT_QUEUE_CAPACITY`：独立限价单/兼容路径队列容量，默认 `4096`。限价查询或 SQLite 变慢不会阻塞极速跟单接收线程。
- `MIGRATE_LOCAL_WALLETS_TO_TRADE_SERVICE`：一次性钱包迁移开关。仅在 `trade_service` 已启用数据库钱包模式后临时设为 `true`；启动时会在内存中解密旧 TG 钱包、通过本地鉴权 API 导入并按公钥校验，然后写入 `external_wallet_id`。迁移成功后立即改回 `false`。

启动时会请求 `TRADE_SERVICE_URL/health`。Telegram 的实盘检查还会请求 `/ready` 并按公钥匹配 `/v1/wallets` 中的执行钱包。health、ready、钱包绑定或 PumpAPI 数据流新鲜度任一不满足时，都不能把策略或限价单切换到 Live；PumpAPI 心跳超过 60 秒未更新视为异常。

### 交易默认值

```env
DRY_RUN_DEFAULT=true
DEFAULT_BUY_AMOUNT_SOL=0.01
QUICK_BUY_PRESET_1_SOL=0.001
QUICK_BUY_PRESET_2_SOL=0.005
QUICK_BUY_PRESET_3_SOL=0.01
QUICK_BUY_PRESET_4_SOL=0.05
QUICK_BUY_PRESET_5_SOL=0.1
QUICK_SELL_PRESET_1_PERCENT=10
QUICK_SELL_PRESET_2_PERCENT=25
QUICK_SELL_PRESET_3_PERCENT=50
QUICK_SELL_PRESET_4_PERCENT=75
QUICK_SELL_PRESET_5_PERCENT=100
DEFAULT_SLIPPAGE_BPS=1500
DEFAULT_TIP_SOL=0.0001
USE_MEV=false
DEFAULT_MEV_PROVIDER=auto
SOL_USD_PRICE=
SOL_USD_PRICE_API_URL=https://api.jup.ag/price/v3
JUP_PRICE_API_KEY=
SOL_USD_REFRESH_SECS=60
```

- `DRY_RUN_DEFAULT`：新策略以及用户交易设置首次初始化时的默认模式。建议保持 `true`；初始化后可在 Telegram 选择 `Dry-run` 或 `Live`。
- `DEFAULT_BUY_AMOUNT_SOL`：默认买入金额，单位 SOL。
- `QUICK_BUY_PRESET_1_SOL` 到 `QUICK_BUY_PRESET_5_SOL`：首次创建用户交易设置时写入 SQL 的五档买入金额；之后可在 Telegram“全局设置 → 买入金额设置”分别修改。
- `QUICK_SELL_PRESET_1_PERCENT` 到 `QUICK_SELL_PRESET_5_PERCENT`：首次写入 SQL 的五档卖出比例；必须是 `1..100` 的整数，之后可在 Telegram 修改。
- `DEFAULT_SLIPPAGE_BPS`：用户设置首次初始化时采用的滑点，单位 bps。`1500` 表示 `15%`。策略和已创建限价单仍保存自己的值，不会被批量改写。
- `DEFAULT_TIP_SOL`：用户设置首次初始化时采用的小费，单位 SOL。已有策略和限价单保存自己的值。
- `USE_MEV`：用户交易设置首次初始化时采用的 MEV 总开关。初始化后以 SQL 中的用户设置为准；关闭时手动交易、跟单和限价触发提交均固定为 `use_mev=false`。
- `DEFAULT_MEV_PROVIDER`：用户设置首次初始化时采用的 Provider。支持 `auto`、`default`、`rpc`、`jito`、`temporal`、`flashblock`、`blockrazor`、`astralane`、`speedlanding`。
- `SOL_USD_PRICE`：可选 SOL/USD 报价。限价单默认按 USD 价格创建；当 PumpAPI 事件只有 SOL 价格时，配置此值后会换算成 USD 再判断触发。留空时只使用事件里明确的 USD 字段触发 USD 限价单。
- `SOL_USD_PRICE_API_URL`：SOL/USD 自动报价接口，默认使用 Jupiter Price API V3。
- `JUP_PRICE_API_KEY`：可选 Jupiter API key。为空时不带 `x-api-key` 请求头。
- `SOL_USD_REFRESH_SECS`：SOL/USD 自动刷新间隔，单位秒。设为 `0` 可关闭自动刷新，只使用 `SOL_USD_PRICE`。

选择 `Live` 后，执行仍需要同时满足：

- 手动交易、对应策略或限价单的模式为 `Live`
- 风控检查通过
- trade_service health 正常
- trade_service ready 正常
- 当前默认钱包的公钥能匹配到 trade_service 已加载钱包，并在请求中明确传入对应 `wallet_id`
- Telegram 实盘操作经过确认

### 极速跟单路径

极速模式不会修改 `trade_service` API。PumpAPI 事件到达后只执行内存地址匹配、策略筛选、金额计算和交易请求构造，然后立即调用 `/v1/trades`。幂等键使用 `策略ID + PumpAPI交易签名 + 方向`，由 `trade_service` 的持久幂等记录防止重复成交。

Live 极速跟单只使用事件自带的 `pool`、`pool_id` 和 `quote_mint`，不会在关键路径调用 Pool RPC。缺少必要路由、缺少交易签名、执行钱包未绑定或买入 quote 不是 SOL/WSOL 时会记录 blocked Signal，不会尝试交易。PumpAPI 原始事件、Signal、订单和远端结果在 Trade API 返回后补写；如果进程在远端接单后立刻崩溃，本地审计可能缺失，但远端交易和幂等记录仍保留。

ZMQ 读取、极速跟单和限价单监听互相解耦。一次 Trade API 请求即使等待一秒，也不会阻塞读取后续钱包事件。日志字段 `event_to_submit_start_ms` 表示本进程接收事件到开始调用 Trade API 的耗时，`submit_ms` 表示 Trade API 整体响应耗时。

## 最小启动示例

只测试数据库和迁移，不启动 Telegram，也不接 ZMQ：

```bash
TELEGRAM_BOT_TOKEN=dummy \
ALLOWED_TELEGRAM_USER_IDS=123456789 \
MASTER_KEY_HEX=填入64位hex \
ENABLE_TELEGRAM_BOT=false \
ENABLE_STREAM_CONSUMER=false \
cargo run
```

完整运行：

```bash
cp .env.example .env
# 编辑 .env 后运行
cargo run
```

## 第一阶段能力

- Telegram 白名单
- 钱包创建、导入、列表、改标签、切换默认、软删除、导出私钥确认流程
- 跟踪钱包添加、列表、启停、改标签、软删除；钱包与关联策略的启停状态保持同步
- PumpAPI ZMQ 事件入库，并按 watched wallet 生成 dry-run 信号、订单和模拟持仓
- watch wallet / strategy / order 表结构
- Telegram 手动买入 / 卖出确认流程，调用 `trade_service`
- dry-run 默认风控
- dry-run executor：买入会累加模拟持仓，卖出会先检查模拟持仓，没有持仓则记录 skipped 订单
- live executor：跟单和限价单使用稳定幂等键调用 `trade_service`，保存请求、响应、远端 request ID、交易签名和失败原因
- Live 钱包绑定：按公钥把本系统钱包映射到 `trade_service.wallet_id`；不匹配时记录 `blocked`，不会退回远端默认钱包
- 订单回查：远端处于中间状态时，后台持续更新本地订单，状态变化会写入 `order_attempts`

## 统一交易参数

交易、限价和跟单共用同一套执行参数结构与校验规则，但每个对象保存自己的参数值：

- `wallet_id`：本系统执行钱包 ID；提交 Live 请求前必须按公钥映射到 `trade_service.wallet_id`。
- `execution_mode`：`dry_run`、`simulate` 或 `live`。Telegram 当前开放 Dry-run / Live 切换，`simulate` 已预留给后续交易预演。
- `execution_method`：`sdk` 或 `pumpapi`。当前界面默认并隐藏为 `sdk`。
- `slippage_bps`：滑点，`0..10000` bps；Telegram 输入时使用百分比，例如输入 `15` 保存为 `1500` bps。
- `tip_sol`：小费，必须大于或等于 `0` SOL。
- `mev_provider`：系统 MEV 总开关开启后，每个交易或限价草稿保存自己的 Provider；关闭时内部使用 `none`。PumpAPI 执行方式暂不允许 MEV。

金额不属于 `ExecutionParams`，而是每个订单或策略自己的 `OrderSize`：

- 快速买入：本次交易独立填写固定 SOL 金额。
- 限价买入：每张限价单独立填写固定 SOL 金额。
- 跟单买入：每个策略在“固定 SOL”与“目标钱包买入比例”之间单选；比例模式可受单 mint 上限约束。
- 卖出：快速交易、限价单和跟单策略各自保存 `1..100` 的整数百分比。

限价动作保存为 `buy_below`、`buy_above`、`sell_below`、`sell_above` 四种完整语义。旧数据库中的 `side + trigger_type` 会在启动迁移时自动回填，不需要重建旧限价单。

Pool 不作为快速交易或限价单的界面参数。跟单和限价执行时从触发它们的 PumpAPI 事件获取 Pool；快速交易目前由内部默认解析。后续可以接入独立 Pool 数据库或 `PoolResolver`，不改变 Telegram 下单参数。

## Telegram 用法

基础命令：

```text
/start
/status
/wallets
/watch
/watch_add <钱包地址> [标签]
/strategies
/signals
/positions
/limits
/buy <mint> <SOL数量>
/sell <mint> <卖出百分比>
/orders
/import <私钥>
/panic
```

菜单能力：

- 钱包管理：创建、导入、改标签、设为默认、导出私钥、删除。钱包列表和详情显示原生 SOL 余额；RPC 查询失败显示“暂不可用”，不会显示为 0。打开钱包面板或查看状态时，会从 `trade_service` 钱包数据库同步 `system/global` 钱包和当前用户自己的钱包引用；TG 本地只保存外部钱包 ID、公钥和标签，不复制私钥密文。
- 自定义钱包：创建时可选普通钱包、自定义 1 到 4 位 Base58 开头或结尾。任务面板原地显示状态、尝试次数，并提供刷新和取消。生成工作在 `trade_service` 内完成，Telegram 不接触生成出的私钥。
- SOL 转账：可选择另一个已绑定钱包或输入外部地址，金额支持 50%、100% 和自定义 SOL；确认页之后才调用内部接口。固定金额和 50% 会保留动态免租余额及安全余量；100% 扣除精确网络手续费后把来源 SOL 账户清到 0。系统全局钱包是否允许转出由 `trade_service` 的内部接口配置控制。
- SOL 转账不属于 Public Trade API：外部 Trade API 仍只支持买卖，不开放通用转账能力。转账没有 Pool、滑点、小费或 MEV 参数。
- 全局设置：在原消息中修改默认模式、执行方式、滑点、小费、`USE_MEV`、MEV Provider、快速交易确认开关，以及五档买入金额和五档卖出比例；模式、执行方式、Provider 和默认档位会展开全部选项，点选后返回上一级。设置写入 SQL，环境变量只负责首次初始化。快速交易确认默认开启。
- 跟单设置：添加 watch wallet、启用/暂停、改标签、删除。
- 策略：命中 watch wallet 后会自动创建默认策略；点击跟踪钱包进入策略面板，可设置启停、Dry-run / Live、固定或比例买入、卖出比例、滑点、小费、MEV 和最大延迟等。
- 信号：`/signals` 查看最近 dry-run 信号、mint 详情和延迟。
- 持仓：`/positions` 查看 dry-run 模拟持仓。
- 限价单：`/limits` 通过“创建限价单”进入原地面板，可组合限价买卖、高低方向以及价格/市值触发；pending 限价单可独立修改模式、滑点、小费和 MEV，PumpAPI 数据流触发后会生成执行订单并发送 Telegram 通知。
- 快速交易：确认开启时，金额按钮只选择数值，底部“确认买入/卖出”按钮会立即提交，不再出现第二次确认；确认关闭时，点击买入 SOL 或卖出百分比预设会直接提交，自定义数值输入完成后也会直接提交。提交期间同一用户只能有一笔请求，旧方向按钮在面板切换后会失效。
- 直接发送一个 Mint 地址会打开原地刷新的交换/限价面板；买卖预设、滑点、小费、钱包、模式和 MEV Provider 都在同一条 Telegram 消息中操作。价格或市值输入按钮只进入等待输入状态，输入完成后才刷新原面板。
- MEV Provider 不再逐个轮换；全局设置、交易面板、跟单策略和限价单会列出全部可用 Provider，并用 `✅` 标记当前选项。
- 添加跟踪钱包必须使用“跟单设置 → 添加”或 `/watch_add`，避免把 Mint 误当成跟踪钱包。跟单策略保留独立 Pool 筛选；交换和限价不提供 Pool 筛选。
- 订单：查看最近 10 条订单。

当前交易边界：

- 不再设置额外的全局实盘开关；手动交易、跟单策略或限价单选择 `Live` 后，在 readiness、钱包绑定、风控和确认通过时调用 `TRADE_SERVICE_URL/v1/trades`。
- 数据库钱包模式下，`trade_service` 是私钥密文和运行时签名钱包的唯一管理者；TG 只保留本地记录 ID、公钥和 `external_wallet_id`。钱包列表和详情会按公钥同步远端 `wallet_id` 与标签。创建、导入、导出、改标签、删除通过本地鉴权钱包 API 完成，交易只传远端 `wallet_id`。快速交易中的钱包按钮会展开全部远端 ID，选中后返回原面板；刷新会保留当前选择。钱包列表、确认页和提交结果也会显示本次使用的远端 ID，便于与 `/v1/wallets` 对照；远端 `wallet_id` 是稳定交易标识，改标签不会修改它。
- PumpAPI 跟单事件会通过 executor 生成 dry-run signal/order/position；策略切换为 `Live` 后，在 readiness、钱包绑定和策略风控都通过时提交到 `trade_service`。
- 通过策略筛选的跟单信号会自动写入 `orders.status=dry_run`，便于在 `/orders` 中复盘“如果实盘会下什么单”。
- `orders.wallet_id` 继续保存本系统钱包 ID，`orders.trade_service_request_id` 保存远端请求 ID；后续 `trade_service` 钱包迁移到 SQL 时可以保持这层接口不变。
## 自定义 Quote 快速交易

## Alpha shadow 只读诊断

shadow 默认关闭，发生模拟异常时先用只读诊断脚本区分本地限流与 Trade Service 路由错误：

```bash
python3 scripts/alpha_shadow_diagnose.py \
  data/copy_trading.sqlite --sample-limit 20
```

脚本只读扫描 `alpha_shadow_orders`，不会修改数据库或调用交易接口。`blocked` 的本地限流记录不计入路由失败；只有带 Trade Service `request_id` 的记录才进入远端错误聚合。

StonkFun / StonkFunSwap 池若使用非 SOL 的 Quote（包括 USDC、USD1 或其他 Token），快速买入面板会显示 Quote Mint 和所选钱包的 Quote 余额。买入金额输入该 Quote 的数量，程序从 Trade Service 查询链上小数位后精确转换为 `amount_quote_units`。钱包必须预先持有 Quote，并留有 SOL 支付手续费及可能的代币账户租金。卖出仍使用百分比，所得为该 Quote，不会自动兑换为 SOL。普通 SOL 预设不会用于自定义 Quote；自定义 Quote 限价买入及 SOL 金额跟买目前仍被拦截。

TG 使用的 Trade Service 须包含 `/internal/wallets/:wallet_id/quote-balance/:mint` 接口。部署时先更新 Trade Service，再更新 copy_trading，避免旧服务没有此接口。
