# NP-01-B2a DAMM 报价账户身份快照

日期：2026-11-13。状态：源码及专用证据测试已接入，完整离线回归通过，未部署。仅普通 SDK DAMM 只读报价；不是完整可信报价、最终交易授权、VerifiedRoute 或 signed journal。

后续 B2b 已在该固定 profile 内以本笔 CPI transfer/同次模拟 Mint、Clock 和账户状态替换旧跨 slot 相减，可选同次模拟 pre/post balances 交叉核对；见 `DAMM_SIMULATED_SWAP_OUTPUT.md`。本文保留 B2a 的历史身份快照证据，以下旧差额风险描述不代表 B2b 仍采用该算法；最终授权、买入/其他协议及真实节点验收仍未完成。

## 实际问题与修改

B1 已隔离固定零签名消息的编译，但原报价只读取 output ATA 基线。池/两侧 Mint/Vault 来自更早的 SDK loader，用户余额来自另一次查询，不能据此证明这些账户在同一 bank 与消息意图一致。

本阶段新增 `src/adapters/sdk/damm_probe/evidence.rs`，在模拟前固定批量读取八个账户：pool、token A/B mint、token A/B vault、Clock、input ATA、output ATA。前七个必需，输出 ATA 可以不存在；不存在时保留原 create-idempotent 与零基线语义。没有 GPA、寻池扫描或节点轮询；将原一次输出 ATA 请求替换为一次固定 GetMultipleAccounts，不声称请求字节或节点 CPU 成本与单账户相同。

读操作使用原 lane 的同一个 RpcClient，processed/base64，整个证据收集有五秒 deadline。账户数据解析复用既有通用 decode_data/check_mint/check_vault；这些函数暂位于历史 `pumpapi_assets.rs`，本轮不改该文件或 PumpAPI backend，也不恢复其交易对接。中立命名迁移留给后续依赖收口，不复制一套不同规则的 Token parser。

## 已接入检查

- 八个账户角色不能别名，返回数必须精确，context slot 非零；Clock owner/布局合法且 Clock.slot 与批量 RPC context 相同。
- Pool program owner、非 executable/非空状态、字节大小/编码/space、discriminator 和 SDK layout 合法；同一快照的两 Mint/两 Vault 必须与 prepared 参数身份完全一致。
- 两 Mint 的 Token Program 与此前加载的 program 一致，Mint 初始化/布局合法；固定 WSOL/USDC 的 program 和精度按各自 canonical mint 核对，不按 symbol 猜资产。
- Token-2022 的 mint 扩展、当前 Clock epoch 费用配置、active hook/未知扩展和 required account extension，使用已有共享政策。
- Vault 的 Mint、Token Program、pool authority、初始化/冻结/delegate/close 状态与必要扩展都检查。用户 ATA 同样核对真实 wallet owner、Mint/program/扩展及状态；实际 input ATA 余额必须覆盖本笔整数 input，而非以其他 token account 的合计余额代替。
- 用户 ATA 的 delegate/close-authority 当前保守拒绝；referral 为 Some 的 profile 因收费账户语义尚未验收而在 RPC 前拒绝。原 SDK loader 的普通 DAMM 参数 referral=None；没有新增 referral 选择或改变公开参数。
- PreparedDammProbe 保存与原消息同一构造时刻的私有公开账户快照。DammProbeEvidence 私有构造并借用原 prepared 和原 RPC；模拟不再由调用方任意替换 RPC、消息或 minimum slot。
- 生产入口 `simulate_damm_v2_sell_output` 已使用 inspect -> simulate_evidenced，旧任意 prepared+slot 的 helper 收为模块私有。模拟配置仍为 B1 的零签名 profile，minContextSlot 取批量快照；返回 slot 比证据更旧时拒绝。

这些是新增的操作级保守拒绝，而不是只换目录的重构。公开请求校验、字段、金额单位、钱包选择、hash、SQL 和状态分类不变；已有 delegate/close-authority 等 SDK 可能接受的用户账户，不应在未做调用方验收前直接部署新限制。

## 离线验证

新增 14 项专用测试（参数化组合不冒充测试项数量）：

- 正常路径：两池方向、SPL/Token-2022、WSOL/USDC、output ATA 有/无共 16 组；捕获原八个地址、批量 RPC 配置及同一消息/零签名的模拟报文。
- 前七个账户逐个缺失，返回多/少、slot=0、Clock 错位/owner/layout，pool 身份漂移/owner/header/短数据/大数据/编码/space/executable/lamports 损坏。
- Mint program/初始化/精度/布局，Vault 与用户 ATA 的 mint/authority/owner/program/frozen/delegate/close 状态，input ATA 不足。
- Token-2022 两方向/两 epoch 的 active hook、未知 mint 扩展、bad fee、缺失 required account extension 和 transferring 状态；另验证输出 Token-2022 ATA 新建/已有的对应扩展约束。
- referral/角色别名在 RPC 前拒绝；transport 脱敏/不重试；模拟 slot 回退；snapshot/模拟取消及五秒内部 deadline 的 future Drop。

测试仅内存 RpcSender，允许 GetMultipleAccounts/SimulateTransaction，其他方法直接使测试失败；无 Keypair、真实 SDK 初始化、账户签名、节点连接或广播。

完整验证命令（均在 Trade Service 目录执行并通过）：

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

结果：专用证据 14、整个 DAMM probe 27、准备 8、最终参数摘要 3、旧分类摘要 1、HTTP 160 项通过；全套默认并发/串行各 852 项通过，相对 B1 新增 14 项。check/fmt/diff 通过，dev-profile bin check 仍仅有两类原兼容导出警告，测试无新警告。

既有受保护 Cargo.toml/lock、release、PumpAPI root、ordinary execution adapter、Executor、vendor DAMM builder 指纹保持 B1/A2b 检查点的值。复用的 `pumpapi_assets.rs` 本轮前后 SHA 均为 `e75da06c2a9ca20b09b5c7f50488b60a3d0c43f2fe280dfb273a1279e4ea87f0`，未改该模块。B1 576 组消息 SHA、A1 2,016 组参数 SHA 与旧 98 样本分类 SHA 均未改写且通过；没有新增配置、依赖、SQL 迁移或生产数据操作。

## 尚未解决

- minContextSlot 只是下限。模拟可以发生在更晚的 slot；当前 root 仍用 post-output 减 snapshot-output，两个 bank 间的外部转入/转出可能污染增量。不能称作同 bank 真实输出量，更不能据此说完整可信报价已完成。下一批需绑定模拟本笔实际 token 流或取得可靠同 bank 前后值，并覆盖费用/账户生命周期证据；不能靠延长等待、强行忽略 slot 或放宽 min_out 掩盖问题。
- 未在这里猜测 DAMM pool_status/activation/fee_version/layout_version/collect_fee_mode/Token flags 的全部语义，Vault 净储备/应计费用、池开放/费率版本及价格/流动性仍需专项审查；正确身份不等于可成交或正确价格。
- SDK loader/节点是数据来源，不是多节点共识或加密状态证明；上下文匹配不代表节点诚实或最终性。
- 非审计 Token 扩展、seed/referral、全协议用户账户与完整整数预算/指令权限仍有缺口。最终 SDK 预检/交易仍可能签名并重新构建，不以本 prepared 的消息替代最终授权。
- 本检查仅接入 DAMM 卖出报价探测；DAMM 买入与其他协议普通 SDK 路径没有自动获得同样的账户证据保护，不把局部完成说成全协议授权完成。
- 普通唯一签名账本、SDK worker 取消和 legacy 聚合原签名恢复仍未完成；不得因 probe/HTTP/状态失败自动换 key 补单。
- 未升级 SDK/依赖/vendor、未构建 release、未改生产配置/钱包/数据库、未部署/重启、未签名广播/提交推送；PumpAPI 交易开发仍暂停。
