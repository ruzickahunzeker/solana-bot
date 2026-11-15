# TS-04-A 只读钱包余额边界

日期：2026-11-13。状态：已实现，离线测试记录见 EXECUTION_STATUS；未部署或加载真实钱包。

## 变更

- Domain 新增 `wallet::QuoteBalance`，只有 decimals、原始 Quote 单位与 SOL lamports，不携带 SDK 或私钥。
- Application 新增 `ports/wallet_reads::WalletReadPort`，仅提供 Quote 余额和批量 SOL 余额查询。参数为项目拥有的 Address，返回整数，无 signer、交易字节、广播或原始 RPC getter。
- `adapters/rpc/wallet_reads::RpcWalletReader` 只持有公钥与私有 RPC handles，不含 WalletRegistry、TradingClient、Keypair 或 SDK 实例。
- WalletRegistry 的 `read_access/default_read_access` 在边界取出已有 Default RPC 与公钥，不为每次余额查询初始化钱包/SDK/RPC。两个余额 Handler 不再获取可签名的 SDK client。
- Quote/批量余额查询从 sdk_client 提取到只读 adapter；原 token-balance 扫描逻辑也提取为只需 RPC+公钥的函数，SDK 买卖的余额检查委托它，算法保持不变。
- ATA 推导从 SDK fast helper 切为项目已有 SPL Associated Token 接口；新增 32 组地址/Token Program 组合对比旧 helper，结果一致，不添加依赖。

## 兼容行为

- Quote 仍只查第一个 Default RPC：先 supply、再 token accounts/ATA fallback、最后 SOL balance；不悄悄加并行、重试或改变返回单位。
- 批量 SOL 仍按 Default lane 原顺序查询，每节点 4 秒超时；缺失账户为 0，返回顺序不变，最终错误文本不变。
- Token 账户仍识别 `spl-token` 和 `spl-token-2022` 的解析结果，以 u64 原始字符串累加并保留既有 saturating 行为。
- 空 token accounts 为 0；扫描失败或没有可解析账户走旧 ATA fallback；字段错误与整数溢出不改成余额 0。
- HTTP 继续先做原 loopback/token、钱包归属/enabled、Mint 校验，再取只读接口；响应字段、错误状态和既有 URL 脱敏策略不变。
- 资金入口的禁止广播策略、reservation、构建、报价/滑点和发送逻辑没有改动。

## 测试

新增 8 项离线测试：完整 Quote 读取、超过浮点精度的原始整数、SPL/Token-2022、批量顺序/缺失账户、RPC 失败后备用节点、无 lane 原错误、空账户/饱和求和、非法金额、扫描/未解析 fallback，以及 ATA 对比。

所有 RPC 均使用内存 `RpcClient::new_mock_with_mocks`，没有 socket、payer、真实私钥或生产配置。

## 未完成的隔离

- HTTP 的其他入口仍持有 AppState 并可获取 SDK client，WalletRegistry 也仍是 SDK 钱包运行容器；没有宣称整个 API/Application 已隔离 signer。
- RPC adapter 内部仍有完整 RpcClient 类型；通过私有字段和只读 port 限制消费者，不是 OS/网络层禁止广播。
- Quote 查询原本没有整体 deadline，本轮保留；超时策略、批量响应数量校验、RPC Mint/owner 可信度等应独立审查，不在纯提取中改变。
- 余额成功不等于可交易，不代替费用/租金预留、Token 扩展和路由验证。未运行真实节点或 HTTP 成功钱包 Handler 的集成验收。
- 下一批继续处理原签名/交易执行的能力边界与最小调用方迁移，不把 SDK 的 build-and-send 整体入口冒充独立 build/simulate。
