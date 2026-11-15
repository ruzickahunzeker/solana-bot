# TS-S1-G3-B1 WSOL 签名前标准消息授权

日期：2026-11-13。接续 `WSOL_MONOTONIC_PROGRESS.md`。仅 Trade Service 源码、离线测试和文档；SDK/依赖、公共 API、数据库格式与运行服务未变。没有访问生产钱包/数据库、连接真实 RPC、执行真实签名或广播。

## 原缺口与本阶段边界

G2/G3-A 的 prepared 句柄已私有构造，并检查 owner、payer 和签名账户；signed artifact 也能核对签名/消息/字节快照。但签名前并未重新确认所有指令、账户权限、原始金额、blockhash 和 Quote 授权；单靠签名者列表不能证明一条消息只包含批准的包装/解包操作。

本阶段补齐 **unsigned → sign/unsigned simulate** 的授权，不改变 Live 的 sign → signed simulate → record_signed 顺序，不宣称签名证据落盘、取消恢复或可信最终性已完成。

## 实际接入

- `sdk_client/wsol_conversion/authorization.rs::WsolAuthorization` 留在 SDK 边界，私有 prepared 保存原 Quote 的完整 13 字段，以及原准备取得的 SOL/WSOL 余额、账户租金/返还、实际网络费和 blockhash；不重新初始化钱包或刷新 RPC。
- Application 的 sign 和 unsigned simulate 端口显式传入原 Quote，而不是让 prepared 自己决定授权。真实 SDK adapter 只转发同一个 Quote；fake prepared 也断言原 Quote 未被替换。没有修改公开请求字段。
- `UnsignedWsolTransaction::validate` 保留原 owner/payer/签名形状检查，再核对完整 Quote、钱包公钥、精确过期边界、金额和预算。
- 使用原临时 signer 的公钥重建标准指令，再编译完整 Message，与待签消息逐项相等；包括指令顺序、数据、program/account 索引、所有账户、公钥和权限 header、付款人及原 blockhash。多余/缺少/重排指令、未知程序或额外收费都不被授权。
- `into_signing_parts` 收窄为本 SDK 模块私有；sign 在所有检查通过之后才获取长期 signer。禁播检查仍先于钱包/临时 signer 访问。
- unsigned simulate 同样在调用 RPC 前校验原 Quote 和标准消息；仍使用零签名、sig_verify=false，不保留 Live key，不获取钱包 signer。
- `instructions.rs` 只有一个标准指令构造函数 `canonical_wsol_instructions`；原 builder 仅负责生成必要的临时构建 key，再委托它。审计不生成新 key、不换 blockhash、不新增指令，不另造一套 builder。

## 标准消息与金额规则

| 形态 | 唯一批准的操作 |
| --- | --- |
| 新 ATA 包装 | 原 owner 创建 WSOL ATA → System transfer 指定原始 lamports → SPL Token sync_native |
| 现有 ATA 包装 | System transfer 到原 WSOL ATA → sync_native，无临时 signer |
| 全量解包 | 关闭原 WSOL ATA，资金和租金均退原 owner，无临时 signer |
| 部分解包 | owner 出资创建原临时账户 → InitializeAccount3(WSOL/原 owner) → TransferChecked(精确数量/9 decimals) → 关闭临时账户退原 owner；不关闭来源 ATA |

部分解包必须使用构建时的原临时 signer，不能与 owner/来源 ATA 混用；包装/全量解包不接受额外临时 signer。Token Program 固定原 SPL Token，Mint 固定 WSOL，不从未知账户推导可替代的 Mint/程序。

整数预算沿用：实际网络费 + 必要账户租金 + 100,000 lamports 预留 + 包装金额。解包不再扣一次输入金额；创建 ATA 的租金不能与部分解包临时租金混淆。网络费不得超过 Quote 上限，溢出拒绝，来源余额必须等于原 Quote；租金返还只允许在全量解包出现。没有套用 native transfer 的预留或排空规则。

校验使用准备时的只读证据，并不承诺余额、节点费用或链状态在随后广播前不变。sign/unsigned simulate 新增准备后过期的拒绝，完整 Quote 不匹配或消息篡改会显式失败，这是有意安全收紧，不是只移动目录。

## 兼容消息证据

重构 builder **之前**，先运行原实现采集四种完整 Message 的 bincode SHA-256；之后固定断言，不根据新实现重新生成预期值。owner `[1;32]`、blockhash `[8;32]`，仅测试将随机临时公钥归一到 `[2;32]`；生产不做公钥替换。

| 形态/测试金额 | 原 Message SHA-256 |
| --- | --- |
| 新 ATA 包装 50 lamports | `1a33609bff7171f17ea0fd36b73eab48b9aeb8543167d9aaafc6a0b8dde99c35` |
| 现有 ATA 包装 50 lamports | `feb624c684eec9c429d2f82ab4de169e77b9c5c9cedea7926fd42e2f42706131` |
| 全量解包 100/100 lamports | `33b2142920b47f13fb5e7ece8fff2a87f4c20f8331ac918e0a880ff35c7d3944` |
| 部分解包 50/100 lamports | `7df1a54a1f96f1a43410ff1315822883fa27e10514d663ef5b55d8547b4a9342` |

另外用标准 SystemInstruction/TokenInstruction parser 独立解码较大整数金额、账户租金、165 字节大小、WSOL Mint、9 位精度及租金接收人，不只拿新 builder 与自身比较。

原 `(submission, quote)` V1 JSON/hash、表结构、reservation/key、七字段结果与 HTTP 错误映射未变；标准请求构造的 Message 与旧字节相同。不生成新订单、不转换历史 key，也不升级 SDK vendor。

## 离线覆盖与检查

相对 G3-A 新增 10 项：1 项四形态冻结消息字节，9 项授权测试。覆盖标准指令独立解码、13 字段 Quote 逐个变更、TTL 精确边界、每个指令的每个数据字节/program/account 索引篡改、账户/权限/额外/缺少/顺序/blockhash、临时 signer 置换/缺失/混用、四形态整数预算边界/费用上升/溢出、来源/租金/返还/无效金额，以及错误/过期/不同 Quote 在 unsigned RPC 前拒绝。

RPC 拒绝测试使用没有网络传输的 sender：任何调用立即 panic，最后断言计数为零。真实 unsigned prepare 的原四形态内存 RPC 回归也追加标准消息授权检查。所有测试均不调用真实钱包签名；临时 Keypair 仅供构建形态，已有 signed artifact fixture 仍用合成无效签名，不冒充真实签名验证。

最终全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                       # 578 passed
cargo test --locked --offline --quiet -- --test-threads=1   # 578 passed
cargo test --locked --offline --quiet http_contract_tests   # 116 passed
cargo test --locked --offline --quiet wsol                  # 88 passed
cargo test --locked --offline --quiet wsol_authorization    # 9 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅 models/persistence 两条既有兼容重导出警告。Cargo.toml、Cargo.lock 和既有 release SHA-256 仍分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。未构建 release、发布或执行生产迁移。G3-A 的 568 项保留为历史检查点，不能替作本批完整证据。

## 下一阶段 G3-B2

1. 将本地签名证据可靠保存到唯一 ConversionStore，再进入任何会把已签字节交给 RPC 的 await；不能在模拟失败/取消后按 unsigned failure 丢弃签名。持久化失败不广播、不重签、不换 key。
2. 审查 `record_signed` 的完整身份/状态 CAS 与事务内精确读回：当前仍仅按 ID/reserved 更新，需要覆盖写失败、触发器改写与读取损坏后禁止广播。
3. 定义已签但未发送的模拟失败/超时/网络不明语义；不能仅调用 fail_before_sign，也不能因本地失败释放潜在已泄露签名字节的 pending wallet。保留原签名/字节及同 key 查询能力。
4. 使用明确 gate 验证 prepare、sign/record_signed、simulate、send 和结果保存各处取消/临时文件重开；不自动重签、补发或改后端。
5. 后续单独完成 status RPC deadline/确认可信度和消费者兼容错误分类。仍不将旧 HTTP 400 当成确定未发送的证据。

以上为继续队列，不是本阶段已完成的保证。整体优化还包括 PumpAPI、普通交易发送前账本、VerifiedRoute/协议扩展、基础设施/依赖方向、SDK 来源/升级演练与发布回滚，持续受当前安全边界约束。

后续执行更新：G3-B2 已完成签名事务/精确读回移到已签模拟前、模拟失败保留 unknown/原证据，以及八个应用取消点/文件重开，见 `WSOL_SIGNED_JOURNAL_AND_SIMULATION.md`。本文件保留 B1 历史计数；下一实际工作以新文档和 `EXECUTION_STATUS.md` 为准，原签名查询可信度与完整故障恢复仍未完成。
