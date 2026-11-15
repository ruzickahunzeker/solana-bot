# NP-01-B2b DAMM 本笔模拟净输出

日期：2026-11-13。状态：源码接入与最终离线验证通过，全套默认并发/串行各 871 项通过。未部署，不是整个普通 SDK 构建/授权或交易账本完成。

## 替换的错误来源

B2a 的八账户 snapshot 保证了读取时的身份与状态，但 minContextSlot 只是下限。原 post-output 减 snapshot-output 会把两个 bank 之间的外部转入/转出算进本笔报价。B2b 不再读取旧 output_before 计算报价；旧访问器及单输出账户模拟 helper 仅为历史测试保留，生产调用从 root 进入新的 quote_evidenced_damm_probe。

本阶段保持公开 API、金额原始单位、wallet/hash/存储及状态分类。仍由既有 minimum_output 应用用户滑点，不放宽 min_out，也没有把原 fixed_output=1 的报价 profile 当成 Live 授权。

## 生产读取与证据

1. 保留 B2a 的一次固定八账户 snapshot 和原 RPC/原不可变零签名消息/slot 绑定。
2. 同一 RPC 用原 Legacy 交易模拟，sigVerify=false、replaceRecentBlockhash=false、processed、minContextSlot 不变。新请求 innerInstructions=true，返回同次模拟后的八个账户，而非原先只返回 output ATA；无额外寻池或账户 RPC。
3. 在模拟响应 slot 再核对 pool/mint/vault/用户账户/Clock/扩展；post input 余额可以为零，不能错误地要求它仍覆盖已花掉的原投入。替换 blockhash 或出现非空 ALT loaded addresses 均拒绝。
4. 固定 exact-in profile 必须有一个精确 input transfer 和一个 output transfer，分别绑定原 ATA、Mint/program、Vault、authority、数量和 checked decimals。额外转账、未知 token mutation、错组/重复组/坏索引/未审计 CPI 不会被静默忽略。
5. 净输出取 output transfer 的 gross 减该次模拟 Mint 与 Clock epoch 对应的 Token-2022 fee。使用 SPL 库整数算法；显式 TransferCheckedWithFee 的 declared fee 也必须一致。input 的净入 Vault 数量同样计算并与 post state 的可行边界检查。
6. 如节点返回同次模拟的 preTokenBalances/postTokenBalances，两数组必须完整成对、索引唯一，四个相关账户的 Mint/owner/program/decimals/raw amount 与 post-account 及两笔转账逐项一致。缺行或矛盾即拒绝，不 fallback 到旧快照；新 output ATA 缺 pre 行须有本次初始化证据。
7. 两个可选余额数组都没有时，仍可使用完整 CPI transfer + 同次模拟账户/Mint/Clock 证据。若节点没有 innerInstructions 或所需 post accounts，则拒绝，不猜零基线或自动改请求重试。

输出 ATA 在 snapshot 缺失，并不证明它在模拟初始 bank 也缺失；可能在两次 RPC 间由其他交易创建。此时无本次初始化也可按真实 transfer 净额报价；若节点提供 pre balances，则必须包含其真实初值。反之，初始化 CPI 与已存在的 pre-token balance 不能同时成立。

## 解析范围与拒绝

- 支持 compiled、partially decoded 和 fully parsed 的受限转账及 output ATA 初始化。使用 SPL instruction parser，parsed JSON 用结构化 serde 字段，不使用 uiAmount 或 uiAmountString 算数。
- 保留经典 SPL 的 legacy Transfer；Token-2022 只支持 checked 或 checked-with-fee，未知 unchecked 路径明确拒绝。不通过全局 allow(deprecated) 隐藏之前两条警告。
- 限制为两个 root group、64 个 inner instructions、raw token data 256 字节/base58 512 字符，严格索引/消息成员关系及已支持指令的 canonical bytes，拒绝 trailing bytes 和额外 signer。
- ATA 组只支持预期输出账户的初始化/immutable owner/size 查询及已知账户建立程序；swap 组允许已知 DAMM 事件 CPI，但 token movement 必须满足本笔严格角色。此限制不是完整 System 租金预算或程序代码授权审计。
- 保留 B2a 的 referral/seed/未知扩展等拒绝。trace/profile 不符是准备阶段只读错误，不能用其授权重发已进入 opaque SDK 的订单。

## 离线验证

新增 `output/tests.rs`，复用扩展后的 B2a test-only 公共账户编码与内存 RpcSender；不复制另一套 pool 编码，不改变生产 Token checker。RpcSender 只接受 GetMultipleAccounts/SimulateTransaction，其他调用让测试失败。

新增 19 项测试，包含：

- 96 组方向/目标 Token Program/买卖式输入方向/ATA 有无/WSOL 或 USDC/三种 trace 编码；逐组交叉核对同次模拟余额，捕获八地址、全部关键 RPC 配置及原零签名字节。
- 旧 output baseline 为 0、小额或大额，模拟真实初值另有数值，外部转入/转出不改变本笔 120 单位输出；两次 RPC 间 ATA 新建不假设零初值。
- 缺 trace/缺任一 transfer/重复 transfer、错误 program/authority/Mint/decimals/destination/数量/多签、未知 mutation、groups/index/order/count/data limits、非 exact-in 拒绝。
- u64::MAX、2^53+1、完全耗尽 input ATA，原始整数不经过浮点；uiAmount 与 raw amount 故意不一致时，仍以 raw integer 为准。
- TransferFee 的向上取整、cap、0/100%、大整数、模拟 epoch 从 17 到 20 切换、declared fee 不符和零净到账；只看旧 snapshot epoch 会失败的用例。
- post Mint hook/扩展/program 与 required account extension 变化，post account/header/slot/replacement blockhash/ALT 上下文异常；同次 pre/post balance 部分缺失、重复、索引错误、身份/raw amount/金额不一致；初始化与已存在 pre balance 矛盾。
- transport 错误脱敏、不重试，snapshot/simulation future 取消不派生 send/sign。

最终验证命令（在 Trade Service 目录执行并通过）：

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

结果：本笔输出 19、整个 probe 46、准备 8、最终参数摘要 3、旧分类摘要 1、HTTP 160 项通过；全套默认并发/串行各 871 项通过，相对 B2a 新增 19 项。check/fmt/diff 通过，测试没有新增警告，dev-profile bin check 只剩 models.rs/persistence.rs 两类原兼容导出警告；没有 release 构建。

旧 B1 576 组消息 SHA、A1 2,016 组参数 SHA 和旧 98 样本分类 SHA 未重生成且全部通过。Cargo.toml/lock、release 二进制、PumpAPI root/共享 token checker、ordinary execution adapter、Executor 和 vendor DAMM builder SHA 与 B2a 前检查点完全一致。暂停时缺失的测试已补齐；该中间状态及恢复过程仍保留在 `DAMM_OUTPUT_EVIDENCE_PENDING_PLAN.md`，不再作为当前未完成状态。

## 仍未完成

- 证据来自选定 RPC 的模拟报告，不是节点共识、链上最终性或节点诚实性证明。真实节点是否提供完整 inner instructions/post accounts 及其格式，尚未实网验收；不靠旧减法 fallback 掩盖缺能力。
- 当前只是 DAMM 卖出探测，不是 DAMM 买入或其他协议的完整资金授权。SDK 最终预检/交易仍可能签名并重新构建，最终消息与这份 probe 的差异尚未封装为完整签名前授权。
- Pool 开放/费用版本/全量指令权限、System 费用与租金预算、程序升级和 Token 扩展长期兼容、全协议证据校准仍属后续；解析条数/字段长度限制不等于整个 RPC 网络响应的硬字节上限。
- 普通交易唯一签名账本、受控原字节发送、SDK 后台 worker 和 legacy 聚合恢复仍未完成。此阶段不得作为恰好一次交易或部署门槛已满足的证明。
- 不升级 SDK/依赖/vendor，不改生产配置/钱包/数据库，不签名广播、不构建 release、不部署/重启、不提交推送；PumpAPI 交易开发仍暂停。
