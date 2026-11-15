# TS-04-E1 普通交易执行后端端口

日期：2026-11-13。状态：新增 12 项离线测试，全套 270 → 282，通过；无编译警告。未升级 SDK/依赖/vendor、构建 release、部署、重启、连接真实 RPC、读取生产钱包/数据库或发送交易。

## 已实施

- `application/ports/ordinary_execution.rs`：只接受项目 `TradeCommand`、32-byte `Blockhash`、确认/MEV 等执行上下文，返回项目 `TradeExecutionResult`。
- `OrdinaryExecutionBackend::resolve` 一次解析钱包，返回不暴露 signer、raw RPC、SDK params 或通用签名方法的 `OrdinaryExecutionWallet`。
- `adapters/sdk/ordinary_execution.rs`：持有现有 `WalletRegistry`；解析后私有持有原 `Arc<SdkTradeClient>`，SDK/PumpAPI 参数 mapper 和买卖调用收回 adapter。
- `AppState` 注入真实 backend，构造仅保存现有组件和 PumpAPI 启动配置，不创建 SDK client/钱包/新网络连接。原配置只在启动加载，本轮不引入热更新。
- Executor 普通 SDK/PumpAPI 分支不再导入其参数 mapper/对象或调用 SDK client；只选择 port 和传递项目执行上下文。
- legacy 聚合分支保持原流程和 SDK wallet 获取，先于普通 backend 分支；没有把无签名前账本的旧聚合自动改成新独立 Quote 提交。

这是具体普通交易调用的能力收口，不是 WalletRegistry、全部 HTTP/Application 或所有 SDK/raw RPC 访问已隔离。

## 顺序与兼容性

保留原执行顺序：

1. 原禁止广播策略；SDK 整体流水线（含 SDK simulate）保持禁播时拒绝，PumpAPI simulate 保留原允许分支。
2. 内存更新 building_tx，按原 durable/shadow 规则写入。
3. 聚合则走原钱包解析；非聚合只调用一次普通 backend.resolve。
4. 找不到钱包返回原 failed/error，并按原规则持久化，不改成 blockhash 错误。
5. PumpAPI 先保存 submitting，再调用 port，不要求本地 blockhash。
6. SDK 在已解析钱包之后取 fresh blockhash；空/过期返回原 rejected/error。通过后保存 submitting，再调用 port。
7. 原状态分类、原签名/错误、结束持久化与元数据缓存更新保持。

解析后的真实句柄持有 `Arc`，不会变成“先查存在，执行时再查询注册表”。即使注册表在持久化等待期间删除/替换该项，仍使用原已解析对象，与旧流程一致。

missing-wallet 的重复代码提为 `fail_missing_wallet`；原分支无论持久化成功与否都返回，因此 helper 保留日志/更新调用并忽略布尔返回值，不继续执行。

API 路径、请求/响应 DTO、normalization、幂等 key/hash、账本格式/schema、协议许可和 98 组结果分类指纹不变。没有对历史失败订单重提或生成新 key。

## 新增验证

`executor/execution_tests.rs` 11 项，调用真实 `execute_trade`，使用空真实钱包注册表和受控 fake backend，不是直接伪造最终订单结果：

| 场景 | 证据 |
| --- | --- |
| SDK/PumpAPI × 买/卖 × Live/模拟 | 原 DTO 校验/command 转换，实际编排和临时 durable SQLite；building/submitting 顺序，单次解析/执行；金额、Quote、滑点、小费、模式、hash/确认/MEV context；原签名/状态保存和缓存更新 |
| 缺失钱包 | 优先于缺失 blockhash；failed 和原错误，0 backend call |
| 空/真正过期 blockhash | SDK 解析后拒绝、0 执行；PumpAPI 两者都不受影响 |
| 解析后 fake 注册项删除 | 句柄继续使用、1 次解析/执行；真实 adapter 的原对象持有由私有 Arc 类型保证，未构造真实 SDK 钱包做竞态演练 |
| 禁播 | SDK Live/模拟与 PumpAPI Live 在解析前拒绝；PumpAPI simulate 保留原允许行为，无真实签名/交易 |
| durable building/submitting 写失败 | 两种 method 均用临时 SQLite trigger 注入实际 update 错误；前者 0 解析/执行，后者 1 解析/0 执行，核对 DB 与内存各自原状态 |
| 缺失内存订单 | 0 解析/执行 |
| 不确定/明确失败/调用错误 | original signature/status 保留，单次编排无自动重试 |
| 执行后数据库关闭 | 仍只执行一次，内存保留原签名；持久化不健康，不能据此保证重启仍保存签名 |
| shadow writer 不可用 | 两种 method 均保持旧内存执行语义，不误用 durable 拒绝策略 |
| legacy 聚合优先级 | 合成内部标志组合在空注册表处按原错误结束，普通 backend 0 解析/执行；不是聚合成功路径验收 |

真实 adapter 另有 1 项：构造及不存在钱包解析不会初始化 SDK、创建钱包或调用网络。

测试辅助 `BlockhashCache::with_snapshot_for_tests`、`TradePersistence::pool_for_tests/unavailable_shadow_for_tests` 全部为 `cfg(test)`，生产构建中不存在。SQL trigger/关闭操作只作用于新建 `sqlite::memory:`，没有新生产迁移或配置。

基础 HTTP fixture 继续使用“必须显式注入”的 panic backend，不捕获 Config Arc，保留既有配置测试的 `Arc::get_mut` 语义；旧拒绝测试不能意外接触真实执行。

## 验证命令

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet executor::
cargo test --locked --offline --quiet
git diff --check
```

全套：282 passed、0 failed、无编译警告。Cargo.toml、Cargo.lock、现有 release 指纹仍与 `SDK_PARAMETER_FIXTURES.md` 一致，vendor diff 为空。

## 仍未完成

2026-11-13 续作 TS-04-E3：普通聚合钱包解析/quote→bind→legacy-submit 已迁至项目端口与 adapter，Executor 不再取 SDK 钱包或 wire DTO；新增 14 项、全套 359 项通过，见 `ORDINARY_SOL_BUY_EXECUTION.md`。下述描述保留 E1/E2 当时范围，不代表本轮已完成签名前账本或所有 Handler 隔离。

2026-11-13 续作：以下前两项的只读已加载身份 port 与基础真实 Router fake 成功/幂等/批量/故障回归已按 TS-04-E2 完成，新增 12 项、全套 294 项通过，见 `WALLET_ADMISSION_AND_HTTP_TRADES.md`。普通接受/持久化 DTO 耦合、完整模拟矩阵和签名前账本仍未完成，以下保留本批结束时的背景。

- POST `/v1/trades` 接受阶段仍从真实 WalletRegistry 获取 SDK client 以确认身份，故本批 fake 测试从已接受订单和校验后的 command 开始，不是完整 HTTP 成功验收。
- 下一批 TS-04-E2：隔离“当前已加载钱包”的只读身份/默认 ID 查询，不能用可能包含未加载/已禁用项的仓库记录冒充可执行钱包；接入真实 Router 的 fake 成功、重复/冲突、批量和拒绝计数测试。执行前仍独立解析原钱包对象，不改幂等/校验顺序。
- 后续继续把普通接受、持久化和执行编排从 AppState/DTO 依赖分离；本 port 不代替这些职责。
- 普通 SDK/PumpAPI/legacy 聚合仍缺完整发送前签名 journal。执行结束后写库失败的测试明确暴露该缺口，不把内存签名当作恢复保证；必须继续独立处理，不能自动重发。
- PumpAPI 模拟的内部签名、不同资金路径故障注入、完整 VerifiedRoute/协议报价/SDK provenance 与真实链上验收保持独立门槛。

## 2026-11-15 原执行编排与资源隔离

本批继续原 TS-04/08/09、NP-02/03，不另立项。源码及七项新合同已在 26445c5 /
Actions 43707853050 完整验收，artifact 14159283766 下载核对：七项专项、完整并发
1473（112.34 秒）/串行 1473（209.87 秒）、全部步骤通过。不套用父批或 E1 的结果。

- `application/ordinary_runtime.rs::OrdinaryExecutionService` 承担原 executor 编排，
  仅借用项目 ports 和四值 Settings；不依赖 AppState、Config、具体存储/缓存、SDK/RPC
  或 signer。根 `executor.rs` 保留原入口和 claims 组装，测试也经过同一生产服务。
- `OrdinaryExecutionStorage` 仅投影原 `ordinary_execution_claims()`，仍使用原 pool 的
  journal；`TradeExecutionIndex` 仅委托原 `TradeStore::update`，不新建任何状态或权威。
- `CachedBlockhashRead` 仅委托原 `get_fresh`，不刷新、不新增 RPC 或延长有效期。
  Config 只复制 broadcast、blockhash_max_age_ms、wait_tx_confirmed、mev_submit_verify_ms，
  不新增值验证、配置或热更新。原不可变 Config 的四个值不会在执行期间改变。
- 原 claim/禁播/聚合优先级、building/submitting 写入、SDK journal 拒绝、结果分类及
  cache 时点保持；日志保留 `executor` target/字段，纯 detail 格式化移至 Application。
  原 98 组指纹和全部旧断言保留。既有 PumpAPI port 分支只搬迁，不修改实现或恢复开发。

新增六项 Application 合同：无 AppState 的 SDK 顺序/context/原缓存、聚合与 journal
parent 不重复镜像、禁播/写失败/缺 hash/缺钱包、Unknown 不换后端重试、取消不伪造结果、
四值投影；另有一项 cache 空/过期/时钟回退对照。资源为 fake journal/index/backend/cache，
SDK journal 关联类型从未实例化、访问即 panic；配置投影用原内存 fixture，不加载生产数据。

本机 test check `20261115T110647Z-210265` 35 秒/RSS 2,195,968 KiB、bin check
`20261115T110752Z-210600` 14 秒/RSS 980,732 KiB 均 exit=0；格式/policy 七项通过，
原七组生产警告保留。首次缺失的测试作用域导入已修复，未修改旧期待值。
CI 增加 `ordinary-runtime` 专项，保留原全部专项/完整并发/串行，本机不链接测试。

根入口仍持原 AppState/claims 生命周期；不宣称全部后台任务已追踪/排空、SDK worker
可取消或 Memory/Shadow 具有 durable 保证。新 fake ports 测试不是双假构建后端准入、
全协议授权、真实节点或生产部署验收。SDK/vendor/依赖、API/持久化合同及运行配置不改。
