# Pump Trade Service

公网可信调用方的完整接口契约、幂等规则、Pool/Quote 矩阵和接入示例见：

- [Public Trade API 对接文档](docs/PUBLIC_TRADE_API.md)

本地 HTTP 交易服务。策略进程可以把 PumpAPI stream 里产生的交易意图发到这个服务，默认由本服务通过 `sol-trade-sdk` 执行交易；也可以在单笔请求里指定 `execution_method:"pumpapi"`，使用 PumpAPI Local Transaction 构造 unsigned transaction 后由本服务本地签名。

重要边界：

- 默认交易方式是 `sol-trade-sdk`。
- `execution_method:"pumpapi"` 只使用 PumpAPI Local Transaction 模式。
- 本服务不会使用 PumpAPI Lightning 私钥模式。
- HTTP 请求体里不能传私钥。
- 私钥只从服务端配置的钱包文件或钱包数据库读取。
- PumpAPI 只会收到钱包公钥、mint、quote、金额、滑点和 priority fee，不会收到私钥。
- 服务默认只监听 `127.0.0.1:18787`。

普通买入的账户存在性提示已接同一个共享读取 runtime，复用 preparation 容量、250 ms 起点间隔与跨类别冷却；2 秒总期限、512 KiB/1000 账户上限，错误不当作不存在。原 SDK program-label 布尔含义保留，不冒充 ATA/余额证据。`27bd996` / Actions `44115951258` 独立验收完整并发/串行各 1652、57 步骤和 99 字段拒绝通过；不代表参数/寻池/模拟等剩余读取已完成迁移或已经部署，见 [账户存在性读取](docs/optimization/PREPARATION_PRESENCE_SHARED_RPC.md)。

## 配置

开发验证入口及小内存机器的资源限制见 [开发检查说明](docs/optimization/DEVELOPMENT_CHECKS.md)。该机器不应直接反复运行无资源限制的全量测试编译；先类型检查，再受限专项验证。此说明不改变生产配置或 release 构建。

完整云端验收及源码快照规则见 [GitHub Actions 验证边界](docs/optimization/GITHUB_ACTIONS_VALIDATION.md)。云端不接收生产密钥/钱包/数据库，测试阶段仅开放回环网络；CI 通过不代表已部署。

服务启动时会自动读取当前目录下的 `.env` 文件。

示例：

```env
RPC_URL=https://your-primary-rpc.example
BACKUP_RPC_URLS=https://your-backup-rpc-1.example,https://your-backup-rpc-2.example

PRIVATE_KEY_BASE58=replace-with-base58-encoded-solana-keypair-secret
LOCAL_TRADE_API_TOKEN=replace-with-local-random-token

# 多钱包可选配置。优先使用加密钱包文件。
DEFAULT_WALLET_ID=main
WALLETS_CONFIG_ENCRYPTED=/root/http/sol-bot/trade_service/wallets.json.enc
WALLETS_CONFIG_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase
WALLET_STORAGE_MODE=file
WALLET_DATABASE_URL=sqlite://./data/wallets.sqlite
WALLET_DB_BUSY_TIMEOUT_MS=5000
WALLET_DATABASE_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase

# 明文兼容模式，不建议长期使用。
# WALLETS_CONFIG=/root/http/sol-bot/trade_service/wallets.json

TRADE_HTTP_ADDR=127.0.0.1:18787
DEFAULT_BUY_SOL=0.001
DEFAULT_SLIPPAGE_PERCENT=15
DEFAULT_TIP_SOL=0
ENABLE_SIMULATE=true
# 可选禁止广播策略；false 保持原 Live 行为，true 拒绝现有发送入口及 SDK 完整买卖流水线。
TRADE_BROADCAST_DISABLED=false
# 管理权限迁移默认兼容；separate 要求独立凭据和 loopback。
WALLET_ADMIN_AUTH_MODE=legacy
WALLET_ADMIN_API_TOKEN=
WAIT_TX_CONFIRMED=false
ENABLE_SDK_LOG=false
TRADE_RESPONSE_WAIT_MS=3000
TRADE_TASK_DRAIN_TIMEOUT_MS=30000
TRADE_SHADOW_DRAIN_TIMEOUT_MS=5000
MEV_SUBMIT_VERIFY_MS=1200

ENABLE_PUMPAPI_EXECUTOR=false
PUMPAPI_URL=https://api.pumpapi.io
PUMPAPI_TIMEOUT_MS=1200
PUMPAPI_DEFAULT_PRIORITY_FEE_SOL=0.0001
PUMPAPI_LIGHTNING_ENABLED=false

BLOCKHASH_REFRESH_MS=400
BLOCKHASH_MAX_AGE_MS=2000

# 交易/幂等持久化。默认 memory 保持原行为。
TRADE_PERSISTENCE_MODE=memory
TRADE_DATABASE_URL=sqlite://./data/trades.sqlite
TRADE_DB_BUSY_TIMEOUT_MS=5000

ENABLE_MEV_PROTECTION=false
ENABLE_JITO=false
JITO_UUID=
JITO_REGION=frankfurt
JITO_URL=

ENABLE_TEMPORAL=false
TEMPORAL_API_KEY=
TEMPORAL_REGION=frankfurt
TEMPORAL_URL=

ENABLE_FLASHBLOCK=false
FLASHBLOCK_API_KEY=
FLASHBLOCK_REGION=frankfurt
FLASHBLOCK_URL=

ENABLE_BLOCKRAZOR=false
BLOCKRAZOR_API_KEY=
BLOCKRAZOR_REGION=frankfurt
BLOCKRAZOR_URL=
BLOCKRAZOR_TRANSPORT=grpc

ENABLE_ASTRALANE=false
ASTRALANE_API_KEY=
ASTRALANE_REGION=frankfurt
ASTRALANE_URL=
ASTRALANE_TRANSPORT=binary

ENABLE_SPEEDLANDING=false
SPEEDLANDING_API_KEY=
SPEEDLANDING_REGION=frankfurt
SPEEDLANDING_URL=
```

## 交易与幂等持久化

新增安全参数的含义和限制：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `TRADE_BROADCAST_DISABLED` | `false` | 启动时固定的服务端禁止广播策略；`true` 时请求的 `simulate:false`、MEV、小费及 `CONVERSION_LIVE_ENABLED=true` 都不能覆盖它；SDK 完整买卖入口（含 SDK simulate）也拒绝进入 |
| `WALLET_ADMIN_AUTH_MODE` | `legacy` | `legacy` 保持旧鉴权；`separate` 将钱包管理与靓号任务管理改为独立管理 Bearer + loopback，不能直接对公网开放 |
| `WALLET_ADMIN_API_TOKEN` | 未设置 | 只在 `separate` 中使用；必须非空，且不能与交易 API Token 相同；不把此值提交 Git、贴到聊天或打印到日志 |

`ENABLE_SIMULATE` 仍只是普通交易默认值；`TRADE_BROADCAST_DISABLED=true` 是独立控制，不是恢复“开启 Live 还要开总实盘”的默认流程。没有设置新参数时，现有调用行为保持兼容。错误值或 separate 缺少有效凭据会在 SDK 初始化前拒绝启动。开启禁止广播后 `/ready.paused` 为 true，但依赖健康与可执行交易仍是不同概念。

禁止广播模式保留只读查询、原签名对账以及项目明确分离的 unsigned 聚合模拟、PumpAPI Local 模拟和 WSOL simulate；它不是禁止签名、禁止钱包导出或断网沙箱，也不会撤销已发送交易。管理凭据按路由隔离，尚不是多租户权限模型。源码和离线测试已加入，本轮没有修改线上配置或部署；详见 [安全策略与管理权限验收](docs/optimization/EXECUTION_SAFETY_AND_ADMIN.md)。

`TRADE_PERSISTENCE_MODE` 支持三种模式：

- `memory`：默认值，普通交易记录和幂等仍在内存，不创建数据库；独立聚合报价的实盘提交被拒绝，模拟仍可用。
- `shadow`：普通交易仍由内存决定是否执行，同时把状态镜像到 SQLite；写库失败不改变普通交易路径；独立聚合实盘提交被拒绝。
- `durable`：SQLite 原子保留 `(wallet_id, idempotency_key)` 后才允许进入原执行器。数据库不可用时返回 `503`，不会降级到内存执行。

durable 模式下，相同钱包、相同幂等键、相同规范化交易参数会返回原订单且不再次执行；参数不同则返回 `409`。服务重启后，未完成记录会标记为 `unknown`，不会自动重放。

源码已增加 SOL/Quote 付款分支核对，避免不同资产整数碰巧相同而误认 duplicate；旧 hash/订单/签名不变。旧 request_json 无法核实时 durable 返回 `503` 并保留记录；memory/shadow 也会拒绝同键不同参数或分支。尚未部署，见 [兼容修复与回归](docs/optimization/ORDINARY_TRADE_HASH_REVIEW.md)。

独立 `/v1/quotes/sol-buy/:quote_id/submit` 在 durable 模式下使用同一 SQLite 文件中的独立 `quote_submissions` 表，签名及原始交易字节落盘后才广播；重复请求先读取账本，不依赖仍有内存报价。模拟请求不进入实盘账本。新增 `GET /v1/quotes/sol-buy/:quote_id/submission?wallet_id=...&idempotency_key=...` 进行原签名的只读对账，不创建订单或重发；不会启用后台定时任务。源码与离线测试已实现，未部署。详见 [提交验收](docs/optimization/QUOTE_SUBMISSION_JOURNAL.md) 和 [对账验收](docs/optimization/SUBMISSION_RECONCILIATION.md)。

### 普通交易原签名观测（源码阶段，默认关闭）

Durable 普通 SOL 聚合买入已有完整原签名账本时，可显式开启后台观测。C3d2 源码另将新 SDK 直接 DAMM v2 卖出的四种已签阶段接同一个有界任务；本扫描批次待完整 CI，未部署/启用。它只查询已有签名并保存明确 finalized 的结果，不恢复执行权、不重新报价/签名/广播；不处理没有完整签名证据的中断，其他 opaque SDK、Memory/Shadow 或 PumpAPI 不因此获得相同保证。普通 GET/列表/重复 POST 仍只读原记录。

| 参数 | 默认值 | 范围与说明 |
| --- | --- | --- |
| `ORDINARY_OBSERVATION_ENABLED` | `false` | `true/false/1/0`；只允许在 durable 模式开启，非 durable 会在钱包初始化前拒绝 |
| `ORDINARY_OBSERVATION_BATCH_SIZE` | `8` | `1..64`，每批最多候选数 |
| `ORDINARY_OBSERVATION_POLL_INTERVAL_MS` | `30000` | `1000..300000`，每批结束/超时后的等待毫秒数 |
| `ORDINARY_OBSERVATION_MIN_AGE_MS` | `30000` | `1000..3600000`，claim 与父行更新时间至少达到该年龄 |
| `ORDINARY_OBSERVATION_REQUEST_INTERVAL_MS` | `1000` | `1000..60000`，前后台观测共享的请求最小间隔；不是全服务 RPC 限额 |

每批最多 20 秒、单次 HTTP 最多 2 秒、响应体最多 64 KiB；使用前两个不同的已配置 RPC，串行选择，429 冷却对应节点，全部冷却或配额忙时保留原未决状态。进程重开后扫描仍未决的已有签名，不重放交易。关闭任务会取消在途查询，但不证明未迁移的 SDK 自建 worker 停止。SDK 与聚合两索引同快照各取最多 limit+1 个游标，合并后仍共用原每批 limit 和 RPC 预算。即使开关关闭，部署此源码的初始化仍增加/核对 007/010 部分索引；010 的标记或结构异常会拒绝而不自动修复，需要独立生产迁移/回滚验收。当前未部署，见 [原聚合扫描证据](docs/optimization/ORDINARY_PENDING_OBSERVATION_WORKER.md) 与 [SDK 扫描](docs/optimization/SDK_PENDING_OBSERVATION_SCAN.md)。

### 共享只读 RPC（源码阶段，未部署）

blockhash 刷新、readiness 探测、可选普通聚合观测、原生 SOL 批量余额、Quote 余额查询和交易准备余额已接同一 HTTP runtime 与 endpoint 冷却；六类各预留一笔容量和独立起点间隔（100/250/1000/250/250/250 ms）。就绪请求忙时立即返回 `rpc_ok=false`、原生余额逐行报错、Quote 余额按原 502 错误返回，准备余额返回类型化不可用且不进入 SDK，不排队占用关键刷新容量。刷新保持 `processed`，单节点 500 ms、总预算 1500 ms，按配置顺序串行备用；readiness 使用同样的节点/总 deadline，但仅在原执行 lane 候选内探测；观测保留 2 秒和原局部 FIFO。每笔不自动重试 HTTP，禁止重定向，响应上限按固定方法区分；取消不返还已发请求预算，错误不更新旧 hash 时间。

Readiness 在 RPC 与数据库等待结束后一次采样缓存年龄/新鲜状态，避免等待前的新鲜标记配上等待后已过期的 age；探测本身不写缓存。原 HTTP/字段及 `ready = has_blockhash && rpc_ok` 保持，paused/数据库健康仍独立。

原生 SOL 余额使用 `getMultipleAccounts` 的空 data slice，每笔最多 100 个地址、整次最多 1000 个；节点整批最多 4 秒、全程最多 8 秒，每笔 HTTP 仍最多 2 秒。按原 Default 候选串行读取，任意页失败后丢弃整批、下一节点从头读取，不拼接半批；数量异常保留全部可见钱包并逐行报错，仅明确缺账户时补零，合法零余额仍正常保留。空请求零 RPC，重复地址不合并。

Quote 余额查询仍只用原首个 Default RPC：依次 supply、token scan/两个 ATA 回退、SOL balance，通常三笔、最多五笔，单步最多 2 秒、总计 8 秒。保留原 decimals/raw u64 和饱和合计；错误 Mint/钱包/精度、重复账户和混合未解析扫描报错，不返回部分合计。仅 token scan 的响应上限为 512 KiB、最多 1000 账户，其他方法维持 64 KiB；超限直接拒绝，不靠 ATA 回退掩盖。Reader 只持项目地址/URL/runtime，不再保留 SDK RPC handles。

只有上述六类 profile 已迁移，不代表真实交易准备的其他参数/ATA 读取、寻池、独立 quote/WSOL、模拟或 SDK 全部读取已统一限流；也不能控制供应商账户/其他进程额度。观测默认关闭，但未来部署此源码时上述读取实现会生效；没有更换交易发送 client 或修改钱包存储。详见 [共享预算与生命周期](docs/optimization/SHARED_READ_RPC_RUNTIME.md)、[Readiness 合同与时序修复](docs/optimization/READINESS_SHARED_RPC_RUNTIME.md)、[原生余额合同](docs/optimization/NATIVE_BALANCE_SHARED_RPC_RUNTIME.md)、[Quote 余额合同](docs/optimization/QUOTE_BALANCE_SHARED_RPC_RUNTIME.md) 和 [准备余额预算](docs/optimization/PREPARATION_SHARED_RPC_RUNTIME.md)。

### DAMM 买入准备（源码阶段，未部署）

普通 SDK 的直接 DAMM 买入已有净报价/min_out 准备保护：沿用原 lane RPC，九账户快照及零签名模拟后，用净输出和整数滑点计算最小输出，绑定本笔钱包/池/输入量/模式并在 SDK 入口前复查三秒期限。不增加 API 参数、不改变 WSOL/USDC 金额单位；失败、零输出/舍入或不完整证据不会回退常量 1。节点须返回同次模拟的 fee、pre/post lamports 和 token balances；仅适用 V0 兼容/canonical 账户、直接 WSOL/USDC 方向，其他情形拒绝。当前源码完整验收状态见优化执行记录。

准备绑定已由 `61af682` 的独立 CI 验收；整笔候选、protocol/full 回调、返回签名绑定及额外费用预留已在 `ec1d2c9` 独立验收，完整并发/串行各 1583 项通过。同次报价后的 native 余额须覆盖固定优先费和选定小费。该 DAMM 买入模拟子路径只提交零签名候选，不调用 SDK signer；原 SDK 模拟返回的非广播签名不再生成，现有可空 `signature` 为 `null`，不是交易提交。仅源码离线验收，不等于真实节点兼容或已部署。

未部署，不是新的 Live 准入或完整签名权威迁移。买入原 owner 的签名字节账本/固定 sender、最后回调之后 SDK worker 的时效及副作用、其他协议/模式与真实节点仍有门槛，不得据此自动重发或宣称全链路安全，见 [原 NP-01 范围](docs/TRADE_SERVICE_OPTIMIZATION_PLAN_V1.0.md)。

后继源码已增加独立的 `damm-v2-buy-v1` 名义记录：保存原始整数金额、净报价/min_out、资产/池/账户、slot、同次模拟资金预算和消息摘要，并接现有买入候选的生成/整笔复核。它不是新的 Live profile 或数据库 authority，记录重构不产生 fresh Quote 或恢复签名许可。该记录基础已在 `19f80cb` 独立 CI 验收，完整并发/串行各 1594 项通过，仍未部署。

共同 `SdkPreparedRecord` 已接原账本/签名许可/回读的类型接口：按 version/profile 严格区分买卖，不增加 JSON 外壳，旧 SellV1 字节保持。该类型边界已在 `2fe436d` 独立 CI 验收，完整并发/串行各 1605 项通过，原 1594 项保留、新增恰为十一项；共同记录专项 9、93 字段拒绝及全部 52 步骤通过，不重跑或部署。详见 [原账本记录边界](docs/optimization/SDK_PREPARED_RECORD_BOUNDARY.md)。

原账本的共同消息重构/签名字节证据、买入原意图绑定、V1 历史回放及 ownerless 观测已在 `f029d55` 独立 CI 验收。2026-11-17 同项后继 `882e514` / Actions `44109233896` 的受控买入 signer/签后产物、原 CAS/落盘回读、固定 Buy sender、runtime 和 claim/扫描也已独立验收：完整并发/串行各 1638 项全部通过，保留父批 1621 项且新增恰为十七项，99 字段门禁及全部 56 步骤通过。仅覆盖 Durable Live SDK 直接 DAMM 的正整数 Native SOL/WSOL、既有 USDC，不扩展其他协议/模式，不代表部署或真实节点验收。历史记录仍不能刷新 Quote、恢复 owner、重新签名或发送。详见 [实际接线](docs/optimization/SDK_ORIGINAL_DAMM_RUNTIME.md) 和 [当前记录](docs/optimization/EXECUTION_STATUS.md)。

### 交易准备余额（源码阶段，未部署）

普通 SDK 买卖准备已用公开地址读取端口和 SDK 入口检查替代旧余额 helper。原 scan 校验之后还会批量读取原始 Mint 和两个规范 ATA，根据实际 Mint owner 核验 canonical 输入、钱包/Mint/程序、精度、initialized、权限/扩展及 raw 整数；有效 parsed 结果必须与 raw 一致。扫描不可用或全未解析时只能依据合法 raw 输入恢复，坏扫描不被 fallback 掩盖。快照从整次读开始计时，最多 3 秒；scan + raw 共用 4 秒逻辑 deadline。

进入 SDK 前检查买入输入和卖出百分比乘法的溢出/零值；坏或过期证据不换 lane，仅读取不可用保留原准备回退，SDK 入口后仍禁止自动换 lane 补单。原有效参数及准备顺序不变；范围仍是原准备会读余额的方向，不自动覆盖全部买入。非 canonical 正余额扫描、seed 模式、active hook/未审计扩展与异常权限会拒绝；裸数字 ATA fallback 不用于交易准备，合法的未使用 ATA 系统预存 SOL 不累加为 token units。

原始账户子项 A3b1 后，A3b2 已将输入账户/program/精度带到实际 SDK 入口：核对付款 Mint、DexType/params、方向、canonical ATA、固定资产/DBC 的声明精度与 simulate 模式，覆盖 Pump 默认/后缀及 StonkFun wrapper；DAMM final preview 也核对相同输入并保持模拟模式。缺失 raw 身份、程序/账户/模式不一致直接拒绝，不修改 SDK 参数迎合证据。它仍不是最终不可变消息、费用/min_out、全部账户权限或全方向可支出金额保证。

A3c 已将该 read port 接第六类共享 HTTP profile，不再持 SDK RpcClient：每笔操作仅当前已注册 lane URL，最多 scan/raw 两笔、起点间隔 250 ms、单步含 pacing 2 秒/整次 4 秒，scan 512 KiB/raw 64 KiB 流式上限，1000 账户/单账户解码 8192 字节规则仍保留。429 共享冷却后不继续 raw、不在读端口直接换节点；忙时/取消不占 blockhash 名额，不自动重试或跟随重定向。更短的单步超时可在剩余新鲜期限内通过合法 raw 恢复，不延长原 3 秒快照期限。它仍不是全服务 RPC/供应商额度或最终签名前授权。

详见 [准备共享预算与真实 HTTP 验收](docs/optimization/PREPARATION_SHARED_RPC_RUNTIME.md)；[SDK 输入绑定](docs/optimization/PREPARATION_SDK_INPUT_BINDING.md)、[原始余额证据](docs/optimization/PREPARATION_RAW_ACCOUNT_EVIDENCE.md)和 [A3a 入口检查](docs/optimization/PREPARATION_BALANCE_ENTRY_GUARD.md)保留历史证据。不应将这批未部署源码直接视为生产验收通过。

普通 SDK 的全方向无 signer 构建仍在实施。[B4b 候选 guard](docs/optimization/DAMM_FINAL_CANDIDATE_GUARD.md) 后，B4c1 把完整不可变零签名交易接实际 DAMM 卖出的 `simulate=true` 与 Live preview，仅调用原 lane 的只读 RPC、不进入 SDK signer；模拟成功仍为 `simulated`，`signature:null`。预检失败/取消/过期不继续正式提交，原参数/hash/生命周期与 SDK 方法准入保持，未部署。B4c1 当时的 opaque Live 子路径已由下方 C3c1/C3c2 接原 owner 的受控签名/原字节发送；其他方向/协议不因此获得同样保证。详见 [不可变产物、模拟与账本接续](docs/optimization/DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md)。

原账本接续的 C3a-C3d1 已接 Durable Live 直接 DAMM v2 卖出的原 owner、prepared/签名账本、受控签名、原字节发送和显式只读恢复/finality，旧聚合 Quote journal 不接受该操作；完整离线 CI 并发/串行各 1416 通过。C3d2 自动扫描/索引/worker 源码已接，16 项新用例仅类型检查，未部署/启用；其他方向/协议/模式及真实 SDK/供应商/生产迁移/发布仍待完成。详见 [实际 SDK 子路径](docs/optimization/SDK_ORIGINAL_DAMM_RUNTIME.md)、[SDK 恢复](docs/optimization/SDK_ORIGINAL_SIGNATURE_RECOVERY.md) 和 [有界扫描](docs/optimization/SDK_PENDING_OBSERVATION_SCAN.md)。

推荐切换顺序：

```text
memory -> shadow（完成对账）-> durable + ENABLE_SIMULATE=true（完成验收）
```

持久化模式本身不替换钱包加载、SDK/PumpAPI 交易构造或 MEV 路径；后续源码的只读 RPC 改造见上节，不改变发送 client。durable 数据库连接使用 `synchronous=FULL`，影响同库写入的刷盘成本，发布前需测量目标磁盘上的延迟；`ENABLE_SIMULATE=true` 仍只是默认值，不是不可绕过的全局禁止广播开关。

普通 SDK 参数加载和预热已接共享只读 scope：复用原 runtime/HTTP/cooldown，8 秒总期限、单步 2 秒、最多 32 次请求；SDK 可选配置不能吞掉读取失败。PumpSwap 两项固定配置合入同次账户快照，避免冷配置 180 ms 与 250 ms pacing 冲突，不改 decoder 或全局缓存。`1792c51` / Actions `44133854054` 独立验收：完整并发/串行各 1675，新增 23、102 字段拒绝及 60 步骤通过；不代表寻池/模拟等全部读取已统一或已部署。详见 [参数读取边界](docs/optimization/PROTOCOL_PARAMETER_SHARED_READS.md)。

## 参数说明

WSOL 只读报价已接原 runtime 的余额/ATA/租金/hash/fee 五步 scope，保留原金额、费用、退款及 Token/native 校验。`c196d1b` / Actions `44182843252` 独立验收完整并发/串行各 1769，新增 20、120 字段及全部 75 步骤通过；未改兑换签名提交/状态，尚未部署。见 [WSOL 报价读取边界](docs/optimization/WSOL_QUOTE_SHARED_READS.md)。

WSOL 签名状态读取后续已接同一个 runtime/原 Observation 配额，保留原首 RPC、单签名 history 和严格 finality/error/slot 校验，不再持有执行 RpcClient。`ec608c0` / Actions `44188954352` 独立验收完整并发/串行各 1783，新增 14、123 字段及全部 78 步骤通过；状态账本/CAS、签名广播保持，尚未部署。见 [WSOL 状态读取边界](docs/optimization/WSOL_STATUS_SHARED_READS.md)。

聚合模拟和余额基线已接原 runtime 的固定零签名两步会话，保留原先验基线后模拟、Token/CPI 和报告政策。`2f12c97` / Actions `44173368003` 独立验收完整并发/串行各 1749，新增 20、114 字段及全部 72 步骤通过；不代表已部署或可实盘。见 [聚合模拟读取边界](docs/optimization/AGGREGATOR_SIMULATION_SHARED_READS.md)。

聚合预检的 Mint/ATA 与 ALT 读取已接原共享 runtime/HTTP/冷却，三条入口共用同一资源；保留原 Token 程序、ATA 和 ALT 政策。`10c5b4b` / Actions `44164727015` 独立验收完整并发/串行各 1729，新增 16、108 字段及全部 69 步骤通过；后继模拟读取已由上方独立验收，尚未部署。见 [聚合账户读取边界](docs/optimization/AGGREGATOR_ACCOUNT_SHARED_READS.md)。

既有 WSOL/Quote CPMM 两跳寻池新增专用只读 scope，共用原 runtime/HTTP/冷却，保留原缓存与 TTL；五秒总期限、单在途/100 ms、最多 32 次请求。`17bc3be` / Actions `44156380200` 独立验收完整并发/串行各 1713，新增 20、108 字段及全部 66 步骤通过；不代表 SOL 路线已实网验收或部署。见 [两跳读取边界](docs/optimization/SOL_HOP_SHARED_READS.md)。

普通寻池算法已接只读 SDK adapter 和原共享 runtime 的两分支/100 ms 专用预算，保留原 Default/Helius 顺序及每节点三秒期限；Pump/DBC 分支故障独立，DBC 扫描只开放固定布局。`cde04bd` / Actions `44146511097` 独立验收完整并发/串行各 1693，新增 18、105 字段及 63 步骤通过；两跳已由上方后继批次独立验收，模拟等其他读取仍待收口，尚未部署。见 [寻池读取边界](docs/optimization/POOL_DISCOVERY_SHARED_READS.md)。

`RPC_URL`

主 RPC 地址。

`BACKUP_RPC_URLS`

备用 RPC 地址，多个用英文逗号分隔。合并顺序为 `RPC_URL`、单个备用变量 `BACKUP_RPC_URL`、`BACKUP_RPC_URLS`、别名 `RPC_BACKUP_URLS`。优化源码已按首次出现顺序去除跨变量的精确重复 URL，同域名不同 key 不合并，不改写 URL/凭据；尚未部署。具体读路径的超时/备用政策不同，SDK 寻池仍保留两个 Helius 的随机串行顺序，不表示所有调用都已获得统一冷却或自动备用。见 [真实 HTTP 验收与入口去重](docs/optimization/ORDINARY_OBSERVATION_HTTP_TRANSPORT.md)。

`PRIVATE_KEY_BASE58`

base58 编码的 Solana keypair secret。单钱包兼容模式使用。只放在本地 `.env`，不要放到请求体、日志、README 示例真实值或远程仓库。

`WALLETS_CONFIG`

多钱包配置文件路径。设置后服务会从 JSON 文件加载多个钱包，并忽略 `PRIVATE_KEY_BASE58`。

示例：

```json
{
  "wallets": [
    {
      "wallet_id": "main",
      "private_key_base58": "replace-with-main-wallet-base58-keypair-secret"
    },
    {
      "wallet_id": "wallet_2",
      "private_key_base58": "replace-with-second-wallet-base58-keypair-secret"
    }
  ]
}
```

建议权限：

```bash
chmod 600 /root/http/sol-bot/trade_service/wallets.json
```

`WALLETS_CONFIG_ENCRYPTED`

加密多钱包配置文件路径。设置后优先级高于 `WALLETS_CONFIG` 和 `PRIVATE_KEY_BASE58`。服务启动时会在内存中解密，不会把明文写回磁盘。

加密格式：

- KDF：PBKDF2-HMAC-SHA256
- 加密：AES-256-GCM
- 默认迭代次数：600000

配套 passphrase 有两种方式：

```env
WALLETS_CONFIG_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase
```

或：

```env
WALLETS_CONFIG_PASSPHRASE=只建议临时 shell 使用，不建议写入 .env
```

迁移明文 `wallets.json` 到加密文件：

```bash
cd /root/http/sol-bot/trade_service
umask 077
openssl rand -base64 48 > .wallets.passphrase
cargo run --release --bin encrypt_wallets -- \
  --in wallets.json \
  --out wallets.json.enc \
  --passphrase-file .wallets.passphrase
chmod 600 .wallets.passphrase wallets.json.enc
```

然后 `.env` 使用：

```env
WALLETS_CONFIG_ENCRYPTED=/root/http/sol-bot/trade_service/wallets.json.enc
WALLETS_CONFIG_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase
# WALLETS_CONFIG=/root/http/sol-bot/trade_service/wallets.json
```

确认服务能用加密文件启动后，再处理旧明文 `wallets.json`。

`DEFAULT_WALLET_ID`

默认钱包 ID。交易请求不传 `wallet_id` 时使用这个钱包。默认值是 `main`。

`WALLET_STORAGE_MODE`

钱包来源模式，支持 `file` 和 `database`，默认 `file`。切换为 `database` 后，首次启动会在空数据库中导入当前 `WALLETS_CONFIG_ENCRYPTED`/`WALLETS_CONFIG` 钱包；后续以数据库为准。旧文件不会被修改，可用于回退。

`WALLET_DATABASE_URL`

钱包专用 SQLite 数据库，默认 `sqlite://./data/wallets.sqlite`。不要与高频交易事件数据库混用。

`WALLET_DB_BUSY_TIMEOUT_MS`

钱包数据库锁等待时间，默认 `5000` 毫秒。

`WALLET_DATABASE_PASSPHRASE_FILE`

数据库逐钱包密文使用的 passphrase 文件。未设置时兼容读取 `WALLETS_CONFIG_PASSPHRASE_FILE`。数据库备份与 passphrase 必须分开保存。

数据库模式提供本机鉴权钱包管理 API：`POST /v1/wallets` 创建或导入、`GET /v1/wallets/manage` 查看、`PATCH /v1/wallets/:wallet_id` 改名、`DELETE /v1/wallets/:wallet_id` 软删除、`POST /v1/wallets/:wallet_id/export` 导出。所有接口都要求 Bearer Token 和允许的客户端 IP，交易请求仍只传 `wallet_id`，不能传私钥。

### Telegram 钱包内部接口

以下接口只供同机 Telegram 服务使用，不属于 Public Trade API，也不应通过反向代理或防火墙暴露到公网：

- `GET /internal/wallet-balances`：批量查询当前用户可见钱包的原生 SOL 余额。链上账户不存在返回 `0`；RPC 查询失败返回空余额和错误，二者不会混淆。
- `POST /internal/wallets/vanity-jobs`：创建自定义地址生成任务；`GET/DELETE /internal/wallets/vanity-jobs/:job_id` 查询或取消任务。
- `POST /internal/wallets/:wallet_id/transfer-sol`：发送原生 SOL。默认只允许来源钱包的 `owner_type/owner_id` 与请求完全一致；仅显式开启 `ALLOW_INTERNAL_SYSTEM_WALLET_TRANSFER` 时允许受控的 `system/global` 来源。

自定义地址匹配区分大小写，只允许 1 到 4 位 Base58 字符，字符 `0`、`O`、`I`、`l` 无效。服务全局最多并行执行一个生成任务，每个 owner 最多一个活动任务，单任务最多尝试 30,000,000 次。生成出的私钥只在进程内存中短暂存在，匹配成功后立即写入现有加密钱包数据库；任务表只保存公钥、进度和状态。

SOL 转账支持固定 lamports、50% 和 100%。固定金额和 50% 基于确认时重新读取的可转余额计算；服务动态调用 RPC 估算网络手续费和系统账户免租最低余额，并在免租余额之上额外保留 10,000 lamports 安全余量。100% 使用 `余额 - 精确网络手续费`，把来源系统账户降到 0 并由链上回收；同一公钥以后收到 SOL 时会重新出现。每次请求必须携带唯一 `idempotency_key`，重复请求返回原记录，不会再次提交。该接口不使用交易 Pool、滑点、小费或 MEV Provider。

转账安全路径已在源码接入发送前账本，**尚未部署或做真实链上验收**。key 仍在转账表内全局唯一；相同 key 必须匹配 owner、来源钱包/公钥、目标公钥和请求金额/百分比，异内容返回 `409`。指令签名和字节使用 `synchronous=FULL` 钱包库事务保存成功后才发送。发送错误、超时或签名后中断保留原签名并显示 `unknown`，不能换 key 自动重发；持久化故障可能返回 `503`，同样不得据此认定未发送。没有新账本且无法核实的旧无签名结果会拦截，不自动重新执行。

内部转账响应仍为原 11 字段，状态包括 `processing`、`submitted`、`confirmed`、`failed`、`unknown`；`confirmed` 沿用所选 RPC 的 commitment，不代表新增 finalized 保证。调用方在发布前须兼容 `unknown/409/503`。Native 转账没有 `simulate` 请求参数，未知 JSON 字段不构成模拟授权。实现和离线验收见 [转账 API 安全接入](docs/optimization/SOL_TRANSFER_API_EXECUTION.md)。

`ALLOW_INTERNAL_SYSTEM_WALLET_TRANSFER`

是否允许本机内部钱包接口从 `system/global` 钱包转出 SOL，默认 `false`。只应在 Telegram 用户范围受控、`/internal/*` 未经代理暴露、Bearer Token 已妥善保管时开启。该参数不赋予 Public Trade API 转账能力。

`LOCAL_TRADE_API_TOKEN`

本地 HTTP API 的鉴权 token。它不是链上参数，也不是钱包私钥。

`CONVERSION_DATABASE_URL`

独立兑换记录数据库，默认 `sqlite://./data/conversions.sqlite`。保存 WSOL 报价、幂等请求指纹及签名交易记录；不得提交到 GitHub，备份时按交易数据单独处理。使用 WAL + `synchronous=FULL`。

`CONVERSION_QUOTES_ENABLED`

是否初始化独立兑换数据库并启用 WSOL 报价，默认 `false`。未开启时现有交易服务启动不依赖兑换数据库。

`CONVERSION_LIVE_ENABLED`

是否开放 WSOL 包装/解包的实盘提交，默认 `false`。启用前必须先开启 `CONVERSION_QUOTES_ENABLED`，完成钱包归属、模拟和小额验收；不要与现有 `ENABLE_SIMULATE` 混淆。

`POST /internal/conversions/wsol/quotes`

使用内部 Bearer 鉴权。请求示例：`{"wallet_id":"2TE","owner_type":"telegram","owner_id":"123","direction":"wrap","amount_lamports":1000000}`；全部解包使用 `{"direction":"unwrap","all":true}`，不同时传金额。服务校验钱包归属、读取当前 SOL/WSOL ATA 余额和网络手续费估算，返回 `quote`（绑定钱包、金额、余额基线、租金、60 秒有效期）及 `executable:false`。部分解包的临时账户租金先垫付后返还，净返还显示为 0；全部解包关闭原 ATA 的租金返还单独展示，不计入兑换数量。此报价接口不签名、不广播。

`POST /internal/conversions/wsol/simulate` 接受 `{"quote_id":"cq_...","wallet_id":"2TE","owner_type":"telegram","owner_id":"123","simulate":true,"idempotency_key":"..."}`，只执行 RPC 模拟，返回 `status:"simulated"`；模拟不占用实盘幂等记录。`POST /internal/conversions/wsol/submit` 使用相同字段但 `simulate:false,"confirmed":true`，且仅在 `CONVERSION_LIVE_ENABLED=true` 时开放。调用方必须在用户确认后才设置 `confirmed:true`。相同幂等键不同请求会拒绝。优化源码的 Live 顺序已改为受控签名、完整身份 CAS 落盘、已签模拟、原记录/证据核对，最后才允许广播；尚未部署。已签模拟失败、RPC 错误或超时保留原签名/字节和 pending wallet，返回 `unknown`，不得换 key 重试；`unknown` 不表示一定调用过发送。`GET /internal/conversions/wsol/submissions/:submission_id?owner_type=telegram&owner_id=123` 按原签名回查状态，不自动重发。**当前未完成链上模拟与小额实盘验收，Bot 不应展示可执行兑换按钮。**

源码已将 WSOL simulate 改为未签名交易（`sig_verify=false`），不再为模拟读取长期钱包 signer 或生成签名；Live 仍在原 reservation 后由 SDK 内部受控签名，后续签名账本修正见下文。尚未部署，见 [签名边界与 unsigned 模拟](docs/optimization/SIGNER_ACCESS_AND_UNSIGNED_WSOL.md)。

优化分支已将 Live 已签交易收回 SDK 私有 artifact，模拟/发送前核对原消息、钱包、签名和落盘字节一致，不再向兑换模块暴露可编辑交易字段；这不等于完整兑换故障恢复或链上验收完成。见 [WSOL 安全工作包](docs/optimization/WSOL_CONVERSION_SAFETY_WORKPACK.md) 与 [已签交易封装](docs/optimization/WSOL_SIGNED_ARTIFACT.md)。本轮未部署或开启兑换实盘。

submit/simulate 的编排已进一步移入 Application，经项目 metadata/runtime/journal 端口复用原 store；一次请求保留原钱包和准备时选择的 RPC，后续不因注册表变化重新选节点。Quote/status 也已接入只读端口，四个 Handler/DTO/路由迁入 API，旧包装移除；外部接口、V1 指纹和表结构保持。状态写入已补单向 CAS，签名前和 unsigned 模拟前核对完整 Quote/标准消息；Live 已签模拟前必须完成签名落盘和精确读回。八个应用取消点/文件重开和提交回执丢失已通过离线回归。见 [报价和 API 边界](docs/optimization/WSOL_QUERY_AND_API_BOUNDARIES.md)、[签名前授权](docs/optimization/WSOL_PRE_SIGN_AUTHORIZATION.md) 与 [签名落盘及模拟](docs/optimization/WSOL_SIGNED_JOURNAL_AND_SIMULATION.md)。

原签名 status lookup 已加入 3 秒外层超时，以及结果数量、原签名、slot、明确确认等级与错误一致性校验。WSOL 只有有效 `finalized` 成功/错误才能推进既有 `confirmed`/`failed`；processed/confirmed 的错误仍保留 pending，不提前释放钱包。无效/缺失/超时响应只读回原记录，不重发、不清空旧 error；旧节点没有明确 confirmationStatus 时也保守保持未决。该变化不自动重查/修正历史终态，也不是整个 HTTP 请求 3 秒 SLA；见 [原签名查询与明确最终性](docs/optimization/WSOL_STATUS_FINALITY_AND_DEADLINE.md)。

reservation 已补完整身份与状态/证据校验，新建记录在同一事务插入、核对并提交；历史仍先于 Quote 过期判断返回。签名前失败也使用原 reservation 的完整身份 CAS、事务内读回及 Application 最终回读，不能按 ID 覆盖已签记录或返回另一意图的失败。取消/提交回执丢失后，原 key 不重签/重发；旧 wire helper 仅用于测试。见 [保留与 unsigned 失败身份](docs/optimization/WSOL_UNSIGNED_RESERVATION_AND_FAILURE.md)。新校验会拒绝旧库中的身份/证据不一致记录，但不自动改写、修复或释放它们。

优化源码已单独迁移 WSOL submit 错误合同：明确参数拒绝 400、幂等或 pending 钱包冲突 409、存储读取不可用/完整性异常/结果不明 503。错误 JSON 仍只有 `error`，新增 `no-store`、`x-wsol-error-code`、`x-wsol-error-stage`、`x-wsol-auto-retry:false` 和恢复建议；操作 ID/签名头仅来自已验证的绑定/签名持久化回执。503 不提供自动重试许可，不得自动换 key。status、quote 和 unsigned simulate 后续已分别按下文独立迁移；不能从 400 判断其他实盘操作未保留/未发送。调用方兼容和真实链上/断电验收仍待完成；未部署或修改 Bot，见 [提交错误合同](docs/optimization/WSOL_SUBMIT_ERROR_CONTRACT.md)。

新增内部只读恢复 `GET /internal/conversions/wsol/submissions/by-key?wallet_id=...&owner_type=...&owner_id=...&idempotency_key=...`，用于不知道操作 ID 的回执丢失场景。使用原 loopback/Bearer 鉴权，SQL 同时过滤原 key、钱包和 owner；只读原七字段快照，不查询 RPC、不签名/广播/更新记录，也不要求当前钱包已加载或 Live 开启。未找到或绑定不匹配统一 404，存储/证据异常 503，应用响应禁止缓存和自动提交重试。404 不证明原请求已停止，不允许自动换 key；POST submit 不能当成只读查询。只在返回副本中脱敏历史 error，不修复或释放原记录，见 [原 key 只读恢复](docs/optimization/WSOL_READ_ONLY_RECOVERY.md)。仅源码和离线验证，未部署。

报价存储已补可返回错误的解码、JSON 与行 ID/expiry 绑定，以及 INSERT/全部 13 字段/原 JSON 字节的事务内读回；损坏列不再使读取任务 panic。旧过期 Quote 和未知 JSON 字段仍按原规则读取，不重写或删除历史；submit 的坏 Quote 在 reservation 前返回完整性 503，quote/unsigned simulate 的同类错误也已按下文分类。原 key 恢复不依赖 Quote 是否可解码。见 [报价存储完整性](docs/optimization/WSOL_QUOTE_STORAGE_INTEGRITY.md)，仅离线验证；不代表报价链上验收或部署完成。

status 已补请求 ID/完整身份/记录形态校验及最终读回的单向推进规则，拒绝倒退、未知状态、终态替换，以及 unsigned failed 事后加入签名。查询仍只用原签名，不重签/重发；RPC lookup 缺失、异常或超时仍保留原状态。owner 不匹配/不存在保留原 400，存储、运行时或证据故障改为 typed 503，观测写入回执不明不能被当作未写入。成功回执保持七字段并禁止缓存，历史 error 只在响应副本中 URL 脱敏。恢复头只能来自已核对的首次记录，不能来自坏的最终读回；见 [状态记录与错误合同](docs/optimization/WSOL_STATUS_RECORD_AND_ERRORS.md)。status 仍可能 CAS 保存原签名观测；需要纯账本读取时用 by-key GET。仅源码/离线验证，未部署或迁移 Bot。

quote/unsigned simulate 已补 typed 阶段错误：明确参数、权限绑定、余额/过期或模拟程序拒绝保留 400，RPC/存储/完整性故障 503，Quote 保存回执不明不能伪装为参数拒绝。成功 JSON 不变，仅禁止缓存；这两个入口不提供实盘操作 ID/签名头。WSOL metadata 使用无私钥、可返回解码错误的七列投影，Quote 在保存前检查请求金额、来源余额、租金/返还及最小 SOL 预算。unsigned 模拟不创建/覆盖实盘 key，取消/文件重开不自动重放；金额仍为原整数单位，unwrap 的 WSOL 余额不被当成手续费 SOL 余额。见 [报价与未签名模拟边界](docs/optimization/WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md)。仅源码/离线验证；当前未执行兑换或部署。

同一钱包一次只允许一笔未决兑换（`reserved`、`signed`、`submitted`、`unknown`）。遇到未决记录时，先用原 `submission_id` 查询状态；不能换幂等键再次提交。`signed` 或 `unknown` 即使暂未查到链上结果也不会自动重发。

调用交易接口时必须带：

```bash
Authorization: Bearer $LOCAL_TRADE_API_TOKEN
```

服务启动会优先读取当前目录或其上级目录的 `.env`；如果没有找到，还会从可执行文件所在目录往上查找 `.env`。因此推荐从项目目录启动：

```bash
cd /root/http/sol-bot/trade_service
./target/release/pump-trade-service
```

curl 所在 shell 也需要有同一个 token。可以临时从 `.env` 加载：

```bash
cd /root/http/sol-bot/trade_service
set -a
source .env
set +a
```

如果返回 `{"error":"unauthorized"}`，优先检查：

- curl 命令是否带了 `-H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN"`。
- 当前 shell 是否真的设置了 `LOCAL_TRADE_API_TOKEN`。
- 服务是否已重启并读取了最新 `.env`。

建议生成一个随机字符串，例如：

```bash
openssl rand -hex 32
```

`.env` 里这样填：

```env
LOCAL_TRADE_API_TOKEN=生成出来的随机字符串
```

`TRADE_HTTP_ADDR`

HTTP 监听地址。默认必须是 loopback，例如：

```env
TRADE_HTTP_ADDR=127.0.0.1:18787
```

默认使用 loopback。配置非 loopback 地址（例如 `0.0.0.0:18787`）时，必须同时配置非空的 `TRADE_ALLOWED_CLIENT_IPS`，否则服务拒绝启动。公网部署还应使用防火墙和 TLS/加密隧道。

`DEFAULT_BUY_SOL`

buy 请求没有传 `amount_sol` 时使用的默认买入金额，单位 SOL。比如 `0.001` 表示买入 0.001 SOL。

`DEFAULT_SLIPPAGE_PERCENT`

buy/sell 请求没有传 `slippage_percent` 时使用的默认滑点，单位百分比。`15` 表示 15%，服务内部会转换成 SDK 需要的 `1500 bps`。

`DEFAULT_TIP_SOL`

默认小费，单位 SOL。SDK 路径会把它传给交易 gas fee strategy，可供 Jito/SWQoS lane 使用。

当请求使用 `execution_method:"pumpapi"` 时，单笔 `tip_sol` 会转换成 PumpAPI 的 `priorityFee`；如果单笔没有传 `tip_sol`，使用 `PUMPAPI_DEFAULT_PRIORITY_FEE_SOL`。

`STONKFUN_VIA_SOL_ENABLED`

是否允许 StonkFun 自定义 Quote 使用原子两跳 `SOL -> Quote -> Meme`（卖出反向执行）。默认 `false`。开启后请求仍必须显式传 `settlement_asset:"sol"`、`sol_hop_pool:"raydium-cpmm"` 和经过确认的 `sol_hop_pool_id`；服务会从链上校验第二跳池同时包含 WSOL 与目标 Quote。

受保护的 `POST /v1/pools/sol-hop` 接受 `{"quote_mint":"..."}`，在此开关开启时查询 Raydium CPMM 的两个 mint 槽位，返回链上验证过的 `WSOL/Quote` 池；无可用池返回 `available:false`。TG 快速交易只在找到该池时提供 SOL 两跳选项，提交前会重新查询。此接口不是成交报价，不能据此展示预计到手数量或价格影响；当前不自动搜索 AMM v4 或三跳路径。

`STONKFUN_VIA_SOL_MAX_BUY_SOL`

Via-SOL 单笔买入上限，单位 SOL，默认 `0.1`。该限制只收紧 Via-SOL，不改变其他交易路径。

自定义 Quote 直接交易时，买入请求传 `amount_quote_units`（Quote 最小单位），钱包须预先持有该 Quote。受内部鉴权保护的 `GET /internal/wallets/:wallet_id/quote-balance/:mint?owner_type=copy_trading&owner_id=<用户ID>` 返回链上 Mint 精度、Quote 最小单位余额及 SOL lamports 余额；钱包必须属于该用户或为可见的 system 钱包。该接口供 Telegram 面板显示余额和精确换算金额使用。

普通 SOL 买入不要求用户持有池子的 Quote。买入请求使用 `amount_sol` 且链上 Quote 不是 WSOL 时，trade_service 会自动走 SOL 聚合器：重新报价、审计、模拟，通过后再签名广播。Telegram 面板只显示 SOL 金额；`amount_quote_units` 仍保留给明确要求 Quote 直买的兼容调用方。

`ENABLE_SIMULATE`

是否只模拟交易。

- `true`：先用于测试，不发真实交易。
- `false`：真实提交交易。

建议第一轮测试先设为：

```env
ENABLE_SIMULATE=true
```

`WAIT_TX_CONFIRMED`

是否等待交易确认。默认 `false`，拿到提交结果后尽快返回。

`TRADE_RESPONSE_WAIT_MS`

创建交易接口等待后台执行结果的时间，单位毫秒。默认 `3000`。在这个窗口内如果交易执行已返回模拟成功、提交成功、失败或 unknown，`POST /v1/trades` 会直接带着 `signature/error/status` 返回；超时才返回 `accepted`，再用查询接口查看后续状态。

`TRADE_TASK_DRAIN_TIMEOUT_MS`

普通后台交易任务的优雅退出等待预算，单位毫秒，默认 `30000`，范围 `1` 到 `300000`。
收到退出信号后停止登记新的普通交易工作；已登记请求仍可派发，交易任务等待结束前保留原 blockhash 刷新和观测资源。
预算从 HTTP graceful shutdown 返回后开始，不是整个进程或 SDK 后台 worker 的总截止时间。
超时、任务 panic 或异常丢弃会记录未完成情况并使退出失败，不代表交易未发送，不释放或重建原幂等/签名权威，也不自动重签补单。
停止接单期间普通交易 POST 返回既有 `503` 错误形状；GET 查询不受该登记闸门影响。正常运行的请求字段、校验与响应等待规则不变。
停机 503 仅说明本次请求没有进入接受流程，不能证明同 key 的先前请求未接受或未发送；恢复后仍使用原 key 查询/重复读取，不自动换新 key 补单。

`TRADE_SHADOW_DRAIN_TIMEOUT_MS`

Shadow 镜像队列的独立退出等待预算，单位毫秒，默认 `5000`，范围 `1` 到 `300000`。
普通任务排空及原刷新/观测任务停止后，关闭原 1024 项队列的新写入，等待同一个 writer 完成已接受的 FIFO 写入。
超时或取消等待不丢弃原 JoinHandle、不重建队列或重放写入；历史入队拒绝、写失败或 writer 异常都不能报告为完整排空。
Memory/Durable 模式下该步骤不创建队列、不关闭数据库，也不改变幂等/签名权威。
此预算不是全服务停机总截止时间；成功只说明原队列已排空及旧 SQL 调用成功，不将 Shadow 升级为 durable、`synchronous=FULL` 或断电安全保证。未修改线上配置或服务。

`MEV_SUBMIT_VERIFY_MS`

MEV/SWQoS 提交后查询原签名是否可见的预算，单位毫秒，默认 `1200`。查不到或查询异常不是未发送证明，保留原签名和 `unknown`，不自动换 lane/重签/补单。`0` 关闭 legacy 可见性检查；新增源码 C3c1 的 Durable Live DAMM 卖出在 provider trait 没有可核验回执且预算为 0 时仍为 `unknown`，Default 匹配的 typed RPC 回执不受此限制。新子路径确认模式由共享只读观测执行，详见 [发送与预算](docs/optimization/SDK_ORIGINAL_DAMM_RUNTIME.md)，尚未部署。

`ENABLE_PUMPAPI_EXECUTOR`

是否允许请求使用 `execution_method:"pumpapi"`。默认 `false`。

第一版只接入 PumpAPI Local Transaction：

- 服务把交易参数发给 PumpAPI。
- PumpAPI 返回 unsigned transaction。
- 服务使用本地 `.env` 里的钱包私钥签名。
- `simulate:true` 时只模拟，不提交。
- `simulate:false` 时通过本服务配置的 Default RPC 提交。

启用示例：

```env
ENABLE_PUMPAPI_EXECUTOR=true
PUMPAPI_URL=https://api.pumpapi.io
PUMPAPI_TIMEOUT_MS=1200
PUMPAPI_DEFAULT_PRIORITY_FEE_SOL=0.0001
PUMPAPI_LIGHTNING_ENABLED=false
```

`PUMPAPI_LIGHTNING_ENABLED`

预留开关，当前必须保持 `false`。本服务不支持也不会调用需要 `privateKey` 的 PumpAPI Lightning 交易模式。

`BLOCKHASH_REFRESH_MS`

blockhash 刷新间隔，默认 `400` 毫秒。优化源码将小于 50 ms 的值按 50 ms 等待，以免所有节点快速拒绝时形成忙循环；共享层关键请求起点至少间隔 100 ms。默认值没有改变，该源码尚未部署。

`BLOCKHASH_MAX_AGE_MS`

blockhash 最大可用年龄。超过这个时间，交易请求会被拒绝，避免使用过期 blockhash。

## MEV / SWQoS 配置

当前已支持通过 `sol-trade-sdk 6.0.2` 接入以下 SWQoS lane：

- Jito：高性能 block space。
- Temporal：时间敏感交易。
- FlashBlock：需要 API key 的高速交易执行。
- BlockRazor：需要 API key 的高速交易执行。
- Astralane：Binary/Plain HTTP 和 QUIC 网络加速。
- SpeedLanding：需要 API key 的高速交易执行。

Default RPC fallback 会一直保留。即使启用了 MEV/SWQoS provider，服务仍会保留普通 RPC 提交通道作为兜底。

发送策略：

- 每笔交易优先使用第一个启用的 MEV/SWQoS lane。
- 如果该 lane 明确失败，再回退到普通 RPC lane。
- 不会在同一笔请求里同时向多个 MEV provider 盲目并发提交，避免 SDK 报 `Multiple swqos transactions require durable_nonce to be set`。
- 默认 `WAIT_TX_CONFIRMED=false` 时只等待提交结果，接口可能先返回 `submitted`，再用交易签名到链上查询确认。

### 总开关

```env
ENABLE_MEV_PROTECTION=true
```

这个开关会传给 SDK 的：

```rust
TradeConfig::builder(...).mev_protection(true)
```

如果只打开这个开关但没有启用任何 provider，实际仍主要走 Default RPC fallback。

### Jito

```env
ENABLE_JITO=true
JITO_UUID=
JITO_REGION=frankfurt
JITO_URL=
```

Jito 可以无 UUID 使用公共 Block Engine endpoint；留空时 SDK 不会带 `uuid` query，也不会发送 `x-jito-auth` header。无 UUID 通常会受更严格限速/优先级影响。有 UUID 时填 `JITO_UUID=你的jito uuid`；也可以用 `JITO_AUTH_TOKEN` 或 `JITO_API_KEY`，服务会按 `AUTH_TOKEN/API_KEY/UUID` 顺序读取。

### Temporal

```env
ENABLE_TEMPORAL=true
TEMPORAL_API_KEY=你的temporal api key
TEMPORAL_REGION=frankfurt
TEMPORAL_URL=
```

### FlashBlock

```env
ENABLE_FLASHBLOCK=true
FLASHBLOCK_API_KEY=你的flashblock api key
FLASHBLOCK_REGION=frankfurt
FLASHBLOCK_URL=
```

### BlockRazor

```env
ENABLE_BLOCKRAZOR=true
BLOCKRAZOR_API_KEY=你的blockrazor api key
BLOCKRAZOR_REGION=frankfurt
BLOCKRAZOR_URL=
BLOCKRAZOR_TRANSPORT=grpc
```

`BLOCKRAZOR_TRANSPORT` 支持：

- `grpc`
- `http`

### Astralane

```env
ENABLE_ASTRALANE=true
ASTRALANE_API_KEY=你的astralane api key
ASTRALANE_REGION=frankfurt
ASTRALANE_URL=
ASTRALANE_TRANSPORT=binary
```

`ASTRALANE_TRANSPORT` 支持：

- `binary`
- `plain`
- `quic`

如果日志出现 `SWQOS Astralane 初始化失败: Invalid server address`，或 VPS/机房 UDP QUIC 不稳定，优先改用：

```env
ASTRALANE_TRANSPORT=binary
```

### SpeedLanding

```env
ENABLE_SPEEDLANDING=true
SPEEDLANDING_API_KEY=你的speedlanding api key
SPEEDLANDING_REGION=frankfurt
SPEEDLANDING_URL=
```

### Region

`*_REGION` 支持：

- `default`
- `newyork`
- `frankfurt`
- `amsterdam`
- `dublin`
- `slc`
- `tokyo`
- `singapore`
- `london`
- `losangeles`

也支持常见简写，例如 `nyc`、`fra`、`ams`、`sg`、`lax`。

### 小费

小费用 SOL 单位配置：

```env
DEFAULT_TIP_SOL=0.001
```

单笔交易也可以传：

```json
"tip_sol": 0.001
```

对于 Jito / SpeedLanding / Lightspeed 等有最低 tip 要求的 lane，需要按 provider 要求设置足够小费。当前本服务支持 Jito、Temporal、FlashBlock、BlockRazor、Astralane、SpeedLanding。

每笔交易可以覆盖 MEV 策略：

```json
"use_mev": true,
"mev_provider": "astralane"
```

- 不传 `use_mev` 和 `mev_provider`：使用 `.env` 默认策略。
- `use_mev:true`：只走启用的 MEV/SWQoS lane，失败就失败。
- `use_mev:false`：只走普通 Default RPC。
- `mev_provider:"astralane"`：这一笔指定 Astralane；也支持 `jito`、`temporal`、`flashblock`、`blockrazor`、`speedlanding`。
- `mev_provider:"default"` 或 `"rpc"`：这一笔走普通 RPC。
- `mev_provider:"auto"`：优先用启用的 MEV lane，没有 MEV lane 时走 Default RPC。
- `simulate:true`：这一笔只模拟，不真实提交；适合测试新池子和新路由。

MEV 测试建议先使用一个 provider，例如：

```env
ENABLE_MEV_PROTECTION=true
ENABLE_ASTRALANE=true
ASTRALANE_REGION=frankfurt
ASTRALANE_TRANSPORT=quic
DEFAULT_TIP_SOL=0.001
WAIT_TX_CONFIRMED=false
ENABLE_SIMULATE=false
```

## 交易方式

请求字段 `execution_method` 可选：

- 不传或传 `"sdk"`：走 `sol-trade-sdk`，支持本服务现有的 MEV/SWQoS lane。
- 传 `"pumpapi"`：走 PumpAPI Local Transaction，由 PumpAPI 构造 unsigned transaction，本服务本地签名。

PumpAPI 第一版边界：

- 不支持 `use_mev:true`，也不走 Astralane/Jito 等 SWQoS lane。
- `simulate:false` 时通过 Default RPC 提交。
- 买入支持 WSOL/USDC/USD1 及自定义 Quote。WSOL 可直接使用 SDK；非 WSOL Quote 使用 `amount_sol` 时自动走 SOL 聚合器，明确传 `amount_quote_units` 时仍走 Quote 直买。
- 卖出第一版只支持 `sell_percent:100`。
- 如果 PumpAPI 返回多笔 transaction bundle，第一版会拒绝，避免半提交。
- 不会发送私钥给 PumpAPI。

PumpAPI Local Transaction 测试建议先开模拟：

```env
ENABLE_PUMPAPI_EXECUTOR=true
ENABLE_SIMULATE=true
```

PumpAPI PumpSwap 买入示例：

```json
{
  "execution_method": "pumpapi",
  "side": "buy",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.0001,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-pumpapi-buy-001"
}
```

PumpAPI Meteora USDC 买入示例：

```json
{
  "execution_method": "pumpapi",
  "side": "buy",
  "pool": "meteora-damm-v2",
  "mint": "TOKEN_MINT",
  "pool_id": "METEORA_DAMM_V2_POOL_ADDRESS",
  "quote_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
  "amount_usdc": 1,
  "slippage_percent": 15,
  "tip_sol": 0.0001,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-pumpapi-meteora-usdc-001"
}
```

PumpAPI 全仓卖出示例：

```json
{
  "execution_method": "pumpapi",
  "side": "sell",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "sell_percent": 100,
  "slippage_percent": 15,
  "tip_sol": 0.0001,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-pumpapi-sell-001"
}
```

PumpAPI curl 模拟买入：

```bash
cd /root/http/sol-bot/trade_service
set -a
source .env
set +a

API=http://127.0.0.1:18787
QUOTE=So11111111111111111111111111111111111111112
MINT=TOKEN_MINT
POOL_ID=PUMPSWAP_POOL_ADDRESS

curl -sS -X POST "$API/v1/trades" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"execution_method\":\"pumpapi\",
    \"side\":\"buy\",
    \"pool\":\"pump-amm\",
    \"mint\":\"$MINT\",
    \"pool_id\":\"$POOL_ID\",
    \"quote_mint\":\"$QUOTE\",
    \"amount_sol\":0.001,
    \"slippage_percent\":15,
    \"tip_sol\":0.0001,
    \"use_mev\":false,
    \"simulate\":true,
    \"idempotency_key\":\"pumpapi-sim-buy-$(date +%s)\"
  }"
echo
```

## 编译

```bash
cd /root/http/sol-bot/trade_service
cargo check
cargo build --release
```

## 启动

```bash
cd /root/http/sol-bot/trade_service
cargo run --release
```

## 健康检查

```bash
curl http://127.0.0.1:18787/health
```

预期：

```json
{"ok":true}
```

## 就绪检查

```bash
curl http://127.0.0.1:18787/ready
```

返回示例：

```json
{
  "ready": true,
  "wallet": "...",
  "default_wallet_id": "main",
  "wallet_count": 2,
  "wallets": [
    {
      "wallet_id": "main",
      "pubkey": "..."
    },
    {
      "wallet_id": "wallet_2",
      "pubkey": "..."
    }
  ],
  "has_blockhash": true,
  "blockhash_age_ms": 123,
  "rpc_ok": true,
  "paused": false,
  "rpc_lanes": 3,
  "swqos_lanes": ["Default"],
  "active_swqos_lanes": ["Default"],
  "mev_protection": false,
  "require_mev_submit": false,
  "pumpapi_executor": false
}
```

默认只会看到 `["Default"]`，表示当前只启用了普通 RPC fallback。启用 Jito/Temporal/FlashBlock/BlockRazor/Astralane/SpeedLanding 并重启服务后，`swqos_lanes` 才会出现对应名称。

如果 `.env` 设置了 `REQUIRE_MEV_SUBMIT=true`，实际下单只会使用 `active_swqos_lanes` 里的 MEV/SWQoS 通道；`swqos_lanes` 里即使仍显示 `Default`，也只是配置清单，不代表下单会走普通 RPC fallback。默认建议只开启 Astralane，其他 provider 有最低 tip 要求时先保持关闭。

`MEV_USE_BACKUP_RPCS=false` 时，MEV/SWQoS 发单只用主 `RPC_URL` 构造一条执行 lane，备份 RPC 仍保留给 blockhash/RPC 能力；这样能避免某条备份 RPC 返回 403 时干扰 Astralane 排障。设为 `true` 后，每个已启用的 MEV Provider 都会基于 `RPC_URL`、`BACKUP_RPC_URLS` 依次建立执行 lane；主 RPC 在读取 Pool、Mint 或构建交易时遇到 429、超时等错误，会使用相同 Provider 和下一条 RPC 重试。生产环境应确保所有备用 RPC 都允许当前 SDK 使用的 `getProgramAccounts`、`getMultipleAccounts` 和交易模拟请求。

```json
{
  "swqos_lanes": ["Astralane", "Default"],
  "active_swqos_lanes": ["Astralane"],
  "mev_protection": true,
  "require_mev_submit": true
}
```

`/ready` 只显示 lane 名称，不显示 UUID、API key、自定义 URL 等敏感参数。

## 钱包列表

```bash
curl -sS http://127.0.0.1:18787/v1/wallets \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN"
echo
```

返回示例：

```json
{
  "default_wallet_id": "main",
  "wallets": [
    {
      "wallet_id": "main",
      "pubkey": "..."
    }
  ]
}
```

## 最近交易

```bash
curl -sS "http://127.0.0.1:18787/v1/trades?limit=20" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN"
echo
```

`POST /v1/trades` 默认会短暂等待交易执行返回结果。如果仍返回 `accepted`，用上面的最近交易接口，或按 request_id 查询：

```bash
curl -sS http://127.0.0.1:18787/v1/trades/你的request_id
echo
```

## 预热参数

PumpFun:

```bash
export LOCAL_TRADE_API_TOKEN=你的token
export MINT=真实TOKEN_MINT

curl -X POST http://127.0.0.1:18787/v1/cache/warmup \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"pool\":\"pump\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\"
  }"
```

PumpSwap:

```bash
curl -X POST http://127.0.0.1:18787/v1/cache/warmup \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"pool\":\"pump-amm\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\"
  }"
```

如果有 PumpSwap pool 地址，优先传 `pool_id`。

## 交易 JSON 字段

`execution_method` 可选：

- `"sdk"`：默认值，使用 `sol-trade-sdk`。
- `"pumpapi"`：使用 PumpAPI Local Transaction。

PumpFun 买入最小示例：

```json
{
  "side": "buy",
  "pool": "pump",
  "mint": "TOKEN_MINT",
  "pool_id": null,
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "use_mev": true,
  "mev_provider": "astralane",
  "idempotency_key": "manual-buy-astralane-001"
}
```

PumpSwap 买入建议传池地址：

```json
{
  "side": "buy",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "use_mev": true,
  "mev_provider": "astralane",
  "idempotency_key": "manual-buy-pumpswap-001"
}
```

同一个接口也可以临时关闭 MEV，走普通 RPC：

```json
{
  "side": "buy",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-buy-rpc-001"
}
```

`pool_id` 可以为 `null`，服务会尝试通过 SDK RPC helper 从 mint 查池。但如果策略进程已经从 PumpAPI 事件拿到了 pool 地址，优先传 `pool_id`，可以减少查找失败和额外 RPC。

`quote_mint` 必须按真实池子填写，优先使用 PumpAPI 事件里的 `quote_mint`：

- WSOL/SOL 池：`So11111111111111111111111111111111111111112`，买入金额用 `amount_sol` 或 `amount_lamports`。
- USDC 池：`EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`，买入金额用 `amount_quote`、`amount_usdc` 或 `amount_quote_units`。
- USD1 池：`USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB`，当前只支持 `bonk`，买入金额用 `amount_quote` 或 `amount_quote_units`。

`amount_quote_units` 是 quote token 的最小单位。USDC/USD1 是 6 位小数，所以 `1 USDC = 1000000`。

Bonk 买入：

```json
{
  "side": "buy",
  "pool": "bonk",
  "mint": "TOKEN_MINT",
  "pool_id": null,
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-bonk-001"
}
```

Meteora DAMM v2 买入：

```json
{
  "side": "buy",
  "pool": "meteora-damm-v2",
  "mint": "TOKEN_MINT",
  "pool_id": "METEORA_DAMM_V2_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-meteora-001"
}
```

Meteora DAMM v2 的部分池子启用了 Rate Limiter，swap 需要携带 `SYSVAR_INSTRUCTIONS`。服务会自动为 `meteora-damm-v2` 请求补这个账号。

Meteora DAMM v2 的 USDC 池买入示例：

```json
{
  "side": "buy",
  "pool": "meteora-damm-v2",
  "mint": "TOKEN_MINT",
  "pool_id": "METEORA_DAMM_V2_POOL_ADDRESS",
  "quote_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
  "amount_usdc": 1,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "simulate": true,
  "idempotency_key": "manual-buy-meteora-usdc-001"
}
```

Raydium AMM v4 买入：

```json
{
  "side": "buy",
  "pool": "raydium-amm-v4",
  "mint": "TOKEN_MINT",
  "pool_id": "RAYDIUM_AMM_V4_AMM_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-raydium-amm-v4-001"
}
```

Raydium CPMM 买入：

```json
{
  "side": "buy",
  "pool": "raydium-cpmm",
  "mint": "TOKEN_MINT",
  "pool_id": "RAYDIUM_CPMM_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-raydium-cpmm-001"
}
```

SDK 第一版池地址规则：

- `pump`：`pool_id` 必须为 `null`。
- `pump-amm`：`pool_id` 可选；有池地址时建议传。
- `bonk`：`pool_id` 必须为 `null`，服务用 `mint` 补参数。
- `stonkfun`：`pool_id` 必填，服务校验 StonkFun 配置、base mint 与 quote mint。
- `stonkfun-swap`：`pool_id` 必填，表示 StonkFun 迁移后的 Raydium CPMM 池。
- `meteora-damm-v2`：`pool_id` 必填。
- `meteora-dbc`：Meteora Dynamic Bonding Curve，`pool_id` 必填；当前支持未迁移池的 `swap2` ExactIn，报价币支持 WSOL、USDC。
- `raydium-amm-v4`：`pool_id` 必填，表示 AMM address。
- `raydium-cpmm`：`pool_id` 必填。

PumpAPI 交易方式不会在本服务校验层强制 `pool_id`，但如果策略进程已经拿到池地址，仍建议传 `pool_id`，可以减少 PumpAPI 选路歧义。

## 买入测试

建议先确认 `.env`：

```env
ENABLE_SIMULATE=true
```

然后发请求：

```bash
curl -X POST http://127.0.0.1:18787/v1/trades \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"side\":\"buy\",
    \"wallet_id\":\"main\",
    \"pool\":\"pump\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\",
    \"amount_sol\":0.001,
    \"slippage_percent\":15,
    \"tip_sol\":0.001,
    \"source\":\"manual\",
    \"idempotency_key\":\"manual-buy-test-001\"
  }"
```

返回会包含 `request_id`。

不传 `wallet_id` 时使用 `DEFAULT_WALLET_ID`。传 `wallet_id` 时，服务会使用对应钱包交易。

## 多钱包批量买入

```bash
curl -X POST http://127.0.0.1:18787/v1/batch/trades \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"wallet_ids\":[\"main\",\"wallet_2\"],
    \"trade\":{
      \"side\":\"buy\",
      \"pool\":\"pump\",
      \"mint\":\"$MINT\",
      \"pool_id\":null,
      \"quote_mint\":\"So11111111111111111111111111111111111111112\",
      \"amount_sol\":0.001,
      \"slippage_percent\":15,
      \"tip_sol\":0.001,
      \"source\":\"manual-batch\",
      \"idempotency_key\":\"manual-batch-buy-001\"
    }
  }"
```

批量交易会自动把幂等作用域变成：

```text
(wallet_id, idempotency_key)
```

所以同一个策略事件可以让多个钱包各买一次，但同一个钱包不会重复提交。

## 查询交易状态

```bash
curl http://127.0.0.1:18787/v1/trades/tr_xxx
```

把 `tr_xxx` 替换为买入请求返回的 `request_id`。

状态可能是：

- `accepted`
- `building_tx`
- `submitting`
- `simulated`
- `submitted`
- `confirmed`
- `failed`
- `rejected`
- `unknown`

## 幂等测试

重复发送相同的 `idempotency_key`：

```json
"idempotency_key":"manual-buy-test-001"
```

预期返回：

```json
"duplicate": true
```

并且不会再次提交交易。

## 卖出测试

```bash
curl -X POST http://127.0.0.1:18787/v1/trades \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"side\":\"sell\",
    \"wallet_id\":\"main\",
    \"pool\":\"pump\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\",
    \"sell_percent\":100,
    \"slippage_percent\":15,
    \"tip_sol\":0.001,
    \"source\":\"manual\",
    \"idempotency_key\":\"manual-sell-test-001\"
  }"
```

`sell_percent` 范围是 `1..=100`。

## 基础校验

第一版不做最大买入金额风控，只保留执行保护：

- 修改类接口必须带 Bearer token。
- 不允许请求体传私钥。
- `execution_method` 支持 `sdk` 和 `pumpapi`，默认 `sdk`。
- `pool` 支持 `pump`、`pump-amm`、`bonk`、`stonkfun`、`stonkfun-swap`、`meteora-damm-v2`、`meteora-dbc`、`raydium-amm-v4`、`raydium-cpmm`。
- Meteora/Raydium 第一版必须传 `pool_id`，不自动按 mint 选池。
- `quote_mint` 会按链上真实池记录校验；WSOL、USDC、USD1 和自定义 SPL Quote 均可通过 SOL 聚合器买入。
- 买入统一建议使用 `amount_sol`。WSOL 直池由 SDK 执行；非 WSOL Quote 由聚合器从 SOL 兑换到目标 Mint。只有明确需要池内 Quote 直买时才传 `amount_quote_units`，此时钱包必须持有该 Quote。
- sell 的 `sell_percent` 必须是 `1..=100`。
- `execution_method:"pumpapi"` 第一版不支持 `use_mev:true`，卖出只支持 `sell_percent:100`。
- `slippage_percent` 必须是 `0..=100`。旧字段 `slippage_bps` 仍兼容。
- SDK 交易方式需要 blockhash cache 可用且未过期；PumpAPI 交易方式使用 PumpAPI 返回交易里的 recent blockhash。

## SDK 说明

当前使用：

```toml
sol-trade-sdk = "6.0.2"
```

本地 vendor 含既有 Meteora DBC 扩展，不与官方同版本逐字节相同。固定上游提交、差异清单、
可重放补丁及升级门槛见 [SDK 来源账本](docs/optimization/SDK_PROVENANCE.md)。可先运行
`python3 scripts/check_sdk_provenance.py` 做只读检查；它不联网、不修改 SDK，也不代表协议交易验收。

SDK 参数通过 RPC helper 补齐：

- `PumpFunParams::from_mint_by_rpc`
- `PumpSwapParams::from_pool_address_by_rpc`
- `PumpSwapParams::from_mint_by_rpc`
- `BonkParams::from_mint_by_rpc`
- `MeteoraDammV2Params::from_pool_address_by_rpc`
- `RaydiumAmmV4Params::from_amm_address_by_rpc`
- `RaydiumCpmmParams::from_pool_address_by_rpc`

SWQoS / MEV lane 通过 SDK 的 `SwqosConfig` 构造：

- `SwqosConfig::Jito`
- `SwqosConfig::Temporal`
- `SwqosConfig::FlashBlock`
- `SwqosConfig::BlockRazor`
- `SwqosConfig::Astralane`
- `SwqosConfig::Speedlanding`
- `SwqosConfig::Default`

PumpAPI 交易方式不经过这些 SDK DEX helper；它调用 `PUMPAPI_URL` 获取 unsigned transaction，再由本服务本地签名，并使用 Default RPC 进行模拟或提交。
