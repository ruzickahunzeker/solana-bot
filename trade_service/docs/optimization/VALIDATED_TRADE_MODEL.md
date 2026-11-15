# TS-02-C4 项目拥有的校验结果

日期：2026-11-13。状态：新增 3 项离线测试，全套 315 → 318，通过，无编译警告。未升级 SDK/依赖/vendor、构建 release、部署/重启、读取生产钱包/数据库、访问真实 RPC 或发送交易。

## 已实施

- `application/validated_trade.rs::ValidatedTrade` 拥有普通已归一化结果，Mint/Quote/Pool ID/hop ID 改用项目 `Address`；其余金额、滑点、小费、模式三态、MEV、分支等字段保持。
- `validation::NormalizedTrade` 重导出同一内部类型，已有导入路径兼容。它没有 serde，不是新的 wire 请求、路由证明或签名权限。
- wire validator 仍使用原 Solana Pubkey parser、原 Quote alias、原校验顺序和政策；仅在全部校验完成后转换四类地址的 32 字节。
- DTO→command 直接使用内部地址，不再导入/转换链类型；持久化 codec/factory 显式引用内部模型归属。
- 旧测试的手工 normalized fixture 也以原 parser 取得字节；没有给 Domain Address 增加另一套字符串解析或放宽输入。

`amount_lamports` 保留旧名字，但值可能是 Quote 原始单位；是否为 SOL 由 settlement/aggregator/Quote 决定。没有因类型迁移把 Quote 数量改成 SOL，也没有提前解析 `simulate=None` 为默认模式。

## 验证

原 7 处 SDK/PumpAPI 参数测试由跨类型直接相等改为比较完整 32 字节及 Option 形态。Pubkey/项目 Address 是不同类型，不能继续直接 assert_eq；这是机械适配，不改任何冻结期待值或放宽地址相等性。

新增 3 项：

1. 32 组不同的 Mint/Quote/Pool/hop 合成地址，经原 validator → 项目结果 → command → SDK mapper，四个位置始终独立，逐字节核对；编译时明确结果是项目 Address。
2. 原 mint → pool_id → quote → hop 的解析错误顺序，以及过长/空白 Mint 的原错误文本；继续走原 parser，没有在 Domain 重写解析。
3. USDC `u64::MAX` 原始单位完整保留，command 仍是 BuyQuote；`simulate=None/false/true` 在结果内保持三态，只在原命令构造时应用默认模式。

本批没有构造 SDK TradingClient、signer、可广播交易或真实池；32 组是合成身份/参数样本，不是全协议链上证明。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：318 passed、0 failed、无编译警告。旧 normalization/hash、37 组入库、720 组 record 字节、98 组结果分类、48 项 HTTP、模式/故障和 SDK loader 回归均保持。Cargo.toml、Cargo.lock、现有 release SHA-256 与前批一致，vendor 和原迁移文件不变。

## 剩余边界

- wire validator 仍接收 TradeRequest/Config 并在边界使用 SDK Pubkey parser；NormalizedWarmup 仍有链类型，本批只处理普通交易结果。
- 校验结果字段仍是兼容的归一化数据结构，不是不可伪造的验证凭据，不代表池 owner、资产扩展、余额、时效或 min_out 已验证。
- rejected 编排仍在原 HTTP/TradeStore 路径，普通资金发送前签名账本未因此完成。

## 下一批 TS-02-C5

2026-11-13 续作：普通校验已改收 5 字段设置，内部 command builder 与执行策略脱离完整 Config，新增 7 项、全套 325 项通过，见 `TRADE_SETTINGS_AND_COMMAND_BUILDER.md`。wire 输入/SDK parser 仍需后续隔离，以下保留本批要求。

1. 审查 validator 真正读取的配置，使用项目校验设置而不是向核心规则传完整含钱包/RPC/凭据的 Config；旧入口只做设置映射，保留政策/默认值/读取和错误顺序。
2. 将 DTO→command 的业务选择移入内部 builder，边界只传 wallet/key/side/pool 和实际所需默认参数；保留 PumpAPI 独立 priority fee 默认值、模式三态和 missing amount 原错误。
3. 执行策略检查也只接收项目 policy/default，不依赖完整配置；不改变禁播时 SDK/PumpAPI simulate 的既有差异。
4. 继续以既有字节/校验/真实 HTTP 合同保护转换，不混入新 parser、金额算法、路由许可或配置格式变化。wire 输入和 rejected 编排后续再分步。
