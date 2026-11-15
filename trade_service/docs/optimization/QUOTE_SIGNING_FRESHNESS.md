# NP-01-B3a 聚合报价签名前时效复核

日期：2026-11-14。状态：源码与离线验证通过，未部署。仅补共享聚合签名边界的时效检查，不是完整 NP-01-B 或全协议签名前授权完成。

## 复现与修复

原 `PreparedQuoteTransaction` 只保存未签交易、钱包公钥及预检授权布尔结果，没有保存 Quote 有效期。普通 Durable 提交的 `authorize_sign` 虽在事务内检查有效期，但随后还有异步 commit；其他提交也不能靠早先的预检结果保证稍后仍未过期。

先只新增无 signer 的回归：构造 `expires_at_ms=0`、其他预检均合法的 prepared，实际调用 `into_verified_message`，原实现仍返回消息，测试按预期失败。没有为复现获取钱包、签名、联网或广播。

修复仅在 `src/adapters/sdk/quote_submission.rs`：

- 私有、非 Clone/Serde 的 prepared 对象保存原 `expires_at_ms` 数值快照，调用方不能通过修改原 bound 延长它。
- `into_verified_message` 先沿用原预检、钱包、payer、单一零签名检查，再要求当前时间严格小于原到期时间。
- 新错误为 `aggregator quote expired before signing`。实际 `SdkTradeClient::sign_prepared_quote` 在取得 message 成功后才获取 signer，因此新增拒绝发生在取 signer/密码学签名之前；该 root 方法无需修改。
- 不改消息、blockhash、金额、min_out、路由、费用、签名字节格式或发送方式，不重新构建或自动重新报价。

现有源码共用这个边界的路径包括 Durable 普通 SOL 聚合、Memory/Shadow legacy 普通 SOL 聚合和独立 Quote 提交。各自的 journal/失败处理仍沿用原控制器；此处没有把 legacy 升级成 durable。PumpAPI 与 opaque SDK buy/sell 不走这个 prepared 类型，不能宣称它们也获得该保护。

## 对外与安全边界

路径、DTO、hash/JSON、SQL/schema 和现有状态映射保持。唯一新增拒绝是“在早先检查后、实际释放待签消息前已过期”。独立 Quote 对这一极晚的 adapter 拒绝仍使用原 signing-error 的 Backend/502 映射，而不是冒称所有到期分支均返回 410；已有显式到期分支的 410 不变。普通 Durable 控制器沿用已有签名前失败处理。没有新增 key、释放/重签/重发或自动恢复许可。

检查时间来自原项目时钟，不是链上时钟或 blockhash 有效性证明；不能保证检查后的进程调度、签名和发送期间绝不跨过 Quote 期限，也不撤回已签名操作。完整最终消息授权、真实报价/指令审计、可信节点及其他协议仍有独立门槛。

## 测试与验证

新增三项无 signer/RPC 的测试：

1. 实际释放消息入口拒绝到期 prepared；该测试先在原实现失败，修复后通过。
2. 明确时间输入检查到期前、恰好到期、到期后、u64/u128 边界及 bound 修改不能延长 snapshot。
3. 未过期 Legacy/V0 消息释放后序列化逐字节相同；仅公共账户/零签名 fixture，V0 无 lookup table，不冒充实际多跳交易或签名验收。

已有五项 adapter 钱包/payer/预检/签名形态及已签字节拒绝测试保留。原两类控制器签名失败/保留 key、legacy 不确定结果、普通 signed journal、HTTP 与参数/消息黄金合同随全套通过，没有改写其期望。

```bash
cargo test --locked --offline --quiet adapters::sdk::quote_submission::tests # 8 passed
cargo test --locked --offline --quiet                         # 989 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 989 passed
cargo test --locked --offline --quiet http_contract_tests     # 196 passed
cargo check --locked --offline --bin pump-trade-service       # passed, 5 warning groups
cargo fmt --check                                            # passed
git diff --check                                             # passed
```

五组警告与 TS-09-A 相同，详见 `APP_STATE_RESOURCE_OWNERSHIP.md`，未用 allow 掩盖。2,016 组最终参数、98 组旧状态分类、576 组 DAMM 零签名消息及 V1 合同随完整回归保持。

本阶段有意改变的 adapter SHA256：

```text
1b8c24124a308e66bbb3621b7735a5a6045b411405da33885afdb6f901e10f19 src/adapters/sdk/quote_submission.rs
```

相对 TS-09-A，不改其他生产源码、SDK/vendor、manifest/lock、原迁移或现有 release。未构建 release、部署/重启、读取生产钱包/数据库或发送交易。

## 后续

完整最终消息/池/程序/费用/权限/预算授权、普通原签名对账、其他 backend 和存储模式的唯一账本、SDK worker 后台副作用仍未完成。普通 GET 保持存储读取，不悄悄加 RPC 或新 URL；对账触发及兼容合同先明确，再实施。NP-03 的其他资源/依赖方向和后续双假后端合同继续，PumpAPI 交易接入仍暂停。持续目标 active，记录更新不暂停。
