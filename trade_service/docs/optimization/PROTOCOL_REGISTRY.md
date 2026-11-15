# TS-03-A 协议注册表与旧合同

日期：2026-11-13。状态：注册表源码/离线测试通过；未部署、未更新 SDK、未扩大协议能力。不是 TS-03 所有协议链上验收完成。

## 已实施

- `src/protocol/registry.rs` 保存 9 个已有 Pool 与 6 个 observe-only 协议的规范身份、展示别名、校验政策和旧能力声明。
- `validation.rs` 从注册表读取 Quote 政策与 SDK Pool ID 政策；原校验顺序、错误文本、PumpAPI 的 Pool ID 豁免、SOL 聚合买入的旧 Quote 校验绕行保持原样。
- `protocol_capabilities.rs` 只将注册表映射为 V1 响应，wire 类型移至 `api/dto/protocols.rs`，旧导入路径保持兼容。
- `persistence.rs` 的规范 Pool 名称读取同一注册表；存储解析复用原 Pool serde 合同，保留原错误文本和仅有的 StonkFun 历史别名。
- Registry 不依赖 SDK、RPC、Axum 或 SQLx。`for_pool` 穷尽映射已有 Pool，身份与映射一致性有测试。

## 明确分开的三种含义

1. **展示/发现别名**不等于下单 JSON 接受的别名。`pumpfun`、`pumpswap`、`dbc`、`raydium-v4` 等仍不能作为 TradeRequest 的 Pool；不能在重构中扩大输入合同。
2. **现有 adapter**不等于本轮完成模拟/实盘验收。为兼容保留旧 `status=live`，内部用 `AdapterStage::Existing` 表示代码已有，不称为已证明可 Live。
3. **旧 V1 能力声明**不等于精细校验或资产扩展证据。`legacy_*` 命名标明兼容用途，不能拿这些字段自动批准新的路由或 Token Program。

已有 6 个 observe 协议保持 `supports_buy/sell/direct_quote/sol_route=false`；即使 vendor 中有 builder 也不开放。

## 已核对的差异与后续门槛

| 项目 | 本地证据 | 本轮处理及后续 |
| --- | --- | --- |
| PumpSwap Pool ID | 旧能力响应 `pool_id_required=true`，validator 允许缺省并交给 resolver | Registry 分别保存旧声明与 Optional 政策；以后改能力 schema 需消费者兼容设计 |
| DBC Token-2022 | `docs/METEORA_DBC.md:68` 声明无 transfer-hook 的 Token-2022；旧能力只列 SPL Token | 不自动扩大 token_programs；待 no-hook、Transfer Fee、扩展账户与双向 fixture/不广播模拟验收 |
| DBC Pool 身份 | vendor `instruction/utils/meteora_dbc.rs:29` 解码固定 VirtualPool discriminator，先检查 owner；寻池用固定大小与 base mint 过滤 | 只能证明当前代码校验路径，不能当作所有 Token-2022 扩展安全证明 |
| DBC Mint owner | vendor `trading/core/params/meteora_dbc.rs:41` 读取两个 Mint 的 owner 作为 token_program | 该处未解析 Mint 扩展或显式证明无 hook；不能把“传入 Token Program”写成完成扩展验收 |
| DBC 最小输出 | vendor `instruction/meteora_dbc.rs:206` 使用 `minimum.max(1)` | 已记录需审查的 dust/零报价风险；本轮不修改 vendor 或报价算法。后续须用有效报价/精度样本证明安全下限，不把 1 当可信报价 |
| USD1/custom Quote | validator 对部分 Pool 允许任意 Quote；`sdk_client` 的 USD1 分支仍只允许 Bonk/StonkFun | 保留既有行为和错误；Registry 的 Any 仅说明旧入口政策，不保证每种资产组合可构建 |
| StonkFun SOL Route | V1 能力随 feature flag 显示；实际路径还校验 hop、池、金额和配置 | 兼容 flag 展示，不以该字段代替路由和链上验证 |
| 未适配 builder | vendor 有 launchlab、Raydium CLMM、Meteora DLMM、Whirlpool instruction 文件；服务 Pool 无对应枚举/完整映射 | 仍 ObserveOnly；不能用“文件存在/能编译”替代服务适配与模拟证据 |

Program ID/布局版本、Token Program 扩展、双向报价与模拟的完整证据矩阵尚未接入 Registry。TS-03-B 校准这些信息时，需要可信 SDK provenance、合成账户 fixture 及逐协议验证；不能通过修改旧布尔值假装完成。

## 冻结与验证

在激活 Registry 前，已对旧实现捕获以下 SHA-256，后续仅对比，不重新生成期待值：

```text
V1 capabilities（feature off/on，完整列表/字段/顺序，不含动态时间）
6bda4c6b2f50544fe81239dabf21ed9c3920f20b3d360c3ecafb010a44e1ce5e

Warmup 校验（9 Pool × 6 Quote 输入 × 2 Pool ID，108 组，含原错误）
ee62bedc8d35b5837b895e5f8b58e658c21e80cf8c7e298e0d4012de765fed7e
```

新增 7 项测试：两项旧实现指纹、身份/Pool 完整性、别名不冲突、不扩大请求别名、旧声明与实际政策差异、存储编解码及错误合同。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：157 passed、0 failed，无编译警告。普通交易 normalization/hash 与聚合 V1 journal 指纹同时保持通过；无真实钱包/RPC/广播。
