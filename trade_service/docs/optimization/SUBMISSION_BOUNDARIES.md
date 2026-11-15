# 签名与广播边界审查

更新日期：2026-11-13。审查范围为本地服务的资金操作入口；不是完整 vendor 内部审计或链上验收。已接入独立聚合和 native SOL 提交账本并补离线故障测试；没有部署或扩大 Live。详见 `QUOTE_SUBMISSION_JOURNAL.md`、`SOL_TRANSFER_API_EXECUTION.md`。

## 当前调用关系

| 入口 | 流程与现有保护 | 仍需处理 |
| --- | --- | --- |
| 普通/批量交易 | `api::handlers::trades::accept_trade_for_wallet` -> durable reserve（仅 durable 模式）-> executor -> SDK buy/sell、PumpAPI 或聚合分支 | 签名/广播封装在后端；普通账本尚无统一发送前 signed bytes 保存点；memory/shadow 不能冒充 durable。SDK/legacy 初轮复核见 `ORDINARY_SDK_SUBMISSION_SAFETY_WORKPACK.md` |
| 独立 SOL quote submit | HTTP -> Application -> 查 durable journal / prepare -> 原子 reserve -> SDK 签名 -> 保存签名字节 -> 广播 -> 保存结果；另有只读 GET 原签名对账 | 已补离线故障及请求触发对账测试；后台定时巡检、真实 RPC、部署及实盘未验收 |
| SOL 转账 | API Handler -> Application 全意图/owner 验证 -> 原子 reserve -> Created 保留原 runtime -> unsigned prepare/sign -> record_signed -> 一次 send -> 保存结果/确认 -> 读回 | 无 journal 组合入口已移除；存储失败 503、不确定发送保留原签名 unknown，旧不可核实记录拦截；离线并发/取消/恢复通过，真实 SDK/节点及调用方兼容待验收。详见 `SOL_TRANSFER_API_EXECUTION.md` |
| WSOL submit | API/Application gates -> Quote/唯一 reservation -> metadata/原 client/原 RPC prepare/sign -> 完整 signed CAS/精确读回 -> signed simulate -> 权威回读/禁播 -> send -> 单向结果记录 | G3/G4 已补身份/授权/签名落盘/取消/错误/文件恢复局部证据；真实节点/断电、历史密码学来源、调用方与发布未验收。见 `WSOL_SIGNED_JOURNAL_AND_SIMULATION.md`、`WSOL_STATUS_RECORD_AND_ERRORS.md` |
| WSOL simulate/quote/status/by-key | API/Application 项目 ports；quote 为无 signer source，simulate 为 unsigned；status 只查原签名并可 CAS 保存观测，by-key 纯 SELECT | G4-C 补 metadata/估算校验和 typed 错误、预览取消/文件重开；表/原字节兼容但不声称 SQL 逐字未变。runtime 仍有 Live 方法，整体 signer/网络沙箱和真实验收待做，见 `WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md` |
| PumpAPI Local Transaction | 完整 command；模拟/Live 先做 unsigned/消息/ALT 绑定；实际 pool 分发接 CPMM/StonkFunSwap 与 PumpSwap 的直接路线账户证据；模拟无签名，Live 仍走原签名/send，后续带最新已检查 slot | H1/H2-A/B1/B2-A/B/C 子集离线通过；其余池未验、完整用户账户/指令/整数支出/min_out/全部费用及 signed journal 仍缺。PumpSwap 非零 disable flags 仅保守拒绝，None/slot/局部检查不是授权或状态锁，见工作包 |

源码定位：`src/api/handlers/trades.rs` 的 `accept_trade_for_wallet`；native Handler 在 `src/api/handlers/transfers.rs`，经 `application/sol_transfers.rs`、`adapters/sdk/sol_transfer.rs` 和 journal 编排。独立报价 Handler 已迁至 `src/api/handlers/quotes.rs`，经 `application/sol_buy_submit.rs`、`application/quote_reconciliation.rs` 及受限 runtime 编排；`src/application/quote_submission.rs`、`src/adapters/persistence/submission_journal.rs`、`src/adapters/sdk/quote_submission.rs` 保留原核心。WSOL 在 `src/api/handlers/conversions.rs`、`application/wsol_submission.rs`、`application/wsol_queries.rs` 与 SDK/RPC/journal adapters，旧 conversion 仅保留 store。普通 SOL 聚合现经 `src/application/ordinary_sol_buy.rs`、`legacy_quote_submission.rs` 和 SDK adapter，不再有 `aggregator_quote::submit` 包装。另见 `src/executor.rs` 的 `persist_before_execution`、`src/sdk_client.rs` 的 `send_aggregator_transaction`、`src/sdk_client/sol_transfer.rs` 的原生转账阶段、`src/pumpapi_client.rs` 的 `execute_live`/`simulate_trade` 和 `adapters/sdk/pumpapi_simulation.rs`。

范围调整：PumpAPI 交易对接开发暂停，以上调用关系保留作为历史/运行风险记录；不恢复其剩余协议或消息授权工作，也不改变线上配置/数据流。当前仅推进普通 SDK/legacy 与其他非 PumpAPI 优化，见主计划 1.1。

普通 SDK 后续 A2a 已提取 buy/sell 准备 source，并把 DAMM 报价探测的 RPC transaction 改成单个零签名槽，见 `SDK_PREPARATION_AND_DAMM_PROBE.md`。这不包含最终 SDK 预检的内部签名、普通提交账本或 lane 回退修复；不能因报价不再签名就宣称整个 SDK simulate 已隔离。

## 权限与模拟现状

- 普通鉴权先检查来源 IP，再验证 Bearer。内部接口再要求 loopback；不使用转发头替代 peer。
- 默认 legacy 仍共享权限；新增 separate 管理模式保护钱包和靓号任务管理，需独立 token 与 loopback。源码已实现/离线通过，运行时迁移尚未启用，见 `EXECUTION_SAFETY_AND_ADMIN.md`。
- JSON/Query extractor 可先于 Handler 鉴权返回错误；HTTP 合同已记录，不在目录迁移时暗改。
- `ENABLE_SIMULATE` 是默认值，普通请求可以显式覆盖；不能作为隔离实例的硬禁止广播措施。
- 存在独立 `CONVERSION_LIVE_ENABLED`，但不能据此宣称所有资金操作受同一个总开关保护。
- 新增启动期 `TRADE_BROADCAST_DISABLED` 覆盖已审查资金入口，默认 false 保持兼容；true 时 SDK 完整流水线（含模拟）也拒绝，不依赖第三方 simulate 语义。不是网络沙箱，raw RPC/signer 收口仍待 TS-04。

## 下一批任务与验收

| 顺序 | 任务 | 可验证完成条件 |
| --- | --- | --- |
| S1-A | 本轮已实现 SDK 无关的提交身份/指纹与 journal port | wallet + operation + quote 内容 + Live mode 绑定；同 key 同内容返回已有操作，异内容冲突；模拟不进 Live 账本 |
| S1-B | 本轮已实现 durable 原子 reserve | 并发、独立连接池竞争、落盘失败和文件重开已测；未部署 |
| S1-C | 本轮已拆分 prepare、sign、persist_signed、broadcast 并接原 API | fake 验证先保存再发送、超时保留原签名；字段不变，但 Live 现在要求 durable，状态/错误行为需调用方验收 |
| S1-D | 已实现原签名只读 GET、单向状态 CAS 和并发后读回 | mock/临时库验证确认、失败、超时、迁移与迟到回执；不重放，未部署，也未启用后台巡检 |
| S2-A | 已审查入口的禁止广播策略及 empty SDK wrapper/fake 测试已接入 | 默认/覆盖/MEV、入口绕过和无资源的发送调用已离线验证；真实 SDK/OS 隔离未验收 |
| S2-B | 可选 legacy/separate 管理鉴权与兼容迁移方案已接入 | 默认不改变调用方；separate 的 8 个管理操作、loopback、凭据互不授权已测；尚未部署或迁移 Bot |
| S1-F | 转账 F1/F2-A/B/C 已源码离线通过；原 API 已接入且无 journal 组合入口移除 | 金额指纹不变，Application/runtime/真实 Router 新增 22 项故障/并发/取消/恢复测试；生产迁移、真实签名/节点和调用方兼容未验收。下一批 WSOL 基线/安全边界，PumpAPI 审计独立推进 |
| S1-G | WSOL G1/G2/G3/G4-A/B/C 局部源码/离线通过，原 API 已接项目边界 | 当前并发/串行各 695、HTTP 152、WSOL 205 通过；不是生产历史、真实节点或整服务发布验收 |
| S1-H | PumpAPI H0/H1、H2-A/B1/B2-A/B/C 子集已局部离线通过，见 `PUMPAPI_PUMPSWAP_ACCOUNT_EVIDENCE.md`；B2-C 新增 15 项，全套 776、HTTP 153、PumpAPI 81 | 下一批 Pump.fun/其余协议 B2、B3/B4 全量用户账户/指令/预算/min_out/费用绑定、H3 普通 signed journal、H4 SDK/legacy；Live 完整授权仍缺，局部证据不开放实盘 |

实施约束：先用临时 SQLite 与 fake 副作用完成测试；数据库结构变更只在测试库验收，不操作生产库。不得为了方便给 Quote 交易伪造一个 Pool 塞入普通 TradeRecord；也不能只加内存 mutex 就宣称已解决跨重启幂等。

发布前必须确认实际使用的 persistence 模式及 API 兼容策略。回滚二进制不能恢复旧交易数据库来“清空”已发送订单，未知状态也不能自动重提。
