# NP-01-B1 DAMM 公开参数报价构建

日期：2026-11-13。状态：源码已接入，最终离线验证通过，未部署。本阶段只是 NP-01-B 的受限只读 probe 构建，不是全协议无 signer 构建、完整授权或发送前账本完成。

后续 B2a 已用固定八账户单 bank 身份快照替换原单独输出 ATA 基线读取，并将生产模拟绑定原 prepared/RPC/slot；见 `DAMM_PROBE_ACCOUNT_EVIDENCE.md`。本文保留 B1 当时的构建及消息摘要证据，不将原基线读取方式当作当前实现。

## 实施理由与边界

当前 SDK 6.0.2 的公共 InstructionBuilder 接收 SwapParams，其中 payer 为 Arc<Keypair>。仅调用 build_sell_instructions 而不 swap，仍让报价构建获取了长期签名能力；把这个调用移动到新文件不能算无 signer 边界。

本阶段只替换普通 DAMM 卖出报价 probe 的固定 profile。它不是最终 SDK 买卖构建器，不改变 SDK/vendor、路由能力、交易金额、用户滑点或最终发送流程，不继续 PumpAPI 交易接入。

## 实际接入

- 新增 `src/adapters/sdk/damm_probe/plan.rs`：只接收 wallet 公钥、公开 MeteoraDammV2Params、方向公钥、整数数量、blockhash 和内部 seed 标志。无 RPC、Keypair、TradingClient、sender、middleware 或费用策略参数。
- 构建保留原固定 profile：标准 output ATA idempotent create + DAMM swap2；不创建/包装 input，不关闭 token account，不加预算或小费，fixed_output_amount=1。三种 swap mode 保留 SDK 原编码，exact-out 的 1 仍为指定输出，不擅自改为 exact-in。
- ATA 地址/创建调用 SDK 已有公钥 helper；swap2 使用 SDK 公共 program/meta/discriminator/mode 常量，适配器只组装固定 25 字节及固定账户位置，不复制报价算法或通用 SDK executor。
- PreparedDammProbe 的交易及 output ATA 私有，外部没有可变交易、任意指令或签名注入入口。生成 Legacy message 和一个零签名；只读 simulate 接收该不可变 profile，而非任意外部消息。
- `sdk_client.rs::simulate_damm_v2_sell_output` 已移除 SwapParams、payer clone 和 InstructionBuilder 调用。保留原 output ATA 基线读取、token account 校验、min_context_slot、模拟配置、正数 delta 检查和错误；只把构建及 simulate 参数改接 prepared profile。
- 当前两个 SDK 客户端构造均 use_seed_optimize(false)。该 profile 明确拒绝 seed=true，不对未验收的 seeded account 行为作兼容承诺；没有增加可供用户设置的新配置。

## 替换前证据与升级门槛

先在未替换运行时 builder 时，用当前 vendored SDK 公共 InstructionBuilder 生成 576 组原消息并冻结摘要：

`0e70ba8d25dce8faaea08a4a556fc974ae39bd0904c3162665c39651e7d226d7`

矩阵为三 mode × 两池方向 × WSOL/USDC × token A/B 各两种 program × referral 有/无 × rate-limiter sysvar 有/无 × 三种 amount（1、1,000,000、u64::MAX）。摘要覆盖按固定顺序 bincode 序列化的完整 Legacy messages，不是只比数量或 discriminator。

SDK oracle 测试因旧 SwapParams 类型要求，使用仅测试的确定性无资金 Keypair，SDK builder 仅读取公钥；没有签名、RPC、SDK/provider 初始化或发送。新生产构建和其余 public-plan 测试不需要该 Keypair。不得把“oracle 没有签名”误称为 SDK builder 类型本身已去除签名能力。

替换后逐组比较新 PreparedDammProbe 的完整 Transaction 与原 SDK 指令编译的零签名 Transaction，额外核对 output ATA 和零签名。后续 SDK 升级若账户位置、权限、数据、helper 或常量漂移，应让固定摘要或逐组对照失败，再专项审查；不能随升级重生成摘要掩盖差异。

## 验证清单

新增七项：SDK 历史摘要与逐组 differential 2、公开参数/拒绝与消息绑定 3、新 prepared 真实 RPC 序列化和取消 2。保留 A2a 原六项通用零签名 helper 测试。

- 公钥参数构建无签名；校验零数量、错误方向、不支持 Quote/mode 与 seeded profile。
- wallet/blockhash/pool/vault/program/referral/sysvar/mode/amount/Quote 改变时消息随之改变，不使用可变共享意图。
- 内存 RpcSender 仅允许 SimulateTransaction；捕获 base64 的完整原交易及 sigVerify=false、replaceRecentBlockhash=false、processed、accounts、minContextSlot、innerInstructions=false。
- 取消只读 RPC 后无后台 send/sign；原 A2b lane/uncertainty、普通最终参数和旧分类摘要仍回归。

最终验证命令（均在 Trade Service 目录执行并通过）：

```bash
cargo test --locked --offline --quiet adapters::sdk::damm_probe
cargo test --locked --offline --quiet ordinary_preparation
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo test --locked --offline --quiet existing_executor_status_mapping_fingerprint
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

结果：专用 probe 13、准备 8、最终参数摘要 3、旧分类摘要 1、HTTP 160 项通过；全套默认并发/串行各 838 项通过，相对 A2b 新增七项。check/fmt/diff 通过，dev-profile bin check 仍仅有两类既有兼容导出警告，没有新测试警告。

复核 Cargo.toml、Cargo.lock、release 二进制、PumpAPI root、ordinary execution adapter、Executor 和 vendor DAMM builder 的 SHA 均与 A2b 检查点一致，具体值沿用 `SDK_LANE_ENTRY_AND_UNCERTAINTY.md`。A1 的 2,016 组最终参数摘要、旧 98 样本状态摘要以及本阶段新增 SDK probe 摘要均固定，不重写原合同摘要。没有新增依赖、迁移或真实节点验收。

## 剩余风险与后续

- 这是本地受限 profile 的消息编译，引用 SDK 公开常量的兼容代码仍需跟随升级审计；不声称 SDK 升级零维护。
- 池参数来源仍为原 loader，未在此建立新的账户 bank 证据、完整 Token-2022 扩展/池状态授权；未知 program/账户不能仅凭消息编译成功就获实盘许可。
- fixed_output=1 只保留原报价模拟 profile，不代表实盘可忽略最少到账。可信报价、费用/滑点和部分成交证据仍需专项验收。
- 最终 SDK DAMM 预检与普通买卖仍为 opaque 组合调用，仍可能签名；新 prepared probe 不是最终提交交易，也不能拿其字节作为签名前完整授权凭据。
- 普通 signed journal、SDK 后台 worker 取消风险、legacy 聚合原签名恢复和全协议构建边界仍未完成。继续 NP-01-B 账户/预算/指令授权，再接 C/D/E；不把这一只读改动代替全部任务。
- 本轮不访问生产钱包/数据库、不做实网 RPC、签名或广播，不升级依赖、不构建 release、不部署/重启、不提交推送；PumpAPI 交易开发保持暂停。
