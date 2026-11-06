# Trade Service 未完成工作的阶段性落地计划

更新时间：2026-11-06

当前实施顺序已调整为 [SDK 6 与 SDK 11 交易优先的最短落地路径](FAST_TRADING_PRIORITY_REVIEW.md)：
并行推进现有 SDK 6.0.2 单池与 SDK 11.0.1 单池；SDK 6 固定依赖缺失期间先推进 SDK 11 的 Bot 隔离模拟，随后完成 SDK 11 原生两跳。
下文保留此前阶段记录；Jupiter Gate 2、24 小时 Shadow、Router 与全协议覆盖
不作为 SDK 交易交付的前置条件。

最新增量：Pump.fun V3 USDC Quote 已具备限定条件的保护报价和 CPMM→曲线只读
指令预检；147 项 Rust 与 20 项 Python 测试及隔离 release 构建通过。Pump 预检不
签发交易凭据，真实非 WSOL 曲线样本、有资金 unsigned 模拟及执行适配器仍待验收，
所以继续返回 `unresolved_retryable`，不开放 Jupiter 交易候选。18887 未更新；
详见 `MULTI_AGGREGATOR_REVIEWED_PLAN.md` 的最新验收状态。
随后已收紧自定义 creator fee、Quote Mint 漂移和 quote vault 储备的报价前置校验；
这不改变 Pump 路线的未解析状态。

当前状态（2026-11-06）：Jupiter quote-only Shadow 已在旧 18887 二进制上完成连续
24 小时验收，1,439 条样本全部 quoted，归档日志和 SQLite 再次通过只读校验；证据见
[`MULTI_AGGREGATOR_REVIEWED_PLAN.md`](MULTI_AGGREGATOR_REVIEWED_PLAN.md) 的“当前验收状态”。
采样服务已退出并关闭开机自启。SDK 两跳 candidate 已在窗口结束后安装到隔离的 18887；
当前二进制为 `7e9bb07c4a9d`，只读决策会返回已解析钱包的公钥和直连 SDK
构建器证据，供 P4 客户端核对。
HTTP smoke、同库重启回读、真实钱包公钥匹配和二进制审计通过；新二进制的
真实钱包 PumpSwap 只读决策与零交易写入证据位于
`/root/solana/trade_service/evidence/staging-direct-sdk-evidence-20261106T055056Z/`。
两跳有资金模拟证据
仍来自独立临时数据库，不能由部署动作扩展为实盘验收。P4 的独立 18887
只读决策客户端已选择性合入现有 bot 源码并完成锁定 release 构建，bot 服务未启动；
运行级 bot UI 尚未验收。合入与回滚证据见
[`MULTI_AGGREGATOR_REVIEWED_PLAN.md`](MULTI_AGGREGATOR_REVIEWED_PLAN.md) 的“当前验收状态”。
Gate 2、交易流程 Shadow、canary 和 live 仍未完成。以下较早的“Shadow 仍在运行”或
“未安装到 18887”记录仅描述当时的检查点。

Pump.fun 非 WSOL Quote 的 CPMM 桥接目前只到链上池身份观察。SDK 包有
`PumpFunQuoteRoute`，但 V2 需要调用方给出曲线最低到手，V3 需要当前费用和实际
vault 状态；candidate 尚未形成经链上费率核验的 token-quote 报价。因此它继续返回
`unresolved_retryable`，不能由“服务适配器尚未接入”推定为 `sdk_unsupported`。
具体快照与模拟门槛见审查版计划的“Pump.fun token-quote SDK 预检门槛”。
candidate 随后已将 Pump Global、pump-fees FeeConfig、曲线、两侧 mint 与曲线
vault 纳入同次 RPC context slot 的只读校验；Token-2022 当前费用按 epoch
核对。费率档位选择、报价与有资金 unsigned 模拟仍待完成。

2026-11-06 增量：candidate 为已核验的 Raydium CPMM→AMM v4 SOL 买路线增加
Trade SDK 11 显式路线的只读指令预检。预检重新读取两池、目标池开放状态、Mint
程序与费率，按桥接最低到手计算目标腿最低到手，并核对构建指令恰有 CPMM、AMM v4
两条 swap。公开 RAY/USDC AMM v4 目标池与 WSOL/USDC CPMM 桥接池已通过隔离 HTTP
正向预检。后续 unsigned 模拟在有资金钱包数据库的临时副本中成功，目标代币增量
52,243 ≥ 最低到手 51,720，SOL 扣款 2,493,440 lamports，69,592 CU，legacy wire
926 字节；三张交易相关表为 0。后续 candidate 已把成功模拟的短期预览凭据接入
服务端 `sdk-decision:bridge:` 与 simulate-only 交易入口。隔离有资金副本的交易
状态 `simulated`、签名为空，幂等重试和重启回读通过；交易表 1 行，聚合器报价与
提交表均为 0。AMM v4 仍不签发 Jupiter 候选，也不允许 live。

当前执行顺序以 [`MULTI_AGGREGATOR_REVIEWED_PLAN.md`](MULTI_AGGREGATOR_REVIEWED_PLAN.md)
的“下一轮修改切片与完成条件”为准：先隔离验证最新 candidate，再完成 SDK 能力判定、
服务端决策快照、Jupiter 候选约束和 bot 只读展示。`candidate-91371b8d2a18` 已通过
116 项 Rust、20 项 Python 测试及隔离 release 构建；该产物随后在独立 18893 通过
HTTP smoke、未授权 401 和 SQLite 回读，临时进程及钱包副本已清理。它未安装到 18887。
Jupiter 24 小时只读 Shadow 与这些 simulate-only 修改并行。

后续隔离构建 `candidate-9a635a67af1f` 已把当前 `/v1/trades` 直连 SDK builder 的
确定性能力判断集中到 `sdk_client.rs`：本入口未接入的路线不会被当作整个
`sol-trade-sdk` 包不支持，更不能直接授权 Jupiter。118 项 Rust、20 项 Python 测试
及锁定 release 构建通过；该产物未部署。后续源码已加入
`POST /v1/routes/decide`，只签发已核验 SDK 路线的短期决策；完整的
`sdk_unsupported` 分类和 Jupiter 候选授权仍待实现。

隔离 release `candidate-280cd070c205` 修复了文件钱包模式的决策绑定，并通过
121 项 Rust 测试、格式检查、锁定 release 构建和独立 HTTP 正负向边界检查。
未授权请求返回 401，钱包/执行方式策略拒绝与 RPC 路线未解析分别返回
`policy_blocked`、`unresolved_retryable`；伪造决策 ID 的交易请求返回 400，
公开 PumpSwap 示例池返回 `sdk_verified`；篡改金额后原决策 ID 被拒绝。用同一 SQLite
重启后，未过期的旧决策被拒绝，新决策可重新签发。临时 SQLite 的 trade、aggregator
quote、submission 表均为 0 行。该产物仍未部署；SDK 构建/模拟、Jupiter 候选和 bot
展示仍待推进。

隔离 release `candidate-bee252bdcf18` 把无桥接池的非 WSOL Quote SOL 买入从策略拒绝
改为待解析：服务先核验目标池，公开 TTWO/PumpSwap 池的 HTTP 检查返回
`unresolved_retryable / bridge_route_not_resolved`。121 项 Rust 测试、锁定 release
构建、WSOL 单池正向决策、重启与零写入检查通过。通用桥接池发现、Trade SDK 多跳
适配和后续 Router/Jupiter 候选仍未实现。

后续隔离 release `candidate-58bb1ba70b3f` 在公开 TTWO/PumpSwap 池上找到并核验
WSOL/TTWO Raydium CPMM 桥接池，返回
`unresolved_retryable / bridge_verified_sdk_capability_unresolved`；122 项 Rust 测试、
锁定 release 构建、HTTP 重启和零写入检查通过。两腿证据尚未保存为短期快照，
Trade SDK 对该路线的 builder 也未验收，因此不签发交易决策。
隔离 release `candidate-c0b895660784` 增加进程内短期 `observed_bridge` 只读记录，
绑定钱包、原请求指纹、两腿快照和最早过期时间；它不是 SDK 决策或 Jupiter 执行许可，
重启后失效。修复无效观察 ID 在 400 前仍落 rejected trade 的问题后，123 项 Rust 测试、
锁定 release 构建、隔离 HTTP 正负向、重启及零写入检查通过。SDK 多跳逐跳报价、构建
和模拟仍未完成，18887 未更新。
隔离 release `candidate-72b42dd55018` 增加独立的 `POST /v1/routes/observations/sdk-preview`：针对已观察的
Raydium CPMM→PumpSwap 路线，用 Trade SDK 11.0.1 重新报价并组装指令。公开 TTWO fixture
的隔离 HTTP 正负向通过，实际构建 7 条指令；决策仍为 `unresolved_retryable`，三张表
零写入。下一步是同一钱包和金额下的完整交易模拟与模拟报告校验，随后再考虑签发
SDK 决策；其它协议路线依然未验收。
进一步的 unsigned legacy 交易尺寸检查发现该 TTWO fixture 为 1,368 字节，超过
1,232 字节上限。旧版按 `bincode` 报告的 V1 1,395 字节不是实际 wire 大小；
`wincode` 隔离复测为 1,387 字节。当前 RPC 接受了模拟请求，但一次性钱包无资金，
模拟返回付款账户扣款失败。尺寸失败不是聚合器回退依据。
隔离 release `candidate-e191726a59b5` 已确认 V1 只读模拟 RPC 接受、因临时钱包
无资金失败，且重启与零写入边界保持通过；尚未有资金钱包的协议成功模拟。

后续 isolated candidate 使用 staging 中有余额钱包的加密数据库临时副本和独立端口，
只调用路线决策与 unsigned V1 模拟。公开 TTWO/PumpSwap 两跳在同一次 RPC 模拟中
`ok=true`，目标代币增加 10,783,332 单位，高于当前 quote 的最低到手 10,675,498；
付款账户扣款 4,078,400 lamports，消耗 171,693 CU。脚本验证交易、聚合器 quote 和
submission 表全为 0；临时数据库已清理。结果证明该池路线在当前 RPC 与该钱包状态下
可模拟成功，尚未证明 V1 发送通道、不同钱包/池或实盘可执行。
隔离 release `candidate-90c01ee3255b` 进一步把模拟返回的目标 ATA、前后 token balance
和付款账户扣款纳入同次报告；复测目标增量 10,704,474 ≥ 最低到手 10,597,351，
171,623 CU，零交易写入。决策仍保持 `unresolved_retryable`；下一道门槛为 V1 发送
通道或 v0/ALT 的隔离验收，以及服务端决策、报价和执行绑定。
`candidate-dbd936098c13` 新增离线 mock RPC 检查：Trade SDK 与 Solana RPC 客户端
对同一已签名 V1 测试交易生成相同 wire bytes，124 项 Rust 测试通过。该测试只用一次性
测试密钥，不触发网络发送；当前正式执行器仍默认 V0，V1 路线接入和真实发送端点支持
尚未验收，`unresolved_retryable` 保持不变。
后续隔离源码将有资金模拟成功的两跳预览保存为短期内存快照，绑定钱包公钥、原请求
指纹、两腿观察、最低到手与目标增量。只读回查、篡改请求、把预览 ID 当决策 ID 下单和
重启后旧 ID 失效均已通过隔离 HTTP；未创建 trade、aggregator quote 或 submission。
该证据仍不代表正式执行器已接入 V1，也不能签发 `sdk_verified` 或 Jupiter 回退许可。

本文只描述当前候选版本之后仍未完成的工作。候选源码的常规 unit 使用
`127.0.0.1:18787`；本次隔离 Shadow 验收使用独立的 `127.0.0.1:18887` 和 staging 数据库，
两者都保持 `TRADE_SERVICE_SIMULATE_ONLY=true`。现有 `bot_trading`、18788 SSH 隧道、
SDK 6.0.2 `current` 和钱包数据不在本计划中直接切换。

## 审查校正（2026-11-04）

SDK 优先、Jupiter 只读补充、多聚合器扩展接口、candidate 构建和 bot 选择性迁移的依赖关系以
[`MULTI_AGGREGATOR_REVIEWED_PLAN.md`](MULTI_AGGREGATOR_REVIEWED_PLAN.md) 为准。
当前 Jupiter 是唯一实际 provider；其他聚合器只保留扩展接口，不在本阶段实现。
已验证的 SDK 单池和 StonkFun 经 Raydium CPMM 的 SOL 两跳仍走 SDK；Via-SOL 的
`/v1/trades` 已接入双跳快照回查或服务端重新核验，通用 SDK/聚合器决策仍未完成，
显式选择聚合器不能证明 SDK 不支持。
具体错误分类、双跳快照和 bot 迁移顺序见审查版计划的“路由决策落地顺序”。
candidate 已在隔离 `18887` staging 完成 HTTP smoke 与重启后 SQLite 只读回查；这些证据
只覆盖 quote-only/read-only 路径，不证明 Jupiter 的 Gate 2 可执行能力。连续 24 小时
Shadow 仍在运行，必须等待同一份有效日志出现 `shadow_complete=true` 后验收。
新 candidate 隔离构建 `999bbe8375ce` 已在 `18889` 使用临时钱包通过只读 HTTP smoke、
Jupiter 报价、schema 1–6 和重启后指标回读；18887 保持原二进制与原采样窗口。

## 运行复核记录（2026-11-05）

candidate P0/P1 门槛已在 Cargo 1.99.0 下重新通过（93 项 Rust 测试、锁定检查、release
构建、simulate-only guard、审计指纹和 current/18788 不变检查）。bot 的独立 P4 candidate
在主机回环测试环境中通过 94 项测试并完成 release 构建；沙箱内曾出现的 8 项本地 HTTP
mock 失败属于监听权限限制，不能作为代码失败证据。18887 staging HTTP smoke、SQLite
只读回读和 DFlow unavailable 检查已通过。Jupiter Shadow 仍以单个 systemd sampler
连续采样，窗口结束前不改变服务或重启 sampler。

## 当前基线

已经完成：

- SDK 11.0.1 candidate 独立编译，锁文件固定；最新构建门槛和测试结果见
  `AUDIT_MANIFEST.json` 与 `scripts/verify_candidate_gates.sh`。
- `BackendCatalog::sdk11()` 已接入启动流程。
- `SdkBackend` 已接入 `/v1/pools/resolve` 和 `/v1/cache/warmup`。
- candidate 使用独立 SQLite 副本、独立 `.env` 和独立 passphrase 链接。
- candidate 强制模拟模式，实盘请求和 Meteora DBC 请求均被拦截。
- `SimulationReport` 已进入 HTTP 响应和 `simulation_json` 持久化字段。
- `ExecutionBackend` 已由 `SdkBackend` 实现，resolve/warmup 已通过 trait dispatch。
- `BackendCapabilities.supports_live` 已固定为 `false`，candidate 不允许误报 live。
- route registry 已接入 route、quote、trade-plan 的 SDK 无关校验；新增三个 route API 路由。
- 新增进程内短期 `RouteStore`：resolve 返回的已验证 route 必须先写入服务端 store，quote/simulate 只接受与服务端快照完全一致的 `route_id`，请求体不能伪造验证标记或修改池/mint 元数据。

尚未完成：

- 执行器的统一 submit 仍未接通；当前只有 Raydium CPMM exact-in quote 和 buy simulation adapter，未覆盖的 route quote/simulate 会明确返回 `501`，不伪造结果。
- route/quote 已接通 HTTP contract 和边界校验；SDK 已登记的 Pump/PumpSwap、Bonk、StonkFun/StonkFunSwap、Meteora DAMM v2、Raydium AMM v4/CPMM 均有明确提示的单池链上核验，短期进程内 route store、Jupiter quote 持久化和聚合器 preflight/audit 证据已完成。StonkFun Via-SOL 双跳快照已在 `/v1/trades` 构建前核验；通用 SDK/聚合器决策、双跳执行模拟证据和其它协议的 quote/simulate adapter 仍待完成。
- Quote 请求已增加 `slippage_bps` 并计算 `min_output_units`；Jupiter quote、submission、状态、preflight/audit 和 SOL 余额快照已按 wallet 持久化。只读 compare 可在余额不足时继续报价；旧 Jupiter quote 和 `/v1/trades` Jupiter 买入共用有资金要求的 quote 创建函数，preflight/模拟会重新核验当前余额、费用上限和模拟扣款余量。
- SDK backend 的 quote/simulation 已按 `wallet_id` 选择对应钱包 client；余额快照和 owner 归属检查仍待完成。
- 当前 SimulationReport 对 SDK 原生模拟只记录基础结果，尚未稳定提取真实的 units、logs、输入输出余额、费用和最小输出。
- route quote/simulate API、过期校验和绑定校验已经接入；Jupiter 持久化 quote 已完成，完整协议矩阵仍待完成。
- CLMM、DLMM、Whirlpool、LaunchLab、DAMM V1、DBC 没有可执行 adapter。
- DBC legacy backend 尚未实现，当前只会拒绝。
- 没有完整的每协议 fixture 矩阵、Token-2022 回归和主网模拟样本。
- 已有隔离 `18887` 的运行级 HTTP smoke 与重启后 SQLite 回读；尚无完成的 24 小时
  Jupiter Shadow、canary、current 切换和正式生产 systemd 发布流程。

上述列表中的聚合器部分已在 candidate 内完成 Jupiter provider contract、durable quote/submission/preflight/audit、钱包 SOL 余额快照、Shadow 延迟汇总、provider binding 校验、HTTP 错误脱敏和旧 `/v1/trades` simulate-only guard；仍未完成的是完整可执行 HTTP 集成验证、真实持续 24 小时 Shadow、SDK 原生路径余额快照和任何 live/canary。`bot_trading` 已接入 capabilities/Shadow 只读诊断，提交开关固定为 false 时 readiness 明确阻断 Live。
- Router SDK 还没有接入；Alpha 仍应保持 Paper。

Router SDK 评估结论：它属于可选原生 Router execution backend，不属于 aggregator provider。
当前不接入 candidate registry，不构建、不签名、不广播；后续若推进，只能从 route replay /
observe / simulate 开始，并在独立 Router Program 部署、账户语义审计和模拟 fixture 完成后
再评估 capability。

## 阶段 0：冻结基线和发布安全

这一阶段每次 SDK 升级前都要执行，第一次候选已经做过，但切换前必须重新生成带时间戳的记录。

产物：

- current、previous、candidate 的路径、二进制 SHA-256、Rust/Cargo 版本和 Cargo.lock 哈希。
- 9 个钱包的 `wallet_id -> pubkey -> enabled` 映射；当前数据库 9 条记录中 8 条启用。
- `wallets.sqlite`、`trades.sqlite`、`conversions.sqlite` 的 SQLite backup 和 WAL checkpoint 记录。
- `.env` 的非敏感配置快照；passphrase 只保留受限文件，不写入日志或文档。
- 当前 `bot_trading` 的 `TRADE_SERVICE_URL=http://127.0.0.1:18788` 和隧道状态。

进入下一阶段的条件：备份可读、钱包公钥完全一致、没有新 live 进程、现有隧道仍可用。

回滚：删除候选启动进程即可；不恢复数据库到旧副本，也不改动 current。

## 阶段 1：完成稳定 backend contract

目标是让上层只依赖 SDK-neutral contract，SDK 11 类型只留在 `backend/sdk.rs` 和
`sdk_client.rs` 内。

实现内容：

1. 为 `SdkBackend` 实现 `ExecutionBackend` trait。
2. 将 `BackendCatalog` 从“协议名称表”扩展为 `protocol_id -> backend instance -> capabilities`。
3. 定义稳定的 `ProtocolId`、`VerifiedRoute`、`Quote`、`TradePlan`、`SimulationResult`、
   `SubmissionResult` 和 `BackendCapabilities`。
4. `AppState` 只保存 backend registry；`executor.rs` 不再直接调用 `wallet.buy/sell`。
5. 保留 `execution_method=pumpapi` 作为显式 backend，不根据事件字符串隐式切换。
6. 将 capability 拆成 `compiled`、`supports_simulate`、`live_allowed` 三类状态；
   candidate 即使编译了 live builder，也不能因为 `supports_live=true` 被误报为可实盘。

验收：

- `cargo check --locked`、`cargo test --locked` 通过。
- `/v1/capabilities` 的 backend 名称和实际 dispatch 一致。
- HTTP JSON、SQLite 和日志中不出现 `DexParamEnum`、`MeteoraDammV2Params` 等 SDK 类型。
- 未注册协议在 dispatch 前被拒绝，错误包含稳定的 `protocol_id`。

回滚：registry 继续指向当前 SDK 6.0.2 backend；不改变数据库 schema。

## 阶段 2：VerifiedRoute 和 Quote 生命周期

目标是把“观察到的池”变成有 slot、owner、token program 和过期时间的可验证路由。

新增内部流程：

```text
resolve candidates -> verify owner/program -> read pool state -> verify mints/token program
-> assign route_id/validated_slot/expires_at -> quote -> bind wallet/side/amount -> simulate
```

实现内容：

- 新增 `POST /v1/pools/resolve-candidates`、`POST /v1/routes/quote`、
  `POST /v1/routes/simulate`。
- `VerifiedRoute` 保存 protocol、program、pool、mint、input/output/quote mint、token program、
  validated slot、expires_at、买卖能力和 backend。
- Quote 绑定 wallet、route、side、整数金额、slippage 和过期时间。
- Pump/PumpSwap route 已在 SDK adapter 中读取 pool、mint、quote mint 的 owner、token program 和 slot；Raydium CPMM quote 会读取完整 pool/vault/mint 快照并使用 SDK fee/Token-2022 公式。其它协议仍待独立 adapter。
- 过期 route、Quote mismatch、金额单位错误、余额不足和 capability 不支持直接拦截。
- 只缓存短期 route/quote；缓存失效后重新读取链上状态。

验收：

- 篡改 pool_id、mint、quote_mint、wallet_id、side 或 amount 会失败。
- 过期 route 和过期 quote 会失败，且不会进入签名路径。
- SOL、WSOL、USDC、USD1 和自定义 token 使用独立整数单位。
- 同一幂等键重复请求只返回原订单，不产生第二次执行。

回滚：关闭新增 route API，保留现有 `/v1/trades` simulate 入口；不清理旧订单。

## 阶段 3：完成 SimulationReport

当前报告已能记录 `ok/protocol/source/logs`，下一步要让它反映真实 RPC 模拟结果。

实现内容：

- 从 SDK 原生模拟结果提取 `units_consumed`、logs、错误、fee、payer debit、目标 token delta、
  目标账户余额和 simulation slot。
- 明确模拟语义：模拟交易的 `signature` 对外返回 `null`；内部交易指纹不能当作已广播签名。
- `SimulationResult` 增加 schema version、input units、expected/min output、fee 和 route_id。
- 已加入 `002_trade_simulation_report.sql` 和 schema version 2 bookkeeping；仍需在 Cargo 兼容环境中完成旧数据库、WAL、重启回读和重复 migration 验收，禁止降级数据库。
- SDK、PumpAPI、Router 的模拟结果统一转换成同一个报告格式。

验收：

- 成功模拟和失败模拟都返回结构化报告。
- 报告写入 SQLite 后重启服务仍可读取。
- simulation 失败不会产生 signature，不会占用 live 幂等键。
- 真实 RPC 错误、账户缺失和路由过期能区分记录。

同时更新 `PUBLIC_TRADE_API.md`，记录 `simulation` 响应字段、`simulation_json` migration、
模拟失败的错误语义和重启后的读取行为。

## 阶段 4：SDK 11 第一批协议模拟

只处理 SDK 11 已有稳定 builder 的协议，每个协议独立开关和 capability：

| 协议 | 必须完成的模拟 |
|---|---|
| Pump | buy、sell、bonding curve 状态、SOL/WSOL 单位 |
| PumpSwap | buy、sell、pool owner、quote mint、迁移状态 |
| Bonk | buy、sell、WSOL/USD1 路径和 launchpad 状态 |
| Raydium CPMM | buy、sell、creator fee、Token-2022 兼容性 |
| Raydium AMM V4 | buy、sell、vault/mint 校验 |
| StonkFun/Swap | buy、sell、platform config、必要的 SOL hop |
| Meteora DAMM V2 | buy、sell、exact-in、最小输出和费用 |

每个协议建立 fixture：有效池、错误 owner、错误 mint、过期状态、错误 quote、Token-2022、
买入模拟、卖出模拟、幂等重放、重启回查。主网 fixture 只保存公开账户和模拟响应，不保存私钥。

退出条件：每个协议买卖模拟成功率、错误分类、p50/p95 延迟和报告字段均有记录；任何协议
fixture 未完成时，该协议保持 `observe` 或 `simulate`，不能进入 live。

## 阶段 5：Observe/Simulate 扩展协议

按独立 adapter 逐个推进：

1. Raydium CLMM：tick array、bitmap、sqrt price、流动性边界。
2. Meteora DLMM：bin array、active bin、bin bitmap、状态过期。
3. Orca Whirlpool：tick array、sqrt price、token program。
4. Raydium LaunchLab：platform config、迁移前后池类型。
5. Meteora Launchpad：账户布局和 mint/quote 校验。
6. Meteora DAMM V1：独立布局审计后再做模拟。

这些协议先只允许 observation、route resolve 和 simulate。每个协议的 live capability 必须
单独注册，不能由全局 `ENABLE_SIMULATE` 推导。

## 阶段 6：DBC legacy backend

DBC 不与 SDK 11 vendor 混编。单独选择以下实现路径之一：

- 从 SDK 6.0.2 release 中抽出最小 DBC adapter，保留自己的 Cargo.lock；或
- 使用旧 Trade Service 的 DBC backend 作为独立进程，通过内部 localhost API 调用；或
- 重新实现 DBC account parser、参数计算、指令 builder 和模拟器。

实现要求：program/pool owner、bonding curve 状态、mint/quote、token program、曲线完成状态、
买卖参数和模拟结果必须由 DBC backend 自己验证。SDK 11 backend 继续把 DBC 标记为 unavailable。

退出条件：DBC 买卖模拟、stale route、错误 owner、幂等重放和重启回查通过；否则长期保持 observe。

## 阶段 7：SOL 两跳和转换统一

只允许经过验证的原子路径：

```text
SOL/WSOL -> Quote Mint -> Target Token
```

两跳的每一跳都要有独立 route、quote、过期时间和 simulation report。明确 SOL lamports、WSOL
units、USDC/USD1 units 的换算；禁止把 `amount_sol` 直接当作 quote token units。现有 WSOL conversion
的 quote、simulate、submit 保持独立幂等和状态回查，不能与普通 trade 混用 request_id。

退出条件：两跳原子模拟成功，任意一跳失败都会拒绝整笔请求，且没有签名或部分提交。

## 阶段 8：Router SDK 作为可选 backend

Router SDK 在第一批 native SDK 模拟通过后再接入。它只实现 `ExecutionBackend`，不能接管钱包、
幂等、审计、订单状态和广播权限。

比较项目：route 命中率、指令数量、交易字节数、模拟成功率、p50/p95 延迟、费用、失败分类、
Token-2022 支持和 SOL 两跳原子性。初期 `router_backend=disabled`，只做 replay 和 simulation。

只有当 Router 在相同 route/quote 输入上不降低模拟成功率，并且能通过全部安全检查时，才允许进入
Shadow；Router Live 必须晚于 SDK 11 第一批协议 canary。

## 阶段 9：bot_trading、首两笔和 Alpha 接入

- `bot_trading` 继续保留首两笔快速跟随和钱包 A 规则；Trade Service 只接收稳定业务请求。
- PumpAPI/ZMQ bridge 输出 raw、observation、normalized 三层数据；协议字符串只作为候选提示，
  不作为交易授权。
- Alpha 继续 Paper；先记录 Alpha signal、评分、命中和模拟结果，不进入 submit。
- candidate Shadow 时只允许一个指定钱包和一个协议，固定幂等键，策略层设置 dry-run。
- 不同时开启 Alpha Shadow、Router Shadow 和 SDK 首次切换，便于归因。

Shadow 需要记录事件时间、route resolve、quote、simulation、决策、请求、持久化和回查延迟。

## 阶段 10：24 小时 Shadow 和性能验收

### 阶段 9.5：隔离 staging 运行验证（新增）

在启动 24 小时 Shadow 前，先取得允许本地监听的 staging/VM 环境。使用 candidate 独立
端口、独立数据库和 simulate-only 配置，完成 HTTP smoke、重启回读和 DFlow unavailable
验证。当前沙箱若拒绝 `TcpListener::bind`/netlink，只能记录为环境阻塞，不得绕过生产隧道。
HTTP smoke 使用 `scripts/staging_http_smoke.sh`，默认访问 `127.0.0.1:18887`，只执行一次
DFlow read-only compare 来验证 `unavailable` Shadow 记录，不启动进程、不构建、不签名或提交；可通过 `CANDIDATE_STAGING_URL` 和
`TRADE_SERVICE_LOCAL_API_TOKEN` 指定隔离实例地址与认证。
进程重启回读使用 `scripts/verify_staging_persistence.py`，它以只读模式检查聚合器表结构和
reservation 恢复状态，不执行 migration 或写入数据库。

Shadow 至少持续 24 小时，覆盖真实 PumpAPI/ZMQ 事件但不广播。指标包括：

- ready、RPC、blockhash 和 wallet mapping；
- observation 命中、route reject、quote reject、stale route；
- simulation success/failure 和错误分类；
- duplicate、database write、restart recovery；
- 端到端 p50/p95/p99 延迟、CPU、内存和 RPC 错误率。

同时进行历史回放和受控压力测试，确认 orjson/数据流优化不会改变事件顺序、首两笔计数和幂等键。

退出条件：无未解释的 unknown 订单、无钱包公钥漂移、无重复执行、数据库持续健康，且错误率和
延迟达到预设阈值。任何指标不达标，回到 simulate 修复，不进入 canary。

## 阶段 11：本地服务切换准备

准备两个明确的 systemd unit：

- `trade-service-candidate.service`：固定 candidate 路径、candidate `.env`、18787、强制 simulate。
- `trade-service-local.service`：只指向被选中的 `current` release，使用显式 `EnvironmentFile`，
  默认 live 关闭。

unit 必须明确 `WorkingDirectory`、`EnvironmentFile`、`ExecStart`、`Restart`、`LimitNOFILE`、
`TimeoutStartSec`、日志目录和 `systemd-analyze verify` 结果。不要直接启用仍引用旧
`/root/http/sol-bot` 路径的旧 unit。

切换顺序：停止 candidate -> 原子切换 `current` symlink -> 启动 local unit -> health/ready/
wallets/capabilities/simulate -> bot 保持停止或 dry-run -> 审核后才改变 bot URL。18788 隧道
始终保留作为回退入口。

## 阶段 12：单协议 Canary

每个协议分别设置：wallet allowlist、pool allowlist、单笔金额上限、频率上限、日总额上限和
独立 live 开关。每笔 live 先使用同 route/quote 做 simulation，再签名和广播；签名后必须用
原 request_id、原 signature 回查，不得换幂等键重试。

先完成买入 canary，再完成卖出 canary；Pump、PumpSwap、Raydium、StonkFun、DAMM V2 分开推进。
任何公钥不匹配、quote 方向异常、token program 不符、unknown、模拟失败或错误率异常，立即
关闭该协议 live 并保留订单调查。

## 阶段 13：按协议 Live 和回滚

Live 只通过 capability + protocol allowlist + wallet/pool limits 三重条件开启。DBC、CLMM、DLMM、
Whirlpool、Launchpad、Router 在各自验收完成前保持 observe/simulate。

回滚只做以下动作：停止当前 local unit、切回旧 current symlink 或 backend、必要时恢复 bot 到
18788 隧道、重新执行 health/ready。数据库不降级，unknown 订单保留并按原 request_id/signature
回查。不能通过换幂等键创建补偿交易。

## 阶段 14：后续 SDK 升级矩阵

运行目录保持“三个角色”，不会无限复制运行实例：

```text
current  -> 当前选中 release
previous -> 最近一次可回滚 release
candidate -> 下一版本验证 release
releases/ -> 不可变历史归档
```

SDK 12 或更新版本只新增 candidate 和对应 lockfile，重复执行阶段 1、2、3、4、10 的 contract、
模拟和 shadow 验收。通过后再原子替换 current/previous。数据库、钱包、bot API 和首两笔策略
保持不变。

## 阶段门禁总表

| 门禁 | 必须满足 |
|---|---|
| Contract | trait、内部类型、API JSON、错误分类通过 |
| Route | owner/program/mint/token program/slot/expiry 全部验证 |
| Simulation | 买卖、报告、失败、重启和数据库回读通过 |
| Protocol | 每协议 fixture、Token-2022、幂等、stale route 通过 |
| Shadow | 单钱包单协议至少 24 小时，指标无未解释异常 |
| Canary | 单 pool、小金额、限频、先模拟后广播、原签名回查 |
| Live | capability、allowlist、风控和回滚路径全部可用 |

任一门禁失败，停留在当前阶段修复，不跳过阶段，也不把 observe/simulate 状态标成 live。
