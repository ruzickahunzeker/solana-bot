# NP-01-D1A 普通 SOL 聚合发送后的原签名观测

日期：2026-11-14。状态：源码和离线验证通过，未部署。只覆盖 **Durable + Live + ordinary aggregator_sol** 的本次发送尝试尾部；不是完整普通交易对账、跨重启恢复或整个 NP-01-D 完成。

后续 D1B1 已提取不依赖 ExecutionOwner 的恢复 context/core，并接入本页实际发送尾部；显式恢复 signed/submitted/unknown 的离线基础通过，但尚无自动启动扫描或持续调度。见 `ORDINARY_RECOVERY_OBSERVATION_CONTEXT.md`，并发/串行各 1018、HTTP 198；下文保留 D1A 的 1009 检查点与当时限制含义。

## 触发与能力边界

实际接入 `application/ordinary_submission.rs`：

```text
原 reservation/owner -> 准备/授权 -> 原签名字节落盘/独立读回
  -> 原字节发送尝试 -> record_broadcast 成功
  -> 只查原签名一次 -> 最终观测 CAS -> 原 journal.observe -> 原 executor
```

- 只有原受控提交实际执行到 `record_broadcast` 成功，才进入观测；签名落盘/读回/回执存储失败不触发新查询，也不重发。
- 发送成功、发送 Err 或不同回执都只查已落盘的本地原签名，不采用不同 RPC 回执替换它。
- `application/ordinary_reconciliation.rs` 只拿 `OrdinaryReconciliationJournal` 和 `SignatureStatusReader`。新 journal port 只有原记录读取和最终观测写入，没有新 reservation、claim、签名、广播或重试能力。
- SDK adapter 从本次原 `SdkTradeClient` 克隆既有 Default RPC 引用，复用原只读 reader；不重新从 registry 选择钱包，不新增 RPC URL/节点/配置。
- 整次 RPC lookup deadline 为 3 秒；原 reader 最多选择两个节点、各 1 秒、顺序调用。现有 Solana RPC client 内部仍可能重试 429：本地 `solana-rpc-client` 的 HttpSender 有五次限流重试预算。因此“两节点”是 client 逻辑调用上限，不是实际 HTTP 请求至多两次的承诺；仍受上述 deadline 中断。不是两个 key 并行，也不是新确认轮询任务；SQL 等待仍受原存储设置约束，3 秒不是整个执行或数据库总 deadline。
- RPC 等待期间不持有 SQLite transaction。查询错误/缺失/弱证据后回读原记录，能看见其他观测者已经保存的终态。

## 最终性政策与外部兼容

仅对原签名、正数 slot、明确 `finalized` 的观测保存最终证据：

| 证据 | 行为 |
| --- | --- |
| finalized 且无链上错误 | 原父订单推进既有 `confirmed` |
| finalized 且有链上错误 | 原父订单推进既有 `failed`，保留原签名 |
| processed / confirmed，包括其执行错误 | 不作为本 profile 的最终结论，不写状态或观测列 |
| 查不到、RPC 错误、超时、签名不符、slot=0 | 保留原 submitted/unknown 与原签名，不重新下单 |
| 原父行已终态 | 返回原结果，不调用 RPC、不覆盖终态；没有旧证据时也不伪造证据 |

普通 TradeStatus 没有新增 `finalized` 值；本 profile 将 finalized 成功映射到既有 confirmed。**不能反推服务所有 backend 的 confirmed 都已经 finalized**，旧 SDK/独立报价/WSOL 各自合同仍需分别理解。

HTTP 路径、DTO、八字段普通提交响应、原 key/hash/JSON 和参数校验不变。普通 GET/列表和重复 POST 仍只读原存储，不触发 RPC。首次 HTTP 等待可能先返回 submitted/unknown，发送任务尾部稍后才保存最终观测；GET 只是读取该结果。查询失败不会伪造成交易失败，也不新增公开查询字段。

原 SDK 发送、独立 Quote 提交、Memory/Shadow legacy 与暂停中的 PumpAPI 不进入这份新观测流程。既有独立 Quote 的状态接口/策略未修改。

## 同一原账本

- 新增 `006_ordinary_final_observations.sql`，只给原 claims 行增加 nullable `final_observation_json`；没有新普通幂等权威或新签名字节表。
- 005/006 在原 migration ledger/同一事务中顺序执行、分别登记；004/005 原文件未修改，旧行保持 null，不补 owner 或执行许可。
- 存储 codec 是严格 `FinalObservationV1`：schema_version、原 signature、明确 finalized、slot、脱敏链上错误，单份不超过 64 KB。不含钱包私钥、RPC 凭据或签名字节，也不进入现有 HTTP JSON。
- 读取和更新核对原 execution/owner、原父行身份、key/hash/request JSON/payment branch、原 signed signature/bytes/hash 和提交阶段。
- 更新先取得 SQLite writer lock，再检查当前证据；只有 submitted/unknown 父行可推进。最终证据与原父行状态/error 在同一事务写入，事务内逐字读回后 commit，再独立读取。
- 已保存的最终证据不可替换。相同证据重复不改父行；不同 slot/error/最终证据冲突拒绝，较弱证据不能降级结果。原父行已有外来终态但无新证据时保持原样，不冒充已验证该终态来源。
- malformed/不一致旧证据仅拒绝读取或更新，不自动修库。写失败、触发器改写、回执丢失或取消均不释放原 key/owner，也不重签/重发。

## 离线证据

新增 20 项：18 Application/真实临时存储合同、2 实际 Router 合同。已有原测试只调整 fake reader 接线和成功流程新增观测步骤，不放宽原签名、状态、字节或次数断言。

- 三种发送回执均只核对原签名；finalized 成功/错误、原字节保留与 URL 脱敏。
- 缺失/processed/confirmed/节点错误/零 slot/错签名不写状态，terminal 跳过 RPC。
- 明确 deadline/Drop；查询中取消、观测 commit 后取消、commit 回执丢失保留原证据及占用。
- 门闩控制并发最终性与较弱结果交错，不用 sleep 证明时序。
- 两个 SQLite pool、16 并发观测同一原记录；终态父行零覆盖审计；写失败和触发器篡改回滚。
- 005 schema 升级/重复初始化、文件关闭重开不丢证据、不生成新 owner、不触发旧执行重放。
- 真正只读的临时 SQLite 可完成 missing lookup 的读取，不能写入最终观测；不在请求里初始化仓储。
- 原 `RpcSignatureStatusReader` + 内存 RPC 原始 JSON 实际经过解析和 journal：明确 finalized、非最终错误、缺确认等级、错误字段矛盾、无结果/零 slot；未访问真实节点。
- 实际 HTTP admission/claim/controller/executor/SQL 配 fake backend 验证八字段、最终 GET、原 key 重复及观测写失败；GET/重复不再查询或发送，owner/blob/新列不外泄。

```bash
cargo test --locked --offline --quiet ordinary_observation    # 20 passed
cargo test --locked --offline --quiet                         # 1009 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 1009 passed
cargo test --locked --offline --quiet http_contract_tests     # 198 passed
cargo check --locked --offline --bin pump-trade-service       # passed, 5 warning groups
cargo fmt --check                                            # passed
git diff --check                                             # passed
```

五组兼容/未用警告与 TS-09-A、B3a 相同，没有新增 allow；初轮测试 check 的局部未用 import 已移除。参数/状态/消息黄金合同 2,016 / 98 / 576 随全套保持，未改期望。

main、wallet registry/repository、executor、root sdk_client、opaque ordinary_execution、PumpAPI 实现、Cargo.toml/lock、004/005 及现有 release 指纹不变。本阶段有意改变 ordinary controller、共享 quote adapter 的只读 reader 接线、原 claims migration 初始化及原 journal 校验，并增加项目 port/Application、最终观测存储子模块和 006；不能误写成所有业务源码不变。

## 未完成与后续任务

1. D1A 是一次发送尝试后的单次观测，不是持续确认。正常发送后可能尚未 finalized，结果继续 submitted；不能靠本次查询证明最终成功或失败。
2. D1B 仍需跨重启的独立只读恢复上下文及触发合同。不能为读取旧单重新签发 ExecutionOwner、重新接受请求、使用新 key，或借 GET 悄悄发 RPC。当前 signing/signed 中断等阶段不在 D1A 触发范围，HTTP 重复也不会补查。
3. 全局 RPC 配额/并发预算、SDK HTTP 内部 429 重试、持续观测生命周期、生产历史/调用方/断电/制品与回滚仍需独立验证；新源码上线后每次合格发送尾部会增加至多两个节点的状态 client 逻辑调用，底层 HTTP 次数可能更多，不能无视现有限流或宣称性能已验收。本轮只核对本地依赖实现，不修改 SDK/其重试或生产配置。
4. Reader 只证明节点返回的格式/显式最终性和一致性，不证明节点诚实性、独立节点共识或真实签名交易成功。测试使用公开不可执行签名/字节与 fake send，没有实际密码学签名或广播。
5. NP-01-B 完整最终消息/池/程序/费用/权限/整数预算授权、C/E 其他 backend/存储模式账本及 SDK worker 副作用仍未完成；后续继续这些安全门槛，不用目录或测试数量冒充整体完成。

只改 Trade Service 源码/文档及离线临时库；不升级 SDK/vendor、不构建 release、不部署/重启、不读取生产配置/钱包/数据库、不联网或发送交易。PumpAPI 对接仍暂停，不停止 ZMQ。持续目标 active，记录和计划更新不暂停。
