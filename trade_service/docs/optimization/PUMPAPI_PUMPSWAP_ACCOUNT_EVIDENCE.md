# TS-S1-H2-B2-C PumpAPI PumpSwap 账户证据

日期：2026-11-13。接续 `PUMPAPI_CPMM_ACCOUNT_EVIDENCE.md`。仅 Trade Service 源码、离线测试和文档；不升级 SDK/依赖/vendor、不构建 release、不部署重启、不访问生产钱包/数据库、不调用真实 HTTP/RPC、不签名或广播。

本阶段处理 PumpAPI backend 的 PumpAmm / PumpSwap 直接路线，**Pump.fun 内盘和其他协议未在本批完成**。普通 SDK 交易路径也未因此接入同样保护；完整支出授权、签名证据和发送前 journal 仍待后续。

## 实际边界

- `pumpapi_pool_accounts.rs` 是模拟/Live 共用的实际分发点，CPMM 两枚举沿原入口、PumpAmm 进入新适配器，其余返回 None 明确表示尚未检查；不是授权或 capability 开放。
- `pumpapi_assets.rs` 提取 CPMM 已有的只读 GMA、Base64/owner/长度与 SPL/Token-2022 校验。现在两个协议确实复用，错误使用固定协议 label，CPMM 原上下文/检查顺序保持；没有要求 PumpSwap 依赖 CPMM 私有实现。
- Mint 证据新增实际使用的 decimals/supply/必需扩展；vault 校验接收协议自己的 authority。CPMM 继续全局 authority，PumpSwap 使用 Pool 自身，不把两者混用。
- 共用 vault 校验补重复扩展拒绝，避免坏 TLV 中后置重复项被首项读取掩盖。原坏 hook payload/active hook 等保护保留；未添加依赖或弃用接口。

## 单一 Bank 快照

先按预期 AMM program、唯一直接 swap、已知 opcode/数据尺寸和固定核心角色识别候选。没有 poolId hint 时，只从该指令的 pool 位置取得候选；有 hint 时必须相等。候选不等于可信路由。

一次逻辑 getMultipleAccounts 读取 pool、两 Mint、两 vault、固定 GlobalConfig/FeeConfig 与 Clock 共 8 个账户，Base64/processed、minContextSlot 不低于 ALT 视图，5 秒共同 deadline。返回后核对 PoolState 的所有对应引用、两个不同的本地资产及相关 owner/布局/状态，不能只看消息地址存在。

没有寻池 GPA、余额查询、getEpochInfo、节点回退或应用层重新查询。使用现有 SDK 的 `pool_decode`、Global/Fee decoder、signed virtual reserve 与 fee-tier 计算接口，不读它们的进程全局缓存，不随机决定费用接收方。RPC transport 的物理重试及底层 HTTP 体积仍属基础设施范围，不冒称已全面限制。

Clock 必须与响应 slot 一致；检查通过的 bank slot 推进后续 simulate/send 的 minContextSlot，原 unsigned bytes/blockhash/零签名槽不改。它不锁定状态、不保证同一分叉或墙钟新鲜度，也不构成可信报价/签名许可。

## 已核对的事实

1. Pool program owner、discriminator 和 SDK 支持的新/旧布局；pool/Mint/vault 引用、两侧不同资产与本地 Mint/有效 Quote 精确匹配。
2. 使用消息角色只是定位，不信任其 Token Program：从同 bank Mint owner 确定程序，再校验 Mint/vault 的初始化、归属、权限、必需 Token-2022 扩展及可接受状态。
3. 两端物理 reserve 必须正数，signed virtual quote 使用 SDK checked helper，负结果/不可表示结果拒绝，不用 saturating 掩盖；Mint supply 来自标准解析。
4. Global/FeeConfig 由固定地址和正确 owner 读取，header/layout/费率总和、严格升序 tiers、当前 pool 选中的 creator override/tier 经 SDK 算法核对。畸形、缺失或空的必要配置不回退到静态默认费率。
5. 高层 buy/sell 与链上 opcode 不强行同名：资金为 pool quote -> base 才应使用 buy/buy_exact_quote_in，否则为 sell；两种池朝向都有测试。买入资金精度仍必须符合 H1 的 6/9 位 wire 规则，不静默更改单位。
6. global、base/quote Mint、vault、Token Programs、system/ATA/event/AMM program、payer 与用户 token 账户的已知请求权限/基本非别名要求。
7. protocol fee recipient 按同 bank GlobalConfig 和 Mayhem 模式选集中核对；quote ATA、creator vault PDA/quote ATA、volume accumulator、cashback quote ATA、可选 pool-v2、buyback recipient 及其 quote ATA 按当前布局逐项检查。额外、缺失、错位 remaining accounts 拒绝。

GlobalConfig SDK decoder 不返回 disable_flags。本批以已知布局偏移定位该字节，**只接受零值**；非零返回“需要已审计策略”，不猜测位语义或把关闭建池等同关闭交易。这是保守限制，发布前需补逐位协议证据。Global 其他未投影的 feature flags 也不称为完整业务状态审计。

## 合同与限制

API/validator/旧 key/hash/schema/默认后端不变，H1 80 请求黄金指纹不刷新。模拟不签名、不返回签名。原 CPMM 的两阶段 bank 检查、错误 label 和读取行为保留，未借提取公共函数顺便改策略。

有意收紧的拒绝包括：不支持的组合/多跳、非唯一直接 AMM 指令、未知布局/额外 remaining account、未审计的非零 disable flags、配置缺失/坏费表、过期 ALT 上下文、错误 fee recipient/ATA/PDA/权限与不匹配的 Quote 精度。并不声称所有旧 provider 输出仍可放行；例如另附 claim_cashback 的同 program 组合仍需单独审计。普通 SDK 不受这一 PumpAPI 拒绝策略影响。

这些是账户/费用配置证据，不验证实际输入整数、可信 min_out、用户 token 账户实际归属/余额与 ATA/seed/临时 WSOL 生命周期、全部顶层或 inner 指令/费用/收款副作用、blockhash 有效期。部分费用/volume/creator ATA 只核对预期地址和消息权限，尚未证明链上存在或可由哪些创建指令初始化。合成 min_out=1 仅为格式 fixture，不授权实盘使用该值。

## 离线证据

新增 15 项，包含 64 组 buy/sell × 池方向 × Mayhem × Cashback × creator × hint 组合：

- 单 GMA 八账户与正确 bank slot、无 hint 候选验证；真实内存 RPC 模拟的原 bytes 与 null signature。
- hint/opcode/多 swap/track flag 的提前拒绝；坏 pool/header/候选引用/资产/缺失，Mint/vault authority/frozen/program 不一致。
- 同 bank recipient 配置轮换不使用静态/进程缓存；坏 Global/Fee/flags/rates/reserves，空/乱序/溢出 tiers 与 Clock 不一致。
- creator/cashback/tail/方向错位、ALT 最低上下文、错误 URL 脱敏、取消和真实 5 秒超时，无应用层第二次读/模拟/发送。
- 混合 Token Program/inactive/active hook/必需与重复 vault 扩展、6/9 位自定义资金精度；SDK 旧 Pool 布局与两种 buy opcode。

只构造公开固定地址/零签名槽，所有节点均是进程内 sender，未构造 signer。CPMM 原 17 项与全部既有合同也随完整回归执行。取消 helper 不代表取消 HTTP 会终止已 spawn 的普通 executor，不授权换 key/释放钱包或自动重签补单。

最终验证全部通过，`EXECUTION_STATUS.md` 已同步：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 776 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 776 passed
cargo test --locked --offline --quiet http_contract_tests  # 153 passed
cargo test --locked --offline --quiet pumpapi_h            # 81 passed, includes this stage's 15
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅两类 models/persistence 兼容重导出警告，无新 allow/弃用警告。Cargo.toml、Cargo.lock、既有 release SHA-256 仍为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原 migration 无 diff，磁盘约 2.3 GB 可用。没有 release 构建、服务重启或运行验证。

## 后续

继续 Pump.fun 曲线、Bonk/StonkFun 曲线、DAMM v2/DBC/AMM v4 账户证据；保留其他协议未验证状态，不引入无界扫描。B3/B4 再完成全部指令/资产支出/min_out/费用/生命周期及授权绑定，H3 为普通交易落盘原签名和字节后才可广播，H4 继续 SDK/legacy 组合路径。完整优化和发布门槛尚未达成。
