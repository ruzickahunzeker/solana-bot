# TS-05-B1 SDK 参数映射集中化

日期：2026-11-13。状态：源码及 236 项离线测试通过；未更新 vendor/依赖、未构建 release、未部署、未执行真实 RPC/签名/交易。

## 已实施

`src/adapters/sdk/protocol_params/` 集中保存 SDK 专有映射：

- `mod.rs`：9 个既有 Pool 的 warmup 与 dex_params、StonkFun 平台/跳池加载、Bonk 错误描述。
- `quote_tokens.rs`：买卖两方向的 SDK TradeTokenType、WSOL/USDC/USD1/custom Quote 选择；不把这些 SDK 类型带进 Application/Domain。
- `checks.rs`：原 Mint/Quote 成员检查和原始错误文本。
- `tests.rs`：无 signer 的 RPC/合成账户与本地政策测试。

SDK wrapper 保留原 public warmup 与内部 dex_params 方法，只委托上述函数。参数模块仅接收 RpcClient、Pool、Pubkey 和 hop hints，不接收 TradingClient、钱包、payer 或 Keypair。没有在“只读参数加载”中保留不必要的签名能力。

## 保留差异

这是责任迁移，不是把 warmup 和构建合并成一套算法：

| 项目 | 原 warmup | 原 dex_params，迁移后保持 |
| --- | --- | --- |
| Pump | 读取并拒绝 complete curve | 原参数加载，没有增加该 warmup 检查 |
| PumpSwap | 原 Quote 成员检查，可显式 Pool ID 或按 Mint 找池 | Mint 与 Quote 都检查，保留原先后顺序 |
| Bonk | 原 SDK 错误 | 原 AccountNotFound 说明，按 USD1/WSOL 命名 |
| StonkFun | 原平台、Mint/Quote 检查 | 相同基本检查，再按显式 hop 生成 curve/graduated Via-SOL 参数 |
| DAMM v2 | 原 Quote 检查 | 原 Mint/Quote 检查，并设置 rate-limiter sysvar=true |
| DBC | 原 base Mint 和 Quote 检查 | 原对应参数，不修改 vendor Quote/扩展/min_out |
| Raydium AMM v4/CPMM | 原 Quote 检查 | 原 Mint/Quote 检查 |

- 外层 execution_lanes、URL 脱敏、provider 错误前缀和失败切换仍留在 SDK wrapper，未改变顺序或预算。
- StonkFun 先检查目标池，再加载 hop；hop 仍只允许 Raydium CPMM 并包含 Quote/WSOL，WSOL Quote 拒绝无意义两跳。
- 买入 Pump 的 WSOL Quote 仍映射 SDK SOL；其他既有池为 WSOL；卖出 WSOL 为 SOL。USDC/Bonk、USD1 许可和 custom Token 分支不改变。
- 没有增加新协议、直接取消白名单、放宽 owner/成员检查或改变费用/滑点/输出算法。

## 冻结证据

迁移前用旧 sdk_client 实现捕获 9 Pool × 5 Quote 输入 × 2 方向共 90 组 SDK token/error 结果，SHA-256：

```text
bea3c71308bb919260ed7e6df7852a10f0657a0096195804df1961b7ef068a96
```

迁移后指纹保持；不得重新生成期待值来接受行为变化。这里的 SOL alias 是直接 helper 输入，不代表绕过 API 的 Quote 归一化，不能据此扩大 API 支持声明。

迁移前另有 2 项纯检查基线：池成员及错误方向/顺序/空列表文本、Bonk 原始错误与 Quote 名称。

## 新增测试

除上述 3 项基线外，新增 7 项：

1. 6 个必须 Pool ID 的协议，warmup 与 dex_params 在 RPC 前返回各自原错误。
2. 错误/缺失 hop 类型在 RPC 前拒绝。
3. 固定宽度合成 LaunchLab 数据经 bincode 构造后，与当前 SDK Borsh decoder 做长度/字段往返验证；不是盲写账户 offsets。
4. LaunchLab 错 owner、discriminator、短布局拒绝。
5. StonkFun Mint/Quote 成员检查先于平台检查。
6. 标准/奖励平台允许，其他平台拒绝。
7. 有效可解码但平台未知的 LaunchLab 账户不能当作 StonkFun。

这些测试不创建 SDK TradingClient 或长期 signer。RPC 是内存 mock，所有账户、金额和地址为合成数据，不是可交易池或真实资产证据。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：236 passed、0 failed，无编译警告；全部旧 API/hash/账本/恢复/禁播/路由回归保持。manifest/lock、vendor 和原 release 指纹不变。

## 尚未完成与下一批

2026-11-13 续作：优先协议的真实 loader 成功样本已按 TS-05-B2 完成，详见 `SDK_PARAMETER_FIXTURES.md`，新增 19 项、全套 255 项通过。下面保留本批结束时的待办背景；当前下一批改为 Pump/PumpSwap/Bonk/AMM v4 和补充 hop/config/扩展分支。

- 7 项新增测试主要验证拒绝和数据边界，不能代替 9 个协议成功参数构建、方向/Token Program/费用的完整 fixture。
- 下一批建立严格的只读脚本 RPC sender：只允许账户/必要状态读取，未知请求或 send/simulate 明确拒绝；使用按账户地址返回的合成数据，支持 SDK loader 的多次读取，不启动 socket。
- 优先补 StonkFun/CPMM、DAMM v2、DBC 的成功参数样本和 warmup/build 差异，再逐步补 Pump/PumpSwap/Bonk/AMM v4；不通过假 params backend 代替实际 SDK loader 验证。
- 明确 fixture 所依赖的当前 SDK 布局/Program ID 与局部补丁版本，升级时比对，不把 SDK 默认值自动认定为服务支持。
- 真实 RPC/报价/不广播模拟、完整 VerifiedRoute、SDK provenance、资金路径故障注入和部署仍未验收，不因参数模块提取就开放更多 Live。
