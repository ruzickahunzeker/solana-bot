# NP-01-A3b2 准备证据与 SDK 输入绑定

日期：2026-11-15。仅 Trade Service 源码/离线验证，尚未部署。SDK 仍为本地 6.0.2，不升级依赖、不改公开 API/金额/hash/存储/配置、不接触生产钱包或数据库、不构建 release、不重启、不签名发送；PumpAPI 交易接入开发继续暂停。

本项接续 A3a 入口检查与 A3b1 原始账户证据，不等于最终不可变消息授权、全部买入资金核验、广播前 signed journal 或 A3c 共享预算完成。

本页保留 A3b2 当时的 SDK 输入/模式检查与测试/指纹。后续读取已由 [A3c 共享预算](PREPARATION_SHARED_RPC_RUNTIME.md)接续；下文的 A3c 待续说明是本阶段历史范围，不覆盖最新执行记录。

## 实际接线

```text
raw Mint / canonical ATA
  -> PreparationInput { account, program, decimals }
  -> PreparationBalanceSnapshot::from_account
  -> PreparedLaneRequest（同一次余额/身份/开始时间）
  -> LaneExecution::validate_prepared
  -> preparation_input::validate
  -> SDK invoke
```

Snapshot 保留显式原始输入地址、Token Program、精度；`checked_input` 先核对钱包/Mint/时效，缺少账户身份不能靠一份 parsed 数字 snapshot 通过。原 metadata constructor 收为 domain 私有，仍保留旧测试 oracle；这不是密码学证明类型，生产证据由已核验的 read port 构造。

真实 pre-invoke hook 保留原实际 payer 公钥、Mint/金额/百分比/卖出余额检查，再用纯 SDK adapter 检查实际 request 的输入资产、DexType/params variant、Token Program、canonical 输入和已声明精度；对实际执行 client 再检查 seed 开关，不能让 canonical 余额被另一个 seed 账户消费。

买卖 request 的 simulate 标志必须与执行意图相同；DAMM Live 卖出必须保留原 final SDK preview，其他模式不接受意外 preview。该 preview 须显式 simulate=true，并按同一 raw snapshot 检查 Mint/输入程序/账户/精度与输出币；不能仅核对最终提交 request。它仍是既有可能签名的 SDK 模拟，不声称零 signer 或最终消息已不可变。

本阶段不增加 RPC：A3b1 的原读取与原始账户政策不变，只将已取得的身份带到真实入口。`ordinary_preparation.rs`、`ordinary_params.rs`、lane preparation、旧金额/hash/JSON/存储与查询 API 均不重写。

## 逐协议选择

| 当前普通协议 | SDK 实际输入选择与检查 |
| --- | --- |
| Pump | PumpFun DexType/params；核对有效 Quote；默认 Token Program 或 Mint `pump` 后缀按当前 SDK 选择 Token-2022，否则用显式 program，不能只相信字段表面值 |
| PumpAmm | PumpSwap；按实际输入/输出匹配池两端，支持反向，读取对应 input program，不固定假定 base 或 quote |
| Bonk / StonkFun 曲线 | 分别要求 Bonk/StonkFun 入口；按 quote_mint/global_config 的有效 Quote 与 classic quote-program 默认规则处理，卖出取 mint program，直接买入取 quote program |
| StonkFunSwap / RaydiumCpmm | 各自 DexType/params variant 保持；两端 Mint/方向严格匹配后取对应 program，不根据显示标签选币 |
| Meteora DAMM v2 | 两端 Token Mint 与方向匹配，选择 token_a/token_b 的实际 input program |
| Meteora DBC | SDK 固定 base 卖出/quote 买入端点，不允许把反向 pair 当合法；还核对 pool_config quote_mint 和 base token_decimal 与 raw 精度 |
| Raydium AMM v4 | 两端 Mint/方向匹配；输入固定 classic SPL，不能假设支持 Token-2022 |
| StonkFun Via-SOL 卖出 | wrapper 类型及 meme leg 与原协议一致；绑定 meme 端真实输入。当前原 loader 仅开放 CPMM hop，核对原 hop ID、WSOL/Quote 两端、meme Quote 与 hop input 的 program 连续性，以及 WSOL classic program |

Mint/Quote 互不相同；池两端不能自我别名；缺失/未知程序、错误协议/variant、错误付款币或卖出输出币、错误 canonical 账户均拒绝。WSOL/USDC/USD1 固定 Mint 的 program/精度再核对为 classic 及 9/6/6，不能靠伪造 snapshot 绕过固定资产规则。

所有上述检查均在 SDK 调用之前失败，不允许另一 lane 掩盖。进入 SDK 后仍保留既有 uncertainty/原签名与不回退政策，不因为合法余额或输入绑定就授权重签/补单。

## 覆盖范围与限制

- 余额读谓词不变：全部当前普通卖出，以及 PumpAmm/StonkFun/StonkFunSwap/RaydiumCpmm 的直接非 WSOL Quote 买入。未读余额的其他买入及 SOL-hop 买入没有因此获得完整资金/账户证明，仍须后续全量授权覆盖。
- Snapshot 的 raw 精度保持整数；SDK 参数没有独立声明精度时不虚构第二份精度证明。DBC config 和固定资产有明确精度合同才额外比较；没有进行新金额转换或修正旧参数。
- 仅核对当前 SDK 参数表达的输入选择与 canonical 派生，不构建最终 SDK 交易，不声称池/费用版本、附加指令、全部可写账户或权限/min_out 已审计。RPC 数据仍依赖同一节点，不是独立共识证明。
- 当前 SDK 的 program/default/方向规则来自实际 vendored 源码审查；升级时须复核 `instruction/{pumpfun,pumpswap,bonk,stonkfun,raydium_cpmm,raydium_amm_v4,meteora_damm_v2,meteora_dbc}.rs` 与 `client/mod.rs` 的 request -> SwapParams 投影、seed/ATA 规则。不能只更新枚举或重算旧黄金指纹。
- 通用 SDK 来源/revision/补丁账本和两个 fake 构建后端门槛仍未完成；本 adapter 不是未来 SDK 行为不变的保证。

## 验证记录

首次测试构建因新 fixture 将 CPMM constructor 写成 `new` 失败，改用实际 SDK 的 `from_trade`；之后一次默认构建进程被 SIGKILL，未取得该次测试结果。磁盘约余 18 GB，无残留 Cargo/rustc 后，以 `-j 1` 重试，首检查点 preparation_ 70、完整并发/串行各 1215 项通过。随后补齐 DAMM preview/模式与 Bonk 显式 Quote 规则，已在最终源码上重新运行专项与完整回归，前检查点结果不当作最终通过。未更改 profile、缓存或系统/生产配置；内核日志读取无权限，未据 SIGKILL 直接断言原因。

最终 preparation_ 71、preparation_input 15、preparation_balance 38、完整并发/串行各 1216、HTTP 199、shared_read 71、quote_balance 25、原参数合同 3 项全部通过；完整并发 127.60 秒、串行 217.33 秒。production check/fmt/diff 通过，仍五组既有警告（models/persistence 兼容重导出、QuoteStore/WsolQuote 兼容方法和 wallet_balance），未新增抑制。阶段新增 17 项（domain 2、SDK 输入 15；其中两个真实 controller + fake invoke 用例包含在这 15 项内），不重复计算扩展后的原 RPC 用例或 fixture 循环。

新增 fixture 只持公开地址、SDK 参数与项目 snapshot，不创建钱包/Keypair/TradingClient，不调用 SDK 执行/签名或真实节点。真实 lane controller 由 fake invocation 计数验证检查前拦截、下一 lane 零准备/零调用与入口后禁止回退；这不是实际广播次数证明。原准备顺序与 2016 组参数黄金合同保持独立。

这些参数是入口选择政策 fixture，不是完整合法链上池/费用配置，不冒充 SDK builder 或链上模拟验收。生产 hook 接同一纯函数，但本轮没有实例化真实 SDK wallet 来运行该 hook 的 payer 路径；实际 signer/最终消息/SDK 后台副作用另行验收。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet -j 1 preparation_
cargo test --locked --offline --quiet -j 1
cargo test --locked --offline --quiet -j 1 -- --test-threads=1
cargo test --locked --offline --quiet -j 1 http_contract_tests
cargo test --locked --offline --quiet -j 1 shared_read
cargo test --locked --offline --quiet -j 1 quote_balance
cargo test --locked --offline --quiet -j 1 ordinary_param_contract_tests
cargo check --locked --offline --quiet -j 1
cargo fmt --check
git diff --check
```

## 后续

下一项 A3c：准备 scan/raw 读取接 main 原 SharedReadRuntime 的第六类有界 profile，独立容量/候选范围/逐笔与总 deadline/流式 body/真实 HTTP 次数/共享 429 冷却/取消。现有 4 秒逻辑 timeout、SDK 内部重试和解码后上限不冒充全局 RPC 配额或无隐藏重试保证。

完整最终消息/全方向与协议/费用/min_out/权限授权、opaque SDK 广播前唯一账本、其他 mode/backend、worker 取消/多签名及恢复、其他读路径与资源/依赖隔离、双 fake backend、可信 SDK 来源与升级演练、真实节点/生产迁移/恢复/发布验收继续保留。整个未部署树已有 004–007，不能因本批无新 migration 就免除未来完整备份/迁移/回滚门槛。

## 指纹与范围

manifest `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、lock `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、原 release `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f` 未变，vendor 无 diff。

原 `ordinary_preparation.rs` / `ordinary_params.rs` SHA 仍为 `b49859e0cb32fd4a01e381d6f356b85f5f40470e877087b00505ac6ef720b438` / `9cb1bee04e18894d3a2090d24019e42099d6dbca5dde4467419b4065a3841374`；lane preparation `77ed94b962d712847c4a0e74ac4b6125227e0fe0c836f4f972cc967f1e20c291`、root SDK `581c01153eb702486e23ab8647304189f9eead8370653e3b819dd1a5e9b25206`、共享 query decoder `e97b35cb1070a0a8be7b3e532503b41a8d740cc3e229a08417b724483fb6b9cb` 与 A3b1 一致。

实际 lane execution 必要接线后 SHA 为 `520d05256008f2127c137bdc15dbc87e8187b3ac804d661af1e5901210ffc1ae`，新增 input guard 为 `def6e630cf63f38d4f065778a96810ab8a993165072a693219688349940351ff`。domain/raw read 的必要变化只传递项目输入身份，不改 RPC 操作/参数/顺序、存储或 publicly serialized 模型。executor、钱包 registry、PumpAPI root 和共享 token validator 指纹与 A3b1 相同。未改 Copy/Bridge/sdk9，其既有脏改动不在本轮处理。
