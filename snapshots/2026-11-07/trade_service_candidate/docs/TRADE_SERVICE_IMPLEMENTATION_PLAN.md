# Trade Service 实施计划

更新时间：2026-11-03

## 目标

Trade Service 是唯一的交易执行边界，负责钱包、路由验证、报价、交易构建、模拟、签名、广播、幂等和状态回查。`bot_trading` 只发送稳定的业务请求，不接触 SDK 类型、私钥或链上账户布局。

系统支持持续升级的 SDK 和逐协议上线。升级 SDK 时不迁移钱包、不修改订单 schema、不改变 Public Trade API，也不影响首两笔快速跟随路径。

## 当前基线

- 代码：`/root/solana/trade_service_backup_20261102`
- 数据：9 个钱包、`wallets.sqlite`、`trades.sqlite`、`conversions.sqlite`
- 钱包：`WALLET_STORAGE_MODE=database`
- 持久化：`TRADE_PERSISTENCE_MODE=durable`
- 监听：`127.0.0.1:18787`
- 初始模式：`ENABLE_SIMULATE=true`
- 旧 SDK：`sol-trade-sdk 6.0.2`
- 候选 SDK：`sol-trade-sdk 11.0.1`

## 服务边界

```text
PumpAPI/ZMQ
  -> bridge: raw / observation / normalized
  -> bot_trading: 策略、首两笔、Alpha、风控、幂等
  -> Trade API: 稳定请求模型
  -> route registry: program/pool/mint/quote/token program 验证
  -> execution backend: SDK adapter 或 DBC legacy
  -> RPC: 模拟、签名、广播、状态查询
```

Trade Service 不接收私钥。私钥只来自数据库加密字段，passphrase 只来自受限文件或受控运行环境。

## 稳定数据模型

`VerifiedRoute` 保存：

```text
route_id, protocol_id, program_id, pool_id, mint
input_mint, output_mint, quote_mint, token_program
validated_slot, observed_at_ms, expires_at_ms
supports_buy, supports_sell, supports_sol_route
supports_token2022, execution_backend
```

`Quote` 必须绑定钱包、路由、方向和整数金额，并返回输入、预计输出、最小输出、费用和过期时间。

`Order` 保留现有 `request_id`、`wallet_id`、`idempotency_key`、状态、签名和错误字段。`unknown` 必须回查原 request_id 或幂等键，不能自动换新幂等键重试。

## Public Trade API

继续兼容：

```text
GET  /health
GET  /ready
GET  /v1/capabilities
GET  /v1/wallets/manage
POST /v1/pools/resolve
POST /v1/trades
GET  /v1/trades/:request_id
GET  /v1/trades?limit=N
```

建议增加：

```text
POST /v1/pools/resolve-candidates
POST /v1/routes/quote
POST /v1/routes/simulate
```

API 不暴露 SDK 版本、SDK 类型或内部账户结构。`/v1/capabilities` 返回协议状态、Quote 支持、Token Program 支持、买卖方向、SOL 路由和当前 backend。

## Backend 设计

所有 backend 实现同一组内部操作：

```text
capabilities()
resolve_pool(hint)
quote_exact_in(route, request)
build_buy(route, quote, request)
build_sell(route, quote, request)
simulate(transaction)
submit(transaction, idempotency_key)
```

版本和协议隔离为：

```text
backend-sdk6
backend-sdk9
backend-sdk11
backend-sdk-next
backend-dbc-legacy
```

协议路由使用内部 `protocol_id` 和 capability，不直接匹配 SDK 的 `DexParamEnum`。SDK 类型只能出现在对应 backend 内。

## 协议上线顺序

第一批：Pump、PumpSwap、Raydium CPMM、Raydium AMM V4、StonkFun/LaunchLab、Meteora DAMM V2。

第二批：Raydium CLMM、Meteora DLMM、Orca Whirlpool。先进入 observe/simulate，完成 tick、bin、bitmap、Token Program 和池状态过期处理后再进入 canary。

独立适配：Meteora DBC、Meteora DAMM V1、Meteora Launchpad。PumpAPI 的协议字符串只能作为候选，必须使用 program ID、pool owner、池状态、mint、quote mint 和 token program 验证。

## SDK 升级规则

每个候选 SDK 使用独立 release 和固定 lockfile：

```text
/root/solana/trade_service/
  current
  previous
  candidate
  data/
  env/
  secrets/
```

`data/`、`.env` 和 passphrase 位于版本目录之外。SDK 11.0.1 是候选 backend，未来 SDK 12 或更新版本继续新增 backend，通过验证后切换 `candidate -> current`。

SDK 11.0.1 已移除旧 DBC/Zap 依赖，而当前代码仍使用 `MeteoraDbcParams`。因此 SDK 11 backend 先负责已支持协议，DBC 走 `backend-dbc-legacy` 或保持 observe/simulate；不把 SDK 6 的 DBC vendor 混入 SDK 11，也不通过删除依赖强行编译。

## 路由验证顺序

1. 校验 mint、pool_id 和 program_id。
2. 验证 Pool owner。
3. 读取池状态、两个 mint、vault 和 token program。
4. 校验 quote mint、decimals、Token-2022 transfer fee。
5. 生成带 slot 和过期时间的 `VerifiedRoute`。
6. 使用整数单位计算 quote 和 `min_out`。
7. 检查钱包余额、ATA 和租金。
8. 构建交易并模拟。
9. 仅在 capability 为 `live` 且风控通过时签名广播。

## SOL 两跳

只允许经过验证的原子路径：

```text
SOL/WSOL -> Quote Mint -> Target Token
```

每一跳都必须有独立池验证、报价、过期时间和模拟结果。不能把 `amount_sol` 直接解释为 USDC、USD1 或其他 Quote 单位。

## 性能设计

- 进程内选择 backend，避免 DBC 通过 HTTP 二次转发；
- 启动时注册 capability，热路径只做表查找；
- 缓存已验证 Pool、Mint、Token Program 和短期 quote；
- 冷路径完整验证，热路径复用未过期 route；
- 首两笔跟随继续在 `bot_trading` 内存匹配；
- 记录事件到达、路由验证、报价、构建、模拟和提交耗时。

## 发布阶段

1. **Observe**：全部协议进入 bridge 和 observation，未知协议只记录。
2. **Simulate**：新增协议只允许报价和 `simulate=true`。
3. **Shadow**：单钱包、单协议、固定幂等键，持续至少 24 小时。
4. **Canary**：单协议、单 Pool、小金额、频率和总额限制。
5. **Live**：按协议逐个开启，失败自动回到拦截或 simulate。

Alpha 保持 Paper；Alpha Shadow、Router Live 和 SDK 首次切换分开进行。

## 验收矩阵

每个协议至少完成 program/pool owner 验证、买入模拟、卖出模拟、Quote 不匹配拦截、stale route 拦截、Token-2022 回归、幂等重放、服务重启回查和小额 canary 买卖。

服务级检查：

```text
cargo check --locked
cargo test --locked
/health
/ready
/v1/wallets/manage
/v1/capabilities
```

## 回滚

回滚只切换 backend 或 `current`，不降级数据库。任何钱包公钥不匹配、Quote 方向异常、模拟失败、幂等重复或状态回查异常，都停止该协议 Live，保留订单和日志调查。

## 当前执行任务

1. 获取 SDK 11.0.1 或更新版本的完整源码和依赖锁定信息。
2. 把 Trade Service adapter 从 SDK 类型中隔离。
3. 完成 Pump、PumpSwap、Raydium、DAMM V2 的 SDK 11 模拟。
4. 为 CLMM、DLMM、Whirlpool 建立 observe/simulate fixture。
5. 保留 DBC legacy backend。
6. 建立 `current/previous/candidate` 发布目录。
7. 本地 Trade Service 完成钱包、ready、能力和模拟验收。
8. 通过 24 小时 Shadow 后再做单协议 canary。

## 具体执行步骤

### 第 0 步：冻结和备份

记录当前 Trade Service 代码提交、Cargo.lock、Rust toolchain、`.env` 的非敏感配置、
数据库 schema、9 个钱包公钥和当前远端 API 状态。保留 SDK 6.0.2 目录作为回退基线，
不在这套目录上直接升级。

验收：钱包数量和公钥完全一致；`wallets.sqlite`、`trades.sqlite`、`conversions.sqlite`
均有可读备份；没有启动新的 Live 进程。

### 第 1 步：拆出稳定内部模型

从现有 `sdk_client.rs` 中抽出不依赖 SDK 的类型和接口：

```text
ProtocolId
VerifiedRoute
Quote
TradeInstructionPlan
SimulationResult
SubmissionResult
BackendCapabilities
```

Trade API、订单持久化和风控只使用这些类型。SDK 的参数、错误和账户结构留在 backend
内部。

验收：SDK 6.0.2 的行为不变，`cargo test` 通过，HTTP JSON 响应结构不变。

### 第 2 步：建立 SDK 6.0.2 adapter

把现有 SDK 调用移动到 `backend-sdk6`，先覆盖当前已经可用的协议：

```text
Pump, PumpSwap, Bonk, StonkFun, DAMM V2,
Raydium AMM V4, Raydium CPMM, DBC legacy
```

这一阶段不引入新协议，也不修改策略和数据库。

验收：旧协议 pool resolve、报价、买卖模拟、幂等重放和服务重启回查全部通过。

### 第 3 步：建立 SDK 11.0.1 adapter

复制完整 SDK 11.0.1 源码和锁文件到 `candidate`，实现 `backend-sdk11`。先适配：

```text
Pump, PumpSwap, Raydium CPMM, Raydium AMM V4, StonkFun, DAMM V2
```

SDK 11 已移除旧 DBC/Zap 依赖，因此 DBC 继续由 `backend-dbc-legacy` 处理，不能把旧
DBC 类型混入 SDK 11 vendor。

验收：SDK 11 单独编译；上述协议的 fixture、quote、simulate 和 JSON 回归通过。

### 第 4 步：接入 backend 注册和路由表

启动时注册所有 backend 的 capability。路由决策只使用：

```text
protocol_id + pool owner + quote mint + token program + backend capability
```

未注册、过期或 capability 不支持的协议只能进入 observe/simulate。

验收：`/v1/capabilities` 与实际执行 backend 一致；未知协议不会误映射为 Pump 或 WSOL。

### 第 5 步：补全全协议的 Observe/Simulate

按顺序加入：

1. Raydium CLMM；
2. Meteora DLMM；
3. Orca Whirlpool；
4. Raydium LaunchLab/平台配置；
5. Meteora Launchpad；
6. DAMM V1。

每个协议先实现 program/pool 验证、池状态读取、报价和模拟，再考虑实盘。每个协议单独
设置 capability，不能通过全局开关一次性放开。

### 第 6 步：实现统一路由和 SOL 两跳

先完成原生 Quote 单跳，再实现：

```text
SOL/WSOL -> Quote Mint -> Token
```

两跳必须使用单笔原子交易，绑定两条 VerifiedRoute、两个 quote 和统一过期时间。任何
一跳过期、余额不足或模拟失败都拦截。

### 第 7 步：本地 Trade Service 验收

使用恢复的钱包数据库和 passphrase，在本机回环地址启动：

```text
127.0.0.1:18787
ENABLE_SIMULATE=true
TRADE_PERSISTENCE_MODE=durable
```

依次检查 `/health`、`/ready`、钱包公钥、capability、pool resolve、quote、simulate、
幂等重放和重启回查。此时 `bot_trading` 仍保持原隧道入口和 dry-run。

### 第 8 步：Shadow 和性能验证

单钱包、单协议、固定幂等键运行至少 24 小时，记录事件到达、路由、quote、构建、模拟、
提交耗时和 RPC 错误。首两笔快速跟随单独记录端到端延迟，确认 adapter 查表没有引入
额外网络跳转。

### 第 9 步：逐协议 Canary

每次只开启一个协议、一个钱包和一个小金额上限。完成一次买入和卖出后核对链上签名、
余额、订单状态和数据库记录。失败时将该协议退回 simulate，其他协议不受影响。

### 第 10 步：切换和后续升级

通过 Canary 后，把候选版本提升为 `current`，旧版本保留为 `previous`。以后 SDK 版本
升级重复第 3 步到第 10 步，不修改钱包数据库、Public Trade API 和策略数据库。
