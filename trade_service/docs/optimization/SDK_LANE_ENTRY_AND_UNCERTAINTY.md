# NP-01-A2b SDK 通道入口与不确定结果

日期：2026-11-13。范围：普通 SDK 交易，不包含暂停的 PumpAPI 交易对接。状态：实现已接入，修正 SDK 自动追加 Default 的发现后，源码及最终离线回归通过；未部署，不是完整 NP-01 或链上验收。

## 实际调用变化

- `sdk_client.rs::buy/sell` 共用 `execute_sdk`，原 lane 选择和禁播检查保留；旧组合回退/第一签名映射/核验 helper 已移除。
- `adapters/sdk/ordinary_lanes.rs` 编排 prepare -> invoke -> observe。只有 prepare 失败允许下一 lane；进入 opaque SDK 后所有结果直接结束当前调用，不再构建下一条交易。
- `sdk_client/lane_execution.rs` 复用原 A2a 准备与 A1 最终参数。DAMM Live 卖出仍先执行原 SDK 最终预检，再最终提交；预检失败、签名缺失/歧义直接结束，保留已知预检签名诊断。
- SDK 返回的零签名占位被过滤，有效签名按原顺序去重。Live SDK 返回错误、非成功、缺失或多个有效签名时使用项目 typed uncertainty，映射既有 `unknown`，保留第一个已知签名，多个签名写入现有 error 诊断，不扩展公开字段。
- 外层 context 不吞掉 typed uncertainty 的原签名；旧 untyped/PumpAPI/legacy 分类及模拟分类不借字符串匹配扩展，旧 98 样本摘要保持。
- 成功 receipt 仍最多为 `submitted`，即使只读观测为 confirmed/finalized，也不在这里提升普通订单状态。坏身份、slot=0、链上错误、暂不可见或 RPC 错误均保留不确定结果及原签名。

## SDK 内部 Default 与费用扇出

复核 `vendor/sol-trade-sdk/src/client/mod.rs` 发现：`normalize_swqos_configs` 会在 provider 配置中自动追加 Default，初始化失败还可能仅留下 Default。直接要求原基础设施只有目标 sender，会误拦正常 provider；允许原多 sender 直接执行又可能按 Default + provider 两种费用构建不同交易。

本阶段在普通 SDK 准备时生成单次调用的执行视图：

1. 仅接受一个目标 sender，或一个目标加 SDK 追加的 Default；缺少目标、重复 sender、其他额外 provider 和未审计 middleware 拒绝。
2. 新基础设施只含原目标 sender 的同一 Arc；原 RPC/config/core handles 保留，不初始化 SDK、不创建 provider 连接、不修改共享基础设施。
3. 克隆原 TradingClient 并仅替换该执行视图；payer、risk gate、配置和其他客户端字段保持，准备及观察继续使用相同 RPC。原共享客户端不变，PumpAPI 不接此视图。
4. 调用侧检查实际执行方向及 provider 恰有一个 Normal fee；SDK 模拟仍选择 Default fee。A1 原请求参数/费用摘要不为适配检查而修改。

这只能约束当前 SDK 源码中已知的 sender x fee 任务扇出，不是普遍恰好一次发送保证；供应商内部行为及 SDK 后台 worker 尚不受独立 signed journal 约束。

## 只读签名核验

`adapters/rpc/sdk_signature.rs` 复用原 SignatureStatusReader，仅查询本次原签名和选中的原 RPC，使用 history。保留 Default、wait_tx_confirmed、simulate、verify budget=0 的原跳过条件。

预算现在限制整个轮询和每次读取，总超时返回不可见，不作为原交易失效证据。明确确认等级、结果数及错误一致性复用原 reader；不修改独立 Quote/WSOL 对账行为，不广播、不查询备用签名或另一 RPC。

## 离线测试与门槛

相对 A2a 新增 25 项：lane/controller/metadata 13、只读 RPC 7、typed 分类 3、真实 Router/临时持久化 2。

- fake lane 覆盖准备回退、所有 SDK 返回后的禁止回退、成功/不确定 receipt、多个签名、核验异常、模拟和准备/invoke/observe 取消。
- metadata-only SDK sender 覆盖全部 SwqosType 的真实 TradingInfrastructure 投影，验证原 sender/RPC/core Arc、config 及原对象未变；假 sender 的发送/小费方法禁止调用。没有 SDK 初始化、密钥或 provider 网络。
- 内存 RpcSender 仅允许 GetSignatureStatuses，捕获原签名和 history 参数；覆盖明确 commitment、缺失后出现、总 deadline、取消、零预算、坏签名/元数据、slot=0、错误一致性和 transport 脱敏。
- 真实 Router -> admission -> Executor 的 fake SDK uncertainty 覆盖买/卖、零/一/多个签名、Memory/Shadow/Durable、GET 和同 key 重复 POST；临时文件数据库重开保留 unknown/签名/全部诊断，不再调用 backend。
- 不重生成旧 98 样本分类摘要和 A1 的 2,016 组字段摘要。API 路径、8 字段响应、金额/钱包语义、hash、SQL 和持久化模式不变。

最终验证命令（均在 Trade Service 目录执行并通过）：

```bash
cargo test --locked --offline --quiet ordinary_lanes
cargo test --locked --offline --quiet sdk_signature
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo test --locked --offline --quiet existing_executor_status_mapping_fingerprint
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

结果：lane 13、RPC 7、最终参数摘要 3、旧分类摘要 1、HTTP 160 项通过；全套默认并发/串行各 831 项通过，相对 A2a 增加 25 项。check/fmt/diff 通过；生产 bin 的 dev-profile check 仍仅有 `models.rs` 和 `persistence.rs` 两类既有兼容重导出警告。本轮没有 release 构建。

相对 A2a 核对未变的 SHA-256：

| 文件 | 摘要 |
| --- | --- |
| Cargo.toml | `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89` |
| Cargo.lock | `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5` |
| target/release/pump-trade-service | `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f` |
| src/pumpapi_client.rs | `acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b` |
| src/adapters/sdk/ordinary_execution.rs | `2246675f820b79d4939efa655ff0d8237456651c945f157059a54d4baf56b28f` |
| src/executor.rs | `e232ecb1d46726de2db85407e3a302c69a113efe413ffdc05c438937c6d71677` |
| vendor/sol-trade-sdk/src/instruction/meteora_damm_v2.rs | `59a0c51b5a8c833fe8f037c5e3f5e33615a06762c970dd0df282f231e7e287c1` |

旧分类 98 样本 SHA `017be4bb035ed56cdf073b646057ba8d8c92407a144e979d5f6be5c77d3f5227`，A1 2,016 组参数 SHA `166aa348712cd4516bccbe10f132caf91acf8a2fb5aa734f06a9d38806255261` 均未修改。vendor 和既有 migration 无新增 tracked diff；manifest/lock 的既有工作树差异保留，不能以相对 Git HEAD 有 diff 误称本轮升级。

## 剩余安全缺口与边界

- SDK 的最终 simulate/预检仍可签名；DAMM 报价 probe 的零签名不等于最终 SDK 完全 unsigned，也不等于 DAMM 只有一次 SDK 调用。
- 取消只证明服务没有进入下一 lane；SDK worker/队列可能在调用取消后继续，不能据此解锁、换 key 或重签。
- 尚无普通交易广播前完整 signed journal，也没有全协议不可变构建/签名前完整授权；未知且无签名时不能凭本阶段恢复原交易字节。
- 多签名仅存于现有诊断，不是逐签名 journal；已签预检签名不自动意味着有交易广播。普通状态不提供新的最终性证明。
- SDK 内部 Err 不能区分完全未提交与提交后丢回执，因此保守 unknown 会覆盖部分真实构建错误；明确准备错误仍是原失败分类。
- 不升级 SDK/依赖，不改 vendor/PumpAPI/runtime 配置，不部署/重启、不读取生产数据库/钱包、不签名广播或重发历史订单。
- 后续进入 NP-01-B 的无 signer 构建能力与授权审计，再做 C/D/E；本检查点不能替代完整优化或生产发布验收。
