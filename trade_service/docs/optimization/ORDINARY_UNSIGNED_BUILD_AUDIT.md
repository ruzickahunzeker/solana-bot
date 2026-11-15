# NP-01-B4a 普通 SDK 无 signer 构建入口审查

日期：2026-11-15。接续 A3c；仅审查当前主 Trade Service vendor 和新增离线编译合同，不升级 SDK、不修改 vendor、不切换执行 backend、不部署。当前 `sol-trade-sdk` 为 6.0.2，不能用 `trade_service-sdk9` 或上游 README 的能力替代本地证据。

## 实际调用链与可复用范围

```text
LaneExecution -> TradingClient.buy / sell_by_percent
  -> SwapParams (包含 Arc<Keypair> / RPC / sender)
  -> GenericTradeExecutor.swap
  -> InstructionBuilder / protocol middleware
  -> 模拟 或 SWQOS worker
  -> build_transaction_with_version
  -> nonce / tip / compute budget / full middleware
  -> compile_message
  -> payer.try_sign_message
  -> 模拟 RPC 或 send_transaction
```

| 本地入口 | 核对结果 | 后续使用约束 |
| --- | --- | --- |
| `trading/core/transaction_pool.rs::compile_message` | 只接公钥、借用指令/ALT、blockhash、版本和 V1 config；无 signer/RPC/sender | 可复用底层消息编译，不等于协议指令构建、可执行 Quote 或签名授权 |
| `trading/core/traits.rs::InstructionBuilder` | buy/sell 都接 `&SwapParams`；该类型含 `Arc<Keypair>` 与其他能力 | `rpc=None`/`without_rpc()` 不移除 signer。不能把该 trait 直接作为项目无 signer 构建 port |
| `PreparedPumpSwapTrade::build_transaction` | 接 `SwapParams`，最终调用公共 signed builder | “不广播”不等于“不签名”；本轮不调用，不因 Prepared 命名给它安全许可 |
| `PreparedStonkFunTrade::apply_to` | 公开可变 prepared 字段，接收/返回 `SwapParams`，仅移除 RPC 并检查模拟/等待标志 | 既不是不可变可签消息，也未隔离 payer 或发送能力 |
| `trading/common/transaction_builder.rs` | nonce、tip、预算、full middleware 后编译；检查单 payer，再立即签名；签名完成后检查 wire 大小 | 不能用于无 signer 验收；新流程应先做最终 wire/权限检查再释放签名权限，不在签名后才发现超大 |
| `GenericTradeExecutor` 的 simulate | 调用同一 signed builder，然后 RPC `sig_verify=false` | 不验证签名不代表没有生成签名；本轮不能用真实 SDK simulate 获取无 signer 构建证据 |
| `TradeRiskGate` | 仅 `check_buy(&TradeBuyParams)`，在指令构建前执行；不覆盖 sell/最终消息 | 不能替代双向最终消息授权或 signed journal |
| SDK middleware | protocol/full 两阶段可返回新指令；full 阶段先于 compile/sign，但仍不返回签名后、广播前控制权 | 未审计 middleware 仍由当前 lane guard 拒绝。本批不安装/放宽；不能只靠 hook 声称完整 C/D 已实现 |

SDK 的模拟与正式提交可能使用不同 fee/tip。当前 DAMM Live 卖出另有一次 SDK preview，之后仍重新构建正式交易；preview 不是正式消息绑定。`GasFeeStrategy::clone` 共享 `Arc<ArcSwap<...>>`，不是不可变费用快照，后续不能以 clone 就认定费用被冻结。单 sender/单 Normal 策略约束也不能替代具体 tip 收款账户、CU 预算与最终 fee 校验。

## 本批合同测试

新增 `src/adapters/sdk/unsigned_compile_contract_tests.rs`，直接调用 SDK 生产 `compile_message`，不建 SDK client/Keypair、不联网，不调用 signed builder 或 worker。只以合成公钥、固定 hash 和内存指令为输入。

九项范围：

1. SDK `V0` 配置无 ALT 时实际返回 Legacy，逐字节对照官方 Message 编译，单 payer 和确定性保持。
2. 有 ALT 时返回 V0，核对 writable/readonly lookup index 和指令 account index；未解析地址留在 static keys。内存 ALT 只证明编码，不证明真实表地址/owner/slot/激活可信。
3. V1 显式 config 逐字节对照官方编译，改变 priority fee 会改变待签消息。
4. V1 带 ALT 必须报原错误，不自动换版本或删 ALT。
5. 额外 signer 和跨指令 writable/signer 提升均能编译。局部 AccountMeta 的 readonly 不等于最终消息权限；本例只用静态账户、显式空 reserved 集，不声称动态地址权限已核验。
6. payer、blockhash、program、账户身份/权限、data 与指令顺序改变会改变完整消息字节；这不代表编译器能判断改变是否获授权。
7. 结果拥有消息数据，编译不修改借用指令；后来修改/清空输入不会改变已编译消息。
8. 超过 1232 字节的 Legacy 消息和默认 blockhash 仍能编译；任意合成 program 也没有白名单校验。编译成功不能成为准入条件。
9. SDK fee strategy clone 共享可变状态；经 clone 更新/clear 会影响原策略，已取出的数值副本不随其变化。后续授权须冻结具体预算，不能只保留 clone。

这些不是新的生产验证器、签名许可或双 fake backend 通用合同。首轮两处使用了已弃用 writable 方法，改为当前显式 reserved-address 方法，不加告警抑制；以修正后实际测试结果为准。

最终修正后专项 9 项、完整并发 1244 项（138.95 秒）/串行 1244 项（238.19 秒）、production check、fmt、diff 检查通过；测试没有新增警告，生产 check 仍五组既有警告。manifest/lock/release、原准备/参数/input guard、root SDK/两个 lane 文件、executor/钱包 registry/PumpAPI、Quote 查询 decoder 指纹与 A3c 一致，vendor diff 为空；A3c 的完整 1235 项作为前序检查点保留，不冒充新增九项的结果。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet -j 1 unsigned_compile
cargo test --locked --offline --quiet -j 1
cargo test --locked --offline --quiet -j 1 -- --test-threads=1
cargo check --locked --offline --quiet -j 1
cargo fmt --check
git diff --check
```

## B4b 已定位的最终 DAMM 卖出基线

这是一份待冻结/接线清单，不是最终构建已实现的声明：

- `ordinary_preparation::sell` 先读余额，再加载参数并强制 `swap_mode=0`；本笔净输出经既有 `minimum_output` 生成正式 fixed output。不能以 probe 的固定 `1` 替代。
- 普通 `TradeSellParams.input_token_amount=0` 是中间占位。SDK `sell_by_percent` 在调用 `sell` 前用 `balance * percent / 100` 覆盖；项目现有入口已 checked-u64 核对溢出和零。新构建须保留相同舍入/拒绝语义，不能将中间 `0` 当作最终可支出量或通过 u128 悄悄改变原溢出合同。
- 原输出 ATA 创建为 true、关闭为 false；输入 ATA 只在 percent=100 时请求关闭。SDK 将 SOL endpoint 规范到 WSOL 后选择真实池两侧和程序；面板的 SOL 名称不证明最终含解包指令。必须核对实际生命周期，不能为了输出展示自动添加 close。
- 最终 swap2 含原固定 referral slot、可选 rate-limiter sysvar、实际输入/输出 canonical ATA、精确 amount/min_out/模式；全账户身份/程序/权限及扩展必须由证据授权，builder 根据参数能生成指令不等于参数已可信。
- 普通请求无 nonce、无 ALT，原费用策略为 CU limit 300,000 / price 1,000。SDK V0 完整顺序为（若有 nonce 则在首位）provider tip、CU price、CU limit、协议指令；Default 正式 lane 与模拟没有 relay tip。非 Default 的真实 tip 目的地址由 worker 的 sender 选取，后续须在不可变 profile 中绑定具体合法目的地址，不能只核对 provider 字符串。
- DAMM Live preview 使用模拟/Default 费用并且不带正式 relay tip，之后正式 SDK 会重建；不能拿 preview digest 授权不同的正式 message。买入的旧 `Some(1)` 是单独风险，不因卖出 profile 冻结而算作买入报价/min_out 已修复。

## 下一执行任务与验收顺序

1. **B4b 最终协议指令与消息 profile**：基于实际普通请求/已核验账户及 SDK ABI，先冻结 DAMM 最终卖出（含 preview/正式差异）的完整指令/消息，不只复用 quote-only `min_out=1` probe。使用公开参数构建，不将真实或伪装的 Keypair 塞进项目 port。已有 SDK 纯 instruction 工具可复用；缺少无 signer API 的部分只在 SDK adapter 内实现受限投影并用冻结 oracle 对照，不向 application/domain 泄漏 SDK 参数或复制整套 executor。不能把未接真实路径的平行 builder 作为生产交付。
2. **B4c 不可变授权对象与真实接线**：冻结钱包、方向、全部输入/输出资产与 program、数量/min_out、池/账户/费用版本、账户生命周期、blockhash/期限、预算/tip 收款人、实际版本/ALT 和最终 compiled 权限；拒绝额外 signer、权限提升/附加指令、默认或过期 hash、超大消息。只有该不可变对象可进入受控签名；不对“SDK 后面会重建的另一笔”授权。改变字段要重新拒绝/显式新请求，不在旧 owner 下静默修复参数。
3. **扩展所有现有方向/协议**：DAMM 买入、其他八类普通 Pool、StonkFun wrapper/多跳和 Token-2022 扩展；保持原 amount/hash/JSON/身份合同，精确区分模板不支持与节点暂时不可用，不能以删除协议或将全部请求拒绝作为完成。
4. **C/D/E 广播前账本与恢复**：在原普通执行 owner/authority 下签名，原字节/全签名证据持久化并独立读回后才能发送；不再调用会内部重建/签名/广播的 opaque SDK 流水线代替受控发送。Memory/Shadow、所有 backend/mode、worker 取消和多签名仍各自验收，不把 Durable 聚合局部成功推广。
5. **完整离线与发布门槛**：API/参数/旧 hash fixtures、两个 fake build backend 共用合同、取消/崩溃/重开/重复提交、依赖边界、可信 SDK revision/补丁账本和升级/协议演练。真实供应商/生产恢复/部署验收另行授权；PumpAPI 交易开发保持暂停但其剩余任务不从总目标删除。

本记录没有解除已有“无 signer、无交易、无部署、无生产数据”限制，也没有恢复 PumpAPI 或修改 Copy/Bridge。SDK 类型和 ABI 差异留在 adapter，以冻结合同在未来升级时暴露差异，而不是让 Bot 随 SDK 版本联动。

## 后续检查点

最新 B4c1 的服务端模拟分支已不调用 SDK simulate/sign，见 [不可变产物与模拟](DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md)。本页关于 SDK 库自身会签名的源码结论仍成立；不能将局部绕开其调用解读为所有普通 SDK 路径或正式签名账本已经完成。

该 B4a 检查点后，B4b 已接实际 DAMM 卖出/preview nominal candidate 私有 guard，见 [候选消息接线](DAMM_FINAL_CANDIDATE_GUARD.md)。新增 25 项、全套并发/串行各 1269；仍 opaque SDK，完整授权与签名账本未完成。此页的“不安装 middleware/不改执行路径”描述仅属于本页 B4a 历史，不应覆盖后续有意接线或原任意 middleware 仍拒绝的规则；上列 B4a 源码指纹也不冒充最新。

## 来源指纹

本批唯一源码增量为 cfg(test) 模块接线与新测试文件，不改变生产执行：

| 本批文件 | SHA-256 |
| --- | --- |
| `src/adapters/sdk/mod.rs` | `32c6b2606bb9874b8f20a0ab9cbad19a05dc8f371832d2a5ba77804a1f3aa44e` |
| `src/adapters/sdk/unsigned_compile_contract_tests.rs` | `93fec5dbb7eae835285081ffc767c3ec4777099190ff7fa5058375fb9c40b29f` |

本批审查的 vendor diff 为空。以下是本地文件内容指纹，不冒充可信上游 revision 或版本来源已闭环：

| 文件（vendor SDK 下） | SHA-256 |
| --- | --- |
| `trading/core/transaction_pool.rs` | `a17bbd6de1a45c2e2632fd6680ee25c3b5928d7cbd5572c9ff9d25e1b073214a` |
| `trading/core/params/dex_swap.rs` | `98330c3bb9ff69b4c3b2bb3a2b564008b01e847ea65fd37c8e533287842089ee` |
| `trading/common/transaction_builder.rs` | `88d0d4f8d36c6c7239997e01b460318cf9877082bd54151e2e016d2f9e37db09` |
| `trading/core/params/pumpswap_preparation.rs` | `d8f4849156b302acad990e1e1a34ce9760bec31df50eea43e33d9271e4e3b413` |
| `trading/core/params/stonkfun_preparation.rs` | `65338010bced1d106b0117132df5ad0a20216ada074b71ada78c93e50f6cb6c7` |
| `trading/middleware/traits.rs` | `45087b3c68112861055cf069a80d7cf6aa1eec71067a875ff908237eae54f36c` |
| `client/mod.rs` | `9b90d61490eea5c586298bed95ff361d65f0adbfce2c4499f2bb05d6c88ac16c` |
| `trading/core/executor.rs` | `d9edfba595dc1ae8701e5b621a2fd4db2389d597d34473226aa2b973b7996d2b` |
| `trading/core/async_executor.rs` | `d42b999be4bd4edb6ec56b412d781aa38b575e0d327fe35fc2be6ae23f9edc98` |
