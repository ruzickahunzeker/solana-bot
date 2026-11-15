# TS-05-B3 既有九类 Pool 参数回归

日期：2026-11-13。承接 `SDK_PARAMETER_FIXTURES.md`。本批新增 12 项实际 SDK loader 测试，全套 255 → 267；仅改测试和执行文档，未升级 SDK、依赖或 vendor，未构建 release、部署、连接真实 RPC、加载生产钱包或发送交易。

## 本批完成

| 文件 | 新测试数 | 覆盖 |
| --- | --- | --- |
| `tests/amm_v4.rs` | 4 | 实际 AMM/market loader，两种请求方向；所有市场账户和 Serum vault signer、链上 swap fee、两侧 vault 减各自 PnL；warmup Quote-only/build 双侧差异；任一侧 PnL 下溢、短 AMM/market 在 vault 读取前拒绝 |
| `tests/bonk.rs` | 2 | 按 WSOL/USD1 推导真实请求地址、LaunchLab 参数和费用；显式不存在池的 warmup 原始错误与 build 说明错误保持 |
| `tests/pump.rs` | 3 | 实际 Pump loader，曲线储备/creator/所有当前附加字段、Mint owner、Global fee recipient；不存在/Active sharing config 的 creator vault 选择；complete warmup/build 差异；Global owner/discriminator/短账户拒绝 |
| `tests/pump_swap.rs` | 3 | 实际 PumpSwap loader，显式 Pool ID、正常/Mayhem、正负虚拟 Quote 储备、两种请求方向、当前 FeeConfig/GlobalConfig 与 tier fee/recipient；warmup Quote-only/build 双侧差异；vault 错 Mint/owner/状态、Mint 与 vault Token Program 不一致在费用读取前拒绝 |

测试调用 `warmup_on_rpc` / `dex_params`，没有使用预构造 params 假成功，没有 signer/TradingClient、广播或模拟权限。跨测试复用现有合成 Mint/vault/LaunchLab 数据 helper，并未抽取新的生产政策。

## 当前覆盖矩阵

| 服务 Pool | 实际参数加载成功 | warmup/build 差异或关键规则 | 尚不能据此宣称 |
| --- | --- | --- | --- |
| Pump | 是，TS-05-B3 | complete 只在现有 warmup 拒绝；Global 与 fee-sharing 分支 | 请求 Quote 的完整链上绑定、曲线 owner/discriminator 的完整验证 |
| PumpAmm | 是，TS-05-B3，显式 Pool ID | 同批 Pool/两个 vault/base Mint；Quote-only vs 双侧检查；虚拟储备和当前 tier | 无 Pool ID 的 GPA discovery；配置失效/TTL/fallback 故障演练 |
| Bonk | 是，TS-05-B3 | WSOL/USD1 PDA；原始 vs 说明错误 | 自动识别任意 LaunchLab 平台或任意 Quote 支持 |
| StonkFun | 是，TS-05-B2 | 标准/奖励平台；严格 base/quote；曲线 + 显式 CPMM hop | graduated 两跳、完整 config/扩展失败矩阵与真实路由 |
| StonkFunSwap | 是，TS-05-B2 | 同 CPMM loader；warmup 双侧成员检查 | graduated Via-SOL 已验收或 CPMM Pool 属于 StonkFun 的完整来源证明 |
| MeteoraDammV2 | 是，TS-05-B2 | Quote-only vs 双侧检查；partial-fill/sysvar | 可信报价、输出量、完整 Token 扩展和实际卖出成功 |
| MeteoraDbc | 是，TS-05-B2 | migrated 优先拒绝；固定 base/quote/sysvar | Config 的曲线可交换性、min_out 或 Token 扩展的完整验收 |
| RaydiumAmmV4 | 是，TS-05-B3 | Quote-only vs 双侧检查；PnL 和市场账户映射 | 完整 AMM/market owner/status/关联性验证 |
| RaydiumCpmm | 是，TS-05-B2 | Quote-only vs 双侧检查；费用扣减、Token-2022 epoch/hook | 可执行 VerifiedRoute、完整状态/上下文时效及真实交易成功 |

九类成功仅指现有参数 helper 在合成可解码账户上成功，不能反向推导服务已安全支持任意输入。Registry、API 白名单和 wire aliases 本轮不变。上表记录 loader 边界，不表示其它服务层检查均不存在。

## 样本与缓存约束

- AMM v4 使用 SDK typed `AmmInfo`/`MarketState` 经固定宽度 bincode 编码，再由 SDK Borsh decoder 检查长度及逐字段往返。Serum vault signer 的期待值以 Solana PDA API 独立推导，不调用被测 SDK helper。
- Bonk 复用当前 LaunchLab 合成账户，显式把池注册到推导地址；测试不把 StonkFun 平台限定自动施加给 Bonk。
- Pump 手工构造当前原始字段 tuple 和附加字段，排除 Borsh-skipped 投影；以 `decode_body` 检查布局。Global 为当前窄 decoder 的 73-byte 合成样本，sharing 为当前 reader 使用的 43-byte 前缀，不是完整配置有效性的证据。
- PumpSwap 同样排除 Borsh-skipped 字段，并与当前 SDK decoder 做 Pool 往返。Global/FeeConfig 使用当前结构化字段序列与 Borsh `u32` tier 长度编码，先独立 decode 检查，不用 bincode Vec 的 `u64` 长度冒充 Borsh。
- PumpSwap SDK 有进程全局配置缓存：所有场景提供完全相同的配置，recipient 集合重复同一地址，避免随机选择/并发缓存写入使断言不确定。只精确断言三次四账户快照的地址和次序；配置读取次数允许命中独立缓存。
- 本批不清空/修改 vendor 静态缓存、不启动后台刷新任务，也不测试 TTL 失效、不同配置争用或节点切换。后续需要独立进程或可隔离的适配器测试，不得为测试便捷修改 vendor 行为。
- 严格内存 RPC 的未登记读取/未知方法会留下 violation；即使 SDK 将错误转换为配置 fallback，测试也不会静默放过未知请求。

## 验证命令

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

默认并发及单线程复跑均为：267 passed、0 failed、无编译警告。参数模块由 26 → 38 项；旧 API/hash/账本/恢复合同全部保留。

Cargo.toml、Cargo.lock、现有 release SHA-256 与 `SDK_PARAMETER_FIXTURES.md` 一致，vendor diff 为空。现有运行二进制未替换，生产配置/数据库/钱包未读取或修改。

## 下一执行项

1. TS-04-E：普通执行后端隔离。先固定 `executor::map_status` 的原始状态/签名/错误映射，再引入项目拥有的执行结果与可注入后端，去除普通编排对 SDK 参数/result 的直接依赖。
2. 后端拆分需保留钱包解析与 building/submitting 持久化、blockhash 检查的原顺序；不能把原持有的钱包 Arc 改成存在性预检查 + 二次查询，避免引入删除竞态。legacy 聚合与普通 SDK/PumpAPI 的分支优先级保持。
3. fake 执行路径必须覆盖成功、明确失败、带签名但不确定、持久化拒绝和禁播，不能用类型移动替代状态和调用计数验收。
4. TS-03-B VerifiedRoute、全协议资产扩展/身份/时效证据、发现与费用缓存故障测试继续作为独立队列；本批参数成功不自动启用这些能力。
5. 无需等待上游 SDK provenance 才做离线边界工作；上游升级、真实节点、release 构建/部署和资金验收仍保持单独授权窗口。
