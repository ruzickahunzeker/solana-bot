# TS-S1-F2-A Native SOL 执行阶段

状态：本文件记录 F2-A 阶段分离检查点。后续 F2-B/C 已完成发送前账本与 Application/API 接入并移除旧组合入口，见 `SOL_TRANSFER_JOURNAL.md`、`SOL_TRANSFER_API_EXECUTION.md`；下文“暂保留”等为 A 阶段历史状态。未部署、未操作生产钱包/数据库、未进行真实 RPC 或交易签名/广播。

## 已实施

- 原 native SOL 实现迁至 `sdk_client/sol_transfer.rs`，没有覆盖 SDK vendor 或改变依赖。
- `prepare_sol_transfer` 只读取原钱包公钥与原 Default RPC lanes，生成未签名消息；不克隆长期 Keypair、不签名、不模拟或发送。原每 lane 5 秒、RPC 顺序和 fallback 时点保持。
- 私有字段的 PreparedSolTransfer 绑定 message/blockhash、source/destination、请求金额、fee/balance/rent/reserve 与已选 RPC；仅模块内部能构造。
- `sign_sol_transfer` 先检查禁播政策，再读取原 payer；重算 F1 金额并核对完整 canonical System transfer 消息，才调用原 Solana 签名构造。
- SignedSolTransfer 私有保留原 prepared/transaction，只读提供项目 SolTransferSnapshot 与已有 SignedSubmission 的本地签名/字节，不提供消息或 raw RPC 编辑入口，不派生 Debug。
- `send_signed_sol_transfer` 再查政策、钱包身份、消息、签名和完整 bincode 字节一致性，仍向 prepare 所选 RPC 发送；不换节点/blockhash/金额，不重签。
- `confirm_sol_transfer` 复用原 RPC 和 commitment。发送 12 秒、确认 15 秒及原 submitted/confirmed 标签保持；未把 processed 配置偷偷改为 finalized。
- 原 `transfer_sol` 暂保留为以上阶段的兼容编排，HTTP 仍走它。F2-C 接入 journal 后必须移除无账本组合入口，不能当作永久 fallback。

## 重要发现

锁定版本的 Solana RpcClient::send_transaction 已核对 RPC 返回签名与本地交易签名，不同会返回错误；不是把外来签名传去确认。此错误依然不能证明未发送，现有旧 Handler 仍可能记录 failed 并丢失本地签名。

本批只让原签名/字节在 immutable handle 上保持可读取。真正解决该问题仍须在 F2-B/C 中先持久化、再调用 send，并将发送错误/超时作为原签名的 unknown 对待。

## 离线证据

新增 10 项测试，传输仅实现内存 RpcSender，无 socket/HTTP：

- 固定/50/100 unsigned prepare，原消息与 fee 估算 transfer=1、查询顺序、整数及租金边界。
- 四个读取阶段的故障切备用；金额不够不切备用；相同 source/destination、无 lane、超时行为。
- source/destination、金额、费用、余额、reserve、额外指令、payer、blockhash、签名数量的私有篡改拒绝。
- 合成无效签名的 bincode 字节与内存 send 参数完全相同；正常确认、错误回执签名、发送错误/超时、确认缺失/错误/超时；不自动重发。
- 签后消息/签名/字节篡改在 send 前拒绝，禁播在空 SDK client 上早于 payer/RPC 访问。
- 确认遵守所选 RPC commitment，而不是测试或代码暗改最终性。

测试没有创建 Keypair 或执行真实签名。实际 sign 方法只经编译和签名前纯审计回归，不能据此宣称真实钱包签名、节点或链上验收完成。短超时仅由私有 helper 的测试参数注入，生产调用仍固定 5/12/15 秒。

初版测试误以为 RpcClient 会原样返回不匹配签名，已按锁定依赖源码和内存实测改为拒绝断言；另修正测试默认 finalized commitment 与 confirmed fixture 不一致的问题。准备测试曾因自身持有 trace Mutex 又读取 trace 而卡住，已终止该测试会话并修正 guard 作用域；未为这些测试问题放宽生产逻辑。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet native_sol
cargo test --locked --offline --quiet sol_transfer
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

2026-11-13：native_sol 10 项、sol_transfer 筛选 25 项、HTTP 92 项通过；全套默认并发/串行各 460 项通过，无测试构建警告。F1 的 1,890 组金额指纹和缺陷基线继续通过，未把缺陷冻结为未来正确行为。

本批检查点的 manifest/lock/release 指纹与 F1 一致；vendor/原迁移 diff 为空；当时钱包 schema、同步策略、生产制品未改。后续 F2-B 已加入增量表和 FULL 初始化，F2-C 已接原 API，见 `SOL_TRANSFER_JOURNAL.md`、`SOL_TRANSFER_API_EXECUTION.md`；仍未执行生产迁移。
