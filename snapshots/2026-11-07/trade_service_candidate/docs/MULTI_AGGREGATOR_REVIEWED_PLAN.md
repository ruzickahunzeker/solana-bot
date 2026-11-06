# 多聚合器持续落地计划（审查版）

更新时间：2026-11-06

交易能力的实施优先级以 [SDK 6 与 SDK 11 交易优先的最短落地路径](FAST_TRADING_PRIORITY_REVIEW.md) 为准：
并行推进现有 SDK 6.0.2 单池与 SDK 11.0.1 单池，再推进 SDK 11 原生两跳；SDK 6 固定依赖缺失期间先完成 SDK 11 可验证切片。本文件继续保存多聚合器
完整范围与历史验收证据；其中 Jupiter Gate 2 和 24 小时 Shadow 只约束 Jupiter 路径，
不作为上述 SDK 交易的前置条件。

本文复核现有 Trade Service，以及将备份分支 `copy_trading` 的适用能力选择性合入本机 `bot_trading` 的计划。`copy_trading` 是该 Bot 在备份仓库中的目录名，不是另一个待部署服务；钱包协议中的 `owner_type=copy_trading` 保持兼容。当前只落地 Jupiter；provider-neutral 接口保留未来接入其他聚合器的空间，但不在本阶段实现 DFlow。

## 目标架构与旧项目复用边界（2026-11-06 修订）

以 `backup/sol-bot-20261102` 的 `trade_service` 交易入口、钱包隔离、幂等、持久化、
风控、签名和发送为功能基线；本机 `bot_trading`（包含 Telegram/跟单）与外部 API 只提交交易意图，
不选择未经服务端确认的执行后端，也不持有 Trade Service 签名权。candidate 的 SDK 11、
路线快照和聚合器安全门槛是基线的增量演进，不以重写客户端作为多跳前提。

```text
本机 bot_trading（Telegram / 跟单） / 外部 API
              -> Trade Service：鉴权、钱包绑定、统一订单与风控
              -> RoutePlanner：链上核对真实 Quote Mint、池身份、状态和新鲜度
              -> 候选发现与只读报价：原生 Trade SDK 优先；必要时评估 Router SDK；
                 仅在服务端证明适用 SDK 路线不支持后展示 Jupiter 候选
              -> 固定一个 backend、route、quote、钱包和幂等键
              -> 构建 unsigned transaction -> 指令/账户/费用审计 -> 完整模拟
              -> 仅在对应 live 门禁开放后签名、广播、确认和持久化
```

这里的 Direct SDK 不限于单跳：Trade SDK 11 已有 Pump 官方两/三跳与
`PumpSwapQuoteRoute`、`PumpFunQuoteRoute`、`StonkFunQuoteRoute` 显式多跳构建器。
Router SDK 是要求匹配已部署 Router Program 的可选执行后端；它接收已解析路线与
快照，不承担全市场池发现或最佳报价。Jupiter 才是通用聚合寻路与报价 provider。
真实交易 `phh8UH9FvhMykLXysMZrhvzoGa4zn9F62T9ABdCZMHQgTpP5JwNxysGoaoJzo75bg37vYZzD7SEDCU8tVN7Xxt9`
在一个外部程序调用内连续执行两次 PumpSwap AMM 买入，路径为
`WSOL -> Percolator -> PerpScape`；这证明多跳需求，不证明必须使用 Router SDK，
也不是 Pump.fun USDC 内盘曲线样本。下一步先以该公开池对验证 Trade SDK 原生
PumpSwap 两跳的当前快照、保护报价、构建和 unsigned 模拟，再决定 Router 是否有实际缺口。

候选可以并行只读报价，但只有一个绑定的交易构造进入签名与提交；模拟失败、超时、
`unknown` 或已提交后不得换后端、换幂等键再次发送。缺失可靠池快照或 RPC 失败保持
`unresolved_retryable`，不能伪装成 `sdk_unsupported` 来绕过 SDK 优先策略。

## 当前验收状态（2026-11-06）

2026-11-06 最新隔离 candidate 已增加 Pump.fun V3 USDC Quote 的窄范围保护报价和
Raydium CPMM→Pump.fun 曲线只读指令预检。仅接受已核验的单个零门槛 stable 档位、
零转账费及未毕业曲线；预检即使 unsigned 模拟成功，也不签发 `preview_id`、
`sdk_verified` 或 Jupiter 交易候选。147 项 Rust、20 项 Python 测试与隔离 release
构建通过，产物尚未部署到 18887。仍缺真实非 WSOL Pump 曲线样本和有资金 unsigned
模拟，因此该路线保持 `unresolved_retryable`。Gate 2、交易流程 Shadow、canary 和 live
状态不变。验收记录见
`/root/solana/trade_service/evidence/pumpfun-usdc-preview-20261106/REPORT.md`。
随后按 Pump `BuyV2/SellV2` 指令账户里的 Quote Mint 扫描连续 600 笔主网交易，
未找到 USDC Quote 样本；一笔余额同时出现 USDC 的交易经曲线账户核对，实际 Quote
为 WSOL。扫描不能证明主网不存在 USDC 曲线，真实样本门槛仍未满足。
代码复核后，USDC V3 窄范围报价还显式拒绝 Quote Mint 漂移、尚未验证的自定义
creator fee，以及 quote vault 少于曲线记录的实际 Quote 储备；这些状态继续保持未解析。

P3 的 Jupiter quote-only Shadow 已完成单次连续 24 小时验收。窗口为北京时间
2026-11-05 08:46:53 至 2026-11-06 08:47:47，共 86,454 秒；1,439 条样本
全部 `quoted`，日志与 SQLite 逐条一致，p95 为 36 ms、p99 为 48 ms，最小输出
覆盖率和通过率均为 100%，窗口内 quote/submission 写入为 0。最终校验器返回
`passed=true`。证据归档于
`/root/solana/trade_service/evidence/jupiter-shadow-20261105T004653Z/`，
包含 `run.log`、`trades.sqlite` 与 `acceptance.json`；归档数据库再次只读验收通过。
采样服务已正常退出并关闭开机自启，18887 Trade Service 保留运行。
该窗口使用旧版二进制 `5f95683ba4c3`，仅验收其 Jupiter 报价稳定性。
窗口结束后，已将通过 143 项 Rust 和 20 项 Python 测试的 SDK 11 candidate
`dd1f6235150a` 部署到隔离的 18887；新进程二进制哈希、HTTP smoke、同库重启回读和
审计清单均通过。旧版二进制及三个 staging SQLite 的回滚副本保存在
`/root/solana/trade_service/evidence/staging-upgrade-20261106T005806Z/`。
新部署不改变旧 Shadow 证据的适用范围，也未验收 Gate 2 指令审计或任何交易提交。
后续 candidate `a384d52cafce` 又将只读决策的已解析钱包公钥加入响应，供 bot
核对 staging 与本地钱包是否仍一致；缺失钱包时省略公钥。143 项 Rust、20 项 Python
测试和隔离 PumpSwap HTTP 正负向通过；部署到 18887 后，真实活跃钱包的决策公钥匹配、
缺失钱包无公钥、交易表零新增，两次 smoke、重启回读和进程哈希均通过。备份及证据见
`/root/solana/trade_service/evidence/staging-wallet-binding-20261106T013817Z/`。
P4 的独立 18887 只读决策客户端、配置和手动面板展示已选择性合入
`/root/solana/bot_trading` 源码；本地配置保持交易地址 18788、
`TRADE_SUBMISSION_ENABLED=false`，并为 18887 设置独立决策地址与 token。
配置 guard、格式检查、锁定 `cargo check`、release 构建通过，合入后完整测试
86/86 通过。新二进制与不含 `.env` 的源码归档保存在
`/root/solana/bot_trading_builds/p4-installed-selective-750a97d7f88f/`，
合入前备份保存在
`/root/solana/bot_trading_builds/pre-p4-selective-20261106T015019Z/`。
bot 服务尚未启动，运行级 bot UI 验收尚未完成；Alpha retention 已选择性合入
已安装源码并通过 88 项 Rust 测试、文件级 WAL 清理回归和锁定 release 构建。
原始观察写入上限、paper 信号容量及分批清理详见
`/root/solana/bot_trading/docs/ALPHA_RETENTION.md`；归档在
`/root/solana/bot_trading_builds/alpha-retention-61bdbf2b120d/`。bot 未启动，
生产 SQLite 尚未执行清理。Shadow executor 和远程 migration 仍在隔离副本。
Jupiter Gate 2 与交易提交仍关闭。
后续 candidate `7e9bb07c4a9d` 为单池与 StonkFun Via-SOL 的 `sdk_verified`
决策补充实际 SDK 构建器和链上路线证据；两类响应仍明确禁止 Jupiter 候选。
公开 PumpSwap 单池及省略 Quote Mint 的隔离 HTTP 正向决策、钱包绑定、未授权/
篡改/重启和交易表零写入检查通过。该二进制已部署到 18887，真实钱包只读
PumpSwap 决策返回 `PumpSwapParams / verified_direct_sdk`，前后 trade、aggregator
quote、submission 行数均为 122/0/0。HTTP smoke、SQLite 回读、运行进程哈希、
`current` 与 18788 不变检查通过。升级前的二进制及三个 SQLite 一致性备份、
验收记录位于
`/root/solana/trade_service/evidence/staging-direct-sdk-evidence-20261106T055056Z/`。
bot 服务仍停用，Jupiter Gate 2 仍关闭。

## 范围修订（2026-11-04）

- Jupiter 是当前唯一实际聚合器和报价基线。
- DFlow 不请求真实 endpoint、不实现 response adapter、不进入 Shadow/A-B，也不参与速度比较。
- `ProviderId`、registry、capabilities 和统一 quote envelope 继续保留，供未来聚合器接入。
- 当前开发主线是 SDK 路线决策与 bot 只读迁移；Jupiter 24 小时报价 Shadow 已验收，后续交易流程 Shadow 仍受 Gate 2 约束。

## 运行审查修订（2026-11-05）

candidate 的 HTTP smoke 与 SQLite 回读曾在主机 staging 通过，随后旧手工进程退出，
Shadow 采样从 HTTP 200 变为 000。现已准备独立的 candidate staging unit 与 Shadow unit；
候选服务在 `18887` 由 systemd 保持运行，sampler 等待 `/ready` 后才开始计时，并为每次运行
创建独立日志。旧采样片段不能拼接成连续 24 小时证据。

进入 P3 持续采样前必须重新确认：

1. 主机安装的两个 unit 与仓库版本一致；candidate 的有效环境指向 staging SQLite、
   `127.0.0.1:18887`，并保持 `TRADE_SERVICE_SIMULATE_ONLY=true`、
   `ENABLE_SIMULATE=true`、`CONVERSION_LIVE_ENABLED=false`。
2. candidate `/health`、`/ready`、`scripts/staging_http_smoke.sh` 和重启后的
   `scripts/verify_staging_persistence.py` 通过。
3. sampler 日志在 `/ready` 成功后出现 `shadow_started=true`，之后每分钟记录 HTTP 状态
   与响应结构校验；HTTP 200 仅代表接口可达，Jupiter `status=quoted` 的比例必须从持久化
   Shadow metrics 单独核对。
4. 24 小时窗口从同一份运行日志的 `shadow_started=true` 开始，到
   `shadow_complete=true` 结束。复核实际时长、采样间隔、非 200/结构错误、Jupiter
   失败率和延迟，再决定是否通过。candidate 或 sampler 重启产生的新日志是新运行，不能
   自动合并旧窗口。任何异常先调查，不能用单个 HTTP 200 宣称 Shadow 完成。

新增 `scripts/verify_jupiter_shadow.py`，只接受同一日志和同一 SQLite 时间窗内的完整
证据：至少 24 小时、连续采样、日志与 metrics 数量/状态一致、无 quote/submission 写入、
无未完成 reservation，并输出 p50/p95/p99。上述窗口已出现 `shadow_complete=true`，
P3 的 Jupiter quote-only Shadow 检查点据此通过。
最终验收现在逐行拒绝格式损坏的 HTTP 样本和重复的完成记录；未结束窗口仍返回失败。

运行途中使用 `scripts/audit_jupiter_shadow_progress.py` 对当前日志快照和只读 SQLite
逐条核对，提前发现重复指标、缺样或状态漂移；它只返回 `passed_so_far`，不签发 24 小时
验收。2026-11-05 06:15 UTC 的检查覆盖 329 条样本及 329 条指标，全部 `quoted`，最大
采样间隔 61 秒，p95 为 33 ms，quote/submission 均为 0。

2026-11-05 08:46（北京时间）发现并停止了遗留的手工 `timeout 24h` sampler；它与
systemd sampler 并行写入，导致旧日志每分钟对应两条 metrics，旧窗口作废。已只重启
`jupiter-shadow-staging.service`，新的干净窗口从 `2026-11-05T00:46:53Z` 开始；之后
只允许 systemd sampler 写入，不能再用手工循环补采样。

24 小时是稳定性观察门槛，不阻止 P4 的只读迁移和其它 simulate-only 开发；它只阻止把
P3 标为完成及进入 canary/live。Gate 2 仍关闭，当前不要求可执行的 Jupiter preflight、
build 或 submit 来启动只读 Shadow。

## Router SDK 归类（2026-11-04）

**选型决定（2026-11-05）：继续以 `sol-trade-sdk 11.0.1` 为交易构建基线。**
`sol-trade-router-sdk 0.5.0` 精确依赖同版 Trade SDK，并额外要求部署、初始化匹配的
Router Program；它不是替代 Trade SDK 的必要升级。先把 Trade SDK 已有的官方 Pump
两/三跳、PumpSwap 和 StonkFun 显式 quote 路线逐项接入并模拟验证。只有某条已解析路线
确实无法由原生 builder 构建，且 Router 的对应 leg 可用时，才在隔离环境评估 Router
作为可选执行后端。不能仅凭跳数、当前服务尚未接入 builder 或 Jupiter 能报价就选 Router。
两者热路径都可以复用缓存、避免 RPC；“Router 更快”尚无同条件数据支持。比较时记录
快照准备、报价、构建、模拟的 p50/p95，以及交易大小、compute units 和实际费用。
Router 评估可与 Jupiter 只读 Shadow 并行，不需要等待其 24 小时窗口结束。

`sol-trade-router-sdk` 是自部署 Pinocchio Router Program 的 client/execution SDK，负责把
调用方已经解析的 DEX market/route 组织成多跳 CPI；它不是 Jupiter/DFlow 这类报价聚合器，
也不提供独立的全市场最佳报价服务。

在 Trade Service 中应归类为可选 `router_backend`：

- 不加入 `AggregatorRegistry` 的 provider 列表；
- 不参与 Jupiter/DFlow quote compare 或 provider fallback；
- 先只做 route replay、observe 和 simulate，必须使用服务端验证过的 route/quote；
- Router Program 未完成匹配部署、初始化、账户审计和模拟验收前，`build`、`submit` 和 live
  capability 保持关闭；
- 即使未来接入，也必须复用现有 wallet binding、幂等、preflight、audit 和 simulate-only
  guard，不能由 SDK 直接触碰钱包或广播。

## 范围与固定边界

- 常规 candidate unit 继续使用 SDK 11.0.1、`127.0.0.1:18787` 和
  `TRADE_SERVICE_SIMULATE_ONLY=true`；当前 Shadow 验收只使用独立的 staging unit、
  `127.0.0.1:18887` 和 staging SQLite，不能把两个端口当作同一个入口。
- 不切换 `current`，不改变 `127.0.0.1:18788` SSH 隧道，不广播真实交易，不覆盖生产钱包、私钥或 SQLite。
- 远程 `backup/sol-bot-20261102` 只作为选择性迁移来源，不能覆盖 SDK 11 candidate，也不能直接安装它的 systemd unit。
- native SDK 是已验证协议的低延迟优先路径；聚合器是独立 backend。provider 在服务端选择，bot 不直接调用 Jupiter 或 DFlow。

## 当前复核证据（2026-11-05）

- candidate P0/P1 门槛已在兼容 Cargo 1.99.0 下重新通过：`fmt --check`、`check --locked`、
  93 项 Rust 测试、release build、simulate-only runtime guard、审计清单和 18788
  listener invariant 均通过；`current` 仍指向 SDK 6.0.2 legacy。
- bot 的隔离 P4 candidate 已完成 `fmt --check`、`check --locked`、`build --release --locked`
  和 94 项测试；它没有安装到现有 bot unit，也没有改动生产数据库或 18788 隧道。
- staging HTTP smoke、聚合器 schema/restart 回读和 DFlow unavailable read-only 检查已通过。
  当前 Jupiter Shadow sampler 只有一份 systemd 运行实例；连续窗口仍未结束，不能提前验收。
- 2026-11-05 04:17 UTC 之后新 candidate release 已完成构建并通过 gates，但现有 staging
  进程仍运行窗口开始时的旧 candidate inode；为保护连续 Shadow 证据，不能在窗口中途重启。
  新二进制已在独立的 `127.0.0.1:18888` 与 `/tmp/candidate-next-data` 上通过 HTTP smoke、
  DFlow unavailable 检查和 SQLite 回读；临时进程已停止。18887 的持续采样仍按旧二进制
  解释，窗口结束后再切入新二进制并重新做常规 staging 验收。
- 新 candidate 在 schema 5 中持久化 wallet SOL 余额快照；独立 18888 上的 Jupiter read-only
  compare 已返回 `quoted` 和绑定的 `balance_snapshot`，且 Shadow metric 已写入临时 SQLite。
  当前 18887 连续窗口仍运行旧二进制，因此该窗口不能证明新余额快照实现的长期稳定性。
- candidate 的 submission 终态写入已改为带 quote 与原状态条件的原子更新；`unknown` 等
  终态不会被迟到的模拟结果覆盖。该修改只进入后续新二进制验收，不改变当前 Shadow 进程。
- 旧 `/v1/quotes/sol-buy` 与 `/v1/trades` 的 Jupiter 路径现在共用有资金要求的 quote 创建
  及 reservation → 模拟提交 → 终态持久化服务函数；钱包、provider 和幂等键在占位前校验。
- 统一提交入口也在占位前拒绝过期或终态 quote；内部 preflight 在读取余额或调用 Jupiter
  前复核 provider、钱包公钥、过期时间、状态、route/provider quote ID、请求指纹，以及
  持久化报价的金额、滑点、输出上下界和路由连续性；损坏 SQLite 报价重启回读测试通过。
- provider-neutral compare 入口也在接收任何 provider quote 时复核非零输出和请求滑点
  下界，防止后续新增 provider 仅凭 `minimum <= expected` 就被记录为有效报价。
- 新 release `6d6110920fb1ce1071af63381d233575cdf1263696c581e859cacfd929059ab5`
  在隔离 18888 上以临时钱包执行一次 Jupiter read-only compare，返回 `quoted`，
  `expected_output_units=95861`、`minimum_output_units=94903`，满足 100 bps 滑点下界；
  临时进程和凭据已清理。18887 仍是旧进程的连续窗口。
- 2026-11-05 06:04 UTC，最新 release SHA-256
  `a66db791406dc4a09724fd172378fb881bbb4591d48aec63f82f41faba7199d6` 已复制到独立
  `127.0.0.1:18888`，用临时钱包和 `/tmp/candidate-isolated-next/trades.sqlite` 验证。
  两次 HTTP smoke、一次 Jupiter read-only `quoted`、重启后指标和 SQLite 回读均通过；
  quote/submission 表仍为 0 行，临时进程已停止。该检查不替代 18887 的连续 Shadow。
- 新源码已在独立构建目录得到 release SHA-256
  `999bbe8375ce0a26ef8fbe8288ecc619aa4c9081ce7e28955d6b823f38738583`；
  101 项 Rust、20 项 Python 测试通过。临时钱包及独立 SQLite 的 `18889` 运行级 smoke、
  Jupiter `quoted`、DFlow `unavailable`、schema 1–6 与重启回读均通过；quote/submission
  表保持 0 行。证据见 `/root/solana/trade_service/builds/candidate-999bbe8375ce/BUILD_INFO`。
  新构建没有安装到 18887，旧 Shadow 窗口仍以原二进制验收。
- 2026-11-05 新隔离 release `candidate-9a635a67af1f` 增加当前直连 SDK builder 的
  静态可用性分类，并明确区分 SDK 包能力与 Trade Service 已接入能力；118 项 Rust、
  20 项 Python 测试、格式、锁定检查和 release 构建通过。该产物尚未进行运行级核对或
  安装；18887 仍运行旧二进制。11:57 UTC 的只读 Shadow 进度核对覆盖 671/671 条，
  全部 `quoted`，quote/submission 均为 0；连续 24 小时窗口尚未结束。

## 审查结论

| 状态 | 已确认内容 | 处理方式 |
|---|---|---|
| GREEN | 新 route API 的 server-side route snapshot、wallet binding、route/quote TTL、slippage/min-output 校验、candidate simulate-only guard；`/v1/routes/decide` 的文件钱包正负向 HTTP 与重启检查 | 作为基线保留；继续补 SDK 构建/模拟，`sdk_unsupported` 与 Jupiter 候选授权仍待完成 |
| YELLOW | `src/aggregator_quote.rs` 已有 Jupiter quote、unsigned transaction 审计、本地模拟和 durable QuoteStore | 继续补 provider-neutral contract、状态回查和 HTTP 集成证据 |
| GREEN | Jupiter quote-only Shadow 的 24 小时日志与 SQLite 证据已通过最终校验 | 只作为报价稳定性基线，不授权交易提交 |
| RED | Jupiter Gate 2 指令审计仍关闭；其交易流程 Shadow 未完成；DFlow 仍明确 deferred | 在进入 Jupiter live/canary 前逐项验收；SDK 交易按独立门禁验收 |

当前不能用“Jupiter 模拟成功”证明交易可执行，也不能根据现有代码断言 DFlow 比 Jupiter 更快。速度结论只能来自相同请求、相同 RPC 条件和持续采样的 A/B 数据。

## 依赖顺序

```text
SDK 6：现有单池模拟与钱包/幂等回查
       -> 在新 release 修复发送不确定性 -> 受限单池买卖验收
       -> SDK 11 单池模拟/发送对照 -> 受限单池买卖验收
       -> SDK 11 原生两跳快照/报价/构建/模拟 -> 唯一发送与买卖验收

Jupiter：现有只读报价 Shadow -> Gate 2 指令审计
         -> Jupiter 交易流程 Shadow -> Jupiter 专项 canary

Router：仅在原生 SDK 经实证不能构建目标路线后独立评估
```

## 运行验证环境前置门槛（新增）

在 P3 Jupiter Shadow 前，必须准备一个允许本地监听的隔离 staging 环境。该环境只使用
candidate 二进制、candidate `.env`、独立 SQLite 副本和非生产端口；不得启动 `current`、
触碰 `127.0.0.1:18788` 或启用 live/broadcast。

环境验收必须证明：

- candidate 能绑定 staging HTTP 端口并完成 `/health`、`/ready`、capabilities smoke；
- `TRADE_SERVICE_SIMULATE_ONLY=true`、`ENABLE_SIMULATE=true`、`CONVERSION_LIVE_ENABLED=false`；
- Jupiter quote-only compare 与指标回读可运行；Gate 2 前不要求可执行 preflight/build/submit；
- DFlow 返回 disabled/unavailable 且不请求真实 endpoint；
- 进程重启后 quote/submission 状态可回读；
- 端口监听、日志和数据库均与生产入口隔离。

若环境不允许 `TcpListener::bind` 或 netlink 端口检查，验收只能停留在静态/持久化门槛，
不得把运行 smoke 或 24 小时 Shadow 标记为完成。

运行验收使用 `scripts/staging_http_smoke.sh`。脚本只读取已启动的隔离 candidate 实例，
默认访问 `http://127.0.0.1:18887`，检查 health、ready、Jupiter quote-only、DFlow
disabled/read-only 和 Shadow 响应形状；它不会启动进程、构建、签名或提交交易，只会写入
一次 DFlow unavailable Shadow 观测用于验收。
如 staging 使用其他端口，设置 `CANDIDATE_STAGING_URL`；受保护接口通过
`TRADE_SERVICE_LOCAL_API_TOKEN` 提供 Bearer token。
进程重启后另行执行 `CANDIDATE_STAGING_DB=/path/to/staging.sqlite
scripts/verify_staging_persistence.py`；该检查以 SQLite 只读模式打开数据库，验证聚合器
表结构、关键绑定字段和未完成 reservation 的恢复状态，不执行 migration 或写入。
完整执行顺序见 [`STAGING_RUNBOOK.md`](STAGING_RUNBOOK.md)。

任何阶段失败都停留在 simulate/observe；不能通过换 provider 或换幂等键绕过失败。

## 阶段 P0：冻结发布资产和回滚证据

先生成独立审计清单，不依赖 Git（当前 `current`、`candidate`、`bot_trading` 没有 `.git`）：

1. 记录 source tree、Cargo.toml、Cargo.lock、candidate 二进制、vendor SDK 的 SHA-256，以及 Rust/Cargo 版本。
2. 记录 current/previous/candidate 路径、钱包 `wallet_id -> pubkey -> enabled` 映射、非敏感环境配置和 SQLite backup/WAL checkpoint。
3. 恢复可读取 Cargo.lock v4 的 Rust/Cargo 工具链。重新编译 candidate 后，必须让二进制时间和源码快照一致；当前 `target/release/pump-trade-service` 仍是旧构建，不能作为最新候选证明。
4. 只在 staging 生成 candidate unit 的校验结果。`trade-service-local.service` 当前没有 `EnvironmentFile`，且 current 没有可启动二进制；旧 `trade-service-db.service` 和备份分支中的 copy-trading unit 旧路径标记为禁用，不直接启用。

验收：备份可读、钱包公钥无漂移、candidate 可在兼容工具链下 `fmt/check/test/build --locked`，systemd 只完成静态 verify；没有启动 live 进程。

## 阶段 P1：修正聚合器安全语义并统一执行入口

### SDK 优先路由契约（2026-11-05 修订）

**SOL 买入目标 Mint 的决策顺序（2026-11-05 补充）：**服务端先发现目标 Mint 的候选池，
按池 ID、协议、流动性和链上账户核验所选目标池及其实际 Quote Mint；一个 Mint 可能有
多个池，不能仅按 Mint 推断唯一 Quote。SOL/WSOL Quote 的已验证单池走直连 Trade SDK。
USDC 或其它 Quote 先检查
钱包是否已持有并明确使用 Quote 直接买入；SOL 出资时查找并验证 SOL/WSOL→Quote
桥接池、两腿 mint/token program 连续性、池快照、方向、逐跳报价、最终最低到手和
有效期。随后依次检查 `sol-trade-sdk 11.0.1` 的官方多跳及显式 quote route builder、
当前服务已接入的 builder、可选 Router SDK 的匹配 leg 与链上程序状态。Trade SDK
可构建并模拟通过的已验证路线
优先走 Trade SDK；Router 只有在原生 builder 确认无法构建、匹配 Router Program 已部署
并审计通过后才成为候选。若两条 SDK 执行路径都明确不支持且 Jupiter 报价有效，
只展示 Jupiter 重新报价的候选；当前仍是只读。缺少池或可靠快照、RPC 失败及服务尚未接入
SDK builder 均返回 `unresolved_retryable`，不得据此签发 `sdk_unsupported` 或自动
改走 Jupiter。SDK 报价、构建、模拟失败也不触发同一订单的后端切换。

这里的“完整快照”包括两腿的链上池身份、vault/储备/费率、token program 与 Token-2022
扩展、方向、账户列表、同次金额的逐跳输出界限、slot 和 TTL；仅有池 ID 或 Quote Mint
不能授权 Router。若 Trade SDK 官方多跳 builder 已支持并可模拟通过，应先选 Trade SDK，
不能仅因是两跳就选 Router。Router 必须另有匹配的链上 Program 部署与审计证据。
桥接池缺失或快照不可靠时，可独立查询 Jupiter 作只读诊断，但本次路由决策仍为
`unresolved_retryable`；只有完整核验后明确证明 SDK 路径不支持，才能提供需重新报价和
确认的 Jupiter 候选。其它聚合器只保留接口扩展位，本阶段不启用。

现有 `/v1/routes/decide` 只对已提供且已核验的单池和 StonkFun 经 Raydium CPMM
Via-SOL 双跳签发 SDK 决策。非 WSOL Quote 的 SOL 买入已能只读查找并核验
WSOL/Quote Raydium CPMM 桥接池；公开 TTWO/PumpSwap 池的双腿证据通过了隔离 HTTP
检查；隔离 release `candidate-c0b895660784` 已保存进程内短期只读观察快照，并通过
观察 ID 拒绝、重启和零写入 HTTP 检查，但尚未验证对应 Trade SDK 多跳 builder。其它协议的
桥接发现、Router 候选与 Jupiter 候选授权仍未落地。此时只返回
`unresolved_retryable`，不宣称 `sdk_verified`、`sdk_unsupported` 或可执行聚合器交易。
隔离 release `candidate-72b42dd55018` 已为 Raydium CPMM→PumpSwap 增加独立只读 SDK 指令预检：重新读取池参数、
计算逐跳最低到手并调用 `PumpSwapQuoteRoute` builder，返回指令数和读取 slot 跨度。
公开 TTWO fixture 在隔离 HTTP 中构建出 7 条指令，桥接最低到手 438 单位、目标最低
到手 10,579,199 单位，读取跨度 11 slot；不签名、不提交，trade/quote/submission 表
均为 0 行。这仍不等同于完整交易模拟或可执行决策。
继续尝试 unsigned legacy 交易模拟时，该 fixture 的 7 条指令序列化为 1,368 字节，
超过 1,232 字节上限。诊断改用 SDK V1 message 编译器后，当前 RPC 接受了模拟请求；
一次性钱包无资金，模拟返回付款账户扣款失败。此前记录的 1,395 字节由 `bincode`
得出，不是 V1 实际 wire format 的有效大小；源码已改用 `wincode`。隔离复测已确认
该公开两跳路线的 V1 wire 大小为 1,387 字节，
低于 V1 的 4,096 字节上限；该值仍随池账户和指令变化。
这证明 V1 报文能到达当前 RPC 模拟层；后续有资金隔离模拟已证明该池路线本次执行成功。
下一步应将 V1 明确接入这条 SDK 路线并验证实际发送端点支持，或验证 v0/ALT 可用性，确保费用、最低
到手和账户校验不被削弱。该结果不能作为 `sdk_unsupported` 或 Jupiter 回退许可。
上述 V1 只读探针已冻结为隔离 release `candidate-e191726a59b5`；当前 RPC 确认
`attempted=true`、`rpc_accepted=true`，但因无资金 `ok=false`。未安装到 18887。
随后以 staging 有余额钱包的加密数据库临时副本作隔离模拟，同一 V1 路线成功：
目标代币同次模拟增量 10,783,332 单位 ≥ 最低到手 10,675,498；付款账户扣款
4,078,400 lamports，消耗 171,693 CU。三张交易相关表仍为 0 行。仍需验证 V1 发送通道
和服务端决策绑定后才能考虑 SDK 执行授权；当前 `unresolved_retryable` 保持不变。
隔离 release `candidate-90c01ee3255b` 已将同次 RPC 的目标 ATA、前后 token balance
和付款账户扣款纳入模拟报告并验证最低到手；最终复测为 10,704,474 ≥ 10,597,351
目标单位，171,623 CU，三张表零行。仍不代表 V1 发送通道可用。
最新隔离 release `candidate-5ec7df2af568` 改用 `wincode` 测量 V1 实际 wire 大小。
123 项 Rust 测试、锁定 release 构建、无资金路由正负向/重启和有资金只读模拟通过；
本次为 1,387 字节、目标增量 10,764,545 ≥ 最低到手 10,656,899、173,037 CU，
trade/quote/submission 表均为 0。`/v1/routes/decide` 继续返回
`bridge_verified_sdk_capability_unresolved`，不签发可执行 SDK 决策。
新增离线 mock RPC 测试核对 Trade SDK 的 V1 发送编码与 Solana RPC 客户端实际提交参数：
二者均为相同的 `wincode` wire bytes，签名与 V1 尺寸检查也通过。测试未访问真实发送端点；
当前正式 SDK 执行器仍默认 V0，所以该证据只解除本地编码兼容性疑问，不解除
V1 路线接入和端点可用性门槛。
新隔离切片已将成功的 unsigned V1 模拟保存为至多 5 秒的服务端只读预览快照，绑定
观察 ID、钱包、请求指纹、逐跳最低到手和实际目标增量。预览 ID 与 SDK 决策 ID 分离；
回读必须复核钱包及原请求，过期或重启后失效。它为后续决策绑定提供可审计证据，
仍不授权 `/v1/trades` 或 Jupiter 回退。
隔离 release `candidate-dba734bbe4ed` 的 125 项 Rust 测试及有资金/无资金 HTTP
检查通过：成功模拟才签发预览 ID，篡改请求、把 ID 当交易决策使用和重启后的旧 ID
均被拒绝；trade、aggregator quote、submission 表保持零写入。
后续隔离切片已在 `POST /v1/trades` 增加显式 `sdk_preview_id` 请求形态，仅允许
`TRADE_SERVICE_SIMULATE_ONLY=true`、`simulate=true` 的 PumpSwap SOL 买入。
接单前复核服务端观察、钱包、原请求指纹、金额、目标池和短期预览 ID；该
入口要求 durable 幂等存储，重复请求还需匹配原始请求摘要。专用适配器
重新读取池状态并使用 Trade SDK `PumpSwapQuoteRoute` 构建 unsigned V1 模拟，
要求当前最低到手不低于先前绑定值，且目标 token 增量达到当前及绑定最低到手。
失败不会切到 Jupiter。隔离有资金 HTTP 验收生成 1 条 `simulated` 记录、无签名，
aggregator quote/submission 为 0；篡改与新请求重用过期 ID 被拒绝，完全相同的
幂等重试在凭据过期及进程重启后仅回读原记录。126 项 Rust 测试及无资金路由
HTTP 回归通过。`/v1/routes/decide` 对此路线仍是
`unresolved_retryable / bridge_verified_sdk_capability_unresolved`；该 simulate-only
入口不是 live SDK 授权，真实 V1 发送端点、路由决策绑定和 Gate 2 仍未验收。
**2026-11-06 桥接决策增量。** 对已成功 unsigned 模拟的
Raydium CPMM→PumpSwap 观察，客户端可把短期 `sdk_preview_id` 连同原请求提交
`/v1/routes/decide`，取得服务端 `sdk_verified` 的
`sdk-decision:bridge:...`。该决策仅授权原请求在 simulate-only 环境中再次执行
unsigned 模拟；接单和执行前重验钱包、请求指纹、观察、预检及有效期。
隔离有资金钱包数据库副本回归得到 1 条 `simulated` 记录、无签名，aggregator
quote/submission 为 0；精确幂等重试在进程重启后只回读原记录。
原始非 WSOL 请求在没有有效预检时仍为 `unresolved_retryable`。此增量不代表
live 多跳发送能力、Router 接入或 Jupiter 候选授权；18887 未更新。
慢 RPC 可能让五秒观察在 unsigned 模拟期间到期。后续 candidate 在成功模拟后
重新核验同一目标池与 CPMM 桥接池的身份、Mint/Program、钱包、请求指纹和 slot；
只有全部一致才用新的短期观察签发预检，旧观察本身不延长。隔离有资金回归已实际
触发该刷新；执行阶段的最低到手下降仍安全记为 `failed`，不切 Jupiter。
当前 `/v1/routes/decide` 在已核验的 CPMM→PumpSwap 桥接观察中增加
`sdk_capability` 诊断证据：SDK 包构建器为 `PumpSwapQuoteRoute`，服务端适配器为
`simulate_only_bridge_decision`，池证据只覆盖链上身份和币种连续性；
`sdk_unsupported_proven=false`、`jupiter_candidate_allowed=false`。该字段不签发
交易许可，也不代替逐跳报价、交易模拟或实际发送端点验收。
`/v1/capabilities` 的协议概览也将 `sdk_package_route_builder` 与
`service_sol_route_adapter` 分开；未盘点的多跳构建器标为
`not_fully_evaluated`，不会从 `sdk_builder_available` 单一布尔值推导
`sdk_unsupported`。顶层 `jupiter_trade_requires_sdk_unsupported=true` 仅说明
进入聚合器交易的策略门槛，目前没有可用的授权决策。
`observed_bridge` 仅绑定钱包、原请求指纹、两腿快照和最早过期时间；其
`observe:sol-bridge:` ID 不能用于 `sdk_decision_id`，重启后也不保留。
这里的两腿快照只证明池身份和 mint 连续性；构建交易还需各协议当前的 vault、储备、
费率、Token-2022 扩展、账户列表以及按本次金额得出的逐跳最低到手。不能把
`ResolvedRoute` 元数据当成 Router 或 Trade SDK 所需的完整可执行快照。

`amount_sol` 只表示金额单位，不授权选择聚合器。服务端先验证 SDK 路线：已验证的
单池路线和 StonkFun 经 Raydium CPMM 的 SOL 两跳仍由 SDK 执行。只有服务端以已核验的
路线和实际构建能力证明 SDK 无法构建本次路线时，才向客户端提供 Jupiter 报价候选；
当前候选只读、不可提交。跳数本身
不决定后端，未来 SDK 支持的其它多跳也继续优先 SDK。

| 条件 | 路由结果 |
|---|---|
| 已验证的 SDK 单池或 SDK SOL 两跳 | SDK；不得因 `amount_sol` 或非 WSOL quote 改走 Jupiter |
| 非 WSOL Quote 的 SOL 买入，目标池已核验，桥接路线缺失或虽已核验但 builder 未验收 | `unresolved_retryable`；不签发 SDK、Router 或 Jupiter 执行许可 |
| 服务端有明确证据证明本次路线不受 SDK 构建器支持，Jupiter 能报价 | 仅返回需重新确认的 Jupiter 报价候选；Gate 2 前不执行 |
| SDK 报价、构建或模拟失败 | 原路线失败；不在同一请求中自动切换 provider |
| pool、hop、wallet、route 证据不足，或已知协议的发现器没有找到池 | 返回未解析或可重试错误；不把未知路线当作 SDK 不支持 |

`validation.rs` 已移除由“SOL 金额 + 非 WSOL quote”推断聚合器的规则：
`/v1/trades` 默认 SDK；当前显式 `execution_method=aggregator` 在占位前返回 409，
因为服务端尚未签发 `sdk_unsupported` 决策。已验证 SDK Via-SOL hop 不接受聚合器方式。
非 WSOL quote 的 SOL 买入须先走已验证 SDK 路线或保持未解析；显式选择聚合器
不再授权创建交易。
聚合器仍通过显式 quote/preflight contract 进入，未来执行必须绑定服务端 route
snapshot、钱包、金额、过期时间和再次确认。PumpAPI 仍是单独的显式执行方式。
Shadow 可比较 Jupiter 报价，但指标不授权改写执行路线。SDK 优先是当前产品策略；
速度结论需在相同请求与 RPC 条件下实测。

仍待完成：服务端核验“SDK 不支持本次路线”的原因并生成不可由客户端伪造的
route snapshot；当前显式聚合器请求本身不证明 SDK 不支持该池。bot 迁移和
聚合器可执行前必须完成这项核验及无自动回退检查。
隔离 HTTP 回归额外检查客户端对非 WSOL 桥接路线显式指定 Jupiter 时返回 409，
且 trade、quote、submission 表均无写入；Jupiter quote compare 与 Shadow 仍只读可用。

### 路由决策落地顺序（2026-11-05 修订）

当前 `/v1/pools/resolve-candidates` 已能将部分链上验证的单池路线写入进程内
`RouteStore`；`/v1/trades` 只在 StonkFun Via-SOL 路径消费双跳快照，尚无覆盖所有
SDK 协议和聚合器的统一决策。
`SdkTradeClient::resolve_pool(mint)` 当前主要搜索 Pump/PumpSwap；搜索失败不能证明
StonkFun、Raydium 等其它 SDK 协议不支持该 mint。新 `SdkBackend` 的 quote/simulate
adapter 目前只覆盖 Raydium CPMM，也不能用它的 `501` 代表旧 SDK 交易执行器不支持。
带 `pump` 或 `pump-amm` 协议提示的 Mint 发现现只查询对应协议；无协议提示仍保持
原有曲线优先顺序。发现结果先与所选协议、池 ID 和 Quote Mint 比对，再读取账户做
链上核验，避免同一 Mint 的其它池造成额外 RPC 读取或错误候选。若所选协议的池未发现，
仍返回未解析，不构成 SDK 不支持证明。
对未传池 ID 的 PumpSwap 单池请求，服务端决策把链上验证后的池保存在短期路线快照中；
客户端使用同一决策 ID 发起 SDK 请求时，执行器从该快照固定池 ID，不再自行按 Mint
重选池。决策过期、钱包/金额/滑点变化或改传另一池 ID 均拒绝。此路径仍遵守
simulate-only，不能授权 Jupiter。
公开 PumpSwap fixture 在当前 Helius RPC 上无池 ID 的 Mint 枚举超过单 RPC lane 的
3 秒预算，隔离 HTTP 因而返回 `unresolved_retryable / route_verification_unavailable`；
同一目标携带已知池 ID 时仍返回 `sdk_verified`。搜索超时不能签发聚合器许可，
热路径应尽量携带事件或已核验候选中的池 ID。
已明确 PumpSwap 池 ID 的只读 `/v1/pools/resolve-candidates` 现可省略 Quote Mint：
服务端先读取该池，再核对目标 Mint、链上池 owner 与两侧 Mint/Token Program，返回实际
Quote Mint。错误目标 Mint 会被拒绝；该能力不代表已完成 SOL 两跳构建或 Jupiter 授权。
`/v1/routes/decide` 现也对省略 Quote Mint 的 Pump/PumpSwap SOL 买入先核验目标池。
实际 Quote 为 WSOL 时走单池 SDK 决策；非 WSOL 时继续验证 CPMM 桥接池，返回
`observed_bridge` 与 `unresolved_retryable`，不签发 `sdk_unsupported`。原请求不被改写；
预检和 simulate-only 执行从服务端观察快照读取真实 Quote Mint，原请求的指纹仍绑定
钱包、金额、滑点、池和省略状态。Pump 原生 SOL 曲线只在路线契约中规范为 WSOL。
隔离有资金钱包副本对省略 Quote 的 TTWO 两跳得到 `simulated`、签名为空，重启后同一
幂等键回读原结果，aggregator quote/submission 均为 0；价格在两次模拟间下移时亦曾
安全返回 `failed`，没有改走 Jupiter。
SDK 11 的高层 `simulateTransaction` 会返回本地构建交易的签名；它不是链上提交凭据。
candidate 现于 SDK 边界及统一交易状态映射两处清除模拟签名，失败的模拟记为 `failed`
而非 `unknown`。隔离文件钱包模式按运行钱包和服务端决策快照绑定公钥，数据库钱包模式
还会二次核对启用状态。两种模式均保持 `simulate-only`，不向发送端点广播。
当前源码已新增 SDK 已登记协议的明确提示单池链上核验：Pump/PumpSwap、Bonk、
StonkFun/StonkFunSwap、Meteora DAMM v2、Raydium AMM v4/CPMM；Bonk 池由 mint 和
quote 派生，其它非 Pump 协议需要明确池提示。另有
`/v1/pools/resolve-via-sol-candidates` 双跳入口。
`RouteStore` 将两跳的 mint、token program、slot 和最早过期时间绑定成单份快照；
Via-SOL 的 `/v1/trades` 在 SDK 构建前按 `sdk_route_id` 回查，未提供 ID 则服务端重新
核验。该接入不表示通用 `sdk_unsupported` 分类、SDK build/simulate 或 Jupiter 自动
选择已完成；新源码未安装到 18887。

**2026-11-06 SDK 报价增量。** `/v1/routes/quote` 已增加 PumpSwap 单池 quote-token-in
买入报价。单池报价和 CPMM→PumpSwap 两跳预检现在共用严格加载器：Pool、两侧 vault 与
base mint 同批读取，必须取得 fee/global config，并再次核对 Mint owner 与 Token Program。
RPC-only 快照不能确定 cashback 费率，或当前转账税非零时拒绝报价/预检；这类失败保持
未解析，不签发 `sdk_unsupported`。文件钱包模式的路线报价与模拟入口现按运行钱包校验，
数据库钱包模式继续核对启用状态和公钥。136 项 Rust 测试通过；公开池隔离 HTTP 得到
PumpSwap exact-in 报价与 7 条双跳指令，unsigned V1 模拟请求被 RPC 接受。另一次有资金
钱包数据库副本的隔离模拟返回 `simulated`、目标代币增量超过最低到手，重启后精确重试
只回读原记录；聚合器报价/提交表均为 0。此增量仍不签发 Router 或 Jupiter 执行许可，
也没有安装到 18887。
后续检查发现报价 RPC 读取可能耗尽五秒路线 TTL；现于生成报价时再次检查路线有效期，
并将报价到期时间限制为路线到期时间，路线校验器也拒绝超出该时间的报价。

**2026-11-06 AMM v4 只读报价增量。** `/v1/routes/quote` 现在接入 Raydium AMM v4
双向 exact-in 报价：先回查链上池 owner、开放状态和两侧 Mint，再使用 SDK 从
链上费率与扣除 PnL 后储备计算输出；池身份或费率在读取过程中变化则拒绝。
公开 WSOL/USDC AMM v4 池的隔离 HTTP 回归确认买卖双向正数报价、最低到手计算、
错误 Mint 拒绝、未授权 401 与 trade/aggregator quote/submission 零写入。
139 项 Rust 测试、格式检查、离线编译及独立 release 构建通过；此报价不代表
AMM v4 SOL 跨池构建、模拟或发送能力，`service_sol_route_adapter=not_integrated`、
`jupiter_candidate_allowed=false` 仍保持。新二进制未安装到 18887。

**2026-11-06 AMM v4 两跳只读预检增量。** 后续 candidate 已将经过链上核验的
WSOL/USDC CPMM 桥接池与 RAY/USDC AMM v4 目标池接入 `StonkFunQuoteRoute` 指令预检。
服务端重新读取池开放状态、费率、Mint 程序与逐跳快照，并检查生成的指令按
CPMM→AMM v4 排列。隔离 HTTP 使用公开 RAY/USDC 池
`6UmmUiYoBjSrhakAobJw8BvkmJtDVxaeBtbt7rxWo1mg` 和一次性钱包验证成功；
初次预检响应没有 `preview_id`，交易、聚合器报价和提交表均为 0。
随后服务端复用两跳 unsigned 模拟校验。公开 RAY/USDC 路线在有资金钱包数据库的临时副本中
得到 `simulation_ok=true`、目标代币增量 52,243 ≥ 最低到手 51,720、
SOL 扣款 2,493,440 lamports、69,592 CU、legacy wire 926 字节；
没有签名、`preview_id` 或交易写入。再下一 candidate 将成功的 unsigned 模拟
签发为短期预览凭据，绑定钱包、原请求、两池和最低到手，支持 simulate-only
`sdk-decision:bridge:` 决策。隔离有资金副本完成交易模拟，状态 `simulated`、
签名为空，精确重试及重启后回读幂等；交易表 1 行，聚合器报价与提交表 0 行。
AMM v4 的服务适配器状态为 `cpmm_amm_v4_simulate_only`，不表示 live 或
Jupiter 候选授权；新二进制未安装到 18887。

**2026-11-06 CPMM→CPMM simulate-only 增量。** 服务端现可将经链上核验的
WSOL/USDC Raydium CPMM 桥接池与 USDC Quote 的 Raydium CPMM 目标池接入原生
`StonkFunQuoteRoute`。两池重新加载时检查开放时间、swap 状态、池身份、方向、
中间币 Token Program、转账费和逐跳最低到手；构建结果必须恰有两条 CPMM swap。
隔离 release `candidate-f74ea99440aa-310381c76244` 通过 142 项 Rust、20 项 Python
测试和 release 构建。公开目标池 `gyYigqG8vdEMkDNTTVL6at2msbHdTDWyb6bccyR238U`
在有资金钱包数据库临时副本中得到 unsigned 模拟增量 23,860,003，超过最低到手
23,613,483；交易状态 `simulated`、签名为空，重启后精确幂等回读，交易/聚合器报价/
聚合器提交分别为 1/0/0。CPMM→AMM v4 回归也为 `simulated`；CPMM→PumpSwap
预检成功，池价移动后的交易安全记为 `failed`，未改走 Jupiter。
`service_sol_route_adapter=cpmm_cpmm_simulate_only` 仅描述此隔离预览与模拟路径；
Router SDK、Jupiter 交易候选授权和 live 发送仍未接入。
后续 `candidate-dd1f6235150a-6823e02a5c33` 补齐选定 CPMM 与 AMM v4
目标池的链上 Quote Mint 发现。客户端省略 Quote Mint 时，服务端从已选目标池读取
另一侧 Mint，再独立复核池、Mint 和 Token Program；无关目标 Mint 或同币池拒绝。
公开 CPMM 目标的省略 Quote 回归得到 `simulated`，目标增量 23,778,226 ≥
最低到手 23,538,381；AMM v4 同流程得到 52,448 ≥ 51,923。两者签名均为空，
重启后幂等回读，交易/聚合器报价/聚合器提交均为 1/0/0。

**能力范围校正。** `sol-trade-sdk 11.0.1` 包内还提供 Pump/PumpSwap 官方
`multi_hop_swap`（两跳或三跳）、`PumpSwapQuoteRoute` 和 `StonkFunQuoteRoute` 的显式
转换路线；这些都需要完整的池快照、方向对应的报价和最终最小输出。当前 Trade Service
直连执行器只接入已登记单池和 StonkFun 经 Raydium CPMM 的 Via-SOL 两跳，不能把
“本服务直连构建器未接入”说成“SDK 包不支持”。`sdk_client.rs` 中的静态能力分类
只用于当前直连入口的预检，不授权 Jupiter。服务端在形成 `sdk_unsupported` 前，还须
检查适用的 SDK 官方多跳和显式 quote route builder；未完成盘点时返回
`unresolved_retryable`，继续保留 SDK 优先。

**2026-11-06 SDK 路线腿盘点增量。** vendored SDK 的 `StonkFunQuoteRoute` 已明确包含
Raydium CPMM、AMM v4、CLMM、Orca Whirlpool 与 Meteora DLMM 的显式交易腿；
`/v1/capabilities` 现将这些协议的 SDK 包路线家族登记为 `StonkFunQuoteRoute`，
当时服务端 SOL 买入适配器仍标记 `not_integrated`，其中 CLMM/Whirlpool/DLMM 的协议状态
继续为 `observe`。后续 AMM v4 已加入上述只读指令预检；`/v1/routes/decide` 对
CPMM/AMM v4 桥接目标仍返回 `unresolved_retryable`。这是能力盘点，
不是可执行快照、`sdk_unsupported` 证明或 Jupiter 回退许可。
本次隔离验收：138 项 Rust 测试、`cargo fmt --check`、`cargo check --locked --offline`
通过；固定 Rust 1.99.0 在 `/tmp` 独立 target 目录完成 release 构建，二进制 SHA-256
为 `27593937260c30c97a72a2c2cd51d022fa660b7d5b9807ad0db9c49b38669e19`。
原 18887 启动路径的二进制仍为
`5f95683ba4c35013e921a1356621634d2324e47040e4cef15c44b61d4830d793`；
本次构建未安装、未重启 staging，也未切换 `current`。
隔离 HTTP 回归读取实际 `/v1/capabilities`，核对上述五个协议均为
`StonkFunQuoteRoute / not_integrated / supports_sol_route=false`；未授权请求为 401，
伪造聚合器交易为 409，RPC 不可用时路线保持 `unresolved_retryable`，
trade、aggregator quote、submission 表均为 0。

Pump.fun 的非 WSOL Quote 路线也有 SDK 包内的 `PumpFunQuoteRoute`。candidate 的
桥接能力登记现将其标为“SDK 包有构建器、服务端尚未集成”；链上桥接池即使已核验，
仍保持 `unresolved_retryable`，不能把当前服务适配器缺口转写成 `sdk_unsupported`。
Pump.fun bonding curve 没有调用方提供的 `pool_id`；桥接快照校验已改为核对其链上
验证过的 Pump 程序身份和 mint 派生曲线，并继续要求 CPMM 腿的池 ID。该改动只让
Pump.fun 目标进入只读桥接观察，不提供交易执行许可。

**Pump.fun token-quote SDK 预检门槛（2026-11-06 复核）。**
`PumpFunQuoteRoute` 是指令构建器，不是任意 token Quote 的可信报价器。
SDK 示例 `examples/support/pumpfun_example.rs` 的 live 双跳路径要求调用方提供
`--curve-min-out`；`instruction/stonkfun_quote_route.rs` 对 Pump.fun V2 仅检查该下限
非零及两侧 transfer fee 为零，不根据曲线和当前费用重新计算它。
`utils/calc/pumpfun.rs` 使用固定 SOL 费率，不能直接用于 USDC/其它 Quote。
V3 的 `instruction/pump_v3_quote.rs` 接受显式的当前 protocol/creator bps、曲线
base vault 实际余额和迁移参数；SDK 说明还要求区分 SOL、USDC 与 exotic/Pump coin
Quote 的费率。`idl/pump.json` 中存在 `FeeConfig` 的 flat/stable/exotic 档位以及
`Global` 的 creator fee 配置，因此不能用常量代替链上费率。
下一切片先增加链上 FeeConfig/Global、曲线、两侧 mint fee 和 base/quote vault 的
同次快照读取与 owner/PDA/slot 校验，再对 V2/V3 分别建立带边界条件的精确输入
报价及最低到手，并以 SDK fixture 和真实 unsigned 模拟交叉验证。只在零 transfer fee
或已证明精确处理费用的范围签发只读预检；完成钱包/请求绑定、重新报价和有资金
unsigned 模拟前保持 `unresolved_retryable`。找不到可靠费用或 vault 快照也保持此状态，
不等于 `sdk_unsupported`，不开放 Jupiter 交易候选。

**2026-11-06 费用账户快照增量。** candidate 已在非 WSOL Pump.fun 池的只读核验中
用一次 `getMultipleAccounts` 读取 Pump Global 和 pump-fees FeeConfig，检查共同 RPC
context slot、PDA、owner、IDL 布局、费率上界及未知非零尾部。主网只读回读确认 Global
为 1088 字节、FeeConfig 为 4097 字节（177 字节 Borsh 有效前缀，其余为零填充）；
Global 的旧 1087 字节布局也可解码。后续 candidate 已将曲线、两侧 mint 和曲线
base/quote vault 加入同一次 RPC 读取，按前后 epoch 一致性解析 Token-2022 当前
转账费，并核对曲线 PDA、账户 owner、mint、vault authority 与报价币连续性。
此处仍未选取具体费率档位或计算 token Quote 的最低到手；观察结果继续为
`unresolved_retryable`。隔离源码与构建验收见
`/root/solana/trade_service/evidence/pumpfun-feeconfig-20261106/` 及后续快照证据。

1. **统一 SDK 能力与路线发现。** 分别记录“SDK 包存在的 builder”和“Trade Service 已
   集成并验证的 builder”，按协议、买卖方向、报价币、跳数、执行方式和已启用功能形成
   能力矩阵；接收 pool/program/mint/quote/hop 提示时逐项链上核验，提示本身不算证据。
   对 StonkFun 经 Raydium CPMM 的 SOL 两跳，分别验证两池、mint 连续性、方向、token
   program、slot 与过期时间，并生成不可拆分的双跳快照。先覆盖当前可执行的 SDK 路径，
   再把官方 Pump 多跳、PumpSwap/StonkFun 显式 quote 路线逐项纳入适配与模拟验证；
   单一 mint 搜索器的失败不得触发聚合器。热路径复用
   有效的服务端快照和参数缓存，到期再读取链上状态，记录决策耗时及缓存命中率。
2. **服务端返回明确的路线决策。** 新增独立的 route-decision contract，避免改变现有
   `/v1/pools/resolve` 响应。结果分为 `sdk_verified`、`sdk_unsupported`、
   `unresolved_retryable` 和 `policy_blocked`；仅在本次请求的能力矩阵覆盖完整、输入条件
   确定且确实没有
   SDK 构建路径时给出 `sdk_unsupported`。RPC 超时、账户读取失败、未知池、资料不足、
   开关关闭和钱包限制均不得冒充 `sdk_unsupported`。决策绑定 wallet、方向、mint、
   pool/hop、输入资产和金额、slippage、validated_slot、TTL、服务端 route ID 与请求指纹；
   仅由服务端保存和签发，客户端提交的布尔标记或 reason 不授权执行。
3. **同一决策贯穿报价和模拟。** `/v1/trades` 与新 route API 共用服务端校验器；
   `sdk_verified` 只能走 SDK，包括已验证的两跳。只有 `sdk_unsupported` 可展示 Jupiter
   只读报价候选，且要重新报价并由用户确认；当前 Gate 2 关闭，不能据此提交。
   SDK 报价、构建、模拟或 RPC 调用失败都结束原请求，不在同一幂等键下切换 provider。
   旧 Jupiter 只读/模拟入口可供 Shadow 诊断，但不把显式 `execution_method=aggregator`
   当成 SDK 不支持的证明；开放聚合器执行前必须要求有效的服务端决策 ID。
4. **bot 只消费决策，不自行猜测。** P4 bot 使用 Trade Service 的决策和快照 ID 展示
   SDK 或 Jupiter 候选；本地缓存、PumpAPI 事件和候选路由表只作提示，不能提升成
   `sdk_unsupported`。首次迁移只读展示，现有跟单与手动 SDK 请求保持原行为。
   2026-11-06 隔离的 `bot_trading_p4_candidate` 已在手动提交关闭时只读调用
   `/v1/routes/decide`，展示服务端返回的路线状态与 SDK 能力；不传递决策 ID 到
   交易提交，也不根据本地缓存签发 Jupiter 候选。此后该只读客户端已选择性合入
   `bot_trading` 源码并完成构建，服务未启动；
   P4 candidate 的完整测试为 99 项通过、1 项快照测试单独在数据库只读副本上通过。
5. **按决策边界验收。** 加入 SDK 单池、StonkFun 双跳、其它已支持协议、过期或篡改
   快照、错误 wallet/mint/hop、RPC 超时、未知池、开关关闭、SDK build/simulate 失败和
   并发幂等测试；逐项证明既不会误走 Jupiter，也不会将失败后的重试变为新订单。
   同条件测量 SDK 直走与新增决策层的 p50/p95 延迟，若缓存和验证使快路径明显变慢，
   先优化决策层再迁移 bot。
   独立 staging HTTP 与 SQLite 回读通过后才更新 P4 客户端；24 小时 Jupiter
   quote-only Shadow 可与这些开发并行，它只验收报价稳定性，不决定 SDK 路由策略。

实施检查点：先完成 1–2 的服务端快照与错误分类，再接 3 的模拟入口，最后做 4 的
bot 只读迁移；每个检查点均保持 `simulate-only`。Gate 2、canary、`current` 和 18788
仍受原有独立门槛约束。

### 下一轮修改切片与完成条件（2026-11-05）

| 顺序 | 修改位置 | 可交付结果 | 完成条件 |
|---|---|---|---|
| 0（已通过） | 隔离运行环境与 `STAGING_RUNBOOK.md` | 用已构建的 `candidate-91371b8d2a18` 在独立端口、临时钱包副本和独立 SQLite 上做只读 HTTP smoke；保留 18887 sampler 原进程 | `/health`、`/ready`、鉴权、聚合器 capabilities/metrics 只读响应通过；三个模拟开关生效，quote/submission 表无新增记录 |
| 1 | `sdk_client.rs`、`backend/sdk.rs`、`backend/types.rs`、`route_registry.rs` | 区分 SDK 包能力与当前服务直连能力，建立协议、方向、输入币、单池/多跳能力表；为路线核验返回可判别的证据或未解析原因 | 已集成的单池和 StonkFun Via-SOL 双跳均为 `sdk_verified`；未集成的 SDK 多跳、发现器缺池、RPC 错误、配置关闭和钱包限制均不能成为 `sdk_unsupported` |
| 2 | `http.rs`、`executor.rs`、route store | 增加独立 route-decision API 和服务端决策快照；`/v1/trades` 与新 API 共用校验器 | 决策绑定 wallet、方向、mint、资产、金额、滑点和路线指纹；过期或篡改拒绝；客户端不能自报 `sdk_unsupported`；SDK 构建前复核绑定与 TTL，到期才重查链上状态 |
| 3 | 聚合器服务、模拟报告与持久化 | 只允许经有效 `sdk_unsupported` 决策取得 Jupiter 交易候选；旧 compare/Shadow 仍可做只读诊断 | 已验证 SDK 路线不能被显式 aggregator 绕开；SDK build/simulate 失败不换 provider 或幂等键；Jupiter 候选仍不可签名、不可提交 |
| 4 | bot 隔离 P4 副本 | 只读展示服务端 SDK 决策或 Jupiter 候选，不在 bot 中推断路线 | 当前跟单和手动 SDK 行为无回归；不接 Jupiter 手动提交；受控数据库副本回读通过 |

切片 0 已于 2026-11-05 在 `127.0.0.1:18893` 通过：隔离 HTTP smoke、未授权 401、
SQLite schema 回读；`aggregator_quotes=0`、`aggregator_submissions=0`、
`aggregator_shadow_metrics=1`（DFlow 禁用诊断）。临时进程和钱包副本已清理，18887 未重启。
切片 1–3 可与 24 小时 Jupiter
quote-only Shadow 并行。每一切片先做针对性的单元/HTTP/SQLite 回归，再更新文档和隔离
release 构建。24 小时窗口只用于验收 P3 报价稳定性，不是 SDK 路由开发的等待条件。
`sdk_unsupported` 必须带服务端可复核的能力或路线证据；只知道 Jupiter 有报价、
某个 mint 搜索器未找到池、或某个新 adapter 返回 `501`，都不足以生成该状态。

1. 在 Gate 2 证据完成前，Jupiter preflight 的 `instruction_audit_passed` 和 `executable` 必须固定为 false；Gate 1 只表示“完成结构检查和独立 unsigned simulation”，不表示可签名。
2. 将 `normalized.aggregator_sol` 从 `executor.rs` 的专用分支迁移到统一 aggregator service。HTTP、`/v1/trades` 和后续 bot client 都使用同一条 quote → preflight → simulate → submit 状态机。
3. 聚合器模拟结果写入统一 `SimulationReport`，至少包含 provider、quote_id、route_fingerprint、request_id、message_hash（如已构建）、expected/min output、费用、slot、错误分类；模拟永远不返回链上 signature。
4. 在服务端再次验证 wallet、mint、金额、slippage、route 和 quote 过期时间；客户端发送的 provider、route 或 validation flags 只能作为提示，不能授权执行。

验收：恶意 provider/route/钱包字段被拒绝；旧 `/v1/trades` 和新 quote endpoint 的 simulate 结果一致；没有任何 aggregator 路径可在 candidate 绕过 simulate-only guard。

## 阶段 P2：Jupiter provider 抽象和持久化状态

保留现有 API 兼容性（旧客户端不传 provider 时默认 Jupiter），但内部拆出：

```text
src/aggregator/mod.rs
src/aggregator/types.rs
src/aggregator/jupiter.rs
src/aggregator/audit.rs
src/aggregator/store.rs
```

统一接口至少包含 `capabilities`、`quote`、`build`、`preflight`；provider-specific JSON 只停留在 adapter 内。统一 quote envelope 包含：

```text
provider, provider_quote_id, wallet_id, wallet_pubkey,
input_mint, output_mint, input_amount, expected_output,
minimum_output, slippage_bps, route_fingerprint,
request_hash, message_hash, created_at, expires_at
```

新增正式 migration（不要复用 bot 的旧编号）保存 `aggregator_quotes` 和 `aggregator_submissions`：

- quote 的 provider、provider_quote_id、请求 hash、route fingerprint、过期时间和 wallet 公钥；
- preflight/audit JSON、状态和 message hash；
- `(wallet_id, idempotency_key)` 唯一约束；
- `quoted -> preflighted -> reserved -> submitted|unknown|failed` 状态机；
- 重启时 `reserved/submitting` 进入 `unknown`，只能用原 quote、原 request_id 和原幂等键回查。

quote 选定 provider 后不可切换。provider 超时只能返回失败或部分 compare 结果，不能在 submit 阶段偷偷换 provider。

验收：并发重复请求只保留一个 reservation；重启后仍能读取 quote/submission；provider、route、message hash 或 wallet 不匹配时拒绝；unknown 不创建新交易。

## 阶段 P3：Jupiter read-only Shadow 和 A/B 基线

本阶段只对 Jupiter 做 read-only Shadow。compare API 可以使用统一 provider registry，但默认只启用 Jupiter；其他 provider 返回 disabled/unavailable，不请求真实服务。

- quote 只落审计和指标，不生成可执行交易；
- 记录 quote p50/p95/p99、成功率、过期率、min-output 通过率、HTTP/RPC 错误、route 长度和费用字段；
- provider API key、RPC URL 和原始敏感响应不得写入日志；
- native SDK 命中已验证协议时仍优先 native SDK。

持续 24 小时后，先验证 Jupiter 的稳定性和回查完整性。sampler 的 HTTP 200 不能代表
Jupiter 报价成功；需读取持久化 metrics 的 `quoted/failed/unavailable`、p50/p95/p99，
并检查运行日志中的断点和结构错误。未来新增 provider 时复用同一 contract 和指标，
不改变 Jupiter 基线。

## 阶段 P4：备份分支 copy_trading 能力选择性合入本机 bot_trading

### P4 隔离执行 checkpoint（2026-11-05）

已创建独立审查副本 `/root/solana/bot_trading_p4_candidate`，未修改已安装的
`/root/solana/bot_trading`。该副本完成了第一批兼容 schema：本地 003 之后新增
`004_mint_pool_route_candidates_compat.sql` 和 `005_alpha_shadow_orders_compat.sql`，并将
`alpha_paper_signals` 的远程生命周期字段以幂等列迁移加入。`mint_pool_routes` 仍是执行查询
的权威表，候选表只用于只读诊断。迁移回填、重复执行和旧路由选择测试已通过；已加入经过
脱敏的 `alpha_paper_report.py` 和 `alpha_shadow_diagnose.py` 只读诊断脚本。staging 中已接入
30 秒只读协议 capability cache（只显示在 preflight，不授权路由或提交），并移植默认关闭的
alpha retention，保留有关联的 signal、unknown order 和未知未来状态。Alpha Shadow executor
也已接入 staging：固定 `simulate=true`，要求 pool/mint allowlist，使用
`alpha-shadow:<signal_id>` 幂等键，并在 submission guard 关闭时只记录 `blocked`，不会发
Trade Service 请求；默认 `ALPHA_SHADOW_ENABLED=false`。Alpha retention 现在默认开启，按
时间和行数上限分批清理 observation、paper signal、shadow order，并在清理后 checkpoint WAL；
P4 隔离副本又为新 paper signal/Shadow order 加入包含 `unknown` 行的总容量门槛，并限制
feature、request、response、error 单条大小；容量满时拒绝新增，保留已有幂等回查和请求 ID。
P4 隔离副本的 retention 现在在提交清理后尝试 `TRUNCATE` WAL checkpoint；文件级回归确认
旧 observation 清理后 WAL 归零，繁忙时下轮重试。隔离 release SHA-256 为
`57e04c9884503c79879f1053600685ba6a5345b3a7810e78e5781d9d4da28f13`，
证据见 `/root/solana/bot_trading_builds/p4-57e04c988450/BUILD_INFO`。该副本仍未安装到当前 bot。
完整 copy-trading 行为尚未迁入。
P4 隔离副本现可在手动提交开关关闭时对支持的请求只读调用
`/v1/routes/decide` 并展示服务端 `sdk_verified`、`unresolved_retryable` 等状态；
不使用本地候选池表提升权限，不把未解析状态转为 Jupiter，也不创建 bot 订单或调用
`/v1/trades`。mock HTTP 测试验证提交开关仍阻断交易请求；99 项 Rust 测试通过、
1 项快照测试在独立只读备份上通过，release 构建和 P4 unit 边界检查通过。
最新 P4 隔离二进制 SHA-256 为
`34aa737c69de82c58a060d2f81c3cc44ae32c1f901c81932e43e0cb0f2e835ef`；
未安装到当前 bot，也未修改现有 18788 隧道。
后续 P4 只读提示已跟进 Trade Service 的
`simulate_only_bridge_decision`、`cpmm_pumpswap_simulate_only`、
`cpmm_amm_v4_simulate_only` 和 `cpmm_cpmm_simulate_only`：在服务端仍返回
`unresolved_retryable` 时提示需要预检与模拟，且 Jupiter 未获授权。该提示不创建订单、
不调用 `/v1/trades`；隔离副本 99 项 Rust 测试通过，1 项数据库快照测试仍单独验收。
该快照测试已在当前 `bot_trading.sqlite` 与 `copy_trading.sqlite` 的只读备份上分别通过；
P4 release 二进制 SHA-256 为
`fcf4b91a56de542c14d9610a7d42f95a794f242dfe5cb6f6b758ec074c227f0c`，
归档于 `/root/solana/bot_trading_builds/p4-fcf4b91a56de`，未安装。

2026-11-06 后续 P4 隔离副本新增独立只读决策地址与 token：
`TRADE_ROUTE_DECISION_URL` 可指向 18887，而交易用 `TRADE_SERVICE_URL` 保持 18788。
配置仅接受显式 `127.0.0.1` HTTP 地址，要求提交开关关闭；只读 token 不发往交易地址，
即使后续代码重新打开客户端提交标记，该配置也会在提交入口拒绝。mock HTTP 证实
交易地址无请求、决策地址只接收 `/v1/routes/decide`；101 项 Rust 测试通过、
1 项数据库快照测试按既有流程单独验收，最终提交 guard 的 8 项针对性测试与
锁定 release 构建通过。最新隔离二进制 SHA-256 为
`326b1b9deb5633b0ba0383b00575f0852e9abcdd0c3de5cce35c8f58fd2dfc4e`，
归档于 `/root/solana/bot_trading_builds/p4-326b1b9deb56/`；该能力尚未安装到当前 bot。
18887 与两个已安装 bot SQLite 的八条远端钱包映射逐条公钥一致；活跃钱包在 staging
启用。按 bot 字段构造的只读 SOL 买入决策返回 `sdk_verified`、服务端绑定 ID 和
`simulate_only=true`，交易、聚合器报价及提交表行数不变。证据见
`/root/solana/trade_service/evidence/staging-upgrade-20261106T005806Z/P4_READONLY_PROBE.json`。
P4 客户端对独立决策地址额外要求响应 `simulate_only=true`；当前 bot 仍未安装 P4 副本。
增加该 guard 后八项定向 mock 测试和锁定 release 构建通过，最新 P4 隔离二进制
SHA-256 为 `52ac91d53673109e67e3ba2b67e0788733311e0ab319e088720337628eb856d3`，
归档于 `/root/solana/bot_trading_builds/p4-52ac91d53673/`。
后续 P4 增加逐次公钥核对：独立 18887 决策响应中的 `wallet_pubkey` 必须等于所选
本地钱包公钥，缺失或不一致时只显示诊断错误，不创建订单。8 项针对性 mock 和完整
101 项 Rust 测试通过；最新隔离 release SHA-256 为
`eb531d69fed3535b09495130d3680828130237f7151eea923883d7cbcb6a9f3b`，
归档于 `/root/solana/bot_trading_builds/p4-eb531d69fed3/`。此后相同的只读功能已
选择性合入当前 bot 源码并重新构建；未启动 bot 服务，运行级 UI 尚未验收。

新增的 `scripts/verify_p4_migration_snapshot.py` 会先对现有 bot SQLite 做只读 backup，再
在 `/tmp` 副本上执行 migration、重复 migration、重启回读和 `PRAGMA integrity_check`；
当前 `copy_trading.sqlite` 与 `bot_trading.sqlite` 两份副本均已通过，原库没有写入。

在独立 staging patch 中迁移远程新增能力：alpha shadow、retention、route candidates、capability cache 和只读诊断脚本。保留本地的：

- `127.0.0.1:18788` 隧道、`TRADE_SERVICE_URL`、bot-trading unit 和提交 guard；
- WSOL read-only conversions；
- PNL/position wallet；
- 现有数据库、钱包和首两笔跟单路径。

备份分支 migration 003–007 与本机已有 `003_alpha_observation.sql` 冲突，不能按文件名覆盖；应建立新的兼容 migration，完成旧库 ALTER/backfill、WAL 和重启回读测试。备份分支 copy_trading 的手动 SOL buy 只有在 candidate 提供新的 provider-neutral contract 后才接入，不能把 SDK 6.0.2 的 Jupiter 文件复制到 SDK 11 candidate。

复核备份分支 commit `00c9b7a47f08e2f66da32926744f09d1aeb36141`：手动 SOL buy
仍直接调用 Jupiter 专用 quote/preflight/submit。candidate 的 provider-neutral compare 当前
只读，Gate 2 仍关闭，所以 P4 不迁入该手动提交路径；现有手动 SDK 路径保持隔离副本原状。

Alpha Shadow 必须明确 `simulate=true`，并继续受本地 `TRADE_SUBMISSION_ENABLED` guard 保护；不替换现有 `alpha.rs` 的 copy-follow 行为。
bot 的 SDK 单池和已验证 SOL 两跳请求保持 SDK；只有服务端明确返回 SDK
不支持时才展示 Jupiter 报价候选，并要求新报价和再次确认。不得把 SDK 的运行失败
自动重试为 Jupiter 订单。现有手动 SOL buy 仍不接入 Gate 2 前的提交路径。

## 阶段 P5：测试和运行门槛

补充 `tests/` 集成测试和可回放 fixture：

- mock Jupiter 及未来 provider：恶意 mint、route discontinuity、过期、错误 provider_quote_id、低于 min output、超时和部分失败；
- provider binding、request/route/message hash、wallet binding 和 quote expiry；
- 并发幂等、重启回查、WAL、migration 重复执行；
- Gate 1/2 恶意交易：recipient substitution、额外 signer、隐藏 tip、恶意 CPI、stale ALT、超额 debit；
- `/v1/capabilities`、compare endpoint、旧 `/v1/quotes/sol-buy` 兼容契约；
- executor 与 HTTP 使用同一 aggregator service；
- SDK 单池、StonkFun SOL 两跳均保持 SDK；非 WSOL quote 加 `amount_sol` 不得触发
  隐式 Jupiter；SDK 不支持、SDK 失败、伪造 hop 和过期 route 均不得自动提交或换 provider；
- 日志不出现 RPC URL、API key、私钥或完整敏感 provider 响应。

当前 compare 边界已补 provider quote ID 非空/长度限制、60 秒有效期上限、最多四跳及
每跳 mint/label 校验；路由断裂用例已单独触发连续性错误。
`tests/fixtures/jupiter_two_hop_quote.json` 可回放 Jupiter 双跳报价，并在单元测试中验证
恶意 input/output mint、变更输入量、低于最小输出、路由断裂、非法路由 mint 和过高价格
影响均被拒绝。其余 P5 HTTP 集成 fixture 和 Gate 2 恶意交易矩阵仍按上述清单验收。
compare 的 mock provider 回归还验证：单个 provider 返回包含敏感内容的错误或任务异常退出时，
仍给每个所选 provider 返回一条状态，保留其它 provider 的有效报价；响应只给固定错误类别，
不透传原始 provider 错误。运行级 HTTP 与持久化回读仍需独立验收。
Gate 1 unsigned envelope 已拆为独立校验，合成交易回归覆盖额外 signer、非空签名、错误
付款钱包、零/过多指令及非法 program index；这些用例只证明结构边界，不能替代 Gate 2
的 recipient、CPI、ALT 时效和总费用审计。

候选构建必须在兼容工具链下依次通过：

```text
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo build --release --locked
```

## 退出条件和回滚

P3 的 Jupiter quote-only Shadow 可以在隔离 staging、simulate-only 和只读 HTTP/SQLite
门槛通过后运行；它只验收报价可用性和采样持久化。Jupiter 的 P0–P5 完整执行与安全证据齐全后，
才允许进入 Jupiter 交易流程 Shadow。该流程 Shadow 至少 24 小时，且必须满足：
无未解释 unknown、无重复 reservation、无钱包公钥漂移、数据库重启回读正常、错误和延迟
在预设阈值内。两种 Shadow 的证据不能互相替代。

SDK 6 单池、SDK 11 单池及 SDK 11 原生两跳分别按交易优先计划完成构建、模拟、
钱包/幂等绑定、唯一发送与回查方案后进入各自受限 canary；在 canary 中核对链上确认，
无需等待 Jupiter Gate 2
或 24 小时 Shadow。任何 canary 均需在具体钱包、池、金额和预检证据准备齐全后单独审批，
限定单钱包、单协议、单 mint/池、单笔和日总额；先买后卖，任何 unknown、模拟失败、
route/mint/token program 不一致立即关闭对应交易能力。

回滚只停候选进程、关闭 feature flag 或切回已验证 release；不降级数据库、不覆盖 authoritative SQLite、不修改 18788 隧道。

## 文档同步清单

在实现 P1 前同步修订：

- `AGGREGATOR_SOL_BUY_ROLLOUT.md`：把当前代码的 Gate 1 返回值与文档统一，明确 Gate 2 前不可执行；
- `PUBLIC_TRADE_API.md`：删除“旧库启动自动补列”的旧描述，改为 migration 002 的 fresh/old/WAL/restart 验收语义；
- `TRADE_SERVICE_REMAINING_ROADMAP.md`：区分“已实现 route store、Jupiter 持久化 quote/preflight/audit、Shadow 汇总”与“尚未实现的余额快照、运行级集成测试和 24 小时 Shadow”，并引用本审查版计划；
- bot staging 文档：注明远程 systemd 的旧 `/root/http/sol-bot` 路径不可直接安装。

