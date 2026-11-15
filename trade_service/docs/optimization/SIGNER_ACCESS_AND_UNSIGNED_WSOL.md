# TS-04-B2 签名权威收口与 WSOL 未签名模拟

日期：2026-11-13。状态：源码及 193 项离线测试通过；未构建 release、未部署、未加载生产钱包或数据库，未发送交易。

## SDK 钱包签名能力

- 移除公开 `conversion_signer()`；现存消费点均已迁移。私有 `wallet_signer()` 只在 SDK 封装内部使用，不向其他模块返回长期钱包 Keypair。
- `sign_prepared_quote` 只接受 B1 的不透明 Prepared 对象，先检查禁播策略与绑定后签名；SDK adapter 不再读取钱包私钥。
- `sign_prepared_wsol` 只接受 WSOL 已校验的 unsigned 对象，先检查禁播策略，再核对钱包、payer 和所需签名账户，最后在 SDK 内签名。
- Prepared 对象没有公开字段、生产构造器、serde 或 Clone。消费对象可以取得已校验的未签名 message，但不能再把任意 message 重新包装成授权对象。
- SDK 默认策略、初始化和 buy/sell 组合流水线不变；没有升级或修改 vendor。

## 普通交易内的旧聚合路径

`aggregator_quote::submit` 保持旧公开包装，转换为内部命令，调用新增 `application/legacy_quote_submission`，通过既有 SDK backend 复用 prepare/sign/broadcast。

保留以下合同：

1. 钱包、key 和 confirmed 校验在 backend 之前；Live 在 prepare 前及 sign 前检查权限。
2. simulate 只 prepare，不调用 Live 权限、签名或广播；preflight 不可执行则拒绝。
3. Live 成功返回原 submitted/signature/preflight；发送不确定仍保留该旧路径的 unknown、signature=null 和原错误字符串，不偷偷改变既有响应。
4. 没有新增重试、并行发送、独立幂等键或签名账本。它仍由普通交易 reservation 保护，**不具备独立 Quote journal 的发送前签名持久化保证**。

最后一项仍需单独迁移。在未完成兼容设计前，不用此次 adapter 复用冒充完整资金恢复机制。

## WSOL 分离

原 `prepare_wsol_transaction` 在预检阶段就获取真实钱包 signer 并签名；现在拆为：

```text
prepare_unsigned_wsol_transaction
  -> 原 owner/Quote/有效期/余额/租金/费用/指令校验
  -> 私有字段 UnsignedWsolTransaction

simulate_wsol
  -> unsigned Transaction，所有签名占位为零
  -> RPC simulateTransaction，sig_verify=false
  -> 原模拟响应，不签名、不保留 Live key、不广播

submit_wsol
  -> 原 ConversionStore reservation
  -> unsigned prepare
  -> SDK 内受控签名
  -> 原 pre-submit simulate
  -> 原 record_signed
  -> 原 send 与结果持久化
```

- 用钱包公开地址完成 prepare；不把长期钱包 Keypair 传入 WSOL 模块。
- 显式模拟配置保留 `rpc.commitment()`，与原 convenience 方法一致；不能省略后静默采用默认 finalized。已核对当前 solana-rpc-client 4.2.2 源码并回归 processed/confirmed/finalized 三种值。
- 包装、已有 ATA、全额解包和部分解包的 instruction builder 不变；同样的 message、blockhash、费用/租金和预留逻辑保持。
- 部分解包仍生成短期临时账户 Keypair，以在 Live 创建临时账户。模拟只把它的公钥放入 message，不生成签名；这不是从长期钱包读取私钥。
- 实盘签名只允许 owner，或 owner 加已构造的临时账户两名 signer。缺失/新增/顺序错误的签名账户拒绝。
- SDK 签名方法的禁播策略先于钱包和临时 signer 访问；原 HTTP/application/send 前的保护继续保留。

这是明确的内部模拟行为收紧：旧“模拟时签名但不广播”改成“不获取长期 signer、不签名也不广播”，HTTP 响应 schema 和数据库不变。需要在受控发布窗口验收真实 RPC 对 unsigned simulate 的支持。

## 离线证据

新增 12 项测试：

- 7 项 legacy application fake backend 测试，验证模拟零 sign/send、Live 顺序和旧 wire、unknown 不重试、校验错误顺序、预检拒绝、prepare/sign 失败、两次权限检查。
- 4 项 WSOL 测试覆盖 4 种账户/方向组合的原 Message 构造一致、零签名、钱包/临时签名绑定、内存 RPC unsigned 模拟和错误传播。
- 1 项无 SDK lane/真实 signer 的 WSOL 禁播测试；如果检查顺序回退，会 panic/失败，不会发网络请求。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
rg -n 'conversion_signer' src
```

结果：193 passed、0 failed，无编译警告。最后一条搜索无结果；私有长期 Keypair 仍存在于 SDK 内并受控使用，没有假称服务不再持有私钥。普通/聚合 V1 fixtures 与 hash、恢复、鉴权、注册表等旧回归同时通过。

## 剩余边界

- 没有真实钱包/真实 RPC 的成功签名、simulate 或发送验收；离线只比较 unsigned Message，不把它写成实际签名字节或链上成功证据。
- WalletRegistry/AppState 仍持有 SDK 运行实例，其他 Handler/转换流程仍可拿 raw RPC。PumpAPI Local 的内部执行仍使用 SDK TradingClient/payer，需要后续 adapter 和审计；并非整个服务完全隔离签名能力。
- 旧内部聚合恢复、SOL 转账持久化失败、WSOL 取消/崩溃/广播后落盘失败等仍有各自风险，发布前需专项故障注入。
- SDK9 工作目录、Copy Trading、Bridge、生产 .env、钱包、数据库与原 release 都未修改。

下一批优先把 blockhash 缓存的 SDK/RPC 依赖移到只读 port/adapter，让缓存与执行命令使用项目拥有的哈希值；再推进其它成功 Handler 的 ports/fakes 和故障回归。
