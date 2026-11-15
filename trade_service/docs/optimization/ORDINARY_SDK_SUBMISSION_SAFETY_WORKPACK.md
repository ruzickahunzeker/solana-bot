# NP-01 普通 SDK 与 legacy 聚合提交安全工作包

最新增量（2026-11-14）：NP-01-D1B2 默认关闭的有界枚举/只读扫描、原证据恢复、007 部分索引、单次 HTTP/局部预算/429 冷却及 worker 生命周期已有源码。新增七项回归后，专项 27、完整回归 1045 项通过；已在测试子进程实际验证 SIGTERM/SIGINT，不操作服务 main。先复现前两个 URL 重复导致备用丢失，再修为前两个独立节点；文件重开三状态的成功/失败及无结果/查询错误均不新增 claim/签名/发送。见 `ORDINARY_PENDING_OBSERVATION_WORKER.md`。没有真实节点/HTTP transport 集成、生产迁移或全服务 RPC 预算验收，完整 B/C/D/E 仍未完成，不恢复 PumpAPI 对接；本轮持续目标已由用户恢复 active。

最新增量（2026-11-14）：NP-01-D1B1 已将实际观测核心改用不透明 context，恢复 port 不需要 AdmittedTrade/ExecutionOwner 或钱包运行时；原请求/父行/claims/quote/preflight/签名字节的不可变证据摘要逐次核对，已有 signed 中断可显式只读恢复，原 owner 绝不重新签发。新增九项，专项 9、并发/串行各 1018、HTTP 198、production check/fmt/diff 通过，仍五组警告；无新 migration。见 `ORDINARY_RECOVERY_OBSERVATION_CONTEXT.md`。未接启动扫描/持续调度，GET/重复不变；全局 RPC 预算、完整 B/C/D/E 仍待续，不恢复 PumpAPI 对接。

最新增量（2026-11-14）：NP-01-D1A 已在 Durable Live 普通 SOL 聚合 record_broadcast 成功后接单次原签名观测；专用只读 reader/journal port、3 秒 RPC deadline、006 原 claims nullable 最终证据与同事务父行 CAS/读回，只有明确 finalized 推进既有 confirmed/failed。新增 20 项，专项 20、并发/串行各 1009、HTTP 198、production check/fmt/diff 通过，仍五组警告。见 `ORDINARY_POST_SEND_OBSERVATION.md`。普通 GET/重复不发 RPC；没有跨重启/持续确认或 signing/signed 中断恢复，完整 D/B/C/E 仍未完成，不恢复 PumpAPI 对接。

最新增量（2026-11-14）：NP-01-B3a 在共享聚合 prepared 对象内保存原 Quote 到期时间，在实际释放待签消息前重查；先以无 signer 回归复现原实现接受过期消息，再完成三项新增测试。并发/串行各 989、HTTP 196、production check/fmt/diff 通过，实际五组警告与 TS-09-A 相同。见 `QUOTE_SIGNING_FRESHNESS.md`。只补签名前时效，不代表完整最终消息授权、普通对账、legacy 或其他 backend 账本已完成，不恢复 PumpAPI 对接。

最新增量（2026-11-14）：NP-01-C2b Durable Live 普通 SOL 聚合的原 owner、同 authority 签名落盘/独立读回与受控原字节发送已通过源码/离线验证，新增 20 项，并发/串行各 929、HTTP 173、ordinary 专项 87、production check/fmt/diff 通过，仅两组原兼容警告。见 `ORDINARY_DURABLE_SIGNED_JOURNAL.md`。以下 C2a 及初轮表格保留历史含义；其中“全部仍走 legacy/无普通账本”不再适用于该 Durable 子路径，但仍适用于未迁移模式/backend。完整 B/C/D/E 未完成，不恢复 PumpAPI 对接。

日期：2026-11-13。状态：第一轮调用能力复核、NP-01-A1/A2a/A2b/B1/B2a/B2b、E0、C1 原输入及 C2a 普通聚合原子执行占用各自源码/离线子项通过；最新并发/串行各 909、HTTP 170 和 check/fmt/diff 通过，详见 `ORDINARY_EXECUTION_CLAIMS.md`。完整实现和安全验收未完成。只针对当前 vendored SDK 6.0.2 和普通交易，不恢复 PumpAPI 对接开发，不升级依赖或修改 vendor，不操作生产配置、钱包、数据库或服务。

## 初轮与前阶段调用证据

以下保留初轮风险证据及 A2a 更新；旧私有 helper 在 A2b 已移除，B1 又移除只读 DAMM probe 的含 payer 构建，当前调用与限制以 `SDK_LANE_ENTRY_AND_UNCERTAINTY.md` 和 `DAMM_PUBLIC_PROBE_COMPILATION.md` 为准。

| 位置 | 当前事实 | 不能据此作出的保证 |
| --- | --- | --- |
| `src/executor.rs::execute_trade` | BuildingTx/Submitting 记录在调用 SDK 之前持久化，原 reservation/key 复用 | 普通记录不含签名前授权或广播前已签名字节；不是 signed journal |
| `src/adapters/sdk/ordinary_execution.rs::ResolvedSdkWallet::sdk` | 项目 command 映射后调用原 client buy/sell | 项目 port 隔离不等于内部已拆 build/sign/send |
| `src/sdk_client.rs::buy_on_lane` / `sell_on_lane` | 最终使用 SDK buy / sell_by_percent 组合入口；错误、签名仅在调用返回后取得 | 不能在未知执行阶段的 Err 上证明“未签名/未广播” |
| `vendor/sol-trade-sdk/src/client/mod.rs::buy` / `sell` | 调用具体 executor.swap；buy 还可执行 SDK risk gate | 不能绕过现有参数/风险/费用逻辑自行替换流水线 |
| `vendor/sol-trade-sdk/src/trading/core/traits.rs` | InstructionBuilder 公开分方向的指令构建方法；SwapParams 包含 payer 等 SDK 资源 | 不是无 signer 的完整交易计划接口；指令 builder 可用不代表所有协议都可安全独立构建 |
| `vendor/sol-trade-sdk/src/trading/core/executor.rs::swap` | 构建协议指令、middleware、模拟或并行发送都在同一调用中 | 缺少服务侧广播前 durable 回执检查点 |
| 同文件 `simulate_transaction` 与 `trading/common/transaction_builder.rs` | 模拟也调用带 payer 的签名 transaction builder，再用 sig_verify=false 模拟 | simulate=true 不是无签名能力；禁止广播策略保守拒绝 opaque SDK 模拟仍有必要 |
| `src/sdk_client.rs::simulate_damm_v2_sell_output` | 初轮发现报价探测调用 new_signed_with_payer；A2a 已将该 RPC 报文改成公开输入/零签名 helper，仍使用原 SDK 指令 builder | builder 的 SwapParams 仍含 payer 资源，完整无密钥边界、最终 SDK 预检与 opaque simulate 仍未收口；见 `SDK_PREPARATION_AND_DAMM_PROBE.md` |
| `src/sdk_client.rs::buy` / `sell` / `verify_lane_submission` | 某些带签名失败、签名短期不可见及 Err 会继续下一 lane；核验 RPC 错误可直接向上返回 | 可能重新构建不同交易；短期不可见不是原交易失效，Err 也不证明没发出 |
| `src/sdk_client.rs::map_sdk_result` | SDK 返回多签名向量，旧映射只取第一个 | 普通 TradeResponse 单签名不足以审计所有后端内部提交；不能靠增加一个字段就宣称解决重复执行 |
| `src/application/legacy_quote_submission.rs::execute` | 初轮 prepare -> sign -> broadcast，无 journal，Err 丢签名；E0 已在广播 Err/不同回执时保留本地原签名与 Unknown | 仍没有广播前持久化原字节，签名后取消/崩溃或最终写库失败可丢恢复证据 |
| `src/adapters/sdk/ordinary_sol_buy.rs` / `application/ordinary_sol_buy.rs` | 普通 SOL 聚合仍走 legacy 而非独立 quote 提交账本；E0 将明确 Live Unknown 交给项目 typed 分类，不再靠错误文字降级 Failed | 独立 quote journal 不能替代普通唯一 journal；E0 不是整个 C/D/E 迁移 |

初轮表格仅为源码证据，没有执行 SDK 初始化、真实签名、供应商 HTTP/RPC 或广播。后续 A2a 明确改变了报价 probe 的签名槽并增加局部额外 signer 拒绝，其余 lane 回退、最终模拟/状态行为保持；不能把历史“未修改模拟”理解为新阶段没有安全变化。旧 HTTP 名称在 Handler 移动后以实际新路径为准。

## 后续实施与准入

NP-01-A1 已在 root 局部纯函数阶段采集三项参数/费用测试和 2,016 组完整字段摘要，随后冻结摘要并移动适配层；移动后三项固定摘要及全套默认并发/串行各 792、HTTP 158、check/fmt/diff 通过，详见 `SDK_FINAL_PARAMETER_BOUNDARIES.md`。这只完成 A 的一部分，不包含异步准备/失败与实际 lane/middleware 副作用、无签名构建证据，更不代替 B-E。

NP-01-A2a 已将原异步准备流程接有限 source 并以公开 fake 验证顺序/失败/取消；DAMM 报价 RPC 使用零签名 helper。八项准备与六项专用 probe、A1 固定摘要三项通过；最终并发/串行各 806、HTTP 158、check/fmt/diff 通过，见 `SDK_PREPARATION_AND_DAMM_PROBE.md`。SDK builder 仍接 payer 资源，最终 SDK simulate、lane/middleware 与发送前账本依旧待续。

NP-01-A2b 已接入普通 SDK 单 sender 执行视图和 lane controller：容纳初始化自动追加 Default，但不会选择隐藏 Default 或执行多 sender；只在准备失败时换 lane。SDK 调用后错误/缺失或多个签名/核验失败保留 typed unknown 及已知原签名，不自动继续另一 lane；核验有总 deadline。新增 25 项，全套并发/串行各 831、HTTP 160、check/fmt/diff 通过，旧参数/分类摘要保持。见 `SDK_LANE_ENTRY_AND_UNCERTAINTY.md`。仍无独立无 signer 构建、完整授权或普通 signed journal；SDK simulate/预检可签名，取消不证明 worker 停止，多签名仅有诊断而无逐签名 journal，B-E 继续。

NP-01-B1 仅将 DAMM 只读报价的固定 profile 编译独立出来。替换前冻结 576 组完整消息 SHA，替换后逐组对照 SDK，生产 builder 只接公开参数，不再接 Keypair/SwapParams/RPC。新增七项，全套并发/串行各 838、HTTP 160、check/fmt/diff 通过。SDK oracle 因旧类型限制使用测试样本身份仅读公钥，不签名/联网；不能宣称 SDK 本身已有无 signer API。最终普通 SDK 买卖/预检、账户证据/完整授权、账本和 legacy 仍待续，B1 不代表整项 B 完成。

NP-01-B2a 为同一 DAMM 卖出 probe 增加八账户单 bank 身份/token 扩展证据及五秒读取 deadline，生产模拟绑定原 prepared/RPC/slot，不可由调用方换消息或降低 min slot。新增 14 项，全套并发/串行各 852、HTTP 160、check/fmt/diff 通过，见 `DAMM_PROBE_ACCOUNT_EVIDENCE.md`。新增保守拒绝须发布前验收；跨 slot 输出余额增量仍可能混入外部转账，不能据此称为可信报价或完整授权。下一项处理本笔实际输出证据，继续池开放/fee version、买入/其他协议和最终签名边界。

NP-01-B2b 已把 DAMM 卖出 probe 的报价改为本笔严格 CPI transfer/同次模拟 post-account/Mint/Clock 费用证据，可选同次模拟 pre/post balances 交叉核对；旧跨 slot 相减不再用于该生产报价。新增 19 项，包含 96 组编码/方向/profile、外部转账/新 ATA、u64/fee/epoch、坏证据及取消；并发/串行各 871、HTTP 160、check/fmt/diff 通过，两条新增弃用警告已消除。见 `DAMM_SIMULATED_SWAP_OUTPUT.md`。这仍不是 RPC 节点诚实性、最终 SDK 消息授权、买入/全协议能力或 ordinary signed journal，继续 B 的其余授权边界与 C-E。

NP-01-E0 先缓解已确认的 legacy 回执风险，不改变完整 C/D/E 的准入依赖：本地签名后广播 Err 或不同回执均保留原签名/Unknown，普通 Live 聚合明确 Unknown 经专用 typed 结果保留，错误 URL 脱敏。原正常成功、模拟、校验与全部 untyped 分类不变；无新幂等权威、迁移或自动补单。最终订单成功保存后的重复/临时库重开及最终写库失败的内存原签名已有离线合同；不能据此宣称发送前 signed journal、取消/崩溃恢复或完整授权，详见 `ORDINARY_AGGREGATE_UNCERTAINTY.md`。

NP-01-C1 将原 Created 身份/hash/模式和冻结项目命令作为唯一 owned 输入接入生产 executor，Duplicate/Conflict/保留失败不产生执行输入。普通聚合 adapter 在 quote 前核对原 ID/公钥；新入口及对应实际 port 已使用，不是预留空接口。新增七项，三模式、字面 key、全命令/整数/浮点 bits、错配/重放/失败/临时父行及公钥变化均通过离线验证，见 `ORDINARY_RESERVATION_EXECUTION_ANCHOR.md`。C1 不是数据库执行 claim 或签名字节 journal；仍需 C2 原父记录核对/CAS/字节落盘读回，完整 B/C/D/E 均未完成。

NP-01-C2a 已在普通聚合 Live 的任何状态更新/报价前接入真实占用：Memory/Shadow 原幂等行标记，Durable 新 004 原父行关联表与同事务 identity/branch/claim 核对。已有占用不重新进入后端，不覆盖原状态/签名；存储异常、取消或丢回执不自动释放/重试。新增 19 项覆盖两 SQLite pool/并发/文件恢复、错误父行/坏证据、执行器取消/回执丢失及 HTTP Pending/key 合同。它仍无签名字节/最终消息授权，也未扩展其他 backend；C2b 原 owner/计划/签名字节落盘与精确读回、D/E 受控原字节发送及完整 legacy 迁移继续，见 `ORDINARY_EXECUTION_CLAIMS.md`。

| 阶段 | 实际交付 | 必须验证 |
| --- | --- | --- |
| NP-01-A 能力与基线 | 冻结 buy/sell、金额/费用/MEV/middleware、模拟/多签名/回退行为与分支；明确各协议 builder 输入能力 | 无真实 signer 的参数/事件基线；不通过新增 capability 声明假装已有安全执行能力 |
| NP-01-B 不可变构建与授权 | 按可证明支持的 SDK 路径，复用现有 adapter/协议参数，构建项目意图绑定的不可编辑计划；SDK 类型只留 adapter | unsigned 预览无 sign/send；校验池/资产/Token Program、金额/min_out、费用、账户权限和附加指令；无法分离的路径明确保留限制 |
| NP-01-C 唯一普通 signed journal | 在原 reservation/key/hash/钱包身份上补版本化签名证据与 CAS；签名落盘精确读回后才允许发送 | 写失败、回执丢失、取消、并发和文件重开不换 key、不重签、不丢原字节；memory/shadow 不冒充 durable |
| NP-01-D 受控发送和对账 | 仅发送已绑定且已持久化的原字节；处理所有签名来源，非最终观测保持不确定 | 签名后不换 lane 重新构建；超时/未查到/Err 不自动补单；明确同字节重播与新交易的差别，按原签名只读查询 |
| NP-01-E legacy 迁移 | 普通 SOL 聚合复用受控阶段及唯一普通 journal，不再丢失不确定发送的原签名 | 保持公开路径、金额和钱包语义；必要状态/错误安全变化单独文档化，旧请求/旧行有兼容及恢复合同 |

本轮 Handler 目录拆分只使用既有 admission/journal/executor，不能跨越这些门槛或算作上述安全阶段已实现。真实节点、生产迁移/刷盘、SDK 版本来源和受控发布仍另有验收，不为验证自动重发历史失败订单。

## 与暂停任务的关系

- PumpAPI H2 剩余协议、全量消息授权、交易接入保持暂停；本工作包不修改该路径。
- 普通 signed journal 的设计保留项目通用边界，但本轮不增加 PumpAPI 接入或 Router SDK 依赖。
- 已有 PumpAPI HTTP/fixture 测试仍可作为兼容回归运行，不代表恢复其功能开发或进行服务商联调。
- 当前 SDK/legacy 尚未满足本工作包全部要求；必须保留这些风险，不能以本地测试数量或目录结构替代完成证据。
