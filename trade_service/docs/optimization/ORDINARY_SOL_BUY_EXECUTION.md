# TS-04-E3 普通 SOL 聚合执行边界

日期：2026-11-13。状态：源码完成，新增 14 项离线测试，全套 345 → 359，通过；未升级 SDK/vendor/依赖、构建 release、部署/重启、加载生产钱包/数据库、连接真实 RPC 或广播交易。

## 拆分前基线

先从原 Executor 原样提取请求构造，再捕获并通过 448 组请求/参数校验指纹和原聚合优先级测试，之后才迁移执行边界。

指纹：`26eb7fea8587588a6bf1bf99c7ee1bd41c2102b6b219d6c55432d39266f448b1`。

样本包含 4 类 literal wallet、两种输出 Mint、缺失/零/dust/边界/超额/最大整数买入量、7 类滑点与 u16 溢出，冻结请求字段及原校验错误顺序。迁移后仅机械改为调用内部 intent 与 adapter mapper，未改期待值。

该基线是旧输入映射和纯校验证据，不是原 SDK 钱包成功下单证据；真实旧成功路径未在本批连接主网运行。

## 已实施

- `application/ports/ordinary_sol_buy` 提供单次钱包解析及 SOL 买入执行能力，既不公开 signer、RPC、SDK client，也不提供任意交易签名方法。
- 无 serde 的 4 字段 `SolBuyIntent` 承载钱包 ID、项目 Address、整数 lamports 和原 u16 滑点。`OrdinarySolBuyPipeline::Request` 是 adapter 拥有的关联类型，Application 不引用 wire DTO，也不在每个阶段复制整个请求。
- `application/ordinary_sol_buy` 接项目 TradeCommand，按原顺序执行金额/滑点转换 → 映射/原参数校验 → 报价 → 绑定 → legacy 提交 → 原结果投影。
- `adapters/sdk/ordinary_sol_buy` 私有持有已有 `Arc<SdkTradeClient>` 与共享 QuoteStore，映射现有 quote request，复用现有报价校验/网络实现与 `legacy_quote_submission`。绑定时才读取该已解析实例的公钥，时点仍在报价之后。
- main 的新 backend 只引用现有 WalletRegistry/QuoteStore，初始化不创建额外 SDK 客户端、钱包或 RPC。普通与独立 quote API 仍使用同一个 QuoteStore。
- Executor 聚合分支只解析项目 wallet port 并调用 `buy(command)`；不再取具体 WalletRegistry/SDK 钱包，也不导入 SOL-buy wire DTO 或网络 helper。
- 旧仅供该分支调用的 `aggregator_quote::submit` wire 包装移除，原内部提交编排直接由 adapter 复用；HTTP API 路径、提交字段与响应 DTO 保留，既有 DTO 重导出的测试路径保留。

## 保持的原流程

1. 原服务端禁播策略先于任何解析。
2. BuildingTx 内存更新与原 durable/shadow 写入先于钱包解析；缺失内存记录或 durable 更新失败不继续。
3. 聚合分支优先于 SDK/PumpAPI method 选择，仅解析聚合钱包一次；找不到钱包仍报原 failed/error，先于金额/滑点错误。
4. 聚合分支不要求本地 blockhash，也不额外插入普通 SDK/PumpAPI 的 Submitting 更新。
5. 原校验仍执行显式检查和 quote helper 内部检查；报价 → QuoteStore 绑定 → legacy submit 的顺序不变。
6. simulate/confirmed 仍分别为原模式与其反值；literal key、原整数数量及 Quote 不混用。
7. 原结果分类、签名/错误、结束持久化和参数缓存写入保持，失败也按原 finish 流程记录缓存。
8. 不自动重新报价重试、重建钱包、补单或更换幂等键。解析后句柄保持原 client Arc，不因后续删除/替换注册项而再次查找钱包。

## 新增测试

| 数量 | 范围 |
| --- | --- |
| 1 | 上述 448 组旧请求/校验指纹，拆分前捕获，拆分后不改期待值 |
| 4 | 纯项目 pipeline：map/validate/quote/bind/submit 顺序、两 method 与 Live/模拟确认、金额/滑点错误零调用、逐阶段故障停止、7 种结果与签名形态原样投影 |
| 1 | 真实 adapter 构造/缺失钱包不会初始化 SDK，两个共享 Arc 保持相同对象 |
| 7 | 实际 Executor → Application → fake pipeline → 临时 SQLite：分支优先且无 blockhash/Submitting、lookup 先于非法金额、禁播差异、building 写失败/缺失订单零解析、逐阶段失败/原缓存、删除后保留句柄、结束写库失败留签名且不重复、legacy unknown 无签名仍按旧分类 |
| 1 | 实际 Router 的自然 `amount_sol + USDC Quote` 分支，两 method/模式/Memory-Durable；SOL 原始量、重复返回原订单、同 hash 的 Quote 直买复用 key 被拦截；普通 SDK/PumpAPI backend 零调用 |

fake provider 返回合成报价/签名，测试无 signer、无网络地址连接或可广播交易。Executor 测试的部分内部 flag 组合仅验证分支优先级；自然 API 分支另由真实 Router 测试覆盖。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet legacy_aggregator
cargo test --locked --offline --quiet ordinary_sol_buy
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

结果：旧聚合 2 项、新边界 13 项、HTTP 52 项、全套默认并发与串行各 359 项通过，无测试构建警告。原 98 组结果、normalization/hash/入库/记录字节、协议 loader/恢复合同保持。

Cargo.toml、Cargo.lock 与现有 release SHA-256 仍分别为：

- `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`
- `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`
- `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`

vendor/原迁移 diff 为空；生产配置、数据、服务与二进制未改。本批没有运行生产 profile 构建或真实 SDK 启动验收。

## 保留的风险

- 普通聚合仍不是独立 durable quote submission：没有在本批增加签名前/广播前 journal。
- 旧 legacy unknown 返回不含签名时，普通分类仍可能为 Failed；本批保留并测试该行为，不能据此自动重发。这不是新的最终性或重启恢复保证。
- 结束写库失败只证明内存保留原签名，不证明进程崩溃后签名已持久化。
- QuoteStore、独立 quote HTTP 与其他资金入口仍有 wire/SDK/AppState 耦合；quote 通过也不是 VerifiedRoute 或全部协议资产扩展证据。
- RPC/真实模拟/权限迁移/发布及回滚验收仍需独立条件与授权，本轮不会自行上线。

## 持续队列

1. TS-02-D QuoteStore：先冻结 15 秒 TTL、512 上限、过期删除、wallet/payer 绑定和错误顺序；将缓存内容与输入收窄为项目类型，wire 映射留兼容边界。不得趁迁移改变并发上限语义、Quote ID、时钟或缓存生命周期。
2. TS-06-A 独立报价读取：将 quote request 的项目输入/只读 source/cache port 分开，注入已有已加载钱包身份；保留鉴权/参数校验/lookup/错误与缓存时点。用无 signer 的 fake HTTP 验证实际成功与故障，再接回现有网络 adapter。
3. TS-04-F 独立 quote 的 preflight/submit/reconcile 接口：继续移除 Handler 对具体 SDK 钱包和 raw RPC 的依赖，复用已存在的受限签名、journal、对账端口；不换 key/hash 或扩展原资金政策。
4. 完成这些边界后推进 Application 编排/Handler 文件拆分、协议证据与依赖规则，不先建空目录冒充职责迁移；发送前账本、来源证明、升级/发布演练保留原完整目标。

上述任务均在当前“不升级、不部署、不触碰生产、不交易”的边界内先做源码与离线验证，外部条件不足不阻塞其他可验证的下一项。
