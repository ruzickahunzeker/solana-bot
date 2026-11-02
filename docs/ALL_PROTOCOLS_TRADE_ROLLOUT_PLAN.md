# Solana 全协议交易支持落地执行方案

版本：v1.0
适用项目：`pumpapi_zmq_bridge`、`copy_trading`、`trade_service`
目标：在保持现有 Public Trade API、钱包、订单、跟单和 SDK 升级边界稳定的前提下，逐步支持 PumpAPI 当前能够接收的交易协议。

## 1. 目标和边界

### 1.1 最终目标

系统能够对以下协议完成事件识别、池验证、报价、模拟买卖和受控实盘买卖：

| PumpAPI 标识 | 统一协议标识 |
|---|---|
| `pump` | `pump` |
| `pump-amm` | `pump-amm` |
| `raydium-launchpad` / `raydium-launchlab` | `raydium-launchlab` 或明确的平台协议 |
| `raydium-cpmm` | `raydium-cpmm` |
| `raydium-clmm` | `raydium-clmm` |
| `raydium-v4` | `raydium-amm-v4` |
| `meteora-launchpad` | `meteora-dbc` 或独立的 Meteora Launchpad 标识 |
| `meteora-damm-v1` | `meteora-damm-v1` |
| `meteora-damm-v2` | `meteora-damm-v2` |
| `meteora-dlmm` | `meteora-dlmm` |
| `orca-whirlpool` | `orca-whirlpool` |

协议名称只是事件提示。交易端必须使用 program ID、Pool 地址和链上账户布局验证协议，不能根据字符串直接选择指令构建器。

### 1.2 服务边界

```text
PumpAPI
  -> pumpapi_zmq_bridge
       原始事件、协议候选、pool_id、program_id、quote_mint
  -> copy_trading
       事件匹配、策略、风控、幂等、跟单决策
  -> trade_service
       路由验证、精度、余额、报价、交易构建、模拟、签名、广播、状态
  -> sol-trade-sdk adapter
       SDK 具体参数和指令构建
```

`trade_service` 是唯一的签名和广播边界。`copy_trading` 不直接调用 SDK，不保存 SDK 具体参数，不自行判断账户布局。

### 1.3 不在本方案内的内容

- 不删除或重建现有钱包、私钥、订单、持仓和审计数据。
- 不修改已有 API 请求字段的含义。
- 不因为 PumpAPI 能收到事件就自动允许实盘交易。
- 不把 SOL 数量直接传给 USDC、USD1 或其他自定义 Quote 池。
- 不使用全量 `getProgramAccounts` 作为每笔交易的默认寻池路径。
- 不用一次全库 `VACUUM` 处理交易数据库空间问题。

## 2. 当前代码审计结论

### 2.1 已有交易能力

当前 `trade_service` 的 `Pool` 和 SDK 路径已经覆盖：

- Pump.fun
- PumpSwap
- Bonk
- StonkFun / StonkFun Swap
- Meteora DAMM V2
- Meteora DBC
- Raydium AMM V4
- Raydium CPMM

这些协议仍需要逐协议验证 Quote、买卖方向、卖出、Token-2022、最小输出量和 RPC 限流后的回退逻辑。

### 2.2 只有 SDK 基础、尚未成为 Public Trade API 能力

当前 vendored SDK 中已经能看到以下构建器或参数类型，但这不等于 `trade_service` 已经可以交易：

- Raydium CLMM
- Meteora DLMM
- Orca Whirlpool
- LaunchLab 的部分参数和指令

交易服务当前的 `Pool` 枚举、校验、路由解析、持久化、跟单白名单和 HTTP API 仍未完整覆盖它们。

### 2.3 当前数据流行为

- Bridge `raw` 通道转发原始事件，不按协议过滤。
- Bridge `observation` 通道用于观察和研究，可以记录未知协议。
- Bridge `normalized` 通道只发送能够映射到当前交易 Pool 的事件。
- `copy_trading` 的跟单路径仍有硬编码协议白名单。
- 交易端 `/v1/pools/resolve` 当前自动解析重点是 Pump、PumpSwap 和 DBC，不能覆盖所有 CLMM、DLMM、Whirlpool 池。

因此，扩展全协议时必须同时修改识别、路由、交易和跟单能力，不能只增加 Pool 字符串。

## 3. 核心设计

### 3.1 统一 Route DTO

在 `trade_service` 内部建立稳定的路由数据结构。该结构属于业务层，不直接暴露 SDK 类型。

```text
VerifiedRoute {
  route_id
  protocol_id
  program_id
  mint
  pool_id
  input_mint
  output_mint
  quote_mint
  token_program
  pool_state_slot
  observed_at_ms
  expires_at_ms
  supports_buy
  supports_sell
  supports_direct_quote
  supports_sol_route
  supports_token2022
  execution_backend
}
```

路由必须记录验证时的 slot 或 block height，避免使用过期池状态构建交易。

### 3.2 协议注册表

建立协议能力注册表，替代散落在多个 `match` 中的协议判断。

```text
ProtocolCapability {
  protocol_id
  aliases
  program_ids
  pool_id_required
  quote_mints
  accepted_token_programs
  supports_exact_in_buy
  supports_exact_in_sell
  supports_sell_percent
  supports_sol_direct
  supports_sol_two_hop
  supports_token2022
  adapter_name
  status: observe | simulate | canary | live
}
```

只有 `status=live` 的协议可以进入 Live 跟单。`observe` 和 `simulate` 协议可以展示或写入测试结果，但交易端必须拦截广播。

### 3.3 SDK Adapter 边界

在 `trade_service` 中增加 adapter 层，业务代码只调用以下抽象操作：

```text
resolve_pool(route_hint) -> VerifiedRoute
load_state(route) -> ProtocolState
quote_exact_in(state, input_amount) -> Quote
build_buy(state, quote, params) -> Instructions
build_sell(state, quote, params) -> Instructions
simulate(instructions) -> SimulationResult
```

SDK 的 `DexParamEnum`、`RaydiumClmmParams`、`MeteoraDlmmParams`、`WhirlpoolParams` 等类型只允许在对应 adapter 内出现。

这样升级 SDK 时，优先修改 adapter；Public Trade API、Telegram、跟单数据库和订单状态结构保持不变。

## 4. 协议实施矩阵

### 4.1 第一批：已有 SDK 基础的标准协议

#### Raydium CLMM

需要实现：

- 增加 `raydium-clmm` Pool 标识。
- 根据 PumpAPI 的 `pool_id` 验证 CLMM program owner。
- 读取 AMM config、pool state、observation state、两个 token vault。
- 根据交易方向加载 tick arrays 和 bitmap extension。
- 读取两个 Mint 的 Token Program。
- 买卖报价使用 exact-in，并计算 `min_out`。
- 验证 tick array 在构建和模拟之间没有过期。

#### Meteora DLMM

需要实现：

- 增加 `meteora-dlmm` Pool 标识。
- 验证 LB pair、reserve、oracle 和 bitmap extension。
- 根据方向加载有效 bin arrays。
- 读取两个 Mint 和 Token Program。
- 处理 bin 状态变化导致的报价失效。
- 买卖均使用当前 bin snapshot 重新报价。

#### Orca Whirlpool

需要实现：

- 增加 `orca-whirlpool` Pool 标识。
- 验证 whirlpool program owner。
- 读取 mint A/B、vault、tick current、tick arrays。
- 处理 swap direction、sqrt price limit 和 tick array 失效。
- 验证 Token Program 和 Token-2022 账户。

### 4.2 第二批：Launchpad 协议

#### Raydium Launchpad / LaunchLab

必须先区分：

- 通用 LaunchLab
- StonkFun 平台配置
- 其他平台配置

不能把所有 `raydium-launchpad` 都映射为 StonkFun。识别顺序为：

1. program ID。
2. platform config。
3. pool 状态和 base/quote mint。
4. 已知平台配置白名单。

通用 LaunchLab 和 StonkFun 需要不同的 `protocol_id` 或明确的 platform 字段，但可以共用底层 LaunchLab adapter。

#### Meteora Launchpad

先确认 PumpAPI 的 `meteora-launchpad` 实际对应哪个 program 和账户布局：

- 如果是 Meteora DBC，映射为 `meteora-dbc`。
- 如果是新的 Launchpad 程序，增加独立协议和 adapter。
- 不能因为名称相似而复用 DAMM V1/V2 指令。

### 4.3 第三批：Meteora DAMM V1

先做 SDK 和链上布局审计：

- 确认是否存在可用的 V1 指令构建器。
- 确认 V1 与 V2 的 pool state、swap 参数和账户顺序不同点。
- 确认 exact-in、exact-out、手续费和最小输出规则。
- 为 V1 单独建立 fixture。

如果 SDK 没有完整 V1 实现，选择以下方式之一：

1. 在 adapter 内补充 V1 指令构建。
2. 使用经过审计的独立协议库。
3. 暂时只观测，直到模拟和账户校验完成。

不能把 V1 请求发送给 V2 adapter。

## 5. PumpAPI Bridge 改造计划

### 5.1 事件字段

原始事件和规范化事件都保留以下字段：

```text
event_id
signature
event_pool
protocol_hint
program_id
pool_id
mint
quote_mint
tx_type
tx_signer
token_program
timestamp
quote_amount
token_amount
raw_hash
```

`protocol_hint` 只能表示候选协议，不能作为交易授权依据。

### 5.2 通道规则

- `raw`：完整转发，用于兼容和故障回放。
- `observation`：完整解析后转发，允许 `observable-only`。
- `normalized`：只发送已识别且字段完整的事件。
- `copy_trading`：收到事件后仍需调用交易服务验证路由。

### 5.3 Bridge 兼容要求

- 旧字段 `pool`、`action`、`txSigner` 继续保留。
- 新字段采用追加方式，不删除旧字段。
- `schema_version` 递增时，消费者兼容旧版本。
- 未知协议不能被错误映射为 `pump`、`WSOL` 或 `raydium-amm-v4`。
- observation spool 必须有大小上限和轮转机制。

## 6. Trade API 改造计划

### 6.1 现有接口保持兼容

保留：

```text
POST /v1/trades
POST /v1/pools/resolve
POST /v1/cache/warmup
GET  /v1/trades/:request_id
GET  /ready
```

旧的 `pool` 值、金额字段、钱包 ID、幂等键和状态字段保持兼容。

### 6.2 新增能力接口

建议增加：

```text
GET  /v1/capabilities
POST /v1/pools/resolve-candidates
POST /v1/routes/quote
POST /v1/routes/simulate
```

`GET /v1/capabilities` 返回协议、Quote、Token Program、直接交易和 SOL 两跳能力。

`POST /v1/pools/resolve-candidates` 接收：

```json
{
  "mint": "...",
  "protocol_hint": "raydium-clmm",
  "pool_id": "...",
  "quote_mint": "..."
}
```

返回候选路由、验证结果、失败原因和过期时间。旧的 `/v1/pools/resolve` 可以继续返回首选路由。

### 6.3 交易金额约束

| 场景 | 输入字段 |
|---|---|
| SOL 买入 | `amount_sol` 或 `amount_lamports` |
| Quote 买入 | `amount_quote_units` |
| Token 卖出 | `sell_percent` 或 token 原始数量 |
| 两跳买入 | `amount_sol` + 两个已验证 Pool |
| 最小输出 | `min_out` 或由 quote 绑定的 `min_out` |

每次执行都必须返回：

- `input_mint`
- `output_mint`
- `input_amount_units`
- `expected_output_units`
- `min_output_units`
- `quote_expires_at_ms`
- `route_id`

### 6.4 路由验证顺序

1. 校验 Mint 地址格式。
2. 优先验证事件提供的 `pool_id`。
3. 校验 Pool owner/program ID。
4. 读取池状态和两个 Mint。
5. 校验 Quote Mint 和交易方向。
6. 读取 Token Program、decimals、转账费配置。
7. 计算 quote 和 `min_out`。
8. 检查钱包余额和 ATA 状态。
9. 构建交易。
10. `simulate=true`。
11. 仅在 Live 且所有风控通过时签名广播。

## 7. SOL 和多 Quote 路由

### 7.1 第一阶段：直接 Quote

先支持钱包已经持有池所需 Quote 的场景：

- WSOL
- USDC
- USD1
- 其他经过能力注册表批准的 Quote

这条路径最容易验证，便于先完成各协议原生买卖。

### 7.2 第二阶段：SOL 两跳

标准流程：

```text
SOL/WSOL -> Quote Mint -> Target Token
```

每一跳都必须拥有：

- 明确 Pool 地址
- 两个 Mint
- 输入输出方向
- 当前报价
- 最小输出量
- 模拟结果

两跳必须尽量使用单笔原子交易。无法保证原子性或其中一跳只有不可信报价时，保持拦截。

### 7.3 路由优先级

```text
已验证原生单跳
  > 已验证 SDK 两跳
  > 经过测试的 Router/聚合器
  > 仅报价，不允许广播
```

Router SDK 作为可插拔 backend，不能替换钱包、幂等、审计、订单状态和交易 API。

## 8. Copy Trading 改造计划

### 8.1 事件处理

跟单收到事件后：

1. 根据 `signature + mint + pool_id + side` 去重。
2. 检查策略允许的协议和 Pool。
3. 调用 Trade Service 路由验证或读取短期缓存。
4. 校验 Quote 与策略金额单位。
5. 校验最大延迟、池状态、市值和钱包金额条件。
6. 生成固定幂等键。
7. Dry-run 先模拟。
8. Live 才进入交易提交。

### 8.2 失败分类

```text
observable-only      可观测，暂不交易
route_unverified     Pool 未验证
quote_unsupported    Quote 不支持
amount_invalid       金额单位或精度错误
stale_route          路由过期
simulation_failed    模拟失败
wallet_insufficient  钱包余额不足
submitted            已提交
unknown              需要按 request_id 回查
```

未知状态不得自动换幂等键重提。

### 8.3 面板能力

当协议能力上线后，Telegram/Web 面板从 `/v1/capabilities` 读取：

- 允许的 Pool
- 允许的 Quote
- 直接买入或 SOL 两跳
- 买入金额单位
- 是否支持部分卖出
- 模拟状态

未上线协议不显示 Live 交易按钮，只显示观察或模拟状态。

## 9. SDK 升级隔离

### 9.1 依赖规则

- 固定 SDK commit 或版本。
- 保留 `Cargo.lock`。
- 每个 SDK 版本建立独立 adapter 测试结果。
- 不把 SDK 类型写入 Public API、数据库或 Telegram 状态。
- SDK 升级不直接覆盖当前生产 vendor 目录。

### 9.2 升级流程

```text
记录旧版本
  -> 编译新版本
  -> adapter 编译
  -> 单元测试
  -> fixture 测试
  -> simulate 测试
  -> shadow 对比
  -> 小额 canary
  -> 按协议切换
```

建议增加配置：

```env
SDK_BACKEND=legacy
SDK_BACKEND_ALLOWED_POOLS=
SDK_BACKEND_LIVE_ENABLED=false
```

升级失败时只切回旧 backend，不修改钱包、订单和数据库结构。

### 9.3 SDK 升级验收

- `cargo check --locked`
- `cargo test --locked`
- 所有旧协议模拟回归
- 新协议买卖模拟
- Token-2022 回归
- ATA 和 immutable owner 回归
- DAMM 固定输出回归
- MEV、tip、blockhash 回归
- API JSON 兼容回归
- 幂等和状态回查回归

## 10. 测试计划

### 10.1 离线测试

每个协议至少准备：

- 一个买入 fixture
- 一个卖出 fixture
- 一个反向交易 fixture
- 一个 Quote 不匹配 fixture
- 一个缺失 pool_id fixture
- 一个 Token-2022 fixture（如协议支持）
- 一个过期池状态 fixture

### 10.2 模拟测试

统一使用：

```json
{
  "simulate": true,
  "idempotency_key": "protocol-test-..."
}
```

模拟测试必须验证日志中实际使用的 program、Pool、输入 Mint 和输出 Mint，不能只看 HTTP 200。

### 10.3 小额实盘测试

每个协议按以下顺序：

1. 只读 Pool 验证。
2. 买入模拟连续 3 次。
3. 卖出模拟连续 3 次。
4. 小额买入 1 次。
5. 确认链上余额和签名。
6. 小额卖出 1 次。
7. 确认订单状态和数据库记录。

任何一步失败，该协议回到 `simulate`，不影响其他协议。

### 10.4 性能和限流测试

重点记录：

- PumpAPI 接收时间
- Bridge 转发时间
- Pool 验证耗时
- Quote 耗时
- 交易构建耗时
- 模拟耗时
- 提交耗时
- RPC 429 次数
- 各 RPC lane 成功率

冷路径和热缓存路径必须分开统计。不能用热缓存结果证明冷路径可用。

## 11. 发布阶段和开关

### 阶段 A：只读识别

- Bridge 接收所有协议。
- 交易端记录候选路由。
- 不修改跟单实盘行为。
- 新协议状态为 `observe`。

### 阶段 B：Trade Service 模拟

- 新协议加入 `Pool` 和 adapter。
- `/v1/pools/resolve-candidates` 可用。
- 所有请求强制 `simulate=true`。
- 记录报价、失败原因和路由耗时。

### 阶段 C：Shadow

- 通过 allowlist 限制单协议、单 Pool、单 Mint。
- 使用固定钱包和固定幂等键。
- 只调用模拟交易。
- 连续观察至少 24 小时。

### 阶段 D：小额 Canary

- 单钱包。
- 单协议。
- 单 Pool 或单 Mint。
- 设置最大金额和每分钟次数。
- 保留旧 backend 作为回退。

### 阶段 E：Live 跟单

- 按协议逐个开启。
- 每个策略独立设置支持的协议。
- 发现路由异常自动降级到拦截。
- 未知协议不自动切换到其他 Pool。

## 12. 回滚方案

### Bridge 回滚

恢复上一版本 bridge，保留 raw ZMQ 字段兼容。normalized 版本变化时，copy_trading 继续接受旧 schema。

### Trade Service 回滚

按协议关闭：

```env
PROTOCOL_RAYDIUM_CLMM_ENABLED=false
PROTOCOL_METEORA_DLMM_ENABLED=false
PROTOCOL_ORCA_WHIRLPOOL_ENABLED=false
PROTOCOL_DAMM_V1_ENABLED=false
```

旧协议继续使用原 adapter。不要删除新协议的数据库记录，以便排查。

### Copy Trading 回滚

关闭对应策略的协议 allowlist，或切回旧二进制。未知和未验证路由默认拦截。

### SDK 回滚

切换 `SDK_BACKEND=legacy`，重启 `trade_service`。不执行数据库降级，不修改钱包和订单数据。

## 13. 执行顺序和工期

| 阶段 | 内容 | 预计时间 |
|---|---|---:|
| 1 | Route DTO、协议注册表、能力接口 | 1-2 天 |
| 2 | Bridge 字段和协议识别 | 1-2 天 |
| 3 | Raydium CLMM | 2-3 天 |
| 4 | Meteora DLMM | 2-3 天 |
| 5 | Orca Whirlpool | 2-3 天 |
| 6 | 通用 LaunchLab 和 StonkFun 分类 | 2-3 天 |
| 7 | Meteora Launchpad/DBC 边界 | 1-2 天 |
| 8 | DAMM V1 审计和实现 | 3-5 天 |
| 9 | SOL/WSOL 多 Quote 两跳 | 3-5 天 |
| 10 | copy_trading 和面板能力接入 | 2-3 天 |
| 11 | 模拟、shadow、canary、文档 | 3-4 天 |

整体预计 `15-25 个工作日`。DAMM V1 和 Meteora Launchpad 如果需要新增 SDK 指令，按上限估算。

## 14. 每阶段完成定义

一个协议只有同时满足以下条件，才算“支持”：

- PumpAPI 能识别并保留正确的 `pool_id` 和 program 信息。
- Trade Service 能从事件或指定 Pool 完成链上验证。
- 买入模拟成功。
- 卖出模拟成功。
- Quote 方向和精度正确。
- `min_out > 0` 且与报价绑定。
- Token Program 和 ATA 处理正确。
- 余额不足会在广播前拦截。
- 重复请求不会重复交易。
- `unknown` 状态能够回查。
- 小额买入和卖出通过链上验收。
- copy_trading 只在能力为 `live` 时提交。
- SDK backend 可以切回旧版本。

仅能收到事件、仅能解析 Pool、或仅能构建离线指令，都只能标记为 `observe` 或 `simulate`，不能标记为生产交易支持。
