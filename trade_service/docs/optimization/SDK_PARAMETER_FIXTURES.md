# TS-05-B2 实际 SDK loader 离线样本

日期：2026-11-13。状态：新增 19 项测试，全套 255 项通过；无生产行为改动、依赖/vendor 改动、release 构建、部署、真实 RPC、钱包签名或交易。

承接 `SDK_PROTOCOL_PARAMS.md`。本批只验证参数集中化后的真实 SDK loader，而不是返回预造 params 的 fake backend；不能据此宣布协议实盘验收或全计划完成。

## 只读测试传输

`src/adapters/sdk/protocol_params/tests/read_rpc.rs`：

- 用 `RpcClient::new_sender` 注入纯内存 `RpcSender`，不创建 HTTP transport/socket。
- 只允许 `getAccountInfo`、`getMultipleAccounts`、显式配置的 `getEpochInfo`。
- 以地址映射账户，重复读取不消耗 fixture；不存在账户必须显式登记为 `None`。未登记地址、未知方法和未登记 epoch 报错并记录 violation。
- `sendTransaction`、`simulateTransaction` 一律拒绝，测试验证失败调用也被计数。没有真实交易字节。
- 每个 loader 测试在结束时检查只读请求和 violation，并核对请求数；不会因 loader 吞掉未知 RPC 错误而静默通过。
- boxed future 按已锁定 RpcSender 的 async-trait ABI 实现，没有为测试增加依赖。

## 本批覆盖

| 文件 | 新测试数 | 成功与拒绝证据 |
| --- | --- | --- |
| `read_rpc.rs` | 2 | 重复账户读取、批量顺序/显式缺失、epoch；广播/模拟/未知方法/未知状态拒绝 |
| `cpmm.rs` | 6 | CPMM 与 StonkFunSwap 的真实 warmup/build、两个请求方向；池储备减 protocol/fund/creator fee；完整费用/creator 标志/Token Program；warmup 差异、费用下溢、Mint owner 不匹配、Pool/AmmConfig owner/discriminator/短布局；Token-2022 epoch 17/20 费用切换、上限/向上取整、inactive/active hook |
| `stonkfun.rs` | 3 | 标准和奖励平台的真实内盘 loader、费用/账户推导/储备；目标曲线 + 显式 CPMM SOL hop 的真实组合与读取顺序；反向内盘 Mint/Quote 在费用和 hop 读取前拒绝 |
| `damm_v2.rs` | 3 | 实际 DAMM loader，两种请求方向、混合 SPL/Token-2022 Mint owner、vault、partial-fill、build sysvar=true；warmup Quote-only 与 build Mint 检查差异；缺失 Mint、Pool owner/discriminator/短布局拒绝 |
| `dbc.rs` | 5 | 实际 DBC loader 的 SPL/Token-2022 基础 Mint 参数、曲线储备/价格/配置/账户、sysvar=true；migrated 先于配置/Mint 读取拒绝；反向/陌生 Quote 拒绝；Pool/Config owner/discriminator/短布局、缺失两侧 Mint 拒绝 |

共 19 项，测试中有协议/方向/平台/费用/错误组合，不按组合数量虚增测试总数。旧参数测试 7 项保留，参数子模块共 26 项；全服务 236 → 255。

### 保留原语义

- CPMM 参数维持链上 token0/token1 顺序，不因请求交换 Mint/Quote 而调换 reserve/vault/fee。
- CPMM warmup 只检查 Quote，StonkFunSwap warmup 同时检查 Mint/Quote；两者 build 都检查双侧。测试记录原差异，不在本轮统一政策。
- StonkFun 内盘仍严格 base→quote，不因外盘可以交换输入方向而允许反向内盘身份。
- DAMM 原始 loader 默认 sysvar=false，服务 build 再设 true；warmup 仍只有 Quote 成员检查。
- DBC migrated 检查在配置和 Mint 加载之前，build/warmup 都保留独立 base/quote 方向。
- 所有错误断言对应当前 vendor 源码，不修改已冻结 API/hash/journal 指纹或重生成旧期待值。

## 布局与升级边界

样本都是合成账户，不含钱包、密钥或真实资产，不是链上可交易池。

- CPMM PoolState/AmmConfig 与 LaunchLab PoolState 使用固定宽度 bincode 构造，并与当前 SDK Borsh decoder 做长度及字段往返。CPMM payload 为 629/228 字节，测试检测布局变化。
- LaunchLab SDK 只提供窄的 offset fee reader，测试固定 GlobalConfig/PlatformConfig discriminator、当前费用 offsets，并先经该 decoder 核对。不是通用配置 codec，也不证明完整配置链上有效。
- DAMM 当前类型有 `borsh(skip)`、但没有 `serde(skip)` 的派生字段，不能直接 bincode 整个 Pool。样本显式编码原始字段顺序，排除投影字段，并检查 1104-byte payload 和 SDK decoder 往返；没有修改 vendor codec。
- DBC 使用当前 vendor IDL 和锁定 `dynamic-bonding-curve@3b540e94b5b20ba37733de6e25f58522a0cd8961` 的 zero-copy 布局。测试固定 416/1040-byte payload、已用字段 offsets，并经实际 SDK typed decoder 核对每个字段；不添加 bytemuck/Anchor 依赖或 unsafe 转换。
- DBC Config 未构造可交换的完整曲线，DAMM 也未验证具体报价；这些是 loader 参数 fixture，不能用来声称 min_out、Token 扩展或实际交易已验证。

升级时若布局/错误/默认政策变化，先审查差异，再决定是否适配；不通过更新测试期待值自动扩大服务支持。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：255 passed、0 failed，无编译警告。未运行 release 构建；以下指纹与上一批一致：

| 文件 | SHA-256 |
| --- | --- |
| Cargo.toml | `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89` |
| Cargo.lock | `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5` |
| 现有 release | `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f` |

## 后续队列

2026-11-13 续作：下列第 1 项的四类基础 loader 成功/差异样本已完成，新增 12 项、全套 267 项通过，见 `SDK_LOADER_COVERAGE.md`。graduated hop、完整 config/扩展和真实路由仍按下列边界续作。

1. TS-05-B3：补 Pump/PumpSwap/Bonk/AMM v4 的实际 loader 成功与原 warmup/build 差异。继续使用严格内存 RPC，不能为方便而允许网络或广播方法。
2. 补 StonkFunSwap graduated Via-SOL、目标/hop 的其他失败顺序及 LaunchLab config/Token-2022 分支；本批曲线 Via-SOL 成功不能代替这些路径。
3. TS-03-B：将 owner/两侧 Mint/Token Program/扩展/context slot/有效期作为完整 VerifiedRoute 设计的验收项。DAMM/DBC loader 目前只从 Mint 取 owner，并非完整扩展授权；本批不改变许可。
4. 普通交易接受/持久化的 DTO 解耦、legacy 聚合发送前签名 journal、其它资金入口故障注入继续按主队列推进。
5. 真实 RPC、链上报价、不广播模拟、SDK provenance、上游升级/部署独立验收，仍未完成。不得因 mock 全绿开放额外 Live 或操作生产钱包。
