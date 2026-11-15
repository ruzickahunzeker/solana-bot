# TS-S1-H2-B2-B PumpAPI CPMM 池与资产检查

日期：2026-11-13。接续 `PUMPAPI_MESSAGE_ACCOUNT_VIEW.md`。本批只针对 PumpAPI backend 的 RaydiumCpmm / StonkFunSwap 直接路线；两个枚举共用真实 CPMM program，不以此证明发行来源。**不是全协议或完整交易授权完成**。

只改 Trade Service 源码、离线测试和记录；不升级 SDK/依赖/vendor、不构建 release、不部署重启、不访问生产钱包/数据库、不调用真实 HTTP/RPC、不签名或广播。

## 实际接线与范围

新增 SDK adapter `pumpapi_cpmm.rs`。模拟与 Live 均在已有消息/ALT 视图后调用，成功返回本批读取的 bank slot，用作后续 simulate/send 的 minContextSlot；没有改变原消息或 blockhash。Live 的完整指令/支出授权和发送前账本仍未完成，未测试真实签名或广播。

- 只处理上述两个协议枚举；其他协议返回明确的 None，不做此批 RPC，不冒充已验证，也不改变 registry/public capability。普通 SDK 和 legacy 聚合入口未因此获得同样保护，后续 H4 仍需处理。
- 当前要求显式 poolId、唯一直接 CPMM swap、13 个账户、24 字节和已知 BaseIn/BaseOut discriminator。拒绝已知 hop/aggregator 标志；普通 executor 原有 aggregator 优先分支保持，不以此实现新的多跳。
- unsigned helper 现在必须同时接收原项目 command，不能只根据远端消息决定预期资产。原格式/ALT 测试的 test-only wrapper 提供项目 Pump fixture，继续只证明通用边界；新增 CPMM 测试直接走真实 command/检查流程。

## 两阶段有界快照

1. 根据本地 poolId 只读池 owner、discriminator、现有 SDK PoolState 布局及两种不同 Mint，确定可信引用。
2. 同一次 getMultipleAccounts 读取池、两个 Mint、两个 vault、AmmConfig、observation 与 Clock 共 8 个账户。minimum slot 至少为第一阶段 slot，第一阶段又至少为原 ALT slot；不信任节点静默忽略该要求。核对数量和 non-null。
3. 第二阶段重读 PoolState，要求地址、program、decimals/bump 等身份字段与初始引用一致；允许 reserves/累计费用、LP supply 等动态字段正常更新。后续检查只用第二阶段同 bank 的数据，不混用第一阶段财务状态。
4. 整个两读阶段共同使用 5 秒 deadline；没有 getProgramAccounts、getEpochInfo、getTokenAccountBalance、节点回退、额外发现或应用层重试。Epoch/时间来自同 bank 的 Clock，要求其 slot 与响应一致。

使用显式 Base64 UI 数据，先限制编码长度，再限制解码数据/space，所有账户有 8 KiB 上限，Clock 恰好使用 40 字节布局；不使用会对坏 UI 数据 expect/panic 的便利转换。底层 RPC HTTP 体积和 transport 自身物理重试不是本批全面解决的范围。

## 已核对的事实

- 池、配置及 observation 的 program owner、非 executable/非零 lamports；SDK decoder 的已知 header/layout。池两侧资产必须不同，且精确等于本地 Mint/有效 Quote，不只检查其中一端。
- 池 swap-disabled bit、open_time、creator fee 模式范围；Clock timestamp/epoch 与 pool recent_epoch 合理性。关闭建池的 config 标志不当作关闭交易。
- Mint owner 必须与 PoolState 声明且为支持的 Token Program 相符；使用标准 Pack/StateWithExtensions 验证初始化/布局和 decimals。
- Token-2022 只允许已审计的 TransferFeeConfig、inactive TransferHook、MetadataPointer/TokenMetadata、MintCloseAuthority、InterestBearingConfig；按同 bank epoch 检查当期 transfer-fee bps，拒绝活跃 hook、重复/未知 mint 扩展。使用现有非弃用 account_len API 推导必需账户扩展。先显式解码 TransferHook 布局，再调用 get_program_id，避免该便捷 helper 的 .ok() 把坏 payload 误当成未启用；已用保持合法 TLV header 的零长度 hook payload 复现并验证拒绝。
- 金库的 Mint、Token Program、全局 CPMM authority、Initialized/non-frozen、无 delegate/close authority，Token-2022 必需扩展及可接受状态；不把 withheld fee 当作普通可用余额。累计 protocol/fund/creator fees 用 checked arithmetic 扣除，剩余 reserve 必须为正。
- AmmConfig 的 header/layout、各费率范围；observation 必须是 SDK 派生 PDA，owner 正确且有非空 header。**尚未解析 observation 的完整内容或 TWAP，不把这一项称为完整 observation 布局验收。**
- 对照买卖方向核对消息中的 authority、config、pool、输入/输出 vault、Token Program、Mint、observation 以及 signer/writable 请求权限。用户输入/输出 token account 要求非别名、非 signer、可写，但其归属、ATA/seed/临时 WSOL 生命周期仍交 B3 处理，不能只凭地址存在就授权。

CPMM 的 Swap bit 采用其 PoolStatusBitIndex::Swap=2；本轮为本地布局/fixture 证据，不是实时链上版本或供应商输出验收，发布前仍须真实只读/不广播验证。

## 有意收紧与未完成项

公开 API DTO、validator、旧 key/hash/schema、默认后端/模式、HTTP 结构保持；原 H1 请求指纹和全部既有合同仍冻结。新错误可能在旧签名/RPC 之前出现，这是明确的签名前拒绝，不是声称所有旧可达消息保持放行。

买入 Quote decimals 必须与旧 PumpAPI 请求的 6/9 位换算一致：USDC/USD1 为 6，其他为 9；不同精度的自定义 Quote 明确拦截，不把本地整数按错误单位发送实盘。没有修改金额 wire 或影响普通 SDK 的自定义 Quote 支持，后续要接入任意精度仍需独立、经过验收的请求/指令金额设计。卖出 100% 不依赖此买入换算。

本批未核对输入整数是否与本地预算相等、可信报价/min_out、全部费用、小费、额外指令/收款、副作用、用户 token 账户余额/生命周期或 blockhash 真实有效期。BaseIn/BaseOut 识别不是金额授权，测试中 min_out=1 只用于合成格式，不构成允许真实交易设为 1。返回 slot 不是签名许可或跨重启证据。

minContextSlot 不锁定状态或分叉；手续费/Mint 状态在后续 block 仍可变化，必须继续 B3/B4 的预算/min_out/消息绑定和 H3 的唯一持久化发送前证据。其他协议的池/Token 核验仍需逐个完成，不能把 None 或已有 SDK loader 成功当作 VerifiedRoute。

## 离线证据

新增 17 项，真实标准解析器/SDK decoder + 内存 RpcSender：

- 两枚举 × 买卖 × 两种 Mint 朝向；两阶段请求、原 unsigned 字节、bank slot 进入 simulate；没有真实 signer。
- 7 类提前消息/路线拒绝、5 类池/资产/header 拒绝、身份变化与动态费用更新、7 类状态/Clock、8 类 Mint/vault/reserve、6 类 config/observation/编码/限额。
- Token-2022 epoch 17/20、active/inactive hook、未知扩展、缺失账户扩展、hook account 状态和非法当期 fee；不新增依赖/弃用警告。
- 自定义 Quote 6/9 位一致性、ALT -> 池 -> bank -> simulate 的 minContextSlot 和原字节、方向/账户角色篡改、读取失败 URL 脱敏、直接 helper 取消和 5 秒有限超时。

样本均为公开固定地址/合成账户/零签名槽；没有访问生产钱包、余额或历史订单。HTTP 取消仍不保证停止已 spawn 的 executor，错误不授权换 key/自动重签重发。

最终全部通过，`EXECUTION_STATUS.md` 已同步：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 761 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 761 passed
cargo test --locked --offline --quiet http_contract_tests  # 153 passed
cargo test --locked --offline --quiet pumpapi_h            # 66 passed, includes this stage's 17
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅两类 models/persistence 兼容重导出警告，没有新增弃用警告或 allow。Cargo.toml、Cargo.lock、既有 release SHA-256 保持 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原 migration 无 diff，磁盘约 2.3 GB 可用。没有 release 构建或运行服务验证。

## 后续队列

继续 B2 的 Pump/PumpSwap/Bonk/StonkFun 曲线、DAMM v2/DBC/AMM v4 池及 Token 证据，不新增默认寻池扫描。然后 B3/B4 验证所有指令、整数支出/min_out、费用/ATA/WSOL/收款和授权绑定；H3 统一普通签名前后/广播前 journal、取消/恢复/未知结果只查原签名，H4 处理 SDK/legacy 组合路径。后续可在完整指令角色审计后评估由候选账户组成一次同 bank 批读，减少前置池读取；不能为了优化延迟丢掉链上引用核对。

H2-B2-C 续作已完成 PumpSwap 的单 bank 直接路线证据，并把 CPMM 的 read/有界数据与 Token 校验移至实际共用的 `pumpapi_assets.rs`；CPMM 入口仍为本文件描述的两阶段检查，原错误 label/顺序不改。共用 vault 额外拒绝重复 TLV 扩展，旧 17 项继续通过。新分发点是 `pumpapi_pool_accounts.rs`，未实现协议的 None 不作为授权；见 `PUMPAPI_PUMPSWAP_ACCOUNT_EVIDENCE.md`。全套更新至 776，不把其他协议一起计为完成。
