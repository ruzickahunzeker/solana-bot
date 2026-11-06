# sol-trade-sdk 11.0.1 本地编译与迁移计划

更新时间：2026-11-03

> 2026-11-06 执行顺序已更新：本文件保留最初的 SDK 版本迁移背景与检查项；当前交付顺序和完成条件以 [FAST_TRADING_PRIORITY_REVIEW.md](FAST_TRADING_PRIORITY_REVIEW.md) 为准。先交付 SDK 11 单池和原生两跳的 Bot → Trade Service 模拟闭环，同时独立恢复 SDK 6.0.2 的锁定构建。Jupiter、Router 和全协议 adapter 矩阵均不阻塞这两条 SDK 交易路径。`current`、18788 与 live 提交在各自链上验收前保持原状。

## 目标

在本机为 Trade Service 建立可替换的 SDK adapter，SDK 11.0.1 只是当前候选版本。
保持现有 HTTP API、钱包数据库、幂等记录和 `bot_trading` 客户端兼容。升级期间继续
使用模拟模式，不广播真实交易。

业务层不直接依赖 SDK 版本。每个 SDK 版本实现自己的 adapter，钱包、策略、订单和
HTTP API 使用稳定的内部模型。

## 当前基线

- 现有隔离 Trade Service：`/root/solana/trade_service_backup_20261102`
- 当前 vendor SDK：`sol-trade-sdk 6.0.2`
- 当前恢复的钱包库：`data/wallets.sqlite`，9 个钱包，私钥密文可用 passphrase 解密
- 当前 Trade Service 配置：`ENABLE_SIMULATE=true`、`TRADE_PERSISTENCE_MODE=durable`
- 当前本地监听目标：`127.0.0.1:18787`
- `bot_trading` 仍通过 `127.0.0.1:18788` 隧道入口调用，切换前不改该入口

## 阶段一：建立 SDK adapter 基线

已核对官方 `v11.0.1` 标签（commit `c717741d9956f248fd4dd74b4523576bfb74d367`）。
它仍使用 `solana-sdk 4.0.1`、`solana-client 4.2.2` 和 `solana-transaction 4.1.6`，
但把 `sol-parser-sdk` 从 `0.7.13` 升到 `0.11.0`，把 `solana-streamer-sdk` 从
`3.0.11` 升到 `6.0.0`，并移除了 `dynamic-bonding-curve` 与 `protocol-zap` Git 依赖。
这会解决当前 SDK 6.0.2 的失效 Zap revision，但需要检查 Trade Service 是否直接使用
被移除的 DBC 类型。

1. 从旧服务器的 `trade_service-sdk9` 或对应最新源码获取完整 `Cargo.toml`、vendor、
   `Cargo.lock` 和源码版本信息。
2. 在本机创建版本目录，例如：
   `/root/solana/trade_service/releases/sdk-11.0.1-<commit>`
3. 不覆盖现有版本目录；运行目录通过 `current` 符号链接选择版本。
4. 确认实际版本，而不是只看目录名：

   ```bash
   cargo tree -p sol-trade-sdk
   grep -n 'sol-trade-sdk\|solana-sdk\|dynamic-bonding' Cargo.toml vendor/sol-trade-sdk/Cargo.toml
   ```

5. 固定 Rust toolchain、目标架构、Cargo.lock 和 vendor 来源，记录 Git commit 与 SHA-256。

## 阶段二：处理依赖风险

重点检查以下变化：

- `solana-sdk`、`solana-client`、`solana-transaction` 等主版本是否变化；
- `sol-trade-sdk` 的交易参数、Pool 枚举、Quote 类型和错误类型是否变化；
- Meteora DBC、DAMM、DLMM、Zap 相关依赖是否仍使用可获取的 Git revision；
- `protocol-zap` 是否已迁移、内置、改名或从依赖树移除；
- `solana-account-decoder`、`spl-token-interface`、ATA interface 是否出现类型不兼容；
- SDK 是否改变 blockhash、priority fee、Jito/MEV 和模拟交易返回结构。

官方 `v11.0.1` 已不再引用 `MeteoraAg/zap-program`。如果迁移后的 Trade Service
源码仍直接引用 DBC 类型，必须单独确认这些路径是否已改为 SDK 11.0.1 的
`MeteoraDammV2`/`MeteoraDlmm` 接口；不能把旧 vendor 的 DBC 文件混入新 SDK。

## 阶段三：稳定内部接口和版本 adapter

定义不依赖 SDK 的内部接口：

```rust
trait ExecutionBackend {
    async fn capabilities(&self) -> Capabilities;
    async fn resolve_pool(&self, request: ResolveRequest) -> Result<ResolvedPool>;
    async fn quote(&self, request: QuoteRequest) -> Result<Quote>;
    async fn simulate(&self, request: TradeRequest) -> Result<Simulation>;
    async fn submit(&self, request: TradeRequest) -> Result<Submission>;
}
```

每个 SDK 版本实现独立 adapter：

```text
backend-sdk6
backend-sdk9
backend-sdk11
backend-sdk-next
backend-dbc-legacy
```

协议路由只依赖内部 `Pool` 和 capability：

```text
Meteora DBC -> backend-dbc-legacy
其他已验证协议 -> 当前 SDK adapter
未知协议 -> observe/simulate
```

以后升级 SDK 只增加新的 adapter，通过矩阵测试后移动 `current`，不修改钱包、HTTP API、
数据库 schema 或策略代码。

## 阶段四：Trade Service 适配

先保持 Trade Service 的 HTTP 契约不变，只在 SDK adapter 层修改：

- `/health`、`/ready`、`/v1/capabilities`；
- `/v1/pools/resolve`、报价接口；
- `POST /v1/trades` 和交易状态查询；
- 钱包加载、钱包公钥映射、`wallet_id`；
- durable 幂等 `(wallet_id, idempotency_key)`；
- `simulate=true`、unknown 状态和订单持久化。

适配时重点编译并测试：

1. Pump/PumpSwap 买入和卖出参数；
2. Raydium CPMM/AMM V4；
3. Meteora DAMM V2、DLMM、DBC；
4. StonkFun/LaunchLab 路径；
5. SOL/WSOL 转换和报价；
6. blockhash、priority fee、MEV lane 和模拟结果解析；
7. `sol-parser-sdk 0.11.0` 和 `solana-streamer-sdk 6.0.0` 引起的流式解析类型变化。

## 阶段五：本地数据接入

从已验证暂存目录复制数据库，不复制或重建钱包：

- `wallets.sqlite`
- `trades.sqlite`
- `conversions.sqlite`
- `.wallets.passphrase`
- Trade Service `.env`

只调整本机路径和监听地址：

```env
WALLET_STORAGE_MODE=database
WALLET_DATABASE_URL=sqlite://./data/wallets.sqlite
TRADE_DATABASE_URL=sqlite://./data/trades.sqlite
TRADE_HTTP_ADDR=127.0.0.1:18787
ENABLE_SIMULATE=true
TRADE_PERSISTENCE_MODE=durable
```

核对 9 个钱包的公钥、默认钱包和启用状态；不以标签或 `wallet_id` 单独确认身份。

## 阶段六：验证顺序

```text
cargo check --locked
 -> cargo test --locked
 -> 服务启动（仅回环地址）
 -> GET /health
 -> GET /ready
 -> GET /v1/wallets/manage
 -> GET /v1/capabilities
 -> Pool resolve / quote
 -> POST /v1/trades simulate=true
 -> 重复幂等键回查
 -> 服务重启后订单回查
 -> bot_trading dry-run
```

验收必须记录：

- 钱包数量、公钥映射和默认钱包；
- 各协议 capability 状态；
- quote_id、输入输出、min_out、过期时间；
- 模拟状态、request_id 和错误信息；
- 相同幂等键不会重复执行；
- 服务重启后未完成订单进入 `unknown`，不会自动重放。

## 切换和回滚

1. 候选 SDK adapter 通过全部模拟验收后，停止 bot 的 Live 提交路径。
2. 备份当前 `bot_trading` SQLite 和 SDK 6.0.2 Trade Service 目录。
3. 先启动 SDK 11.0.1 Trade Service，确认 readiness 和钱包映射。
4. 让 `bot_trading` 继续通过原 `127.0.0.1:18788` 入口做 dry-run/模拟验证。
5. 连续稳定后再评估将隧道入口指向本地服务；切换前保留原远端地址和旧二进制。
6. 任一钱包映射、quote、模拟、幂等或状态回查异常，立即回到 SDK 6.0.2 目录和原隧道。

## 禁止事项

- 不直接修改当前 SDK 6.0.2 目录的 `Cargo.toml` 版本号；
- 不删除 `protocol-zap` 依赖来绕过编译；
- 不在 SDK 11.0.1 首次启动时使用 `simulate=false`；
- 不把恢复的数据库迁移到另一套 schema 后再回滚；
- 不在 Trade Service 未 ready 时修改 `TRADE_SERVICE_URL`；
- 不同时升级 Router backend、Alpha Shadow 和 SDK 11.0.1。
