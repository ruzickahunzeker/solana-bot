# Public Trade API 对接文档

文档版本：2026-11-13（源码说明；部署值需另行核对）

适用服务：`trade_service`（`pump-trade-service`）  
默认端口：`18787`

本文档描述当前代码已经实现的 HTTP API。它用于受信任的策略服务、Telegram Bot 或其他后端系统提交 Solana 交易，不是面向匿名用户的多租户开放平台。

## 1. 接入边界

当前接口具有以下边界：

- 仅支持 JSON over HTTP。
- 除 `GET /health` 外，所有接口都要求来源 IP 白名单和 Bearer Token。
- API 请求只传 `wallet_id`，不能在交易请求中传私钥。
- 默认执行器是本地 `sol-trade-sdk`；可选 PumpAPI Local Transaction 执行器。
- 下单是异步过程。HTTP 接收成功不等于链上交易成功。
- 当前服务通常把交易记录更新到 `submitted`，不会持续把它自动推进到链上 `finalized`。调用方必须根据 `signature` 查询 Solana 链上最终状态。
- 限价单、跟单策略、市值筛选和仓位管理不属于本 API，它们由上层系统实现，最终调用本 API 执行交易。

### 1.1 历史部署配置

以下为早期接入时记录的部署值，不代表本轮已核对或修改线上环境。新增安全策略、账本与对账源码尚未在本轮部署；接入前需核对实际制品、配置和 `GET /ready`：

| 配置 | 当前值 | 影响 |
|---|---:|---|
| Trade API 服务端 | `43.157.26.254:18787` | `trade_service` 所在的当前服务器 |
| 本次接入调用端 | `45.76.94.27` | 从该服务器向 Trade API 发送交易请求 |
| `TRADE_HTTP_ADDR` | `0.0.0.0:18787` | 服务监听公网网卡 |
| `TRADE_ALLOWED_CLIENT_IPS` | `43.166.214.34,45.76.94.27` | 允许的**调用方出口 IP**列表 |
| `WALLET_STORAGE_MODE` | `database` | 钱包密文存储在数据库中 |
| `TRADE_PERSISTENCE_MODE` | `durable` | 交易和幂等键持久化 |
| `ENABLE_SIMULATE` | `true` | 请求未传 `simulate` 时默认模拟 |
| `WAIT_TX_CONFIRMED` | `false` | 默认拿到提交结果后返回，不等待链上确认 |
| `ENABLE_PUMPAPI_EXECUTOR` | `true` | 允许 `execution_method=pumpapi` |
| `ENABLE_MEV_PROTECTION` | `true` | SDK 全局启用 MEV 保护配置 |
| 可用 Provider | Jito、Temporal、FlashBlock、BlockRazor、Astralane、Default | 以 `/ready` 返回为准 |
| SpeedLanding | 未启用 | 指定它会执行失败 |

未开启禁止广播策略时，`simulate:false` 会覆盖默认 `ENABLE_SIMULATE=true` 并可能发送真实交易，因此必须作为高风险参数处理。`TRADE_BROADCAST_DISABLED=true` 不能被请求覆盖；它还会拒绝 SDK 完整买卖流水线，包括 SDK simulate。该选项默认 false，本轮没有替线上启用。

## 2. 网络与认证

### 2.1 Base URL

在调用端服务器 `45.76.94.27` 上使用以下 Trade API 地址：

```text
http://43.157.26.254:18787
```

在 `45.76.94.27` 上配置环境变量：

```bash
export TRADE_API_BASE_URL="http://43.157.26.254:18787"
export TRADE_API_TOKEN="<LOCAL_TRADE_API_TOKEN>"
```

本次连接方向如下：

```text
交易调用端 45.76.94.27
        |
        | HTTP :18787 + Bearer Token
        v
Trade API 43.157.26.254:18787
```

`45.76.94.27` 是调用方出口 IP，不是 API 地址。它已加入 Trade API 的 `TRADE_ALLOWED_CLIENT_IPS`。如果该服务器经过 NAT，应确认其实际公网出口仍是 `45.76.94.27`，否则会收到 `403 client ip not allowed`。

请在 `45.76.94.27` 上执行快速检查：

```bash
curl -sS --connect-timeout 5 "$TRADE_API_BASE_URL/health"

curl -sS --connect-timeout 5 "$TRADE_API_BASE_URL/ready" \
  -H "Authorization: Bearer $TRADE_API_TOKEN"
```

当前地址使用明文 HTTP。跨公网调用时应限制来源 IP，并优先通过 HTTPS 反向代理或加密隧道访问。

### 2.2 Bearer Token

受保护接口必须携带：

```http
Authorization: Bearer <LOCAL_TRADE_API_TOKEN>
Content-Type: application/json
```

Token 错误或缺失返回：

```http
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{"error":"unauthorized"}
```

### 2.3 IP 白名单

调用方的实际 TCP 来源 IP 必须出现在服务端 `TRADE_ALLOWED_CLIENT_IPS` 中，否则返回：

```http
HTTP/1.1 403 Forbidden
Content-Type: application/json

{"error":"client ip not allowed"}
```

如果 API 前面部署了反向代理，当前应用读取的是与应用建立 TCP 连接的 peer IP，不读取 `X-Forwarded-For`。因此应让反向代理完成公网来源校验，并把代理本机 IP 加入应用白名单。

### 2.4 公网安全要求

当前应用本身不提供 TLS。默认 legacy 仍共用交易与管理凭据；新增的 separate 模式必须显式迁移后才按路由隔离。公网部署仍必须满足：

1. 使用防火墙只允许指定来源 IP 访问 `18787`。
2. 跨公网传输时在前面加 HTTPS、WireGuard 或其他加密隧道，避免 Bearer Token 明文传输。
3. 反向代理仅放行本文“公开交易接口”列出的路径。
4. 拒绝外部访问全部 `/internal/*` 路由，以及 `/v1/wallets/manage`、`POST /v1/wallets`、`PATCH/DELETE /v1/wallets/*` 和 `/v1/wallets/*/export`。SOL 转账只存在于本机内部钱包接口，不属于 Public Trade API。
5. 不在日志、URL、错误上报或客户端代码仓库中记录 Token。

尤其不要向普通 Public API 调用方开放私钥导出接口。当前同一个 Token 可以访问全部已加载钱包，不具备租户隔离能力。

### 2.5 可选管理鉴权与禁止广播

`WALLET_ADMIN_AUTH_MODE` 默认 legacy，保持旧客户端兼容；separate 要求独立的 `WALLET_ADMIN_API_TOKEN` 且 TCP peer 为 loopback。管理 token 必须非空、与 `LOCAL_TRADE_API_TOKEN` 不同。钱包管理列表、创建/导入、改名、删除、导出及靓号任务管理改用管理 Bearer；普通钱包公钥列表、交易、报价和状态仍使用交易 Bearer。管理凭据不能直接调用交易 API，交易凭据也不能管理钱包。非 loopback 返回 403，缺/错管理凭据返回 401。模式无效或 separate 凭据不合格会拒绝启动，不回退到 legacy。

`TRADE_BROADCAST_DISABLED` 默认 false；true 是启动时固定的服务端执行策略，不是请求参数。现有已审查的 SDK、聚合、PumpAPI、SOL 转账与 WSOL 提交均检查该策略。普通交易、转账和 WSOL 提交的合法新资金请求被策略拒绝时返回 403；批量交易保留逐项错误合同。已存在订单可继续查询，原签名只读对账不受影响。现有 READY 的 `paused` 在禁止广播时为 true，`ready` 不代表发送许可。

禁止广播模式不是禁止签名/导出或网络沙箱：SDK 完整买卖（含 SDK simulate）整体拒绝，但项目明确分离的 unsigned 聚合模拟、PumpAPI Local 模拟、WSOL simulate 和只读 RPC 仍可用。操作者修改配置需受控重启，不会撤销已发交易。迁移前必须更新调用方路由凭据及可信本机/SSH 入口；不会在本轮自动修改 Bot。详细参数、发送入口清单、测试与剩余门槛见 [安全策略验收](optimization/EXECUTION_SAFETY_AND_ADMIN.md)。

## 3. 推荐对接流程

```text
健康检查
  -> Ready 检查
  -> 查询可用 wallet_id
  -> 根据 mint 解析 Pool，或使用上层数据流提供的 Pool
  -> 可选预热交易参数
  -> 生成唯一 idempotency_key
  -> POST /v1/trades
  -> 若状态仍在处理中，按 request_id 轮询
  -> 得到 signature 后查询 Solana 链上确认状态
  -> 更新上层订单和仓位
```

建议在每次实盘交易前至少确认：

- `/ready` 的 `ready=true`。
- `trade_database_healthy=true`。
- 目标 `wallet_id` 存在。
- Pool、`pool_id` 和 `quote_mint` 来自同一条可信数据源。
- `simulate` 的值符合当前环境。
- `idempotency_key` 是这次业务动作的稳定唯一标识。

## 4. 接口总览

| Method | Path | 鉴权 | 用途 |
|---|---|---|---|
| `GET` | `/health` | 无 | 进程存活检查 |
| `GET` | `/ready` | IP + Bearer | 交易就绪检查 |
| `GET` | `/v1/capabilities` | IP + Bearer | 查询协议能力；仅 `status=live` 且对应能力为 `true` 的协议允许进入交易流程 |
| `GET` | `/v1/wallets` | IP + Bearer | 查询可用于交易的钱包 |
| `POST` | `/v1/pools/resolve` | IP + Bearer | 按 Mint 自动解析 Pump、PumpSwap 或 Meteora DBC Pool |
| `POST` | `/v1/cache/warmup` | IP + Bearer | 预热指定 Pool 的交易参数 |
| `POST` | `/v1/trades` | IP + Bearer | 创建单钱包交易 |
| `POST` | `/v1/batch/trades` | IP + Bearer | 创建多钱包交易 |
| `GET` | `/v1/trades/:request_id` | IP + Bearer | 查询单笔交易 |
| `GET` | `/v1/trades?limit=N` | IP + Bearer | 查询最近交易 |

## 5. 健康与就绪

### 5.1 健康检查

```http
GET /health
```

请求：

```bash
curl -sS "$TRADE_API_BASE_URL/health"
```

响应：

```json
{
  "ok": true
}
```

这个接口只表示 HTTP 进程存活，不表示可以交易。

### 5.2 协议能力

```http
GET /v1/capabilities
Authorization: Bearer <token>
```

请求：

```bash
curl -sS "$TRADE_API_BASE_URL/v1/capabilities" \
  -H "Authorization: Bearer $TRADE_API_TOKEN"
```

响应示例（字段可能随 SDK 和配置变化）：

```json
{
  "schema_version": 1,
  "generated_at_ms": 1760000000000,
  "protocols": [
    {
      "protocol_id": "raydium-cpmm",
      "aliases": ["raydium-cp"],
      "status": "live",
      "execution_backend": "sdk",
      "pool_id_required": true,
      "supports_buy": true,
      "supports_sell": true,
      "supports_direct_quote": true,
      "supports_sol_route": false,
      "sdk_builder_available": true,
      "token_programs": ["spl-token"]
    },
    {
      "protocol_id": "raydium-clmm",
      "aliases": [],
      "status": "observe",
      "execution_backend": "sdk-adapter-pending",
      "pool_id_required": true,
      "supports_buy": false,
      "supports_sell": false,
      "supports_direct_quote": false,
      "supports_sol_route": false,
      "sdk_builder_available": true,
      "token_programs": ["spl-token"]
    }
  ]
}
```

客户端处理规则：

- `status=live` 只表示该协议已进入交易能力注册表；仍必须先通过 Pool 解析、账户归属、Quote、余额和模拟校验。
- `status=observe` 只能展示、记录或进入研究/模拟流程，服务端必须拦截签名和广播。
- `supports_buy`、`supports_sell`、`supports_direct_quote` 和 `supports_sol_route` 是能力开关，不是对某个具体 Mint 的可交易保证。
- `pool_id_required=true` 时，请求必须使用经过验证的池地址，不能只根据 PumpAPI 的协议字符串选择指令。
- `supports_sol_route=false` 时，不得把 SOL 数量直接当作该协议的 Quote 数量发送；应使用协议原生 Quote，或等待已验证的两跳路由。
- 接口与交易接口使用相同的 IP 白名单和 Bearer Token；只有 `/health` 不需要鉴权。

### 5.3 Ready 检查

```http
GET /ready
Authorization: Bearer <token>
```

请求：

```bash
curl -sS "$TRADE_API_BASE_URL/ready" \
  -H "Authorization: Bearer $TRADE_API_TOKEN"
```

响应示例：

```json
{
  "ready": true,
  "wallet": "<DEFAULT_WALLET_PUBKEY>",
  "default_wallet_id": "main",
  "wallet_count": 6,
  "wallets": [
    {
      "wallet_id": "main",
      "pubkey": "<WALLET_PUBKEY>"
    }
  ],
  "has_blockhash": true,
  "blockhash_age_ms": 183,
  "rpc_ok": true,
  "paused": false,
  "rpc_lanes": 1,
  "swqos_lanes": ["Jito", "Temporal", "Default"],
  "active_swqos_lanes": ["Jito", "Temporal", "Default"],
  "mev_protection": true,
  "require_mev_submit": false,
  "pumpapi_executor": true,
  "trade_persistence_mode": "durable",
  "trade_database_healthy": true
}
```

关键字段：

| 字段 | 含义 |
|---|---|
| `ready` | 当前实现为 `has_blockhash && rpc_ok` |
| `has_blockhash` | SDK 执行需要的新鲜 blockhash 是否存在 |
| `blockhash_age_ms` | 当前缓存 blockhash 的年龄 |
| `rpc_ok` | 默认钱包的 RPC 探活结果 |
| `active_swqos_lanes` | 当前实际可用的 MEV/SWQoS 和 Default 通道 |
| `pumpapi_executor` | 是否允许 `execution_method=pumpapi` |
| `trade_persistence_mode` | `memory`、`shadow` 或 `durable` |
| `trade_database_healthy` | 交易数据库是否健康 |

调用方在 `ready=false` 或 `trade_database_healthy=false` 时不应创建真实交易。

## 6. 查询交易钱包

```http
GET /v1/wallets
Authorization: Bearer <token>
```

```bash
curl -sS "$TRADE_API_BASE_URL/v1/wallets" \
  -H "Authorization: Bearer $TRADE_API_TOKEN"
```

响应：

```json
{
  "default_wallet_id": "main",
  "wallets": [
    {
      "wallet_id": "main",
      "pubkey": "<WALLET_PUBKEY>"
    },
    {
      "wallet_id": "wallet_abc123",
      "pubkey": "<WALLET_PUBKEY>"
    }
  ]
}
```

交易请求不传 `wallet_id` 时使用 `default_wallet_id`。外部系统建议始终显式传入 `wallet_id`，避免默认钱包配置变化后使用错误的钱包。

## 7. Pool 解析与预热

### 7.1 自动解析 Pool

```http
POST /v1/pools/resolve
```

请求：

```bash
curl -sS -X POST "$TRADE_API_BASE_URL/v1/pools/resolve" \
  -H "Authorization: Bearer $TRADE_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "mint": "<TOKEN_MINT>"
  }'
```

Pump bonding curve 未完成时：

```json
{
  "mint": "<TOKEN_MINT>",
  "pool": "pump",
  "pool_id": null,
  "quote_mint": "So11111111111111111111111111111111111111112"
}
```

已迁移到 PumpSwap 时：

```json
{
  "mint": "<TOKEN_MINT>",
  "pool": "pump-amm",
  "pool_id": "<PUMPSWAP_POOL_ADDRESS>",
  "quote_mint": "So11111111111111111111111111111111111111112"
}
```

Meteora DBC 时：

```json
{
  "mint": "<TOKEN_MINT>",
  "pool": "meteora-dbc",
  "pool_id": "<METEORA_DBC_POOL_ADDRESS>",
  "quote_mint": "So11111111111111111111111111111111111111112"
}
```

当前自动解析器探测 Pump bonding curve、PumpSwap 和 Meteora DBC。Bonk、Meteora DAMM v2、Raydium AMM v4/CPMM 的 Pool 信息仍须由 PumpAPI 数据流、Pool 数据库或其他可信索引器提供。

无法解析时通常返回 `502 Bad Gateway`：

```json
{
  "error": "no supported pool found for mint; ..."
}
```

### 7.2 预热交易参数

```http
POST /v1/cache/warmup
```

请求示例：

```bash
curl -sS -X POST "$TRADE_API_BASE_URL/v1/cache/warmup" \
  -H "Authorization: Bearer $TRADE_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "pool": "pump-amm",
    "mint": "<TOKEN_MINT>",
    "pool_id": "<PUMPSWAP_POOL_ADDRESS>",
    "quote_mint": "So11111111111111111111111111111111111111112"
  }'
```

响应：

```json
{
  "ok": true,
  "cache_key": "pump-amm:<PUMPSWAP_POOL_ADDRESS>"
}
```

预热失败不会创建订单。对于人工快速交易，可以在用户输入 Mint 时解析并预热，而不是等到点击确认时才查 Pool。

## 8. 创建交易

### 8.1 买入示例

下面示例使用 SDK、指定钱包、0.001 SOL、15% 滑点，并显式关闭 MEV 走 Default RPC：

```bash
IDEMPOTENCY_KEY="manual-buy-$(date +%s)-001"

curl -sS -X POST "$TRADE_API_BASE_URL/v1/trades" \
  -H "Authorization: Bearer $TRADE_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"wallet_id\": \"main\",
    \"side\": \"buy\",
    \"pool\": \"pump\",
    \"mint\": \"<TOKEN_MINT>\",
    \"quote_mint\": \"So11111111111111111111111111111111111111112\",
    \"amount_sol\": 0.001,
    \"slippage_percent\": 15,
    \"tip_sol\": 0.0001,
    \"execution_method\": \"sdk\",
    \"use_mev\": false,
    \"mev_provider\": \"default\",
    \"simulate\": true,
    \"source\": \"public-api-example\",
    \"idempotency_key\": \"$IDEMPOTENCY_KEY\"
  }"
```

真实交易必须显式设置：

```json
{
  "simulate": false
}
```

### 8.2 卖出示例

卖出当前钱包所持目标 Mint 的 100%：

```bash
curl -sS -X POST "$TRADE_API_BASE_URL/v1/trades" \
  -H "Authorization: Bearer $TRADE_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "wallet_id": "main",
    "side": "sell",
    "pool": "pump-amm",
    "mint": "<TOKEN_MINT>",
    "pool_id": "<PUMPSWAP_POOL_ADDRESS>",
    "quote_mint": "So11111111111111111111111111111111111111112",
    "sell_percent": 100,
    "slippage_percent": 15,
    "tip_sol": 0.0001,
    "execution_method": "sdk",
    "use_mev": true,
    "mev_provider": "temporal",
    "simulate": true,
    "source": "public-api-example",
    "idempotency_key": "manual-sell-20260804-001"
  }'
```

### 8.3 PumpAPI Local Transaction 示例

```bash
curl -sS -X POST "$TRADE_API_BASE_URL/v1/trades" \
  -H "Authorization: Bearer $TRADE_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "wallet_id": "main",
    "side": "buy",
    "pool": "pump",
    "mint": "<TOKEN_MINT>",
    "amount_sol": 0.001,
    "slippage_percent": 15,
    "tip_sol": 0.0001,
    "execution_method": "pumpapi",
    "use_mev": false,
    "mev_provider": "default",
    "simulate": true,
    "idempotency_key": "pumpapi-buy-20260804-001"
  }'
```

PumpAPI 执行器当前限制：

- 服务端必须启用 `ENABLE_PUMPAPI_EXECUTOR=true`。
- 只使用 PumpAPI Local Transaction；私钥始终在本服务本地签名。
- 不支持 `use_mev=true`。
- `mev_provider` 只能省略或使用 `default`/`rpc`。
- 卖出只支持 `sell_percent=100`。
- `tip_sol` 在这条路径中作为 PumpAPI `priorityFee` 使用。
- `pool` 在请求中仍为必填，但 PumpAPI 上游实际根据 Mint、Quote 和可选 `pool_id` 构造交易。

### 8.4 TradeRequest 字段

| 字段 | 类型 | 必填 | 说明 |
|---|---|---:|---|
| `wallet_id` | string | 否 | 不传时使用默认钱包；建议显式传入 |
| `side` | `buy` / `sell` | 是 | 买卖方向 |
| `pool` | enum | 是 | 见 Pool 支持矩阵 |
| `mint` | string | 是 | 目标 Token 的 Solana Mint 公钥 |
| `pool_id` | string/null | 视 Pool | 交易池地址 |
| `quote_mint` | string | 否 | 默认 WSOL；SOL Mint 会被规范化为 WSOL |
| `settlement_asset` | `quote` / `sol` | 否 | 默认 `quote`；StonkFun 自定义 Quote 两跳交易显式使用 `sol` |
| `sol_hop_pool` | enum | Via-SOL 必填 | 首版只允许 `raydium-cpmm` |
| `sol_hop_pool_id` | string | Via-SOL 必填 | WSOL/Quote 第二跳池；服务会读取链上账户校验交易对 |
| `amount_sol` | number | 买入可选 | SOL/WSOL 买入金额，9 位精度 |
| `amount_lamports` | uint64 | 买入可选 | SOL/WSOL 最小单位金额 |
| `amount_quote` | number | 买入可选 | 通用 quote 金额；WSOL 9 位，USDC/USD1 6 位 |
| `amount_usdc` | number | 买入可选 | USDC 买入金额，仅 USDC quote 可用 |
| `amount_quote_units` | uint64 | 买入可选 | quote 最小单位金额，优先级最高 |
| `sell_percent` | uint64 | 卖出必填 | `1` 到 `100` |
| `slippage_percent` | number | 否 | `0` 到 `100`，例如 `15` 表示 15% |
| `slippage_bps` | uint64 | 否 | `0` 到 `10000`；`slippage_percent` 优先 |
| `tip_sol` | number | 否 | 小费，必须大于等于 0 |
| `execution_method` | `sdk` / `pumpapi` | 否 | 默认 `sdk`；也接受 `pump-api` 别名 |
| `use_mev` | boolean | 否 | 单笔交易是否使用 MEV/SWQoS |
| `mev_provider` | enum | 否 | 指定单笔 Provider |
| `simulate` | boolean | 否 | 单笔覆盖服务端 `ENABLE_SIMULATE` |
| `source` | string | 否 | 上层来源标识，仅用于关联/审计输入 |
| `source_signature` | string | 否 | 跟单来源交易签名 |
| `idempotency_key` | string | 是 | 同一钱包范围内的业务幂等键，不能为空 |

买入金额字段最多只能传一个，传多个会拒绝。直接 Quote 结算按 Quote 类型选择金额字段；`settlement_asset:"sol"` 只接受 `amount_sol` 或 `amount_lamports`。

```text
amount_quote_units
  -> USDC: amount_usdc -> amount_quote
  -> USD1: amount_quote
  -> WSOL: amount_quote -> amount_sol -> amount_lamports -> DEFAULT_BUY_SOL
```

### 独立 SOL 买入报价与提交

`POST /v1/quotes/sol-buy` 使用现有 Trade API 鉴权，接受 `{"wallet_id":"2TE","mint":"目标 mint","amount_lamports":1000000,"slippage_bps":1500}`。请求仅支持 0.0001–0.1 SOL、1–3000 bps；返回绑定钱包、15 秒有效期、`expected_output_units`、`minimum_output_units`、`price_impact_percent` 和路线各跳。服务重启会使内存报价失效。

`POST /v1/quotes/sol-buy/:quote_id/preflight?wallet_id=2TE` 会重新取价并执行交易审计与本地模拟；路线变化、最低到手降低、目标 ATA 不可写、费用超限或模拟失败都会拒绝。成功时 `instruction_audit_passed:true`、`executable:true`。它仍不签名、不广播。

`POST /v1/quotes/sol-buy/:quote_id/submit` 接受 `{"wallet_id":"2TE","confirmed":true,"simulate":false,"idempotency_key":"唯一键"}`。服务会再次取价、重新审计和模拟，通过后由 Trade Service 钱包签名并广播；`simulate:true` 只返回 `simulated`，不会广播。提交异常返回 `unknown`，调用方不得自动创建新幂等键重试，应按链上签名或 Trade Service 记录核查。该接口只支持 SOL Exact-In 买入。

独立提交的持久化安全规则（2026-11-12 源码实现，是否已部署需核对制品）：

- 实盘要求 `TRADE_PERSISTENCE_MODE=durable`；`memory`/`shadow` 返回 `503`，不静默退回内存。模拟仍可用，不签名、不广播，也不占用实盘幂等键。
- 服务在独立 `quote_submissions` 表原子保留钱包、报价完整内容及幂等键；签名与原始交易字节同步保存成功后才发送。交易字节仅保存在服务端，不通过 API 返回。
- 同一钱包和幂等键只执行一次；另一个报价或钱包公钥映射返回 `409`。不同钱包的同名 key 互不影响。
- 相同 `quote_id`、`wallet_id`、幂等键及 `confirmed:true,simulate:false` 再次请求时，先读取原记录。即使内存报价已过期或服务重启，也只返回原状态，不重新取价、签名或发送。这不是重新下单。
- 重复请求可能返回 `submitting`、`submitted`、`unknown`、`failed`，或对账后保存的 `confirmed`/`finalized`；广播超时、签名不一致或签名后中断时保留本地原签名。数据库故障可能直接返回 `503`，同样不得换 key 自动重试。
- 进程重启将未完成的 `reserved`/`signed` 记录转为 `unknown`，保留签名及交易字节；不会自动重放。原记录中的 `preflight` 是当时的快照，不能视为当前报价或再次执行授权。
- 独立提交不伪造成普通 TradeRecord，不能通过 `/v1/trades/:request_id` 查询；推荐使用下面的只读状态接口。重复原提交请求仍可读取已有记录，但它本身不触发链上查询。

这不代表所有聚合、转账或 SDK 提交路径已完成相同改造，也不代表完整指令审计和实盘验收通过。实现、测试与发布门槛见 [独立提交验收记录](optimization/QUOTE_SUBMISSION_JOURNAL.md)。

### 独立 SOL 买入订单只读对账

新增 `GET /v1/quotes/sol-buy/:quote_id/submission?wallet_id=2TE&idempotency_key=原幂等键`，复用 IP allowlist 和 Bearer 鉴权，要求 durable。它只读取已有订单并查询原签名，将确认信息写回服务端账本；不创建订单、不寻池、不取交易报价、不重新签名或广播。不需要仍有内存报价。当前是请求触发的对账，没有后台定时任务。

```json
{
  "submission_id": "qs_example",
  "quote_id": "原 quote_id",
  "wallet_id": "2TE",
  "status": "confirmed",
  "signature": "原交易签名",
  "error": null,
  "commitment": "confirmed",
  "observed_slot": 123456789,
  "reconciliation_status": "updated",
  "reconciliation_error": null
}
```

| 字段/返回 | 语义 |
| --- | --- |
| `commitment` | 账本已保存的 `processed` / `confirmed` / `finalized`；未观测到为 null |
| `reconciliation_status=updated` | 已保存更高确认级别 |
| `unchanged` | 已有相同或更强确认信息，不降级覆盖 |
| `not_found` | 至少一个被查询节点当前未返回原签名；不表示全网不存在、交易失败、已过期或可以重发 |
| `unavailable` | RPC 查询超时、异常或响应不一致；保留原订单状态，原因在 `reconciliation_error` |
| `not_required` | 无可查询签名，或账本已保存 finalized 结果；不调用 RPC |
| HTTP 404 | 账本中不存在该钱包/幂等键的订单；与 RPC 当前查不到签名不同 |
| HTTP 409 | quote_id 或钱包公钥映射与原订单不一致 |
| HTTP 503 | 非 durable 或账本读取/写入失败；不能换幂等键自动重提 |

交易执行错误保存在 `error`；查询错误单独保存在 `reconciliation_error`，不会把查询失败伪造成交易失败。processed 的执行错误保持 `unknown`；confirmed 的执行错误显示 `failed`，但尚非最终结论，仍可继续查询至 finalized。较低或相同确认级别不覆盖已保存结果；更高确认级别可修正分叉上的早期观测。即使为 `failed`，也不授权自动重发。

复用钱包已有的最多两个 Default RPC 顺序查询，每节点最多 1 秒，应用层 RPC 查询预算 3 秒，数据库排队/写入另计。没有自动轮询、并行 RPC 或池扫描。建议调用方轮询间隔不少于 1 秒，出现 unavailable 时退避；不宜因查询无结果就持续快速刷新。成功响应带 `Cache-Control: no-store`，不返回签名字节、私钥、原交易意图或幂等键。完整离线验收与迁移边界见 [原签名对账记录](optimization/SUBMISSION_RECONCILIATION.md)。

Jupiter 优先费声明与交易指令计算值允许最多 1 lamport 的舍入差；CU limit 必须完全一致。真正的费用上限始终按交易指令计算值校验。

预检还会从同一默认 RPC 读取付款钱包和目标 ATA 的交易前账户状态，并返回 `baseline_slot`、`simulation_slot`。只有两个 slot 相同时才展示 `observed_same_slot_payer_debit_lamports` 与 `observed_same_slot_target_token_delta_units`；这两个值仅供排查，**不用于通过最低到手或最大扣款校验**，因为同一 slot 内账户状态也可能变化。严格校验仍只依赖模拟 RPC 自身返回的前后余额字段；缺失时相关校验仍是未知。

RPC 提供内部指令时，预检会校验内部指令所属的外层编号、程序 ID 和账户索引是否属于交易消息，并返回 `simulated_inner_instruction_count`、`simulated_inner_programs`。这些字段仅是清单，不表示内部调用已通过语义或费用审计。

若内部 System Program 指令都可识别为标准转账，预检还返回 `simulated_inner_payer_system_spend_lamports`，并与顶层显式转账合并检查买入金额加 0.02 SOL 的上限。任一内部 System 指令无法解析时该字段为 `null`，不能据此推断总支出；Token Program 和其他 CPI 费用仍未覆盖。

目标 ATA 的交易前和模拟后账户使用 SPL Token / Token-2022 的结构化解析器验证初始化状态、Mint、钱包 owner 与原始数量；无效账户直接拒绝，避免只凭固定字节位置推断余额。

顶层和 RPC 返回的内部 Token 指令若把目标 ATA 作为首个操作账户，预检仅接受账户初始化类指令；从目标 ATA 转出、授权、关闭或冻结等操作会拒绝。目标 ATA 作为转入目的地不会因这条规则被拒绝。该规则不覆盖其他中间 token 账户或聚合器程序内部语义，不能作为 Live 放行条件。

StonkFun Via-SOL 示例（功能开关默认关闭）：

第一跳可通过同一鉴权方式调用 `POST /v1/pools/sol-hop`，请求体为 `{"quote_mint":"CUSTOM_QUOTE_MINT"}`。响应包含 `available`、`pool`、`pool_id` 和 `reason`；目前仅扫描并校验 Raydium CPMM 的 `WSOL/Quote` 池，且 `STONKFUN_VIA_SOL_ENABLED` 必须开启。`available:true` 只代表池结构可用于两跳，不是成交报价；下单前应重新查询并核对 `pool_id`，不能据此显示预计到手数量或价格影响。找不到直达池时返回 `available:false`，不会自动改用三跳或聚合器。

```json
{
  "wallet_id": "main",
  "side": "buy",
  "pool": "stonkfun",
  "mint": "MEME_MINT",
  "pool_id": "STONKFUN_POOL",
  "quote_mint": "CUSTOM_QUOTE_MINT",
  "settlement_asset": "sol",
  "sol_hop_pool": "raydium-cpmm",
  "sol_hop_pool_id": "WSOL_QUOTE_CPMM_POOL",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "execution_method": "sdk",
  "simulate": true,
  "idempotency_key": "stonk-via-sol-dry-run-001"
}
```

### 8.5 Pool 与 Quote 支持矩阵

| `pool` | SDK 的 `pool_id` | 支持 Quote | 说明 |
|---|---|---|---|
| `pump` | 必须省略 | WSOL、USDC | Pump bonding curve |
| `pump-amm` | 可选，建议传 | WSOL、USDC | PumpSwap |
| `bonk` | 必须省略 | WSOL、USD1 | Bonk/LaunchLab 路径 |
| `stonkfun` | 必填 | WSOL、USDC、USD1、任意 SPL Token | 自定义 Quote 可直接传 `amount_quote_units`，或在开关开启后显式走 Via-SOL |
| `stonkfun-swap` | 必填 | WSOL、USDC、USD1、任意 SPL Token | 自定义 Quote 可直接传 `amount_quote_units`，或在开关开启后显式走 Via-SOL |
| `meteora-damm-v2` | 必填 | WSOL、USDC | 必须传真实池地址 |
| `meteora-dbc` | 必填 | WSOL、USDC | 未迁移的 Dynamic Bonding Curve 池；使用 `swap2` ExactIn |
| `raydium-amm-v4` | 必填 | WSOL、USDC | 必须传真实池地址 |
| `raydium-cpmm` | 必填 | WSOL、USDC、任意 SPL Token | 自定义 Quote 直买必须传 `amount_quote_units`，不是 SOL 金额；服务校验池的两个 Mint 和买入 Quote 余额。此路径仍需逐池模拟验收，不代表支持 SOL 跨池交易 |

已知 Quote Mint：

| 资产 | Mint | 精度 |
|---|---|---:|
| WSOL | `So11111111111111111111111111111111111111112` | 9 |
| SOL 别名 | `So11111111111111111111111111111111111111111` | 输入后转为 WSOL |
| USDC | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` | 6 |
| USD1 | `USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB` | 6 |

### 8.6 MEV Provider 规则

`mev_provider` 可选值：

```text
auto
default
jito
temporal
flashblock
blockrazor
astralane
speedlanding
```

兼容别名包括 `rpc`、`flash-block`、`block-razor` 和 `speed-landing`。

| 请求组合 | 实际行为 |
|---|---|
| `use_mev=false` | 只使用 Default RPC |
| `mev_provider=default`，省略 `use_mev` | 等价于关闭 MEV |
| 指定具体 Provider，省略 `use_mev` | 自动视为 `use_mev=true` |
| `use_mev=true`，指定 Provider | 只选择该 Provider 的已启用 lane |
| `use_mev=true`，不指定 Provider | 使用已启用的 MEV lanes，不走 Default |
| `mev_provider=auto` | 有 MEV lane 时使用 MEV，否则使用 Default |
| 两个字段都省略 | 使用服务端全局 lane 策略 |

冲突参数会被拒绝，例如：

- `use_mev=false` 配合 `mev_provider=temporal`。
- `use_mev=true` 配合 `mev_provider=default`。
- 指定服务端未启用的 Provider。

生产调用建议显式传入 `use_mev` 和 `mev_provider`，不要依赖服务端默认顺序。

## 9. 创建交易响应

### 9.1 已完成本地提交

```http
HTTP/1.1 200 OK
```

```json
{
  "request_id": "tr_202f8f8618194ed4b18f528d9b0305aa",
  "wallet_id": "main",
  "wallet_pubkey": "<WALLET_PUBKEY>",
  "execution_method": "sdk",
  "status": "submitted",
  "signature": "<SOLANA_TRANSACTION_SIGNATURE>",
  "error": null,
  "duplicate": false
}
```

`submitted` 只表示服务已获得提交签名，不等于 Solana 链上 `confirmed` 或 `finalized`。

### 9.2 仍在后台处理

如果 `TRADE_RESPONSE_WAIT_MS` 窗口内没有完成本地执行，返回：

```http
HTTP/1.1 202 Accepted
```

```json
{
  "request_id": "tr_xxx",
  "wallet_id": "main",
  "wallet_pubkey": "<WALLET_PUBKEY>",
  "execution_method": "sdk",
  "status": "submitting",
  "signature": null,
  "error": null,
  "duplicate": false
}
```

调用方应保存 `request_id` 并查询 `/v1/trades/:request_id`，不能因为 HTTP `202` 再创建一笔新交易。

### 9.3 参数被拒绝

字段已经能组成交易记录但校验失败时，接口返回 `400` 和 TradeResponse：

```json
{
  "request_id": "tr_xxx",
  "wallet_id": "main",
  "wallet_pubkey": "<WALLET_PUBKEY>",
  "execution_method": "sdk",
  "status": "rejected",
  "signature": null,
  "error": "sell_percent is required for sell",
  "duplicate": false
}
```

在进入交易记录前失败的请求返回简单错误对象：

```json
{
  "error": "wallet_id unknown_wallet not found"
}
```

客户端必须同时兼容这两种错误 JSON 结构。

## 10. 幂等规则

### 10.1 作用域

幂等唯一键是：

```text
(wallet_id, idempotency_key)
```

因此：

- 同一个幂等键可用于不同钱包。
- 同一钱包的一次业务动作必须固定使用同一个幂等键。
- 网络超时后重试必须原样复用原幂等键和交易参数。
- 不要用当前时间作为每次 HTTP 重试的新幂等键。

推荐格式：

```text
<system>:<business-order-id>:<action>:<sequence>
copy:signal-8b53:buy:1
limit:order-19382:sell:1
manual:telegram-update-8821:buy:1
```

### 10.2 重复请求

相同钱包、相同幂等键、相同规范化交易参数会返回原订单：

```json
{
  "request_id": "tr_original",
  "status": "submitted",
  "signature": "<ORIGINAL_SIGNATURE>",
  "duplicate": true
}
```

HTTP 状态为 `200 OK`，不会再次执行交易。

相同钱包和幂等键用于不同交易参数时返回：

```http
HTTP/1.1 409 Conflict
```

```json
{
  "error": "idempotency key was already used for a different trade"
}
```

校验阶段产生的 `rejected` 记录不会占用可执行幂等键。修正参数后可以继续使用原业务幂等键。

源码已补充 SOL/Quote 分支检查：例如 `0.001 SOL` 聚合买入与 `1 USDC` 直接买入即使规范化整数碰巧相同，也属于不同交易，同一 key 返回 `409`。旧持久化 hash、订单与签名保持不变。durable 命中旧行时若已存请求 JSON 无法安全核对，会返回 `503` 并保留旧记录，不能换 key 自动重发。memory/shadow 同样拒绝同键不同参数或付款分支，但仍没有跨重启保障。本修复尚未部署，详见 [输入资产幂等修复](optimization/ORDINARY_TRADE_HASH_REVIEW.md)。

## 11. 查询交易

### 11.1 查询单笔

```bash
curl -sS "$TRADE_API_BASE_URL/v1/trades/tr_xxx" \
  -H "Authorization: Bearer $TRADE_API_TOKEN"
```

响应为完整 TradeRecord：

```json
{
  "request_id": "tr_xxx",
  "wallet_id": "main",
  "wallet_pubkey": "<WALLET_PUBKEY>",
  "side": "buy",
  "pool": "pump",
  "mint": "<TOKEN_MINT>",
  "pool_id": null,
  "execution_method": "sdk",
  "status": "submitted",
  "signature": "<SOLANA_TRANSACTION_SIGNATURE>",
  "error": null,
  "duplicate": false,
  "created_at_ms": 1785811200000,
  "updated_at_ms": 1785811200123
}
```

不存在时返回 `404`：

```json
{"error":"trade not found"}
```

### 11.2 查询最近交易

```bash
curl -sS "$TRADE_API_BASE_URL/v1/trades?limit=20" \
  -H "Authorization: Bearer $TRADE_API_TOKEN"
```

响应是 TradeRecord 数组。`limit` 默认 `20`，服务会限制在 `1` 到 `100`。

当前接口没有按钱包、Mint、状态分页或过滤的参数，不适合作为完整历史订单分页接口。

## 12. 状态机与链上确认

最新源码增量（NP-01-D1B2，未部署、默认关闭）：显式启用 `ORDINARY_OBSERVATION_ENABLED` 且 durable 时，可在 bind 成功后有界扫描普通 SOL 聚合原账本中的 signed/submitted/unknown，按原签名查询并保存明确 finalized 结果。重开恢复不再签发 owner、不重建/签名/重发，缺失/损坏证据拒绝。前两个独立 RPC 共用局部请求预算/429 冷却；这不是全服务 RPC 配额。没有新公开恢复接口或请求字段，GET/列表/重复 POST 仍不触发 RPC；调用方不能假定线上已开启、所有 backend 都会自动确认或所有 `confirmed` 都有相同最终性。见 [可选待确认扫描](optimization/ORDINARY_PENDING_OBSERVATION_WORKER.md)。

前阶段源码基础（NP-01-D1B1）：普通聚合观测使用不重新获取执行权的 context/core，显式内部恢复和原证据核对是 D1B2 的基础；D1B1 本身不提供启动扫描。原 API 和重启不重放合同不变。见 [无 Owner 恢复上下文](optimization/ORDINARY_RECOVERY_OBSERVATION_CONTEXT.md)。

源码开发增量（NP-01-D1A，未部署）：Durable Live 普通 SOL 聚合在本次发送尝试和回执落盘后，按原签名做一次有 3 秒 RPC deadline 的只读观测；明确 finalized 成功可保存为既有 confirmed，明确 finalized 错误保存为 failed 并保留原签名。弱确认、查不到或查询故障均保留原 submitted/unknown；首次响应可能早于这次观测结束。GET/列表和重复 POST 不会因此发起 RPC 或重新发送，不能依赖此增量持续确认或恢复重启前的中断订单。其他 backend/mode 的状态语义不变，服务所有 confirmed 也不能一概解释为 finalized。详见 [发送后原签名观测](optimization/ORDINARY_POST_SEND_OBSERVATION.md)。下表继续保留现有发布合同与调用方保守处理要求。

服务状态：

| 状态 | 含义 | 调用方动作 |
|---|---|---|
| `accepted` | 请求已接收 | 继续轮询 |
| `warming_params` | 参数预热中，预留状态 | 继续轮询 |
| `building_tx` | 正在构建交易 | 继续轮询 |
| `submitting` | 正在提交交易 | 继续轮询 |
| `simulated` | 模拟结束 | 不会产生真实链上成交 |
| `submitted` | 已获得签名 | 查询 Solana 链上状态 |
| `confirmed` | 已确认，预留状态 | 当前版本不要依赖服务自动进入此状态 |
| `failed` | 本地执行或已知提交失败 | 检查 `error`，按业务规则处理 |
| `rejected` | 参数校验失败 | 修正请求，不要盲目重试 |
| `unknown` | 最终结果不明确 | 使用签名、钱包余额和链上交易人工/自动对账 |

建议调用方状态机：

```text
POST 200/202
  -> accepted/building_tx/submitting: 按 request_id 轮询
  -> simulated: 模拟结束
  -> failed/rejected: 失败结束
  -> submitted + signature: 查询链上状态
       -> finalized 且 err=null: 成功
       -> confirmed/finalized 且 err!=null: 链上失败
       -> 长时间不可见: unknown，进入对账，不自动重新下单
```

服务重启时，持久化库中的 `accepted`、`warming_params`、`building_tx`、`submitting` 会转为 `unknown`，不会自动重放。调用方必须先对账再决定是否创建新的业务交易。

普通 SDK 路径的新通道安全实现（源码阶段，部署须另行验收）：只有进入 SDK 前的参数准备失败允许尝试下一配置通道。一旦开始 SDK 买卖或最终预检，即使返回 Err、缺失签名、多个签名、签名短期不可见或核验 RPC 失败，也不会自动换通道重新构建交易；Live 结果不明使用现有 `unknown` 状态，有原签名则保留。多个已知签名在现有 `error` 诊断中保留，`signature` 字段仍为第一个；这不是逐签名持久化账本。SDK 模拟仍可能签名，不等同于项目的 unsigned probe。

后续源码阶段已将普通 DAMM v2 **卖出**的 `simulate:true` 与 Live 最终 preview 改为完整零签名产物的只读模拟，不再调用 SDK simulate/sign。该模拟请求成功仍返回 `simulated`，`signature:null`；模拟拒绝为 `failed` 且无签名，字段格式/原 key/重复查询保持。Live preview 失败不继续正式 SDK，preview 不再提供可能被误认作执行回执的签名。此改动未部署，也不覆盖其他 SDK 方向/协议；原 SDK 方法禁播准入仍保留，普通 SDK 广播前账本尚未完成。详见 [不可变候选与只读模拟](optimization/DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md)。

`unknown` 不是未提交证明，也不是重新下单许可。调用方应保存原 `request_id`、原幂等键与参数，查询原记录并核对原签名；不得自动换 key、重复签名或补单。签名核验超时或 HTTP 取消不证明 SDK 后台任务停止，当前普通 SDK 尚无广播前完整 signed journal；其他未分类结果（含 PumpAPI）的旧状态合同不因本局部实现而改变。

普通 SOL 聚合买入另有源码阶段的回执保护（未部署）：本地签名后，广播报错或 RPC 返回不同签名时，现有 `signature` 字段保留本地原签名，`status` 为 `unknown`，错误中的 URL 会脱敏。不会采用另一条 RPC 回执冒充原交易提交成功，也不会因为错误文字包含 `sendTransaction failed` 等字符串而将该明确的不确定结果改成 `failed`。原幂等键、请求/响应字段、正常提交和模拟语义保持；同键重复请求只返回原订单，不重新取价、签名或发送。

E0 回执保护本身不是广播前签名字节账本；该限制仍适用于 Memory/Shadow legacy 路径，取消、崩溃或最终订单写库失败时仍可能没有可恢复的原签名/字节。后续 C2b 已为 Durable Live 普通 SOL 聚合实现下述独立读回保护，不能把两者混为所有普通交易均已完成。见 [普通聚合回执安全检查点](optimization/ORDINARY_AGGREGATE_UNCERTAINTY.md)。

后续源码阶段的订单身份接线（未部署）：新普通订单携带原 reservation 身份及冻结的项目执行命令，原重复请求不生成新执行输入。普通 SOL 聚合在报价前还会核对已加载钱包 ID/公钥与接受订单时一致；不一致时停止并返回 `failed`，原 key 仍保留，不自动重新执行。本次没有新增字段或改旧 hash/请求格式；这不是最终消息授权、广播前账本或所有 SDK 路径的钱包核对完成，见 [原订单身份与执行输入](optimization/ORDINARY_RESERVATION_EXECUTION_ANCHOR.md)。

新增源码阶段的执行占用（未部署）：普通 SOL 聚合 Live 执行在任何状态更新或报价前占用原订单。Memory/Shadow 使用原内存幂等记录，Durable 使用关联原订单的 `ordinary_execution_claims` 行；已有占用不重执行、不覆盖原状态/签名。占用存储异常或身份不符时只停止并记录诊断，原订单可能保持 `accepted`（新请求等待结束为 202，同键重复仍为 200/duplicate），查询数据库故障可能为 503；不能把这些响应当作换 key 重试许可。旧交易字段、V1 hash/JSON 和唯一业务 key 不变。C2a 原始表仅保存占用；C2b 后续扩展签名字节，但 Memory/Shadow 没有跨重启保证。见 [原子执行占用](optimization/ORDINARY_EXECUTION_CLAIMS.md)。

Durable Live 普通 SOL 聚合新增源码阶段签名账本（未部署）：同一原订单占用保存 owner、报价/预检及原消息，签名后原签名/字节与父订单 `submitting` 原子写入；独立读回并逐字节核对后才发送读回字节。写入、读回或消息绑定失败不发送，不退回 legacy；不同回执/广播错误保留原签名和 `unknown`。签名字节写入失败时内存可保留 Unknown/原签名，但数据库可能仍为 `building_tx` 且无签名，等待可为 202；外层不会补写父行冒充签名字节已提交。签名事务已提交后重启则沿用原 pending→Unknown 恢复，保留签名，不自动重播。旧 004 占用无 owner 不会因迁移重新执行。接口不返回 owner 或交易原字节，无新请求字段。这仅覆盖 Durable Live `aggregator_sol`，不覆盖 Memory/Shadow、普通 opaque SDK 或 PumpAPI，不证明完整消息授权/跨重启 exactly-once，详见 [Durable 普通签名账本](optimization/ORDINARY_DURABLE_SIGNED_JOURNAL.md)。

推荐轮询策略：前 2 秒每 100 到 200 毫秒一次，之后每 1 秒一次；得到 `submitted`、`simulated`、`failed`、`rejected` 或 `unknown` 后停止查询本服务。不要持续轮询等待 `finalized`。

C3c1/C3c2 源码增量（完整离线 CI 并发/串行各 1398 通过，未部署）：新 Durable Live 直接 DAMM v2 SDK 卖出已接原 owner、受控签名和广播前原签名字节账本，不调用 SDK 高层后台 worker；只发送独立读回并单次领取的原字节。Default 回执必须匹配原签名；MEV 的 SDK trait Ok 不冒充远端签名，按原签名只读观测，缺少成功证据保留 `unknown`。`MEV_SUBMIT_VERIFY_MS=0` 不授权把无可核验回执当成功，`WAIT_TX_CONFIRMED=true` 等待原签名的 confirmed/finalized。父行 `rejected/failed` 的无签名前处理与字段/hash/JSON/单位兼容，不增加 owner 或字节返回字段；同键重复/GET/列表仍不发起签名、广播或 RPC。该子路径的持续确认/跨重启只读恢复、其他方向/协议/backend/mode 和生产验收尚未完成，不能把此源码增量当全普通 SDK 已安全迁移。详见 [原 SDK 实际接线](optimization/SDK_ORIGINAL_DAMM_RUNTIME.md)。

C3d1 后续源码（18 项新用例仅类型检查，未部署）：显式只读恢复按原 operation/请求/hash/候选和已签 packet 校验，只有匹配原签名的 finalized 证据可使 SDK 原订单进入既有 `confirmed/failed` 状态；迟到提交回执不覆盖终态。无新 API 路径/字段/数量单位/hash/JSON 或配置，SDK 自动后台扫描仍待 C3d2，不能因本段说明认为生产已经启用恢复。详见 [只读恢复核心](optimization/SDK_ORIGINAL_SIGNATURE_RECOVERY.md)。

源码阶段的普通 SDK DAMM 卖出 probe 增加固定账户快照门槛，核对 pool/mint/vault/用户 ATA 身份和 Token 扩展，用户 ATA 的 delegate/close-authority 及未审计 referral 暂时保守拒绝，输入 ATA 必须持有本笔需要的数量。这是新增操作级安全限制，尚未部署，需发布前验证调用方兼容；不是新增请求字段，也没有将该保护自动扩展到买入/其他协议。账户 snapshot 与模拟的 slot 下限绑定不等于跨 bank 的完整可信报价，仍不得把模拟失败或 `unknown` 当作换 key 重试许可。见 [DAMM 证据与剩余门槛](optimization/DAMM_PROBE_ACCOUNT_EVIDENCE.md)。

补充源码阶段进展：DAMM 卖出 probe 已用本笔 CPI token 转账及同次模拟 Mint/Clock/账户状态计算净输出，不再用较早 RPC 的余额与模拟后余额相减。模拟需要 innerInstructions 和固定八账户结果；节点不提供完整证据时会拒绝报价，不自动退回旧算法。节点若返回同次模拟 pre/postTokenBalances，还会严格交叉核对。公开请求/响应字段、原始单位和滑点政策不变，未部署、未实网验收；这也不是最终 SDK 交易的完整签名前授权。见 [本笔模拟输出验收](optimization/DAMM_SIMULATED_SWAP_OUTPUT.md)。

## 13. 批量多钱包交易

```http
POST /v1/batch/trades
```

请求：

```bash
curl -sS -X POST "$TRADE_API_BASE_URL/v1/batch/trades" \
  -H "Authorization: Bearer $TRADE_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "wallet_ids": ["main", "wallet_abc123"],
    "trade": {
      "side": "buy",
      "pool": "pump",
      "mint": "<TOKEN_MINT>",
      "amount_sol": 0.001,
      "slippage_percent": 15,
      "execution_method": "sdk",
      "use_mev": false,
      "simulate": true,
      "idempotency_key": "batch-signal-20260804-001"
    }
  }'
```

响应固定使用 `202 Accepted`：

```json
{
  "items": [
    {
      "wallet_id": "main",
      "request_id": "tr_xxx",
      "status": "simulated",
      "error": null,
      "duplicate": false
    },
    {
      "wallet_id": "wallet_abc123",
      "request_id": "tr_yyy",
      "status": "simulated",
      "error": null,
      "duplicate": false
    }
  ]
}
```

注意：

- `wallet_ids` 不能为空。
- `trade.wallet_id` 会被每个 `wallet_ids` 项覆盖。
- 同一个 `idempotency_key` 可以安全用于该批次的不同钱包，因为幂等作用域包含 `wallet_id`。
- 当前实现按 `wallet_ids` 顺序逐个接收，每一项可能等待最多 `TRADE_RESPONSE_WAIT_MS`，不是并行批量广播。
- 批量接口整体返回 `202`，每个项目的成功或失败必须看 `items`。

## 14. HTTP 状态码与常见错误

| HTTP | 含义 |
|---:|---|
| `200` | 查询成功，或交易已得到非 pending 本地状态，或命中幂等旧订单 |
| `201` | 钱包管理 API 创建钱包；不用于交易接口 |
| `202` | 交易仍在处理，或批量请求已接收 |
| `400` | 请求参数、钱包或 Pool 参数不合法 |
| `401` | Bearer Token 错误或缺失 |
| `403` | 来源 IP 不在白名单 |
| `404` | 交易不存在 |
| `409` | 幂等键已用于不同参数 |
| `502` | Pool 解析、参数预热或钱包加载等同步上游调用失败；后台交易执行失败通常仍是 `200 + status=failed` |
| `503` | durable 交易数据库不可用，交易没有开始 |

常见错误：

| 错误片段 | 原因与处理 |
|---|---|
| `mint must be a valid pubkey` | Mint 不是有效 Solana 公钥 |
| `wallet_id ... not found` | 钱包未加载，重新查询 `/v1/wallets` |
| `pool_id is required` | Meteora/Raydium SDK 路径缺少池地址 |
| `pool_id is not used for this pool` | Pump/Bonk SDK 路径不应传池地址 |
| `quote_mint must be WSOL, USDC, or USD1` | Quote 不在允许列表 |
| `stonkfun Via-SOL is disabled` | 服务端功能开关未开启 |
| `sol_hop_pool_id is required` | Via-SOL 缺少 WSOL/Quote 第二跳池地址 |
| `Via-SOL hop pool does not contain` | 第二跳池与 WSOL/Quote 不匹配，拒绝交易 |
| `sell_percent must be between 1 and 100` | 卖出比例非法 |
| `slippage must be between 0% and 100%` | 滑点非法 |
| `<provider> has no enabled execution lane` | 指定 Provider 未启用或初始化失败 |
| `blockhash cache is empty or stale` | SDK 当前不可交易，稍后重试并检查 `/ready` |
| `trade persistence unavailable; trade was not started` | durable 数据库不可用；本次保证未进入执行器 |
| `submit signature not visible ...` | 提交通道返回后暂未被 RPC 看到；必须链上对账，不能直接重复发送 |

## 15. TypeScript 接入示例

```ts
type TradeStatus =
  | "accepted"
  | "warming_params"
  | "building_tx"
  | "submitting"
  | "simulated"
  | "submitted"
  | "confirmed"
  | "failed"
  | "rejected"
  | "unknown";

interface TradeResponse {
  request_id: string;
  wallet_id: string;
  wallet_pubkey: string;
  execution_method: "sdk" | "pumpapi";
  status: TradeStatus;
  signature: string | null;
  error: string | null;
  duplicate: boolean;
}

const baseUrl = process.env.TRADE_API_BASE_URL!;
const token = process.env.TRADE_API_TOKEN!;

async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`${baseUrl}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      ...init.headers,
    },
  });

  const body = await response.json();
  if (!response.ok) {
    const message = body.error ?? `${body.status ?? "request"} failed`;
    throw new Error(`Trade API ${response.status}: ${message}`);
  }
  return body as T;
}

async function createTrade(): Promise<TradeResponse> {
  return api<TradeResponse>("/v1/trades", {
    method: "POST",
    body: JSON.stringify({
      wallet_id: "main",
      side: "buy",
      pool: "pump",
      mint: process.env.TOKEN_MINT,
      amount_sol: 0.001,
      slippage_percent: 15,
      execution_method: "sdk",
      use_mev: false,
      mev_provider: "default",
      simulate: true,
      idempotency_key: `example:${crypto.randomUUID()}:buy:1`,
    }),
  });
}
```

生产代码需要把 `idempotency_key` 与业务订单一起持久化。这里的随机 UUID 只能在首次创建业务动作时生成，HTTP 重试时必须复用，不能重新生成。

## 16. 上线验收清单

1. 从真实调用服务器执行 `/health` 和 `/ready`。
2. 验证错误 Token 返回 `401`，非白名单来源返回 `403`。
3. 查询 `/v1/wallets` 并核对 wallet ID 和公钥。
4. 使用 `simulate:true` 分别完成 SDK 买入和卖出。
5. 对同一请求重复提交，确认 `request_id` 不变且 `duplicate=true`。
6. 用相同幂等键修改金额，确认返回 `409`。
7. 验证 `/v1/pools/resolve` 的 Pump、PumpSwap 和 Meteora DBC 结果。
8. 使用 `simulate:false` 做经授权的最小金额实盘。
9. 根据返回签名独立查询链上 `confirmed/finalized` 和交易错误。
10. 验证服务重启后原订单仍可通过 `request_id` 查询。
11. 确认公网反向代理无法访问钱包创建、删除、导入和私钥导出路径。

## 17. 当前未提供的能力

以下能力尚未成为当前 Public Trade API 合同的一部分：

- API Key 多租户、按用户/钱包授权和配额。
- 路由级 Token 权限。
- HTTPS 终止。
- 请求签名、时间戳和防重放认证。
- 限流和每钱包并发控制。
- 链上 `confirmed/finalized` 自动回写。
- 按钱包、Mint、状态分页查询历史订单。
- Webhook/SSE/WebSocket 订单状态推送。
- Bonk、Meteora DAMM v2、Raydium AMM v4/CPMM 的统一 Pool 自动解析数据库。
- OpenAPI/Swagger schema。

如果要提供给非完全受信任的第三方，必须先补齐路由级权限、TLS、调用方身份、限流、审计和私钥管理接口隔离。
