# NP-01-D1B1 无 Owner 的普通交易恢复观测上下文

日期：2026-11-14。状态：源码/离线验证通过，未部署。只覆盖已有完整签名证据的 Durable ordinary SOL aggregate；**不是自动跨重启对账、持续确认或完整 D1B 已完成**。

## 实际接线

```text
本次发送尾部：原 execution/owner/signature
  -> capture_live_observation（同一事务校验原 owner 与原账本）
  -> 不透明 observation context -> reconcile_captured

恢复核心：原数据库 request_id
  -> capture_observation（只读核验既有记录，不签发 owner）
  -> 同一种 context -> 同一个 reconcile_captured
```

D1A 的实际生产提交路径已经使用新核心，不是未接线的空接口。原 live 捕获仍要求原 reservation/owner，且仍限发送回执保存后的阶段；旧 owner 形式的读取/写入方法仅作为 `cfg(test)` 兼容 oracle。恢复核心的项目 port `RecoveredOrdinaryObservations` 只有 capture/read/finality，方法不接 AdmittedTrade 或 ExecutionOwner，不返回 signed bytes，也没有 reserve/claim/sign/send 方法。

本轮不增加启动扫描、后台任务、HTTP 路由/参数、配置或定时器。现有 GET/列表/重复 POST 仍仅取原记录，不自动发 RPC；重启后也不会因新增核心而自行开始查询。

## 上下文与证据绑定

`OrdinaryObservationContext` 字段私有，无 Clone/Serde，Debug 完全脱敏。它仅持有原 request/wallet/公钥/字面 key/hash/signature 及证据指纹，不持有 owner_nonce、钱包运行时、未签/已签交易字节或执行命令。

捕获与后续使用都会核对：

- 原父行、字面 key、request hash、原请求 JSON、Buy/aggregate 解释、原 pool/mint/pool_id/execution method；地址只用原项目 parser 做本地语法解析，不查询池。
- 原 claims schema/operation/hash/时间与既有有效 owner nonce；只是核对其存在及格式，不创建、重置或返回 nonce。
- 原 quote/preflight、金额、滑点、路由与正数最小输出，完整 unsigned/signed bytes/hash 和原父行 signature。
- 已有最终证据的版本/最终性/签名/slot/error 与父行一致性；坏记录拒绝，不修库。

版本化内存 SHA256 指纹绑定上述原身份、原请求 JSON 字节、父行创建时间、claims 身份/原 nonce、quote/preflight JSON 字节及 unsigned/signed hash/signature。nonce 只参加 adapter 内的摘要计算，不进入对外上下文。状态、error、更新时间、最终观测列不参加该不可变摘要，允许合法状态推进。

每次读取、RPC 返回后最终写入、事务内及提交后读回，均再次捕获并比较原指纹。查询中原数据被更改也拒绝落盘。它仍使用同一个原 claims/父行和同一 006 最终观测 codec/CAS，**没有新幂等权威、持久化格式或 migration**。

这不是数据库真实性的密码学认证：request hash 只按原值跨父行/claims 核对；没有凭空重算缺少历史默认值的完整 V1 hash，也不抵御捕获前完整重写所有一致数据的数据库控制者。上下文不代表重新证明原交易的完整指令授权，更不授予重新签名或重发权限。

## 历史解释与 Signed 中断

- 原请求的 amount_sol 沿用 V1 的 9 位 round 整数转换。显式 percent/bps 沿用旧优先级及舍入；缺失滑点使用原已保存 bound 的快照，不读取今天的 Config/默认值/feature flags。
- 显式 simulate=true、非 aggregate、金额为零/错误、当前记录与原请求矛盾均拒绝。simulate=None 不重新套今天默认模式，完整 signed journal 才是本 profile 的既有证据。
- 原 quote 已过期不妨碍查原签名；这不允许重新报价、构建或签名。Live 的签名前时效保护保持，原准入校验逻辑不变。
- 未签名/部分签名、旧 NULL owner、无完整 prepared/signed 证据仍不能恢复观测，不为其补 owner 或编造签名。
- 恢复核心可以查询 signed/submitted/unknown 阶段的原完整签名；signed 后重启的父行仍沿用原启动恢复成为 unknown。
- 明确 finalized 可使 signed 阶段的原父行推进 confirmed/failed，最终证据仍保留原 signed 阶段和原字节；原受控发送的 read_signed 仍要求父行 submitting，不会因此获得新的发送资格。

`validate_bound` 仅把既有参数/路线条件机械抽为共用 `validate_quote_intent`，检查/整数表达式保留。新增恢复解释与准入分离，不修改原 DTO、V1 hash/JSON、request key 或已发布校验行为。最终性、3 秒生产 lookup deadline、错误脱敏和弱证据保持规则沿用 D1A；本轮不增加 RPC 次数或 SDK 内部重试策略。

## 新增九项验证

1. 三种临时文件阶段（signed/submitted/unknown）关闭数据库，并销毁旧 owner、AdmittedTrade/fixture、旧 backend/journal，再由原 request_id 捕获并完成原签名观测。SQL 触发器禁止恢复时插入 claim、写 owner 或替换原 signed artifact；原 nonce/签名/字节和记录数保持。
2. 十类不可变证据变化，包括原 hash 两列同时更改、未参与旧 hash 的 source、有效 nonce 更换、quote/report、重算 hash 的 unsigned/signed 变化等，旧 context 在 RPC 前拒绝，直接最终写入也拒绝。
3. 十三类不完整/矛盾记录拒绝捕获，包括无 owner、错误 operation/schema、坏最终 JSON、显式模拟、金额、地址、pool_id/method 等。CHECK 已阻止的坏 operation/version 仅在临时测试库短暂关闭 CHECK 构造损坏文件样本，恢复前重新打开；生产约束未放宽。
4. signed -> submitted -> confirmed 合法推进不改变原证据身份，不调用 signer/send。
5. 已过期原 quote 仍能获得链上失败结论、保留原签名/key，不重新报价、签名或发送。
6. Debug 脱敏、跨订单及四类记录身份混用拒绝。
7. 门闩精确控制 RPC 等待期间篡改原数据，最终写回拒绝，不用 sleep 假设时序。
8. 两个 SQLite pool、16 个无 owner 的上下文观测写入，共同收敛原终态。
9. 冻结 V1 金额半单位/大整数样本、percent/bps 舍入/优先级、缺失默认值和非法金额/显式模拟样本，不能靠将来新准入默认值解释旧单。

原 D1A 二十项故障/取消/回执丢失/只读库/实际 Router 合同随完整回归保留，包括 finalized 证据不可覆盖与父行零回退。所有样本为 fake signer/send、公开不可执行签名/字节、内存 RPC 和临时库，不是实际钱包恢复、密码学签名或真实节点验收。

```bash
cargo test --locked --offline --quiet ordinary_recovery_context # 9 passed
cargo test --locked --offline --quiet                           # 1018 passed
cargo test --locked --offline --quiet -- --test-threads=1        # 1018 passed
cargo test --locked --offline --quiet http_contract_tests       # 198 passed
cargo check --locked --offline --bin pump-trade-service         # passed, 5 warning groups
cargo fmt --check                                              # passed
git diff --check                                               # passed
```

生产仍为原五组兼容/未用警告，无新 allow；初轮测试访问私有 digest 的编译问题通过测试独立 SHA 解决，没有扩大生产可见性。原 2,016 参数/98 分类/576 消息黄金合同随全套保持。

本轮有意调整普通观测/controller bounds、原 journal 校验与最终写入及原验证器的只读 replay helper；新增 context/adapter 子模块和测试。main、executor、root SDK client、钱包/worker、PumpAPI、Cargo.toml/lock、004/005/006 与 release 二进制未改；无 SDK/vendor 升级、release 构建、部署/重启、生产配置/钱包/数据库读取或联网交易。

## 后续任务与门槛

- D1B2：有界 pending 枚举/游标、持久身份与只读 RPC runtime、原签名扫描触发/生命周期及关停；不重新签发 ExecutionOwner，也不悄悄改 GET 或重复提交。
- RPC 全局并发/配额及底层 HTTP 429 重试预算必须纳入调度。D1A 的两个节点逻辑调用不是 HTTP 请求上限；本轮没有解决这一发布门槛。
- 持续观测、真实节点/断电/历史数据/调用方与性能/迁移回滚仍未验收；本轮更多了尾部 metadata/SQL 核对，不能声称延迟或吞吐已优化。
- 其他 backend/mode 的唯一签名账本、完整最终消息/池/费用/权限/整数预算授权、SDK worker 副作用、后端通用合同及剩余架构隔离继续，不因恢复基础完成删去。

PumpAPI 对接开发仍暂停，保留其既有代码/测试，不停止 ZMQ。持续目标 active，修改记录不暂停。
