# TS-S1-G2-A WSOL 已签交易封装

日期：2026-11-13。源码已接入原兑换路径，仅离线验证；未部署、未访问生产钱包/数据库、未执行真实 RPC/签名/广播。前置合同和后续工作见 `WSOL_CONVERSION_SAFETY_WORKPACK.md`。

## 本批职责

- 原 `conversion::PreparedWsolTransaction` 的公开可编辑 Transaction/signature/bytes 移除。
- 签名实现迁到 `sdk_client/wsol_conversion.rs`；仅该 SDK 子模块能构造 `SignedWsolTransaction`，全部字段私有，不实现 Clone/Debug/serde，不返回裸 Transaction。
- 原 unsigned owner/payer/所需签名账户核对、SDK 内长期 signer 与可选临时 signer 保持；上游 SDK、指令构建、金额、blockhash、RPC 顺序不变。
- artifact 只读提供签名/序列化字节，保留授权 Message 和 owner；签后、signed simulation 前及发送前核对 owner/payer、完整 Message、签名数量、本地签名与完整 bincode 字节一致。
- signed simulation 仍使用原 RpcClient convenience 方法，其 program error 返回值由原调用点解释；8 秒边界不变。send 仍用所选 RPC、12 秒边界，进入 SDK 方法时再次校验禁播与 loaded wallet 身份。
- 原 Application 位置仍先模拟、record_signed 成功、再检查禁播并 send；没有替换状态/SQL、报价语义或对账算法。

该批有意收紧内部构造权限和发送前快照一致性，不改变合法请求的外部响应。若旧调用点在流程中换了不匹配的钱包，现在发送前明确拒绝；这不是运行时保留同一个 Arc 已完成的证据。

## 离线证据

四项新测试：

1. 一个/两个所需 signer 的只读签名与完整 journal bytes 一致；只有合成无效签名，没有 Keypair 或真实签名。
2. 模块内故意篡改 owner、payer、指令、签名数量、签名字符串、交易签名、bytes 共八种情形及错误 wallet 均在 RPC 前拒绝。
3. 内存 RpcClient mock 验证 signed simulation 保留成功/程序错误/网络错误语义；坏 snapshot 在 mock 调用前拒绝。
4. 空 SDK lanes、无 payer 的 client 在 signed send 前首先拒绝禁播，不访问 wallet/RPC。

合成 Message 只验证 artifact 一致性，不充当 WSOL 指令授权 fixture 或密码学签名验证。真实签名仍只来自受控 unsigned prepare；本批没有声称补齐所有指令审计或真实链上验收。

## 验证记录

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet wsol
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

默认并发/串行全套各 517 项、HTTP 106 项、wsol 筛选 24 项通过，测试无编译警告。生产 check 通过，仍有两条既有兼容重导出警告，未用 allow 隐藏。格式/diff 检查通过。

manifest/lock/原 release 的 SHA-256 与 `SOL_TRANSFER_API_EXECUTION.md` 一致，vendor/原迁移 diff 为空；本批只增加源码封装、离线测试和文档，没有构建 release 或运行生产迁移。当前磁盘约余 2.4 GB，后续不另建 release 构建树或擅自清理数据。

## 仍未完成

后续 G2-B 已完成 submit/simulate 的项目输入、metadata/runtime/journal ports 和整个提交请求的原钱包/RPC 保留，见 `WSOL_SUBMISSION_RUNTIME.md`。完整 G2/G3 仍未完成：Quote query/status、wire DTO/codec 与 builder、状态单向 CAS、迟到结果、重启与各处取消、故障错误映射和调用方兼容仍需继续。普通交易和 PumpAPI 的发送前账本/外来消息审计也未包含。本文 517 项等计数是 G2-A 历史检查点。

发布前维持 Live 默认关闭和现有调用方，不因封装完成修改生产环境。release/SDK/manifest/lock/生产库均保持不变。
