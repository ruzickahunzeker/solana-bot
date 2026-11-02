# PumpAPI + sol-bot Alpha 信号套利落地方案

## 1. 目标与边界

第一阶段目标是做一个可回放、可模拟、可灰度上线的 Solana meme coin Alpha 执行链：

```text
PumpAPI WebSocket
  -> pumpapi_zmq_bridge（单连接、原始/归一化事件）
  -> alpha-engine（特征、评分、风控、目标仓位）
  -> trade_service HTTP API（幂等、钱包、签名、提交、持久化）
  -> PumpFun / PumpSwap / StonkFun / 其它已验证路由
```

职责保持清晰：

- `pumpapi_zmq_bridge` 只接收、规范化和广播数据，不持有钱包、不下单。
- `alpha-engine` 只做事件消费、信号计算、策略状态和下单编排，不接触私钥。
- `trade_service` 是唯一的签名和交易状态权威；策略不能绕过它直接调用链上 RPC。
- 仅把已经通过协议、模拟和实盘回归的池加入可执行白名单。PumpAPI 可观测不等于可交易。

## 2. 推荐先做的 Alpha

先做“事件驱动的短周期顺势 + 早期流动性过滤”，不要一开始做复杂机器学习。

### 2.1 事件窗口

对每个 `mint` 维护滚动窗口：30 秒、2 分钟、10 分钟。由归一化 `buy/sell` 事件更新：

- 买入笔数、卖出笔数和买卖比。
- 买入 SOL/quote 金额、净流入和流入速度。
- 独立交易者数、重复交易者数、最大单占比。
- 价格收益、成交量和估算市值变化。
- `sol_in_pool`、`quote_in_pool`、池深度和毕业/迁移状态。
- 目标钱包命中事件，复用现有 `wallet_watch_recorder` 的观察结果，但不把单个钱包买入直接当作交易信号。

### 2.2 初始评分

每个事件到达时计算：

```text
score =
  0.30 * z(net_buy_flow_30s)
+ 0.20 * z(buy_sell_ratio_2m)
+ 0.15 * z(unique_traders_2m)
+ 0.15 * z(price_momentum_2m)
+ 0.10 * z(volume_acceleration_30s)
+ 0.10 * z(watched_wallet_confirmation)
```

评分前执行：

- 按池类型和时间窗口做横截面标准化。
- 对极端值截断到 `[-3, 3]`。
- 对缺失 `quote_mint`、无有效 `pool_id` 或未知协议只记录，不生成可执行信号。
- 将手续费、滑点、优先费、资金费和预估价格冲击从预期收益中扣除。

### 2.3 入场条件

同时满足以下条件才生成 `candidate`：

- `action` 为 `buy` 或 `sell`，且事件签名唯一。
- 池在 trade-service 白名单内，且路由状态为可执行。
- 最近 30 秒净买入为正，且买卖比超过阈值。
- 独立交易者数达到最小值，最大单占比未超过上限。
- 估算可成交金额大于策略订单金额的 10 倍。
- 预期 Alpha 大于全部成本的 2 倍。
- 资产未处于迁移、暂停、数据延迟或风控熔断状态。

建议默认参数从保守值开始：

```text
单笔名义金额：账户净值的 0.25%~0.50%
单币最大风险：账户净值的 1%
持仓上限：3~5 个
最小池深度：按实际订单金额的 10~20 倍
最大参与率：最近 30 秒成交量的 5%
信号有效期：2 秒；过期必须重新计算
```

## 3. 执行与退出

### 3.1 执行状态机

```text
observed
  -> scored
  -> candidate
  -> risk_approved
  -> submitted
  -> confirmed / failed / expired
```

每次下单必须带：

- `idempotency_key = strategy + mint + signal_signature + side`。
- `source = alpha-engine`。
- `source_signature`，用于从信号追溯到 PumpAPI 事件。
- `simulate`、滑点、优先费和超时参数。

`trade_service` 负责金额字段互斥校验、钱包绑定、幂等、预热、交易提交和最终状态。StonkFun 自定义 quote 只能按现有 Via-SOL 规则执行，不能把 `amount_sol` 伪装成 quote token 数量，也不能把缺失的 `quote_mint` 默认当 WSOL。

### 3.2 退出规则

同时使用硬止损、时间止盈和流量衰减退出：

- 初始止损：入场价下方 8%~12%，按池波动率动态调整。
- 第一止盈：+15%~25% 减仓一半。
- 移动止盈：从最高价回撤 10%~15% 全部退出。
- 时间止损：2~5 分钟没有继续净流入则退出。
- 流动性止损：池深度下降、迁移状态变化或卖出模拟失败立即进入退出队列。

退出必须复用 `trade_service` 的正常卖出路由；不得由策略服务直接发送紧急交易。紧急卖出可调用已有受控 emergency-sell 能力，但仍应记录关联的策略持仓和原始信号。

## 4. 服务和数据改造

### 4.1 Bridge 层

保留现有 `tcp://127.0.0.1:8939` PUB 和单 PumpAPI WebSocket。补充或统一以下字段：

```json
{
  "schema_version": 1,
  "event_id": "signature:action:mint",
  "received_at_ms": 0,
  "trade_service_pool": "pump|pump-amm|stonkfun|stonkfun-swap",
  "mint": "...",
  "quote_mint": "...",
  "pool_id": "...",
  "action": "buy|sell",
  "signature": "...",
  "tx_signer": "...",
  "quote_amount": 0,
  "token_amount": 0,
  "sol_in_pool": 0
}
```

建议增加指标：连接状态、重连次数、解析失败、未知 pool、缺少 quote、ZMQ 丢弃数和每秒事件数。原始事件继续保留，便于回放和审计。

### 4.2 Alpha Engine

当前仓库已经有 `copy_trading/src/strategy/StrategyEngine`、`stream`、`order`、SQLite migration 和 `RiskEngine`。第一阶段应优先在现有 Rust 进程内增加 `alpha` 策略模式和全量事件存储，避免同时维护两套事件去重、订单关联和 Trade API 客户端。只有在吞吐、部署隔离或策略迭代速度有明确需求时，才拆成独立 Python `alpha-engine`；拆分前必须先定义数据库所有权和订单 owner。

推荐的第一阶段模块划分为：

```text
copy_trading/src/stream       ZMQ 订阅、raw/normalized 分流和 spool 消费
copy_trading/src/db           全量事件、特征、信号、订单和风险账本
copy_trading/src/strategy     alpha 模式、窗口特征和评分
copy_trading/src/risk         原子限额、冻结资金和熔断
copy_trading/src/executor     Trade API 调用、幂等和提交状态
copy_trading/src/order        request_id 查询、签名确认和未知恢复
copy_trading/src/position     真实余额镜像、PnL 和退出队列
copy_trading/src/replay       capture/spool 回放
```

如果最终采用独立进程，模块可以按以下 Python 结构迁移，但必须复用相同的事件 schema、`signal_id`、幂等键和数据库迁移：

```text
consumer.py       ZMQ 订阅、断线重连、事件去重
normalizer.py     schema 校验、时间和金额统一
features.py       mint 窗口和特征更新
scorer.py         Alpha 评分和预期收益
risk.py           暴露、池深度、熔断、全局限额
executor.py       调用 trade_service，管理幂等和超时
positions.py      本地持仓镜像和退出任务
replay.py         JSONL/SQLite 历史回放
metrics.py        指标和结构化日志
```

状态建议使用 SQLite：事件索引、特征快照、信号、订单关联、持仓和退出原因分表。交易最终状态以 `trade_service` 的持久化记录为准，本地只保存关联和缓存。

### 4.3 Trade Service 增强

优先复用现有 `TradeRequest` 字段，不扩展签名边界。只补充：

- 明确拒绝未知 `source` 或超出策略订单上限的请求。
- 将 `source`、`source_signature` 写入交易持久化记录，便于收益归因。
- 提供只读的订单状态查询和最近成交查询给 Alpha Engine。
- readiness 中暴露 PumpAPI 执行器、数据库健康、RPC 和 MEV 状态；engine 在非 ready 时停止开仓。

## 5. 风控和停机条件

硬性限制应在 Alpha Engine 和 `trade_service` 两层同时校验：

- 单币、单钱包、全策略和全账户名义限额。
- 每日亏损、连续亏损次数和滚动回撤熔断。
- RPC 延迟、blockhash 过期、模拟失败率和确认超时熔断。
- PumpAPI 事件延迟超过阈值、ZMQ backlog 增长或 schema 错误率升高时停止开仓。
- 迁移、毕业、池深度骤降和卖出不可模拟时只允许减仓，不允许加仓。
- 所有金额使用整数最小单位；浮点数只能在策略层用于估算，进入 API 前必须转换并校验。

## 6. 分阶段上线

### Phase 0：数据验收（1~2 天）

- 固化事件 schema 和字段兼容矩阵。
- 用现有 bridge 录制 24~72 小时 JSONL，不下单。
- 统计事件延迟、重复率、缺失 quote、各 pool 占比和迁移事件。

### Phase 1：离线回放（2~4 天）

- 实现 Alpha Engine 的特征和评分。
- 用历史事件重放，加入手续费、滑点、优先费和失败率模型。
- 输出逐笔信号、理论成交、最大回撤、换手和收益归因。

### Phase 2：影子模式（2~3 天）

- 实时消费 ZMQ，只生成信号和虚拟订单。
- 调用 `trade_service` 的 simulate/quote 能力，不广播真实交易。
- 校验信号到成交报价的延迟、路由匹配、幂等和退出逻辑。

### Phase 3：极小资金灰度（至少 3~7 天）

- 只开 PumpFun/PumpSwap，单笔 0.001~0.005 SOL 或账户净值 0.1% 以内。
- StonkFun Via-SOL 保持显式开关，先单独回归后再加入。
- 每日复核实际滑点、失败原因、卖出成功率和成本后 Alpha。

### Phase 4：扩大协议和资金

- 只有在连续样本外结果稳定后，逐个放开 StonkFun、Meteora DBC 等路由。
- 每次只改变一个池类型或一个风险参数，保留可回滚配置。

## 7. 验收指标

上线前至少满足：

- 原始事件到信号生成的 p95 延迟可测且稳定。
- 事件去重率和订单幂等测试通过。
- 模拟成交与真实成交的金额、quote 和池路由一致。
- 所有真实订单都能通过 `source_signature` 追溯到事件和评分快照。
- 扣除手续费、滑点、优先费和失败订单后仍有正的样本外期望收益。
- 退出成功率、交易失败率和最大回撤均在预设阈值内。

## 8. 第一批实现清单

1. 在 bridge 中固定 `schema_version/event_id/received_at_ms`，补齐指标。
2. 新建 `sol-bot/alpha_engine/`，完成 ZMQ consumer、SQLite schema 和 JSONL replay。
3. 实现上述 30 秒/2 分钟窗口与基础评分，不接真实下单。
4. 增加 `trade_service` 的来源字段持久化和只读状态查询。
5. 完成回放和影子模式报告后，再打开极小资金开关。

## 9. 复审结果和必须修正的假设

原方案可以作为方向性设计，但不能直接按当前代码上线。复审当前源码后，有以下事实需要纳入实现计划。

### 9.1 “Alpha”与“套利”必须拆成两条产品线

当前 PumpAPI 数据主要描述单个池的买卖事件。根据这些事件做净买入、动量和钱包跟随，得到的是**单边事件驱动 Alpha**，收益依赖价格继续向信号方向变化；它不构成无风险套利。

真正的套利需要同时存在两个可执行市场，并且能在成本和延迟后仍保持价差：

```text
市场 A 低价买入
  -> 市场 B 高价卖出
  -> 两腿均成功，或由同一原子交易保证原子性
```

因此上线顺序应为：

1. 先做单边 Alpha，验证数据质量、执行和退出。
2. 再做同一 mint 的跨池价差监控，只生成影子套利订单。
3. 最后才评估双腿原子交易。若只能顺序执行，必须把第二腿失败当作库存风险处理，并设置独立限额。

不能把单个钱包买入或单个 PumpAPI `buy` 事件直接称为套利信号。

### 9.2 当前 Bridge 的真实能力与原方案有差异

`pumpapi_zmq_bridge.py` 当前会：

- 用一个 PumpAPI WebSocket 连接接收原始事件。
- 通过同一个 ZMQ PUB socket 发布原始帧。
- 以 `normalized` multipart topic 发布可被 Trade API 使用的事件。
- 对无法映射到交易白名单的 pool 直接不发布归一化事件。
- 只在原始 JSON 中存在 `quoteMint` 时保留自定义 quote；只对旧 Pump pool 回退 WSOL。

因此原方案中的 `schema_version`、`event_id` 和 `received_at_ms` 目前只是目标字段，不是现有字段。Alpha Engine 必须同时订阅 raw 和 normalized 两种数据，或者 Bridge 增加一个不会丢弃未知协议的 observation 频道。

建议拆成三个逻辑 topic：

```text
raw          原始 PumpAPI JSON，完整保留
observation  经过字段校验但不要求可交易，供 Alpha 和回放使用
normalized   已映射到 Trade API Pool 枚举，供执行器使用
```

ZMQ PUB 没有历史重放和可靠确认机制。实时交易不能只依赖 PUB 的瞬时订阅：Bridge 应同时写入按日滚动的 JSONL 或 SQLite spool，Alpha Engine 启动时先补读 spool 的最近窗口，再订阅实时消息。

### 9.3 价格字段不能直接假设存在

当前归一化消息包含 `quote_amount`、`token_amount`、池储备等字段，但不保证有统一的 `price` 或 `market_cap`。价格计算必须使用 `Decimal` 和明确的 decimals：

```text
price_quote_per_token = quote_amount / token_amount
```

当金额字段缺失、为零、单位不明或 decimals 缺失时，事件可以记录，但不能进入可执行评分。不同 pool 的 reserve 字段语义不能混用；Pump bonding curve、PumpSwap、Raydium CPMM 和 Meteora DBC 应分别实现价格适配器。

### 9.4 Trade API 是异步 API，不是同步成交 API

当前 `POST /v1/trades` 的成功只表示请求被接收或模拟完成。响应可能是 `202 Accepted`，调用方必须保存 `request_id`，随后访问 `GET /v1/trades/:request_id`。当前 Trade API 文档也明确说明：服务通常不会持续推进到 `finalized`，需要链上确认或独立 reconciler。

因此 Alpha Engine 不能把 HTTP 返回的 `submitted` 当成持仓成立，也不能在超时后用新幂等键重复下单。

### 9.5 来源字段目前未形成一等审计字段

`TradeRequest` 已有 `source` 和 `source_signature`，请求 JSON 会被保存，但 `TradeRecord` 和 `trades` 表当前没有独立列，也没有按来源查询接口。要做信号收益归因，应增加迁移，把 `strategy_id`、`signal_id`、`source`、`source_signature`、`signal_score` 和 `risk_decision` 写入 Alpha Engine 自己的数据库；若需要从 Trade API 查询，则再给 `trades` 表增加来源列和索引。

### 9.6 `/ready` 需要鉴权，且不代表策略可开仓

当前 `/ready` 通过和交易相同的 Bearer Token、IP 白名单保护，并只反映 blockhash、RPC、钱包、MEV 和 Trade Persistence 等状态。Alpha Engine 必须把自己的数据延迟、spool backlog、事件解析错误率、可用余额和退出队列也加入 `can_open_position` 判断。

## 10. 目标系统的详细拓扑

```text
                         ┌────────────────────┐
                         │ PumpAPI WebSocket   │
                         └──────────┬─────────┘
                                    │ 单连接
                         ┌──────────▼─────────┐
                         │ pumpapi_zmq_bridge  │
                         │ raw/observation/    │
                         │ normalized + spool  │
                         └───────┬───────┬─────┘
                                 │       │
                         ┌───────▼──┐ ┌──▼──────────────┐
                         │ recorder  │ │ alpha-engine     │
                         │ 原始归档  │ │ 特征/评分/风控   │
                         └───────────┘ │ 持仓/退出/回放   │
                                       └──────┬──────────┘
                                              │ Bearer + IP allowlist
                                       ┌──────▼──────────┐
                                       │ trade_service    │
                                       │ 钱包/签名/幂等   │
                                       │ SDK/PumpAPI      │
                                       └──────┬──────────┘
                                              │ signature
                                       ┌──────▼──────────┐
                                       │ chain-reconciler │
                                       │ 余额/成交/确认   │
                                       └─────────────────┘
```

建议进程边界：

| 进程 | 运行目录 | 职责 | 是否持有私钥 |
|---|---|---|---:|
| `pumpapi-zmq-bridge` | `sol-bot/pumpapi_zmq_bridge` | 单连接、原始数据、spool、topic 广播 | 否 |
| `alpha-engine` | `sol-bot/alpha_engine` | 特征、策略、风控、调用 Trade API | 否 |
| `trade-service` | `sol-bot/trade_service` | 钱包、签名、提交、幂等、交易记录 | 是 |
| `chain-reconciler` | `sol-bot/alpha_engine` 或独立进程 | 读取签名、确认交易、更新持仓 | 否 |
| `wallet-watch-recorder` | `sol-bot/pumpapi_zmq_bridge` | 观察钱包统计和审计 | 否 |

Alpha Engine 和 reconciler 可以先合并为一个进程，但必须保持模块边界，避免把链上确认逻辑塞进评分循环。

## 11. 数据合同和消息设计

### 11.1 事件 ID 和时间

事件 ID 采用以下优先级：

```text
signature + action + mint + pool_id
```

如果 signature 缺失，使用原始 JSON 的稳定 canonical hash；不能用当前时间单独生成 ID，否则重连回放会重复下单。

统一保存三个时间：

```text
event_time_ms       PumpAPI 或链上事件时间，未知则为空
received_at_ms      Bridge 收到时间
processed_at_ms     Alpha Engine 处理时间
```

延迟使用 `received_at_ms - event_time_ms` 计算；事件时间不可信或时钟偏移过大时，降级为观测，不参与短线开仓。

### 11.2 建议 observation schema

```json
{
  "schema_version": 1,
  "event_id": "sha256:...",
  "event_time_ms": 0,
  "received_at_ms": 0,
  "source": "pumpapi",
  "action": "buy",
  "event_pool": "raydium-cpmm",
  "trade_service_pool": "raydium-cpmm",
  "route_status": "executable|observable-only|unknown",
  "mint": "...",
  "quote_mint": "...",
  "pool_id": "...",
  "tx_signer": "...",
  "signature": "...",
  "token_amount_raw": "...",
  "quote_amount_raw": "...",
  "token_decimals": 6,
  "quote_decimals": 9,
  "token_program": "spl-token|token-2022|unknown",
  "sol_in_pool_raw": "...",
  "quote_in_pool_raw": "...",
  "platform": "...",
  "programs_used": [],
  "raw_hash": "sha256:..."
}
```

字段缺失要用 `null` 或 `unknown`，不能把缺失数值写成零。`trade_service_pool` 只代表当前服务的枚举映射；`route_status=observable-only` 的事件可以用于研究，但不得触发下单。

### 11.3 ZMQ 消费规则

- 订阅 `raw`、`observation`、`normalized` 时明确设置 topic，不能假设所有单帧格式相同。
- 对 multipart 消息先检查 frame 数量和 topic，再解析 payload。
- consumer 必须有本地去重表，至少保存最近 24 小时的 `event_id`。
- 当 ZMQ HWM 满或 consumer 进程重启时，从 spool 补读；补读事件默认只用于特征重建，不直接执行过期信号。
- 事件处理应幂等：同一个事件重复到达不能让净流入、交易者数或成交量重复累计。

## 12. Alpha Engine 的具体模块和状态

建议目录：

```text
sol-bot/alpha_engine/
├── pyproject.toml
├── alpha_engine/
│   ├── main.py
│   ├── config.py
│   ├── zmq_consumer.py
│   ├── spool_replay.py
│   ├── schema.py
│   ├── dedupe.py
│   ├── feature_store.py
│   ├── scoring.py
│   ├── cost_model.py
│   ├── route_policy.py
│   ├── risk.py
│   ├── trade_client.py
│   ├── order_state.py
│   ├── position_store.py
│   ├── reconciler.py
│   └── metrics.py
├── migrations/
├── fixtures/
└── alpha-engine.service
```

信号状态：

```text
SEEN
  -> ELIGIBLE
  -> SCORED
  -> CANDIDATE
  -> RISK_APPROVED
  -> ORDER_SUBMITTED
  -> ENTRY_CONFIRMED
  -> POSITION_OPEN
  -> EXIT_REQUESTED
  -> EXIT_CONFIRMED
```

终止状态：`REJECTED_DATA`、`REJECTED_ROUTE`、`REJECTED_RISK`、`EXPIRED`、`ORDER_FAILED`、`UNKNOWN_NEEDS_RECONCILE`。

状态迁移必须带 `reason_code`，例如 `missing_quote_mint`、`stale_event`、`depth_too_low`、`trade_api_not_ready`、`simulation_failed`。日志只记录摘要和 ID，不记录 token 或 Bearer Token。

## 13. 数据库设计

Alpha Engine 使用独立 SQLite，交易数据库仍由 Trade Service 管理。建议最小表结构如下：

```sql
CREATE TABLE events (
  event_id TEXT PRIMARY KEY,
  signature TEXT,
  mint TEXT NOT NULL,
  pool TEXT,
  pool_id TEXT,
  action TEXT,
  route_status TEXT NOT NULL,
  event_time_ms INTEGER,
  received_at_ms INTEGER NOT NULL,
  processed_at_ms INTEGER,
  raw_hash TEXT NOT NULL,
  payload_json TEXT NOT NULL
);

CREATE TABLE feature_snapshots (
  mint TEXT NOT NULL,
  pool_id TEXT,
  as_of_ms INTEGER NOT NULL,
  window_30s_json TEXT NOT NULL,
  window_2m_json TEXT NOT NULL,
  window_10m_json TEXT NOT NULL,
  feature_version TEXT NOT NULL,
  PRIMARY KEY(mint, pool_id, as_of_ms)
);

CREATE TABLE signals (
  signal_id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL,
  mint TEXT NOT NULL,
  pool TEXT,
  pool_id TEXT,
  side TEXT NOT NULL,
  score REAL NOT NULL,
  expected_edge_bps REAL,
  estimated_cost_bps REAL,
  status TEXT NOT NULL,
  reason_code TEXT,
  feature_version TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER NOT NULL
);

CREATE TABLE orders (
  signal_id TEXT NOT NULL,
  request_id TEXT,
  idempotency_key TEXT NOT NULL UNIQUE,
  side TEXT NOT NULL,
  amount_units INTEGER,
  trade_status TEXT NOT NULL,
  signature TEXT,
  error TEXT,
  submitted_at_ms INTEGER,
  updated_at_ms INTEGER NOT NULL,
  PRIMARY KEY(signal_id, side)
);

CREATE TABLE positions (
  position_id TEXT PRIMARY KEY,
  mint TEXT NOT NULL,
  pool TEXT NOT NULL,
  pool_id TEXT,
  wallet_id TEXT NOT NULL,
  token_units INTEGER NOT NULL,
  cost_quote_units INTEGER NOT NULL,
  entry_signal_id TEXT NOT NULL,
  status TEXT NOT NULL,
  high_price REAL,
  opened_at_ms INTEGER NOT NULL,
  closed_at_ms INTEGER
);
```

SQLite 写入使用 WAL 和唯一键。事件、信号和订单写入顺序为：事件去重成功 → 特征快照 → 信号 → 风控决定 → 调 Trade API → 订单关联。调用 Trade API 失败时，订单状态必须保留 `UNKNOWN_NEEDS_RECONCILE`，不能直接标成“未成交”。

## 14. 信号、成本和组合计算

### 14.1 流量特征

对每个 `(mint, pool_id)` 单独维护窗口，避免同一 mint 在不同池之间混合：

```text
net_flow = buy_quote_units - sell_quote_units
buy_sell_ratio = (buy_count + 1) / (sell_count + 1)
unique_traders = distinct(tx_signer)
concentration = max_trader_buy / total_buy_quote
flow_acceleration = flow_30s / max(flow_2m / 4, epsilon)
```

当 `tx_signer` 缺失时，不能把所有事件归为同一个交易者；应将独立交易者特征标记为不可用，并降低信号置信度。

### 14.2 价格和成本

预期净边际应写成：

```text
expected_net_edge_bps =
  expected_move_bps
  - protocol_fee_bps
  - lp_fee_bps
  - estimated_price_impact_bps
  - priority_fee_bps
  - mev_cost_bps
  - expected_slippage_bps
  - failure_reserve_bps
```

只有 `expected_net_edge_bps >= 2 * estimated_total_cost_bps` 才允许进入候选。这个阈值是起始参数，需要由回放校准，不是收益保证。

### 14.3 组合风险

Alpha Engine 维护四类暴露：

- 单 mint token exposure。
- 单池和单协议 exposure。
- 钱包 SOL 可用余额和待结算余额。
- 同时处于 `submitted` 或 `unknown` 的订单 exposure。

风险计算必须把未确认订单算入冻结资金。只要有 `UNKNOWN_NEEDS_RECONCILE`，对应钱包和 mint 暂停新开仓，直到 reconciler 解决。

## 15. Trade API 集成细节

### 15.1 开仓流程

1. Alpha Engine 从 observation 得到候选。
2. 通过本地 route policy 确认 `pool`、`pool_id`、`quote_mint` 和 route status。
3. 调用 `GET /ready`，带 Bearer Token；非 ready 不开仓。
4. 对需要池地址的池调用 `POST /v1/cache/warmup`。
5. 若是标准 SOL exact-in 且需要价格审计，调用 `POST /v1/quotes/sol-buy` → preflight；该 quote 有效期短，过期必须重新取。
6. 构造 `POST /v1/trades`，显式设置 `wallet_id`、金额最小单位、滑点、`simulate` 和幂等键。
7. 保存 `request_id`，轮询 `GET /v1/trades/:request_id`，直到终态或进入未知恢复。
8. 只有 reconciler 看到链上成交和余额变化后，才创建 `POSITION_OPEN`。

### 15.2 金额和路由规则

- SOL/WSOL exact-in 优先使用 `amount_lamports`，减少浮点误差。
- 自定义 quote 必须使用 `amount_quote_units`；StonkFun Via-SOL 才能使用 `settlement_asset=sol` + `amount_lamports`。
- `quote_mint` 缺失或未知时只能记录事件，不能让 Trade API 默认解释为 WSOL。
- PumpAPI executor 当前只支持全量卖出；策略的分批卖出必须走 SDK 路径，并确认余额和池路由。
- 退出订单必须使用新的幂等键，例如 `exit:<position_id>:stop_loss:v1`，不能复用开仓键。

### 15.3 状态同步和恢复

Trade API 重启会将进行中的 durable 记录标记为 `unknown`。Alpha Engine 启动时应：

1. 从本地 `orders` 找出非终态记录。
2. 用 `request_id` 查询 Trade API。
3. 有 signature 时查链上确认和余额变化。
4. 无 signature 时查该 wallet 在提交时间窗口内的相关交易，仍无法确定则人工处理。
5. 解决前冻结该 wallet 的新开仓。

## 16. Chain Reconciler 设计

Reconciler 不重新提交交易，只负责确认事实：

- signature 的交易状态、slot、err。
- 钱包 SOL 余额变化。
- 目标 token 余额变化。
- 交易执行的 pool、quote mint、实际成交金额和费用。
- token account 是否被创建、关闭或冻结。

建议采用指数退避查询：0.5 秒、1 秒、2 秒、4 秒、8 秒，最长 60 秒；超过上限进入 `UNKNOWN_NEEDS_RECONCILE`。实际确认结果写入订单和持仓表，不能用 HTTP 状态推断。

## 17. 风控规则细化

### 17.1 开仓前检查

```text
bridge_connected
and event_age <= 1500ms
and schema_error_rate < 1%
and route_status == executable
and quote_mint is known
and pool_id is valid_for_pool
and liquidity_depth / order_notional >= 10
and max_trader_concentration <= 35%
and wallet_available_sol >= order + reserve
and open_positions < max_positions
and pending_unknown_orders == 0
and daily_loss < daily_loss_limit
```

任何一项失败都只生成拒绝记录。不要用“降低金额后继续下单”绕过数据质量或路由失败。

### 17.2 运行中熔断

| 条件 | 动作 |
|---|---|
| Bridge 连接断开超过 5 秒 | 停止开仓，允许退出 |
| 事件延迟 p95 超过 1.5 秒 | 停止短周期开仓 |
| ZMQ/spool backlog 超过 10,000 | 停止开仓，先恢复消费 |
| 模拟失败率 5 分钟超过 10% | 停止对应 pool |
| 卖出失败连续 2 次 | 停止该 mint 新开仓，提升退出优先级 |
| 日内净亏损达到 3% | 全策略只允许减仓 |
| 组合回撤达到 8% | 全策略暂停，等待人工恢复 |
| 钱包余额低于 gas reserve | 停止开仓 |

参数放在 Alpha Engine 的环境文件或版本化配置中，配置变更必须记录 `config_version` 并支持回滚。

## 18. 观测指标和告警

Bridge 指标：

```text
pumpapi_connected
pumpapi_reconnect_total
pumpapi_raw_events_total
pumpapi_parse_errors_total
pumpapi_unknown_pool_total
pumpapi_missing_quote_total
zmq_publish_total
spool_bytes
```

Alpha Engine 指标：

```text
alpha_event_lag_ms{pool}
alpha_feature_update_latency_ms
alpha_signal_total{status,pool}
alpha_risk_rejection_total{reason}
alpha_trade_api_latency_ms
alpha_trade_api_requests_total{status}
alpha_order_unknown_total
alpha_position_exposure_sol
alpha_pnl_realized_sol
alpha_drawdown
```

Trade Service 侧至少监控 `/ready`、交易状态分布、RPC 错误、模拟失败、签名提交失败、持久化健康和 blockhash age。日志关联键统一使用 `event_id`、`signal_id`、`idempotency_key`、`request_id` 和 `signature`。

## 19. 测试矩阵和验收门槛

### 19.1 单元和契约测试

- PumpAPI 字段别名：`action/txType/tx_type`、`quoteMint/quote_mint`、`poolId/pool_id`。
- 事件 ID 在重连、重复消息和字段顺序变化下稳定。
- 缺失 quote、未知 pool、零金额、异常 decimals 拒绝开仓。
- PumpFun、PumpSwap、StonkFun、Meteora DBC 的 route policy 映射。
- amount 字段互斥和 lamports/quote units 转换。
- 每个状态迁移都有 reason code。
- 同一 signal 重放不会产生第二个 Trade API 请求。

### 19.2 回放测试

使用 Bridge 的原始 capture 和 normalized capture，至少覆盖：

- 正常 buy/sell 序列。
- PumpFun 迁移到 PumpSwap。
- StonkFun 内盘到外盘。
- 缺失 quoteMint 的自定义 quote。
- 两个钱包同 mint 买入。
- 重复 signature 和乱序事件。
- 高流量下 ZMQ HWM 和 spool 补读。

回放报告必须包括：事件数、去重数、可交易数、信号数、理论成交、成本后收益、最大回撤、按 pool 和按小时归因。

### 19.3 影子和小额验收

影子模式至少运行 72 小时，满足以下条件才进入小额：

- 事件去重和 spool 恢复无数据重复。
- Alpha Engine p95 处理延迟低于 200 ms。
- 信号过期率低于 20%。
- Trade API 模拟拒绝率低于 5%，并能解释主要原因。
- 每一笔虚拟订单都能关联到事件、特征快照和风险决定。

小额真实交易至少运行 7 天，逐池评估：

- 实际成交和模拟报价偏差。
- 真实滑点、优先费和失败订单成本。
- 开仓到可确认持仓的延迟。
- 退出成功率和未知订单数量。
- 成本后收益和最大回撤。

## 20. 部署和回滚

### 20.1 配置分层

Alpha Engine 环境文件至少包含：

```env
ZMQ_ENDPOINT=tcp://127.0.0.1:8939
ZMQ_TOPICS=observation,normalized
ALPHA_MODE=replay|shadow|paper|live
TRADE_API_BASE_URL=http://127.0.0.1:18787
TRADE_API_TOKEN_FILE=/path/to/token
TRADE_WALLET_ID=main
ALPHA_DB_URL=sqlite:///var/lib/alpha-engine/alpha.sqlite
SPOOL_DIR=/var/lib/pumpapi/spool
MAX_ORDER_LAMPORTS=5000000
MAX_OPEN_POSITIONS=3
DAILY_LOSS_LIMIT_SOL=0.03
EVENT_MAX_AGE_MS=1500
SIGNAL_TTL_MS=2000
```

真实交易开关必须同时满足：`ALPHA_MODE=live`、配置文件中的 live marker、Trade Service `/ready` 成功和人工部署标记。单独修改一个环境变量不能意外打开实盘。

### 20.2 systemd 顺序

```text
pumpapi-zmq-bridge.service
  -> alpha-engine.service
  -> trade-service.service（独立运行，但 Alpha 依赖 /ready）
  -> chain-reconciler.service
```

Bridge 重启不应清空 spool；Alpha Engine 重启应从最近窗口重建特征；Trade Service 重启后先完成 durable recovery，再允许 Alpha 开仓。

### 20.3 回滚

回滚动作按顺序执行：

1. 将 `ALPHA_MODE` 切到 `shadow`，停止新开仓。
2. 保留 reconciler 运行，完成已有持仓退出。
3. 导出 Alpha Engine 数据库和 Trade Service 订单记录。
4. 回滚 Alpha Engine 版本或配置，禁止改变 Trade API 幂等键格式。
5. 如果是 Bridge schema 变更，保留旧 schema 解析器至少一个发布周期。

## 21. 具体开发顺序

### Sprint 1：数据可靠性

1. Bridge 增加 `schema_version`、`event_id`、三个时间字段和 observation topic。
2. 增加 spool 写入、轮转、磁盘上限和丢弃指标。
3. 为 raw/observation/normalized 增加 fixture 和回放测试。

### Sprint 2：Alpha Engine 影子骨架

1. 建立 Python 包、SQLite migration、ZMQ consumer、去重和 replay。
2. 实现 PumpFun/PumpSwap 的窗口特征和 route policy。
3. 实现信号表、评分版本、成本模型和 risk rejection reason。
4. 所有输出为 paper order，不调用真实 Trade API。

### Sprint 3：Trade API 对接和 reconciler

1. 实现 Bearer Token 文件读取、`/ready`、warmup、trade submit、status polling。
2. 增加 `request_id`、`source_signature` 和本地订单关联。
3. 实现 signature 确认、余额差异和未知订单恢复。
4. 给 Trade Service 添加来源列和迁移，或明确以 Alpha DB 为审计主库。

### Sprint 4：影子和小额

1. 运行 72 小时 shadow，生成报告。
2. 只开放 PumpFun/PumpSwap、单钱包、单笔上限和 3 个持仓。
3. 运行至少 7 天，逐个处理失败原因。
4. 通过验收门槛后再考虑 StonkFun Via-SOL 和 Meteora DBC。

## 22. 最终审核意见

原方案的服务边界和分阶段思路可以保留，但必须按本节修正后再实施：

- 先把“单边 Alpha”和“跨池套利”分开验收。
- 不能只订阅当前会过滤未知池的 normalized 频道。
- 不能把 HTTP `submitted` 直接当作成交和持仓。
- 不能依赖不存在的统一 price/market-cap 字段。
- 不能在 Bridge PUB 丢消息后直接继续实盘。
- 不能只在 Alpha Engine 做限额；Trade Service 也要执行来源和金额上限。

按本方案完成数据可靠性、回放、影子、确认和回滚后，才具备小额实盘条件。

## 23. 当前 copy_trading 的差距清单

这部分是重新审核后新增的实施前阻断项。它们不是文档格式问题，而是当前代码能力和目标策略之间的差距。

### 23.1 当前事件库存在选择偏差

`copy_trading` 当前的 stream consumer 主要为了跟单服务：只有命中跟踪钱包或 pending 限价单的事件才写入 `pumpapi_events`。普通 mint 的完整 buy/sell 流不会进入可回放数据库，因此直接用现有表计算净流入、买卖比、独立交易者和动量，会得到严重的选择偏差。

实施要求：

- 增加全量 `observation_events` append-only 表或使用 Bridge spool 作为原始事实源。
- 跟单筛选和 Alpha 全量统计分为两条处理路径。
- 原始事件先落盘，再异步分发给策略；策略队列满时写入 dead-letter，不能静默丢失。
- `create`、`migration`、`createPool`、流动性变化等非买卖事件也要进入观察表，用于路由和风险状态；它们仍不能直接触发买卖。

### 23.2 当前去重不足以支持重放

现有内存 recent-event 去重窗口有限，数据库的 `signature + tx_type` 唯一性也不能覆盖 signature 缺失、同签名多池、多动作或字段修订的情况。

实施要求：

```text
event_id = signature:action:mint:pool_id
```

没有 signature 的事件只能记录为 `observation-only`，不能进入 live order。事件表使用 `event_id` 主键，保存 raw hash、schema version、source offset 和 received time。回放时按 event_id 幂等更新窗口，不重复累加金额。

### 23.3 当前 RiskEngine 不是完整资金风控

现有 `RiskEngine` 主要提供全局暂停检查，`StrategyConfig` 中已有的单 mint、每日金额和冷却字段并未覆盖完整的真实持仓、PnL、回撤和待确认资金。

Alpha 上线前需要新增风险账本：

- 订单预留、成交占用和可用余额。
- 未确认订单和 `unknown` 订单冻结金额。
- 每 mint、每池、每协议、每钱包和全策略限额。
- 已实现/未实现 PnL、每日亏损和峰值回撤。
- 原子 `reserve -> submit -> settle/release` 状态迁移。

单笔限额必须在 Alpha Engine 和 Trade Service 两层检查。Trade Service 目前没有策略级限额字段，第一阶段可通过专用 wallet、专用 API token/IP 和服务端环境限额隔离；后续再增加 `source` namespace 和策略限额。

### 23.4 当前没有真实持仓和退出闭环

现有 `dry_run_positions` 只能记录模拟名义，不能代表链上 token 数量、成本、手续费或真实退出结果。要支持止损、止盈和流量衰减退出，需要：

1. 从 entry 成交事实创建 position。
2. 从链上 token balance 定期校准 position。
3. 把 partial fill、手续费、Token-2022 transfer fee 纳入成本。
4. 由退出 worker 生成独立退出信号和幂等键。
5. 退出失败或未知时提升优先级并停止该 mint 新开仓。

### 23.5 当前 order reconciler 范围不足

现有 order reconciler 主要检查 Trade Service 的非终态请求；Trade Service 默认 `WAIT_TX_CONFIRMED=false`，`submitted` 只代表签名已提交。Alpha 必须增加独立 RPC confirmation watcher，处理 `processed`、`confirmed`、`finalized`、链上 err、余额变化和服务重启后的 `unknown`。

`202 Accepted`、HTTP 超时、Trade Service `unknown` 或已有 signature 时均不能使用新幂等键重发。只有确认没有 signature、没有链上交易且原订单明确失败时，才允许生成新的 signal attempt，并保留原订单关联。

### 23.6 当前 readiness gate 不够严格

Trade Service `/ready` 的 `ready` 计算主要依赖 blockhash 和 RPC；响应中的 `pumpapi_executor`、`trade_database_healthy`、MEV 状态和钱包信息不会自动改变 `ready`。Alpha Engine 必须自己执行复合 gate：

```text
ready.ready == true
and ready.trade_database_healthy == true
and required_executor_available == true
and wallet_available == true
and bridge_connected == true
and alpha_db_healthy == true
and no_global_pause == true
```

`copy_trading` 客户端也需要解析并检查这些字段，不能只看 `ready.ready`。

### 23.7 当前路由和金额语义需要显式化

Trade Service 在缺少 `quote_mint` 时会默认 WSOL，且自定义 quote 的 `amount_sol` 语义与普通 WSOL 不同。Alpha Engine 必须在策略层拒绝未知 quote，而不是依赖服务端默认值。

具体规则：

- Pump/PumpSwap 的 SOL 买入优先使用 `amount_lamports`。
- 自定义 quote 直接交易使用 `amount_quote_units`，并要求已知 decimals、池地址和 quote 余额。
- StonkFun Via-SOL 使用 `settlement_asset=sol`、`amount_lamports`、`sol_hop_pool=raydium-cpmm` 和已验证 `sol_hop_pool_id`。
- `/v1/quotes/sol-buy` → preflight → submit 只用于它明确支持的 SOL exact-in 路径；quote 有效期短，提交前必须重新审计，不能把 quote 当作持仓成交。
- PumpAPI executor 当前卖出只支持 `sell_percent=100`；分批卖出必须走 SDK 路径。

### 23.8 当前 queue 满时存在静默丢弃风险

现有 fast-follow 和 ZMQ 消费路径在队列满时可能丢弃事件或请求。Alpha 路径需要背压策略：

- 全量事件先落盘后入内存队列。
- 队列满时保留 durable spool，并暴露 drop/dead-letter 指标。
- 超过最大延迟的事件只用于更新观测和回放，不参与开仓。
- 退出任务使用独立高优先级队列，不能被新信号挤掉。

### 23.9 当前 `source` 只能作为输入审计

`source` 和 `source_signature` 已存在于 `TradeRequest`，但当前主要被序列化到 `request_json`；`TradeRecord`、交易 API 响应和 SQL 独立列没有完整返回。Alpha Engine 必须把完整请求、规范化金额、特征版本、评分和风险决定保存在自己的 `orders`/`signals` 表。

若要求由 Trade Service 直接查询归因，再增加迁移：

```sql
ALTER TABLE trades ADD COLUMN source TEXT;
ALTER TABLE trades ADD COLUMN source_signature TEXT;
CREATE INDEX idx_trades_source_signature ON trades(source_signature);
```

迁移后还要把这些字段纳入 request hash，避免同一幂等键静默改变来源。

### 23.10 选择单进程还是独立 Alpha Engine

当前建议如下：

| 阶段 | 推荐实现 | 原因 |
|---|---|---|
| 数据验收、回放、影子 | 复用 `copy_trading` Rust 事件和 DB 层，新增 alpha mode | 共享去重、订单和回放基础设施 |
| 小额实盘 | 仍优先使用 Rust 单进程，但禁用 Telegram/跟单写路径的交叉状态 | 减少重复订单 owner |
| 策略快速迭代或多策略 | 拆独立 Alpha Engine | 需要明确 DB owner、source namespace、事件版本和部署依赖 |

如果拆分，`copy_trading` 不得继续同时写同一 signal/order/position 表；只能由一个 owner 负责下单和状态迁移。

## 24. 可直接执行的 API 合同

### 24.1 开仓请求

以下请求只适用于已经通过 route policy、ready gate 和风险检查的可执行池：

```http
POST /v1/trades
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "wallet_id": "main",
  "side": "buy",
  "pool": "pump-amm",
  "mint": "<mint>",
  "pool_id": "<pool>",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_lamports": 1000000,
  "slippage_bps": 1500,
  "tip_sol": 0.0001,
  "execution_method": "sdk",
  "use_mev": false,
  "simulate": true,
  "source": "pumpapi-alpha-v1",
  "source_signature": "<event_signature>",
  "idempotency_key": "alpha:<short_event_id>:buy:1"
}
```

实盘阶段将 `simulate` 设为 false；同时要求 Trade Service `/ready`、数据库健康和 Alpha 的复合 gate 全部通过。幂等键自行限制在 128 字符以内，只使用字母、数字、`:_-.`，避免把完整 JSON 或超长 hash 放入 API。

### 24.2 响应处理

- `200` 需要解析 body 中的 `status`，不能只看 HTTP 码。
- `202` 或 `accepted/submitting` 保存 `request_id`，使用同一个 request ID 查询，不生成新幂等键。
- `simulated` 只表示 RPC simulation 成功，不创建真实持仓。
- `submitted` 只表示已有签名或提交结果；必须由 reconciler 查询 signature status 和余额。
- `failed` 且没有 signature 时可记录为失败；是否重试必须产生新的 signal attempt 并受冷却和限额约束。
- `unknown` 或已有 signature 时禁止自动再次广播。

建议轮询间隔为 100ms、250ms、500ms，之后 1 秒一次，最多等待 3~5 秒；超出后转入 reconciler，不阻塞策略主循环。

不要使用 `/v1/batch/trades` 作为多钱包的原子套利接口。当前 batch 是按 wallet 顺序逐项提交，整体返回 `202`，没有 all-or-nothing 或并发保证；多钱包调度必须由 Alpha 自己逐钱包持久化 request_id，并分别处理失败和未知状态。Trade Service 没有取消已接受订单的接口，2 秒信号 TTL 只能阻止过期信号发出，不能撤销已经 accepted 的链上任务。

### 24.3 退出请求

退出使用独立幂等键：

```text
alpha:<position_id>:exit:stop-loss:1
alpha:<position_id>:exit:take-profit:1
alpha:<position_id>:exit:flow-decay:1
```

PumpAPI executor 的退出请求固定 `sell_percent=100`。需要部分卖出时检查池和 SDK 路由能力，不能仅修改 `sell_percent` 后继续使用 PumpAPI。

## 25. 安全和配置要求

`trade_service/.env` 当前本地存在 RPC、Bearer Token 和 MEV provider 凭据。扩展方案和日志不得复制这些值；生产部署应使用未提交的 `EnvironmentFile`、systemd credentials 或 secret manager，并检查凭据是否曾进入 Git 历史。若已经泄露，应先轮换，再进行 Alpha 实盘。

Alpha Engine 只读取 `TRADE_API_TOKEN_FILE`，不把 token 放在命令行、ZMQ 消息、SQLite payload 或日志中。公网 Trade API 仅允许交易调用路径，钱包管理、导出、内部转换和 `/internal/*` 必须由反向代理和 IP 白名单隔离。

`trade_service-sdk9/` 是快照/工作副本，不作为运行实例；开发、测试和部署必须明确使用 `sol-bot/trade_service/`，避免版本和配置漂移。

## 26. 修订后的第一批交付物

1. `pumpapi_zmq_bridge`：增加 observation topic、事件 ID、时间字段、spool、死信和计数指标。
2. `copy_trading`：增加全量事件表和稳定去重；保留现有 tracked-wallet fast-follow 兼容路径。
3. Rust Alpha mode：复用现有 StrategyEngine、Trade API client、order reconciler，先实现 paper/shadow。
4. Alpha DB migration：events、feature_snapshots、signals、orders、risk_ledger、positions。
5. Chain reconciler：signature 状态、余额、费用、unknown 恢复和退出队列。
6. Trade API：补充 source/source_signature 查询能力或明确 Alpha DB 为审计主库；增加策略来源和金额上限校验。
7. 回放测试：覆盖重复事件、乱序、迁移、缺 quote、未知 pool、队列满和服务重启。
8. 运行手册：systemd 顺序、shadow/live 开关、报警、回滚、凭据轮换。

完成这些交付物后，再进入 72 小时 shadow 和至少 7 天单钱包小额灰度。

## 27. 执行进度

截至当前工作区，第一批数据可靠性改造已经完成：

- Bridge 已发布 `observation` topic，并保留 raw/normalized 兼容。
- Bridge 已为 observation 和 normalized 消息增加 `schema_version`、稳定 `event_id`、`received_at_ms` 和 `raw_hash`。
- Bridge 已增加 JSONL observation spool 和大小轮转配置：`PUMPAPI_OBSERVATION_SPOOL_FILE`、`PUMPAPI_OBSERVATION_SPOOL_MAX_BYTES`。
- `copy_trading` 的跟单消费者固定订阅 `normalized`；Alpha 观察消费者独立订阅 `observation`。
- `copy_trading` 已增加 `alpha_observation_events` 表、稳定去重、候选/仅观察路由状态和按时间读取接口。
- 观察事件使用有界队列和阻塞式入队，避免在队列满时静默丢弃。
- Bridge Python 测试 10/10 通过。
- `copy_trading` Rust 测试 40/40 通过，`cargo check`、`cargo fmt --check` 和 diff 检查通过。

当前仍处于数据采集阶段，尚未打开 Alpha 实盘下单。下一批按以下顺序执行：

1. 为 `alpha_observation_events` 增加回放命令和窗口特征统计，先输出净流入、买卖比、独立交易者和延迟报告。
2. 将 Alpha 评分写入独立 signal 表，先生成 paper orders，不调用真实 Trade API。
3. 接入 Trade Service `/ready`、`/v1/cache/warmup`、`/v1/trades` 和 `request_id` reconciler，强制 durable persistence 和复合 readiness gate。
4. 增加真实 token balance、成交费用、PnL 和退出队列，再进入 72 小时 shadow。

本批已继续完成回放和第一版 paper signal：

- 新增 [alpha_replay.py](/root/http/sol-bot/pumpapi_zmq_bridge/alpha_replay.py)，从 observation JSONL 读取事件。
- 按 `(mint, pool_id)` 维护 30 秒、2 分钟、10 分钟窗口。
- 输出买卖笔数、买卖金额、净流入、买卖比、独立交易者数和最大交易者集中度。
- 只对 `route_status=executable`、`buy/sell` 和有 mint 的事件计算信号。
- 用 `event_id` 去重，输出 `mode=paper` 的 JSONL signal，不调用 Trade API。
- 回放器测试 2/2 通过，Bridge/replay 全量测试 10/10 通过。

本批收尾又完成：

- `copy_trading` 已将 paper signal 写入 `alpha_paper_signals`，按 `signal_id` 和 `(event_id, side)` 幂等去重。
- 新增只读报告脚本 [alpha_paper_report.py](/root/http/sol-bot/copy_trading/scripts/alpha_paper_report.py)，可按时间窗口输出 observation 数量、signal 状态/评分、Pool 分布和最近信号。
- `.env.example` 和 README 已明确 `ALPHA_PAPER_ENABLED=false` 默认关闭；开启后仍不会调用 Trade Service。
- `alpha_paper_signals` 已预留 `order_id`、`trade_service_request_id`、`remote_status`、`updated_at_ms` 生命周期字段，并提供幂等更新方法，为 shadow/simulate 对账保留 durable 关联。
- 已新增默认关闭的 `alpha_shadow` 路径：使用配置的钱包 ID，经过 `/ready` 的 ready/RPC/blockhash/wallet 复合门禁和 `/v1/cache/warmup` 后，仅以 `simulate:true` 调用 `/v1/trades`；shadow order、request_id、状态和错误写入 `alpha_shadow_orders`。
- shadow 使用固定 `alpha-shadow:<signal_id>` 幂等键；`unknown` 只进入原 request_id 回查，不创建新交易键。
- shadow 增加 `ALPHA_SHADOW_MAX_INFLIGHT` 有界并发，默认 2，避免 observation 突发时压垮 Trade Service。
- shadow 增加 `ALPHA_SHADOW_MIN_SCORE=0.90` 和 `ALPHA_SHADOW_MAX_PER_MINUTE=10`，先限制候选质量和请求速率，再进入 72 小时 simulate 观测。
- shadow 在 readiness 或 warmup 失败时先持久化 `blocked` 结果，再停止该 signal，不把基础设施故障伪装成可重试成交。
- `alpha_paper_report.py --check-shadow` 已提供只读验收门禁，可要求 `request_id` 覆盖率并限制 `unknown`/`blocked` 数量；门禁失败以非零退出码结束，适合 systemd timer 或值班脚本调用。
- 新增 `alpha_shadow_readiness.py`，只读调用 Trade Service `/ready`，检查 ready、RPC、blockhash 和配置钱包；不会调用 `/v1/trades`。
- 新增 systemd service/timer 示例，周期运行只读 shadow 审计门禁；示例文件位于 `copy_trading/docs/`，不会自动安装或改变当前服务。
- Rust 测试 43/43、Bridge/replay Python 测试 10/10、`cargo check`、格式和 diff 检查通过。

下一批工作是接入实际运行环境的只读 shadow 观测，连续验证 72 小时 readiness、warmup、simulate 状态和 request_id 对账完整性；只有门禁连续通过后，才评估是否进入更严格的人工批准灰度。仍不打开真实交易。

## 28. Shadow 失败诊断复审（2026-10-30）

本次复审对现有 `alpha_shadow_orders` 做了只读扫描，没有复制、压缩或重建跟单数据库。结果必须与路由成功率分开解释：

- 共 217 条 shadow 记录，其中 206 条是本地每分钟限流 `blocked`，没有送到 Trade Service，不能算作路由失败。
- 实际获得 Trade Service `request_id` 的请求为 10 条，均为 `unknown`，错误为 `UiTransactionError(InstructionError(..., Custom(1)))`。
- 失败分布为 `pump` 8 条（指令索引 2）和 `pump-amm` 2 条（指令索引 3）；没有 Meteora 真实模拟样本。
- 另有 1 条停留在 `created`，没有 request_id，属于进程中断或未完成对账，不能算模拟成功。
- 失败样本集中在少数 mint/pool，且都使用 `main` 钱包、0.01 SOL、WSOL quote 和 SDK 模拟。

新增只读诊断命令：

```bash
python3 copy_trading/scripts/alpha_shadow_diagnose.py \
  copy_trading/data/copy_trading.sqlite --sample-limit 20
```

在恢复服务后，下一次最小验证必须先确认：

1. `main` 钱包有足够 SOL 支付订单金额、ATA 租金和交易费；不能只看 `/ready` 的 `ready=true`。
2. 每次模拟响应带 request_id，并在 Trade Service durable 记录中落到终态 `simulated` 或明确 `failed`；`unknown` 必须停止该池继续试验。
3. Pump 先选一枚当前仍在 bonding curve 且 token program 已知的 mint；PumpSwap 先选一枚当前池状态可读、base/quote 与事件一致的 mint。
4. 单池、单钱包、每分钟最多 1 条、`ALPHA_SHADOW_MIN_SCORE=0.98`，连续通过至少 10 条后再考虑放大。
5. 在 RPC 可用且能收集完整 `Program log` 前，不把 `Custom(1)` 归因到具体协议错误，也不切换到真实广播。

当前环境无法在本地沙箱绑定 `0.0.0.0:18787`，且外部 RPC DNS 不可用，因此尚未完成新的在线单请求模拟。该环境限制不改变安全结论：shadow 保持关闭，实盘保持关闭。

## 29. 当前执行快照（2026-10-30）

本次只读复核使用 `copy_trading/data/copy_trading.sqlite`，没有修改数据库内容，也没有调用 Trade API 下单接口。

### 29.1 数据采集状态

- 最近 1 小时 observation：`258,159`
- 最近 1 小时 paper signal：`18,707`
- paper signal 状态全部为 `paper`
- 最近 1 小时 signal 平均分：约 `0.7892`
- 当前主要池分布：`pump-amm 11,218`、`meteora-damm-v2 5,427`、`pump 1,708`
- 数据库文件在 10 秒复核窗口内持续增长，采集链路仍在运行

### 29.2 Shadow 门禁状态

当前共 `217` 条 shadow 记录：

| 状态 | 数量 | 解释 |
|---|---:|---|
| `blocked` | 206 | 本地每分钟限流，未送达 Trade Service |
| `unknown` | 10 | 已取得 request_id，但返回 `InstructionError(... Custom(1))`，必须停止该池继续试验 |
| `created` | 1 | 未完成发送或对账，需由 reconciler 处理 |

因此 request_id 覆盖率为 `4.61%`，远低于 shadow 放行要求；没有任何 `simulated` 成功样本。`pump` 的 8 条远端错误集中在同一 mint，`pump-amm` 的 2 条远端错误集中在另一 mint；不能把它们解释为全协议成功率。

### 29.3 本批执行改动

- `ALPHA_SHADOW_REQUIRE_ALLOWLIST` 默认设为 `true`。
- 当 shadow 开启但 `ALPHA_SHADOW_ALLOWED_POOLS` 或 `ALPHA_SHADOW_ALLOWED_MINTS` 任一为空时，所有候选 fail-closed，不会进入 Trade Service。
- 池白名单大小写不敏感，mint 白名单精确匹配。
- 新增缺少白名单时拒绝、池名归一化和 mint 精确匹配测试；定向 `alpha_shadow` 测试 `6/6` 通过。
- `cargo fmt --check` 通过；PumpAPI bridge/replay 测试 `10/10` 通过；此前完整 Rust 测试 `44/44` 通过。

### 29.4 完成度

按交付物衡量，数据可靠性、回放、paper signal、shadow 持久化和基础门禁已实现，代码实现完成度约 **70%**。按“可安全小额实盘”衡量完成度仍为 **0%**：shadow 尚未取得连续成功样本，且当前环境无法完成在线 RPC/Trade Service 单请求验证。

下一步只允许执行以下顺序：

1. 配置一个明确的 `ALPHA_SHADOW_ALLOWED_POOLS` 和一个明确的 `ALPHA_SHADOW_ALLOWED_MINTS`，每分钟最多 1 条、分数阈值 `0.98`。
2. 确认钱包余额、RPC、blockhash、路由池状态和 token program；对同一池/币收集至少 10 条 `simulated` 或明确 `failed` 终态。
3. 任何 `unknown`、signature 未对账或 `Custom(1)` 都暂停该池，并先补充链上 program log 诊断。
4. 只有连续 72 小时 shadow 门禁通过，才进入人工批准的单钱包小额灰度；在此之前保持真实广播关闭。

### 29.5 本轮继续执行

- 最新 1 小时只读窗口增长到 `351,295` 条 observation、`25,543` 条 paper signal，说明 PumpAPI → bridge → Alpha paper 链路仍在持续工作。
- 修复 shadow reconciler：带有 `trade_service_request_id` 的 `unknown` 订单现在会进入轮询队列，避免未知状态永久滞留。
- 新增 `alpha_shadow_reconciler_includes_unknown_orders_with_request_ids` 回归测试。
- 本轮定向测试全部通过：shadow `6/6`，unknown 对账 `1/1`；未调用 Trade API，也未改变真实交易开关。

