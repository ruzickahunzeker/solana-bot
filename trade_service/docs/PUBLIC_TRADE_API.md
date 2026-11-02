# Public Trade API 对接文档

文档版本：2026-09-03  
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

### 1.1 当前部署配置

当前环境的关键配置如下，接入前仍应通过 `GET /ready` 动态确认：

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

`simulate:false` 会覆盖全局 `ENABLE_SIMULATE=true` 并发送真实交易，因此接入方必须把这个字段作为高风险参数处理。

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

当前应用本身不提供 TLS，也没有按路由区分交易权限和钱包管理权限。公网部署必须满足：

1. 使用防火墙只允许指定来源 IP 访问 `18787`。
2. 跨公网传输时在前面加 HTTPS、WireGuard 或其他加密隧道，避免 Bearer Token 明文传输。
3. 反向代理仅放行本文“公开交易接口”列出的路径。
4. 拒绝外部访问全部 `/internal/*` 路由，以及 `/v1/wallets/manage`、`POST /v1/wallets`、`PATCH/DELETE /v1/wallets/*` 和 `/v1/wallets/*/export`。SOL 转账只存在于本机内部钱包接口，不属于 Public Trade API。
5. 不在日志、URL、错误上报或客户端代码仓库中记录 Token。

尤其不要向普通 Public API 调用方开放私钥导出接口。当前同一个 Token 可以访问全部已加载钱包，不具备租户隔离能力。

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

### 只读 SOL 买入路线预览

`POST /v1/quotes/sol-buy` 使用现有 Trade API 鉴权，接受 `{"wallet_id":"2TE","mint":"目标 mint","amount_lamports":1000000,"slippage_bps":1500}`。请求仅支持 0.0001–0.1 SOL、1–3000 bps；返回绑定钱包、15 秒有效期、`expected_output_units`、`minimum_output_units`、`price_impact_percent` 和路线各跳。服务重启会使内存报价失效。

`POST /v1/quotes/sol-buy/:quote_id/preflight?wallet_id=2TE` 会重新取价并执行交易审计与本地模拟；路线变化、最低到手降低、目标 ATA 不可写、费用超限或模拟失败都会拒绝。成功时 `instruction_audit_passed:true`、`executable:true`。它仍不签名、不广播。

`POST /v1/quotes/sol-buy/:quote_id/submit` 接受 `{"wallet_id":"2TE","confirmed":true,"simulate":false,"idempotency_key":"唯一键"}`。服务会再次取价、重新审计和模拟，通过后由 Trade Service 钱包签名并广播；`simulate:true` 只返回 `simulated`，不会广播。提交异常返回 `unknown`，调用方不得自动创建新幂等键重试，应按链上签名或 Trade Service 记录核查。该接口只支持 SOL Exact-In 买入。

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

推荐轮询策略：前 2 秒每 100 到 200 毫秒一次，之后每 1 秒一次；得到 `submitted`、`simulated`、`failed`、`rejected` 或 `unknown` 后停止查询本服务。不要持续轮询等待 `finalized`。

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
