# sol-trade-router-sdk 评估结论

评估对象：`https://github.com/0xfnzero/sol-trade-router-sdk`

## 结论

**当前不建议把 `sol-bot` 直接替换成 sol-trade-router-sdk。**

建议采用“保留现有 `trade_service`，将 Router SDK 作为可插拔的报价/路由/交易构造后端”的方案，先针对标准 AMM 做影子对比，再决定是否替换某一条路由。

原因是当前 `sol-bot` 的交易边界明显大于一个路由器：

```text
钱包密钥隔离
  + Trade API 鉴权和 IP 白名单
  + 幂等和持久化状态
  + PumpAPI Local Transaction 执行器
  + PumpFun / PumpSwap / StonkFun / Meteora DBC
  + StonkFun Via-SOL 两跳
  + blockhash、模拟、确认、MEV 和 SWQOS
  + 紧急卖出与运营 readiness
```

Router SDK 最多可以替换其中的“路由和交易构造”部分，不能直接替换这些服务能力。

## 当前 `sol-bot` 基线

仓库当前 `trade_service/Cargo.toml` 已使用 vendored `sol-trade-sdk = 6.0.2`，并且 vendor 目录存在大量本地修改。代码已直接依赖：

- PumpFun bonding curve 和 PumpSwap。
- StonkFun 内盘、StonkFunSwap 和 Via-SOL。
- Meteora DAMM V2、Meteora DBC。
- Raydium AMM V4、Raydium CPMM。
- 本地 PumpAPI executor。
- 自定义池检查、quote/余额检查、Token-2022 兼容和交易状态持久化。

这意味着“替换 SDK”实际上会影响 `sdk_client.rs`、`executor.rs`、`validation.rs`、配置、API 文档、持久化和所有已验证路由，而不只是改一行 Cargo 依赖。

## 对比维度

| 能力 | 当前 sol-bot | Router SDK 必须证明的能力 | 结论 |
|---|---|---|---|
| PumpFun 未毕业曲线 | 已接入并有专用账户解析 | 必须支持当前 PumpFun 版本和动态账户 | 不可假设 |
| PumpSwap 毕业池 | 已接入 | 必须支持实际池发现、报价和卖出 | 需要逐池验证 |
| StonkFun / Via-SOL | 已有自定义两跳和 quote 校验 | 必须原生支持，或继续保留现有适配器 | 不应直接替换 |
| Meteora DBC | 本地扩展 | 必须支持 DBC 状态和毕业切换 | 高风险 |
| PumpAPI Local Transaction | 已有独立执行器 | Router SDK 通常不覆盖第三方交易 API | 必须保留 |
| 私钥和钱包生命周期 | `trade_service` 独占 | SDK 不应接管业务钱包 | 保留现有边界 |
| 幂等、持久化、审计 | 已有 | 通常不是 SDK 职责 | 保留现有实现 |
| MEV/SWQOS/可靠发送 | 已有多路实现 | 需确认发送接口、确认策略和重试语义 | 不能默认等价 |
| 交易模拟和风险门 | 已有服务层检查 | 需逐项映射 | 必须双层保留 |

## 直接替换的主要风险

### 1. 协议覆盖不等于可生产交易

即使 Router SDK README 列出 Pump、Raydium、Meteora 等协议，也需要验证：

- 当前主网 program ID 和 IDL 是否一致。
- 新旧 PumpFun/PumpSwap 账户布局是否兼容。
- Token-2022 transfer fee、hook、ATA 和 decimals 是否正确。
- quote 和 sell 的计算是否与链上实际结果一致。
- 迁移前后的 pool 路由是否自动切换。
- 失败交易是否能识别为可重试、不可重试或需要退出。

### 2. 自定义协议是替换阻断点

当前 StonkFun Via-SOL 是特定的两跳：

```text
SOL -> 自定义 quote token -> StonkFun token
StonkFun token -> 自定义 quote token -> SOL
```

如果 Router SDK 没有同等的 StonkFun 指令、quote 路由和原子交易支持，就不能替换现有实现。缺少 `quote_mint` 时也不能退化为 WSOL。

### 3. PumpAPI 不是普通 DEX 路由

`pumpapi_client.rs` 处理的是 PumpAPI 返回的本地交易，包含响应解析、交易反序列化、金额注入、签名和提交。Router SDK 即使能链上构造交易，也不能自然替代这一条执行路径。

### 4. 本地 vendor 改动会丢失

当前 vendor 目录包含本地 SDK 修改、Meteora DBC 扩展和大量回归 fixture。直接切换 Git 依赖会丢失这些补丁，风险包括：

- 编译接口变化。
- 交易账户顺序变化。
- 模拟通过但实盘失败。
- 原有回归测试失效。
- `Cargo.lock` 和生产构建不可复现。

## 推荐落地方案

### Phase 1：只做源码审计和编译试验

不改生产路径，单独建立 `router_sdk_probe`：

- 锁定 commit，而不是直接跟随 branch。
- 记录许可证、依赖版本、Solana 与 Anchor 版本。
- 统计支持的协议、交易类型、quote 类型、Token-2022 和卖出路径。
- 用当前项目已有 mint/pool fixture 做 quote、instruction、transaction message 对比。
- 运行 `cargo test`、`cargo clippy` 和离线模拟。

### Phase 2：定义后端 trait

在 `trade_service` 内部抽象：

```rust
trait TradeBackend {
    async fn quote(&self, request: &TradeRequest) -> Result<Quote>;
    async fn build(&self, request: &TradeRequest, quote: &Quote) -> Result<UnsignedTx>;
    async fn simulate(&self, tx: &UnsignedTx) -> Result<SimulationResult>;
}
```

保留现有 SDK 后端和 PumpAPI 后端；Router SDK 先作为第三个后端，只允许显式配置的 pool 使用。

### Phase 3：先接入标准 Raydium CPMM

选择标准 `raydium-cpmm + WSOL/USDC` 做对照，暂不接：

- PumpFun 未毕业。
- PumpSwap。
- StonkFun/Via-SOL。
- Meteora DBC。
- PumpAPI executor。

同一请求同时走旧 SDK 和 Router SDK 的 quote/simulation，比较：

- 输出数量误差。
- 最小输出和滑点。
- instruction/account 列表。
- 模拟日志和 compute units。
- 交易落地时间和失败率。

### Phase 4：小额灰度

只允许一个专用 wallet、一个 pool 类型、固定极低金额和 `simulate=false` 白名单。所有交易保留旧后端回退，但禁止自动在两种后端之间重复提交同一订单；仍使用原有 `idempotency_key`。

### Phase 5：逐协议迁移

只有以下条件全部满足，才迁移某个 pool：

- 至少一组买入和卖出主网交易验证成功。
- Token-2022、低余额、过期 blockhash、迁移和滑点失败均有测试。
- 样本外小额交易失败率不高于旧后端。
- 实际成交价、手续费和 compute units 没有明显恶化。
- 已有紧急卖出和持久化审计链路保持可用。

## 必须向上游确认的问题

在拿到目标仓库源码或固定 commit 后，逐项确认：

1. 是否支持 PumpFun、PumpSwap、StonkFun、Meteora DBC，而非只支持聚合器常规 AMM。
2. 是否支持自定义 quote 和 SOL 两跳原子交易。
3. 是否支持卖出、部分卖出和精确输入/精确输出两种模式。
4. 是否支持 Token-2022 transfer fee、hook、ATA 创建和关闭。
5. 是否提供 quote、池发现、账户刷新和交易构造的独立 API。
6. 是否允许调用方自行签名和提交，而不是把私钥交给 SDK。
7. 是否支持现有 SWQOS、Jito/MEV 或自定义 RPC 发送器。
8. 是否有主网 fixture、模拟测试和最新 program ID。
9. 许可证是否允许当前项目的 vendoring 和商业部署。
10. 发布版本、commit、IDL 和依赖是否可复现。

## 最终建议

当前决策为：

```text
不做整体替换
保留现有 sol-trade-sdk + PumpAPI executor
Router SDK 仅作为可插拔后端进行标准 CPMM 对照验证
```

只有当 Router SDK 在主网验证中覆盖当前关键协议，并且交易失败率、成交质量和维护成本均优于现有实现，才考虑迁移单个路由。尤其是 PumpFun、StonkFun Via-SOL、Meteora DBC 和 PumpAPI executor，不应因为 Router SDK 宣称支持多协议就直接切换。
