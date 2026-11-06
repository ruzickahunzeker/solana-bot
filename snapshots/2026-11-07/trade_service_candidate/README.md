# Pump Trade Service

公网可信调用方的完整接口契约、幂等规则、Pool/Quote 矩阵和接入示例见：

- [Public Trade API 对接文档](docs/PUBLIC_TRADE_API.md)

本地 HTTP 交易服务。策略进程可以把 PumpAPI stream 里产生的交易意图发到这个服务，默认由本服务通过 `sol-trade-sdk` 执行交易；也可以在单笔请求里指定 `execution_method:"pumpapi"`，使用 PumpAPI Local Transaction 构造 unsigned transaction 后由本服务本地签名。

重要边界：

- 默认交易方式是 `sol-trade-sdk`。
- `execution_method:"pumpapi"` 只使用 PumpAPI Local Transaction 模式。
- 本服务不会使用 PumpAPI Lightning 私钥模式。
- HTTP 请求体里不能传私钥。
- 私钥只从服务端配置的钱包文件或钱包数据库读取。
- PumpAPI 只会收到钱包公钥、mint、quote、金额、滑点和 priority fee，不会收到私钥。
- 服务默认只监听 `127.0.0.1:18787`。

## 配置

服务启动时会自动读取当前目录下的 `.env` 文件。

示例：

```env
RPC_URL=https://your-primary-rpc.example
BACKUP_RPC_URLS=https://your-backup-rpc-1.example,https://your-backup-rpc-2.example

PRIVATE_KEY_BASE58=replace-with-base58-encoded-solana-keypair-secret
LOCAL_TRADE_API_TOKEN=replace-with-local-random-token

# 多钱包可选配置。优先使用加密钱包文件。
DEFAULT_WALLET_ID=main
WALLETS_CONFIG_ENCRYPTED=/root/http/sol-bot/trade_service/wallets.json.enc
WALLETS_CONFIG_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase
WALLET_STORAGE_MODE=file
WALLET_DATABASE_URL=sqlite://./data/wallets.sqlite
WALLET_DB_BUSY_TIMEOUT_MS=5000
WALLET_DATABASE_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase

# 明文兼容模式，不建议长期使用。
# WALLETS_CONFIG=/root/http/sol-bot/trade_service/wallets.json

TRADE_HTTP_ADDR=127.0.0.1:18787
DEFAULT_BUY_SOL=0.001
DEFAULT_SLIPPAGE_PERCENT=15
DEFAULT_TIP_SOL=0
ENABLE_SIMULATE=true
WAIT_TX_CONFIRMED=false
ENABLE_SDK_LOG=false
TRADE_RESPONSE_WAIT_MS=3000
MEV_SUBMIT_VERIFY_MS=1200

ENABLE_PUMPAPI_EXECUTOR=false
PUMPAPI_URL=https://api.pumpapi.io
PUMPAPI_TIMEOUT_MS=1200
PUMPAPI_DEFAULT_PRIORITY_FEE_SOL=0.0001
PUMPAPI_LIGHTNING_ENABLED=false

BLOCKHASH_REFRESH_MS=400
BLOCKHASH_MAX_AGE_MS=2000

# 交易/幂等持久化。默认 memory 保持原行为。
TRADE_PERSISTENCE_MODE=memory
TRADE_DATABASE_URL=sqlite://./data/trades.sqlite
TRADE_DB_BUSY_TIMEOUT_MS=5000

ENABLE_MEV_PROTECTION=false
ENABLE_JITO=false
JITO_UUID=
JITO_REGION=frankfurt
JITO_URL=

ENABLE_TEMPORAL=false
TEMPORAL_API_KEY=
TEMPORAL_REGION=frankfurt
TEMPORAL_URL=

ENABLE_FLASHBLOCK=false
FLASHBLOCK_API_KEY=
FLASHBLOCK_REGION=frankfurt
FLASHBLOCK_URL=

ENABLE_BLOCKRAZOR=false
BLOCKRAZOR_API_KEY=
BLOCKRAZOR_REGION=frankfurt
BLOCKRAZOR_URL=
BLOCKRAZOR_TRANSPORT=grpc

ENABLE_ASTRALANE=false
ASTRALANE_API_KEY=
ASTRALANE_REGION=frankfurt
ASTRALANE_URL=
ASTRALANE_TRANSPORT=binary

ENABLE_SPEEDLANDING=false
SPEEDLANDING_API_KEY=
SPEEDLANDING_REGION=frankfurt
SPEEDLANDING_URL=
```

## 交易与幂等持久化

`TRADE_PERSISTENCE_MODE` 支持三种模式：

- `memory`：默认值，完全沿用原内存交易记录和幂等行为，不创建数据库。
- `shadow`：仍由内存决定是否交易，同时把状态镜像到 SQLite；写库失败不改变原交易路径。
- `durable`：SQLite 原子保留 `(wallet_id, idempotency_key)` 后才允许进入原执行器。数据库不可用时返回 `503`，不会降级到内存执行。

durable 模式下，相同钱包、相同幂等键、相同规范化交易参数会返回原订单且不再次执行；参数不同则返回 `409`。服务重启后，未完成记录会标记为 `unknown`，不会自动重放。

推荐切换顺序：

```text
memory -> shadow（完成对账）-> durable + ENABLE_SIMULATE=true（完成验收）
```

本功能不改变钱包加载、SDK/PumpAPI 交易构造、blockhash、RPC 或 MEV 路径。

## 参数说明

`RPC_URL`

主 RPC 地址。

`BACKUP_RPC_URLS`

备用 RPC 地址，多个用英文逗号分隔。服务会按顺序尝试主 RPC 和备用 RPC。也支持单个备用变量 `BACKUP_RPC_URL`，或别名 `RPC_BACKUP_URLS`。

`PRIVATE_KEY_BASE58`

base58 编码的 Solana keypair secret。单钱包兼容模式使用。只放在本地 `.env`，不要放到请求体、日志、README 示例真实值或远程仓库。

`WALLETS_CONFIG`

多钱包配置文件路径。设置后服务会从 JSON 文件加载多个钱包，并忽略 `PRIVATE_KEY_BASE58`。

示例：

```json
{
  "wallets": [
    {
      "wallet_id": "main",
      "private_key_base58": "replace-with-main-wallet-base58-keypair-secret"
    },
    {
      "wallet_id": "wallet_2",
      "private_key_base58": "replace-with-second-wallet-base58-keypair-secret"
    }
  ]
}
```

建议权限：

```bash
chmod 600 /root/http/sol-bot/trade_service/wallets.json
```

`WALLETS_CONFIG_ENCRYPTED`

加密多钱包配置文件路径。设置后优先级高于 `WALLETS_CONFIG` 和 `PRIVATE_KEY_BASE58`。服务启动时会在内存中解密，不会把明文写回磁盘。

加密格式：

- KDF：PBKDF2-HMAC-SHA256
- 加密：AES-256-GCM
- 默认迭代次数：600000

配套 passphrase 有两种方式：

```env
WALLETS_CONFIG_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase
```

或：

```env
WALLETS_CONFIG_PASSPHRASE=只建议临时 shell 使用，不建议写入 .env
```

迁移明文 `wallets.json` 到加密文件：

```bash
cd /root/http/sol-bot/trade_service
umask 077
openssl rand -base64 48 > .wallets.passphrase
cargo run --release --bin encrypt_wallets -- \
  --in wallets.json \
  --out wallets.json.enc \
  --passphrase-file .wallets.passphrase
chmod 600 .wallets.passphrase wallets.json.enc
```

然后 `.env` 使用：

```env
WALLETS_CONFIG_ENCRYPTED=/root/http/sol-bot/trade_service/wallets.json.enc
WALLETS_CONFIG_PASSPHRASE_FILE=/root/http/sol-bot/trade_service/.wallets.passphrase
# WALLETS_CONFIG=/root/http/sol-bot/trade_service/wallets.json
```

确认服务能用加密文件启动后，再处理旧明文 `wallets.json`。

`DEFAULT_WALLET_ID`

默认钱包 ID。交易请求不传 `wallet_id` 时使用这个钱包。默认值是 `main`。

`WALLET_STORAGE_MODE`

钱包来源模式，支持 `file` 和 `database`，默认 `file`。切换为 `database` 后，首次启动会在空数据库中导入当前 `WALLETS_CONFIG_ENCRYPTED`/`WALLETS_CONFIG` 钱包；后续以数据库为准。旧文件不会被修改，可用于回退。

`WALLET_DATABASE_URL`

钱包专用 SQLite 数据库，默认 `sqlite://./data/wallets.sqlite`。不要与高频交易事件数据库混用。

`WALLET_DB_BUSY_TIMEOUT_MS`

钱包数据库锁等待时间，默认 `5000` 毫秒。

`WALLET_DATABASE_PASSPHRASE_FILE`

数据库逐钱包密文使用的 passphrase 文件。未设置时兼容读取 `WALLETS_CONFIG_PASSPHRASE_FILE`。数据库备份与 passphrase 必须分开保存。

数据库模式提供本机鉴权钱包管理 API：`POST /v1/wallets` 创建或导入、`GET /v1/wallets/manage` 查看、`PATCH /v1/wallets/:wallet_id` 改名、`DELETE /v1/wallets/:wallet_id` 软删除、`POST /v1/wallets/:wallet_id/export` 导出。所有接口都要求 Bearer Token 和允许的客户端 IP，交易请求仍只传 `wallet_id`，不能传私钥。

### Telegram 钱包内部接口

以下接口只供同机 Telegram 服务使用，不属于 Public Trade API，也不应通过反向代理或防火墙暴露到公网：

- `GET /internal/wallet-balances`：批量查询当前用户可见钱包的原生 SOL 余额。链上账户不存在返回 `0`；RPC 查询失败返回空余额和错误，二者不会混淆。
- `POST /internal/wallets/vanity-jobs`：创建自定义地址生成任务；`GET/DELETE /internal/wallets/vanity-jobs/:job_id` 查询或取消任务。
- `POST /internal/wallets/:wallet_id/transfer-sol`：发送原生 SOL。只允许来源钱包的 `owner_type/owner_id` 与请求完全一致，`system/global` 钱包不能通过 Telegram 转出。

自定义地址匹配区分大小写，只允许 1 到 4 位 Base58 字符，字符 `0`、`O`、`I`、`l` 无效。服务全局最多并行执行一个生成任务，每个 owner 最多一个活动任务，单任务最多尝试 30,000,000 次。生成出的私钥只在进程内存中短暂存在，匹配成功后立即写入现有加密钱包数据库；任务表只保存公钥、进度和状态。

SOL 转账支持固定 lamports、50% 和 100%。固定金额和 50% 基于确认时重新读取的可转余额计算；服务动态调用 RPC 估算网络手续费和系统账户免租最低余额，并在免租余额之上额外保留 10,000 lamports 安全余量。100% 使用 `余额 - 精确网络手续费`，把来源系统账户降到 0 并由链上回收；同一公钥以后收到 SOL 时会重新出现。每次请求必须携带唯一 `idempotency_key`，重复请求返回原记录，不会再次提交。该接口不使用交易 Pool、滑点、小费或 MEV Provider。

`ALLOW_INTERNAL_SYSTEM_WALLET_TRANSFER`

是否允许本机内部钱包接口从 `system/global` 钱包转出 SOL，默认 `false`。只应在 Telegram 用户范围受控、`/internal/*` 未经代理暴露、Bearer Token 已妥善保管时开启。该参数不赋予 Public Trade API 转账能力。

`LOCAL_TRADE_API_TOKEN`

本地 HTTP API 的鉴权 token。它不是链上参数，也不是钱包私钥。

`CONVERSION_DATABASE_URL`

独立兑换记录数据库，默认 `sqlite://./data/conversions.sqlite`。保存 WSOL 报价、幂等请求指纹及签名交易记录；不得提交到 GitHub，备份时按交易数据单独处理。使用 WAL + `synchronous=FULL`。

`CONVERSION_QUOTES_ENABLED`

是否初始化独立兑换数据库并启用 WSOL 报价，默认 `false`。未开启时现有交易服务启动不依赖兑换数据库。

`CONVERSION_LIVE_ENABLED`

是否开放 WSOL 包装/解包的实盘提交，默认 `false`。启用前必须先开启 `CONVERSION_QUOTES_ENABLED`，完成钱包归属、模拟和小额验收；不要与现有 `ENABLE_SIMULATE` 混淆。

`POST /internal/conversions/wsol/quotes`

使用内部 Bearer 鉴权。请求示例：`{"wallet_id":"2TE","owner_type":"telegram","owner_id":"123","direction":"wrap","amount_lamports":1000000}`；全部解包使用 `{"direction":"unwrap","all":true}`，不同时传金额。服务校验钱包归属、读取当前 SOL/WSOL ATA 余额和网络手续费估算，返回 `quote`（绑定钱包、金额、余额基线、租金、60 秒有效期）及 `executable:false`。部分解包的临时账户租金先垫付后返还，净返还显示为 0；全部解包关闭原 ATA 的租金返还单独展示，不计入兑换数量。此报价接口不签名、不广播。

`POST /internal/conversions/wsol/simulate` 接受 `{"quote_id":"cq_...","wallet_id":"2TE","owner_type":"telegram","owner_id":"123","simulate":true,"idempotency_key":"..."}`，只执行 RPC 模拟，返回 `status:"simulated"`；模拟不占用实盘幂等记录。`POST /internal/conversions/wsol/submit` 使用相同字段但 `simulate:false,"confirmed":true`，且仅在 `CONVERSION_LIVE_ENABLED=true` 时开放。调用方必须在用户确认后才设置 `confirmed:true`。相同幂等键不同请求会拒绝；交易先模拟，再把签名和交易字节写入独立数据库，最后广播。超时返回 `unknown`，不得用新交易重试。`GET /internal/conversions/wsol/submissions/:submission_id?owner_type=telegram&owner_id=123` 按原签名回查状态，不自动重发。**当前未完成链上模拟与小额实盘验收，Bot 不应展示可执行兑换按钮。**

同一钱包一次只允许一笔未决兑换（`reserved`、`signed`、`submitted`、`unknown`）。遇到未决记录时，先用原 `submission_id` 查询状态；不能换幂等键再次提交。`signed` 或 `unknown` 即使暂未查到链上结果也不会自动重发。

调用交易接口时必须带：

```bash
Authorization: Bearer $LOCAL_TRADE_API_TOKEN
```

服务启动会优先读取当前目录或其上级目录的 `.env`；如果没有找到，还会从可执行文件所在目录往上查找 `.env`。因此推荐从项目目录启动：

```bash
cd /root/http/sol-bot/trade_service
./target/release/pump-trade-service
```

curl 所在 shell 也需要有同一个 token。可以临时从 `.env` 加载：

```bash
cd /root/http/sol-bot/trade_service
set -a
source .env
set +a
```

如果返回 `{"error":"unauthorized"}`，优先检查：

- curl 命令是否带了 `-H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN"`。
- 当前 shell 是否真的设置了 `LOCAL_TRADE_API_TOKEN`。
- 服务是否已重启并读取了最新 `.env`。

建议生成一个随机字符串，例如：

```bash
openssl rand -hex 32
```

`.env` 里这样填：

```env
LOCAL_TRADE_API_TOKEN=生成出来的随机字符串
```

`TRADE_HTTP_ADDR`

HTTP 监听地址。默认必须是 loopback，例如：

```env
TRADE_HTTP_ADDR=127.0.0.1:18787
```

默认使用 loopback。配置非 loopback 地址（例如 `0.0.0.0:18787`）时，必须同时配置非空的 `TRADE_ALLOWED_CLIENT_IPS`，否则服务拒绝启动。公网部署还应使用防火墙和 TLS/加密隧道。

`DEFAULT_BUY_SOL`

buy 请求没有传 `amount_sol` 时使用的默认买入金额，单位 SOL。比如 `0.001` 表示买入 0.001 SOL。

`DEFAULT_SLIPPAGE_PERCENT`

buy/sell 请求没有传 `slippage_percent` 时使用的默认滑点，单位百分比。`15` 表示 15%，服务内部会转换成 SDK 需要的 `1500 bps`。

`DEFAULT_TIP_SOL`

默认小费，单位 SOL。SDK 路径会把它传给交易 gas fee strategy，可供 Jito/SWQoS lane 使用。

当请求使用 `execution_method:"pumpapi"` 时，单笔 `tip_sol` 会转换成 PumpAPI 的 `priorityFee`；如果单笔没有传 `tip_sol`，使用 `PUMPAPI_DEFAULT_PRIORITY_FEE_SOL`。

`STONKFUN_VIA_SOL_ENABLED`

是否允许 StonkFun 自定义 Quote 使用原子两跳 `SOL -> Quote -> Meme`（卖出反向执行）。默认 `false`。开启后请求仍必须显式传 `settlement_asset:"sol"`、`sol_hop_pool:"raydium-cpmm"` 和经过确认的 `sol_hop_pool_id`；服务会从链上校验第二跳池同时包含 WSOL 与目标 Quote。

受保护的 `POST /v1/pools/sol-hop` 接受 `{"quote_mint":"..."}`，在此开关开启时查询 Raydium CPMM 的两个 mint 槽位，返回链上验证过的 `WSOL/Quote` 池；无可用池返回 `available:false`。TG 快速交易只在找到该池时提供 SOL 两跳选项，提交前会重新查询。此接口不是成交报价，不能据此展示预计到手数量或价格影响；当前不自动搜索 AMM v4 或三跳路径。

`STONKFUN_VIA_SOL_MAX_BUY_SOL`

Via-SOL 单笔买入上限，单位 SOL，默认 `0.1`。该限制只收紧 Via-SOL，不改变其他交易路径。

自定义 Quote 直接交易时，买入请求传 `amount_quote_units`（Quote 最小单位），钱包须预先持有该 Quote。受内部鉴权保护的 `GET /internal/wallets/:wallet_id/quote-balance/:mint?owner_type=copy_trading&owner_id=<用户ID>` 返回链上 Mint 精度、Quote 最小单位余额及 SOL lamports 余额；钱包必须属于该用户或为可见的 system 钱包。该接口供 Telegram 面板显示余额和精确换算金额使用。

普通 SOL 买入不要求用户持有池子的 Quote。买入请求使用 `amount_sol` 且链上 Quote 不是 WSOL 时，trade_service 会自动走 SOL 聚合器：重新报价、审计、模拟，通过后再签名广播。Telegram 面板只显示 SOL 金额；`amount_quote_units` 仍保留给明确要求 Quote 直买的兼容调用方。

`ENABLE_SIMULATE`

是否只模拟交易。

- `true`：先用于测试，不发真实交易。
- `false`：真实提交交易。

建议第一轮测试先设为：

```env
ENABLE_SIMULATE=true
```

`WAIT_TX_CONFIRMED`

是否等待交易确认。默认 `false`，拿到提交结果后尽快返回。

`TRADE_RESPONSE_WAIT_MS`

创建交易接口等待后台执行结果的时间，单位毫秒。默认 `3000`。在这个窗口内如果交易执行已返回模拟成功、提交成功、失败或 unknown，`POST /v1/trades` 会直接带着 `signature/error/status` 返回；超时才返回 `accepted`，再用查询接口查看后续状态。

`MEV_SUBMIT_VERIFY_MS`

MEV/SWQoS lane 返回提交成功后，服务继续用 RPC 短暂查询签名是否可见，单位毫秒。默认 `1200`。如果签名在这个窗口内仍然查不到，服务会认为该 MEV lane 没有可靠落地，并继续回退普通 RPC lane。设为 `0` 可关闭这个校验。

`ENABLE_PUMPAPI_EXECUTOR`

是否允许请求使用 `execution_method:"pumpapi"`。默认 `false`。

第一版只接入 PumpAPI Local Transaction：

- 服务把交易参数发给 PumpAPI。
- PumpAPI 返回 unsigned transaction。
- 服务使用本地 `.env` 里的钱包私钥签名。
- `simulate:true` 时只模拟，不提交。
- `simulate:false` 时通过本服务配置的 Default RPC 提交。

启用示例：

```env
ENABLE_PUMPAPI_EXECUTOR=true
PUMPAPI_URL=https://api.pumpapi.io
PUMPAPI_TIMEOUT_MS=1200
PUMPAPI_DEFAULT_PRIORITY_FEE_SOL=0.0001
PUMPAPI_LIGHTNING_ENABLED=false
```

`PUMPAPI_LIGHTNING_ENABLED`

预留开关，当前必须保持 `false`。本服务不支持也不会调用需要 `privateKey` 的 PumpAPI Lightning 交易模式。

`BLOCKHASH_REFRESH_MS`

blockhash 刷新间隔，默认 `400` 毫秒。

`BLOCKHASH_MAX_AGE_MS`

blockhash 最大可用年龄。超过这个时间，交易请求会被拒绝，避免使用过期 blockhash。

## MEV / SWQoS 配置

当前已支持通过 `sol-trade-sdk 6.0.2` 接入以下 SWQoS lane：

- Jito：高性能 block space。
- Temporal：时间敏感交易。
- FlashBlock：需要 API key 的高速交易执行。
- BlockRazor：需要 API key 的高速交易执行。
- Astralane：Binary/Plain HTTP 和 QUIC 网络加速。
- SpeedLanding：需要 API key 的高速交易执行。

Default RPC fallback 会一直保留。即使启用了 MEV/SWQoS provider，服务仍会保留普通 RPC 提交通道作为兜底。

发送策略：

- 每笔交易优先使用第一个启用的 MEV/SWQoS lane。
- 如果该 lane 明确失败，再回退到普通 RPC lane。
- 不会在同一笔请求里同时向多个 MEV provider 盲目并发提交，避免 SDK 报 `Multiple swqos transactions require durable_nonce to be set`。
- 默认 `WAIT_TX_CONFIRMED=false` 时只等待提交结果，接口可能先返回 `submitted`，再用交易签名到链上查询确认。

### 总开关

```env
ENABLE_MEV_PROTECTION=true
```

这个开关会传给 SDK 的：

```rust
TradeConfig::builder(...).mev_protection(true)
```

如果只打开这个开关但没有启用任何 provider，实际仍主要走 Default RPC fallback。

### Jito

```env
ENABLE_JITO=true
JITO_UUID=
JITO_REGION=frankfurt
JITO_URL=
```

Jito 可以无 UUID 使用公共 Block Engine endpoint；留空时 SDK 不会带 `uuid` query，也不会发送 `x-jito-auth` header。无 UUID 通常会受更严格限速/优先级影响。有 UUID 时填 `JITO_UUID=你的jito uuid`；也可以用 `JITO_AUTH_TOKEN` 或 `JITO_API_KEY`，服务会按 `AUTH_TOKEN/API_KEY/UUID` 顺序读取。

### Temporal

```env
ENABLE_TEMPORAL=true
TEMPORAL_API_KEY=你的temporal api key
TEMPORAL_REGION=frankfurt
TEMPORAL_URL=
```

### FlashBlock

```env
ENABLE_FLASHBLOCK=true
FLASHBLOCK_API_KEY=你的flashblock api key
FLASHBLOCK_REGION=frankfurt
FLASHBLOCK_URL=
```

### BlockRazor

```env
ENABLE_BLOCKRAZOR=true
BLOCKRAZOR_API_KEY=你的blockrazor api key
BLOCKRAZOR_REGION=frankfurt
BLOCKRAZOR_URL=
BLOCKRAZOR_TRANSPORT=grpc
```

`BLOCKRAZOR_TRANSPORT` 支持：

- `grpc`
- `http`

### Astralane

```env
ENABLE_ASTRALANE=true
ASTRALANE_API_KEY=你的astralane api key
ASTRALANE_REGION=frankfurt
ASTRALANE_URL=
ASTRALANE_TRANSPORT=binary
```

`ASTRALANE_TRANSPORT` 支持：

- `binary`
- `plain`
- `quic`

如果日志出现 `SWQOS Astralane 初始化失败: Invalid server address`，或 VPS/机房 UDP QUIC 不稳定，优先改用：

```env
ASTRALANE_TRANSPORT=binary
```

### SpeedLanding

```env
ENABLE_SPEEDLANDING=true
SPEEDLANDING_API_KEY=你的speedlanding api key
SPEEDLANDING_REGION=frankfurt
SPEEDLANDING_URL=
```

### Region

`*_REGION` 支持：

- `default`
- `newyork`
- `frankfurt`
- `amsterdam`
- `dublin`
- `slc`
- `tokyo`
- `singapore`
- `london`
- `losangeles`

也支持常见简写，例如 `nyc`、`fra`、`ams`、`sg`、`lax`。

### 小费

小费用 SOL 单位配置：

```env
DEFAULT_TIP_SOL=0.001
```

单笔交易也可以传：

```json
"tip_sol": 0.001
```

对于 Jito / SpeedLanding / Lightspeed 等有最低 tip 要求的 lane，需要按 provider 要求设置足够小费。当前本服务支持 Jito、Temporal、FlashBlock、BlockRazor、Astralane、SpeedLanding。

每笔交易可以覆盖 MEV 策略：

```json
"use_mev": true,
"mev_provider": "astralane"
```

- 不传 `use_mev` 和 `mev_provider`：使用 `.env` 默认策略。
- `use_mev:true`：只走启用的 MEV/SWQoS lane，失败就失败。
- `use_mev:false`：只走普通 Default RPC。
- `mev_provider:"astralane"`：这一笔指定 Astralane；也支持 `jito`、`temporal`、`flashblock`、`blockrazor`、`speedlanding`。
- `mev_provider:"default"` 或 `"rpc"`：这一笔走普通 RPC。
- `mev_provider:"auto"`：优先用启用的 MEV lane，没有 MEV lane 时走 Default RPC。
- `simulate:true`：这一笔只模拟，不真实提交；适合测试新池子和新路由。

MEV 测试建议先使用一个 provider，例如：

```env
ENABLE_MEV_PROTECTION=true
ENABLE_ASTRALANE=true
ASTRALANE_REGION=frankfurt
ASTRALANE_TRANSPORT=quic
DEFAULT_TIP_SOL=0.001
WAIT_TX_CONFIRMED=false
ENABLE_SIMULATE=false
```

## 交易方式

请求字段 `execution_method` 可选：

- 不传或传 `"sdk"`：走 `sol-trade-sdk`，支持本服务现有的 MEV/SWQoS lane。
- 传 `"pumpapi"`：走 PumpAPI Local Transaction，由 PumpAPI 构造 unsigned transaction，本服务本地签名。

PumpAPI 第一版边界：

- 不支持 `use_mev:true`，也不走 Astralane/Jito 等 SWQoS lane。
- `simulate:false` 时通过 Default RPC 提交。
- 买入支持 WSOL/USDC/USD1 及自定义 Quote。WSOL 可直接使用 SDK；非 WSOL Quote 使用 `amount_sol` 时自动走 SOL 聚合器，明确传 `amount_quote_units` 时仍走 Quote 直买。
- 卖出第一版只支持 `sell_percent:100`。
- 如果 PumpAPI 返回多笔 transaction bundle，第一版会拒绝，避免半提交。
- 不会发送私钥给 PumpAPI。

PumpAPI Local Transaction 测试建议先开模拟：

```env
ENABLE_PUMPAPI_EXECUTOR=true
ENABLE_SIMULATE=true
```

PumpAPI PumpSwap 买入示例：

```json
{
  "execution_method": "pumpapi",
  "side": "buy",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.0001,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-pumpapi-buy-001"
}
```

PumpAPI Meteora USDC 买入示例：

```json
{
  "execution_method": "pumpapi",
  "side": "buy",
  "pool": "meteora-damm-v2",
  "mint": "TOKEN_MINT",
  "pool_id": "METEORA_DAMM_V2_POOL_ADDRESS",
  "quote_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
  "amount_usdc": 1,
  "slippage_percent": 15,
  "tip_sol": 0.0001,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-pumpapi-meteora-usdc-001"
}
```

PumpAPI 全仓卖出示例：

```json
{
  "execution_method": "pumpapi",
  "side": "sell",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "sell_percent": 100,
  "slippage_percent": 15,
  "tip_sol": 0.0001,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-pumpapi-sell-001"
}
```

PumpAPI curl 模拟买入：

```bash
cd /root/http/sol-bot/trade_service
set -a
source .env
set +a

API=http://127.0.0.1:18787
QUOTE=So11111111111111111111111111111111111111112
MINT=TOKEN_MINT
POOL_ID=PUMPSWAP_POOL_ADDRESS

curl -sS -X POST "$API/v1/trades" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"execution_method\":\"pumpapi\",
    \"side\":\"buy\",
    \"pool\":\"pump-amm\",
    \"mint\":\"$MINT\",
    \"pool_id\":\"$POOL_ID\",
    \"quote_mint\":\"$QUOTE\",
    \"amount_sol\":0.001,
    \"slippage_percent\":15,
    \"tip_sol\":0.0001,
    \"use_mev\":false,
    \"simulate\":true,
    \"idempotency_key\":\"pumpapi-sim-buy-$(date +%s)\"
  }"
echo
```

## 编译

```bash
cd /root/solana/trade_service/candidate
scripts/verify_candidate_gates.sh
```

该命令会执行格式检查、锁定依赖检查、全量测试、release 构建、simulate-only 配置校验、
产物 hash 校验、`current` 未切换校验和 candidate systemd 未启动校验。

## 启动

candidate unit 仅用于 staging 静态校验，验收流程不会启动它。当前路线保持
`TRADE_SERVICE_SIMULATE_ONLY=true`，不切换 `current`。

## 健康检查

```bash
curl http://127.0.0.1:18787/health
```

预期：

```json
{"ok":true}
```

## 就绪检查

```bash
curl http://127.0.0.1:18787/ready
```

返回示例：

```json
{
  "ready": true,
  "wallet": "...",
  "default_wallet_id": "main",
  "wallet_count": 2,
  "wallets": [
    {
      "wallet_id": "main",
      "pubkey": "..."
    },
    {
      "wallet_id": "wallet_2",
      "pubkey": "..."
    }
  ],
  "has_blockhash": true,
  "blockhash_age_ms": 123,
  "rpc_ok": true,
  "paused": false,
  "rpc_lanes": 3,
  "swqos_lanes": ["Default"],
  "active_swqos_lanes": ["Default"],
  "mev_protection": false,
  "require_mev_submit": false,
  "pumpapi_executor": false
}
```

默认只会看到 `["Default"]`，表示当前只启用了普通 RPC fallback。启用 Jito/Temporal/FlashBlock/BlockRazor/Astralane/SpeedLanding 并重启服务后，`swqos_lanes` 才会出现对应名称。

如果 `.env` 设置了 `REQUIRE_MEV_SUBMIT=true`，实际下单只会使用 `active_swqos_lanes` 里的 MEV/SWQoS 通道；`swqos_lanes` 里即使仍显示 `Default`，也只是配置清单，不代表下单会走普通 RPC fallback。默认建议只开启 Astralane，其他 provider 有最低 tip 要求时先保持关闭。

`MEV_USE_BACKUP_RPCS=false` 时，MEV/SWQoS 发单只用主 `RPC_URL` 构造一条执行 lane，备份 RPC 仍保留给 blockhash/RPC 能力；这样能避免某条备份 RPC 返回 403 时干扰 Astralane 排障。设为 `true` 后，每个已启用的 MEV Provider 都会基于 `RPC_URL`、`BACKUP_RPC_URLS` 依次建立执行 lane；主 RPC 在读取 Pool、Mint 或构建交易时遇到 429、超时等错误，会使用相同 Provider 和下一条 RPC 重试。生产环境应确保所有备用 RPC 都允许当前 SDK 使用的 `getProgramAccounts`、`getMultipleAccounts` 和交易模拟请求。

```json
{
  "swqos_lanes": ["Astralane", "Default"],
  "active_swqos_lanes": ["Astralane"],
  "mev_protection": true,
  "require_mev_submit": true
}
```

`/ready` 只显示 lane 名称，不显示 UUID、API key、自定义 URL 等敏感参数。

## 钱包列表

```bash
curl -sS http://127.0.0.1:18787/v1/wallets \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN"
echo
```

返回示例：

```json
{
  "default_wallet_id": "main",
  "wallets": [
    {
      "wallet_id": "main",
      "pubkey": "..."
    }
  ]
}
```

## 最近交易

```bash
curl -sS "http://127.0.0.1:18787/v1/trades?limit=20" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN"
echo
```

`POST /v1/trades` 默认会短暂等待交易执行返回结果。如果仍返回 `accepted`，用上面的最近交易接口，或按 request_id 查询：

```bash
curl -sS http://127.0.0.1:18787/v1/trades/你的request_id
echo
```

## 预热参数

PumpFun:

```bash
export LOCAL_TRADE_API_TOKEN=你的token
export MINT=真实TOKEN_MINT

curl -X POST http://127.0.0.1:18787/v1/cache/warmup \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"pool\":\"pump\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\"
  }"
```

PumpSwap:

```bash
curl -X POST http://127.0.0.1:18787/v1/cache/warmup \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"pool\":\"pump-amm\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\"
  }"
```

如果有 PumpSwap pool 地址，优先传 `pool_id`。

## 交易 JSON 字段

`execution_method` 可选：

- `"sdk"`：默认值，使用 `sol-trade-sdk`。
- `"pumpapi"`：使用 PumpAPI Local Transaction。

PumpFun 买入最小示例：

```json
{
  "side": "buy",
  "pool": "pump",
  "mint": "TOKEN_MINT",
  "pool_id": null,
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "use_mev": true,
  "mev_provider": "astralane",
  "idempotency_key": "manual-buy-astralane-001"
}
```

PumpSwap 买入建议传池地址：

```json
{
  "side": "buy",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "use_mev": true,
  "mev_provider": "astralane",
  "idempotency_key": "manual-buy-pumpswap-001"
}
```

同一个接口也可以临时关闭 MEV，走普通 RPC：

```json
{
  "side": "buy",
  "pool": "pump-amm",
  "mint": "TOKEN_MINT",
  "pool_id": "PUMPSWAP_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0,
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "manual-buy-rpc-001"
}
```

`pool_id` 可以为 `null`，服务会尝试通过 SDK RPC helper 从 mint 查池。但如果策略进程已经从 PumpAPI 事件拿到了 pool 地址，优先传 `pool_id`，可以减少查找失败和额外 RPC。

`quote_mint` 必须按真实池子填写，优先使用 PumpAPI 事件里的 `quote_mint`：

- WSOL/SOL 池：`So11111111111111111111111111111111111111112`，买入金额用 `amount_sol` 或 `amount_lamports`。
- USDC 池：`EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`，买入金额用 `amount_quote`、`amount_usdc` 或 `amount_quote_units`。
- USD1 池：`USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB`，当前只支持 `bonk`，买入金额用 `amount_quote` 或 `amount_quote_units`。

`amount_quote_units` 是 quote token 的最小单位。USDC/USD1 是 6 位小数，所以 `1 USDC = 1000000`。

Bonk 买入：

```json
{
  "side": "buy",
  "pool": "bonk",
  "mint": "TOKEN_MINT",
  "pool_id": null,
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-bonk-001"
}
```

Meteora DAMM v2 买入：

```json
{
  "side": "buy",
  "pool": "meteora-damm-v2",
  "mint": "TOKEN_MINT",
  "pool_id": "METEORA_DAMM_V2_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-meteora-001"
}
```

Meteora DAMM v2 的部分池子启用了 Rate Limiter，swap 需要携带 `SYSVAR_INSTRUCTIONS`。服务会自动为 `meteora-damm-v2` 请求补这个账号。

Meteora DAMM v2 的 USDC 池买入示例：

```json
{
  "side": "buy",
  "pool": "meteora-damm-v2",
  "mint": "TOKEN_MINT",
  "pool_id": "METEORA_DAMM_V2_POOL_ADDRESS",
  "quote_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
  "amount_usdc": 1,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "simulate": true,
  "idempotency_key": "manual-buy-meteora-usdc-001"
}
```

Raydium AMM v4 买入：

```json
{
  "side": "buy",
  "pool": "raydium-amm-v4",
  "mint": "TOKEN_MINT",
  "pool_id": "RAYDIUM_AMM_V4_AMM_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-raydium-amm-v4-001"
}
```

Raydium CPMM 买入：

```json
{
  "side": "buy",
  "pool": "raydium-cpmm",
  "mint": "TOKEN_MINT",
  "pool_id": "RAYDIUM_CPMM_POOL_ADDRESS",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_percent": 15,
  "tip_sol": 0.001,
  "idempotency_key": "manual-buy-raydium-cpmm-001"
}
```

SDK 第一版池地址规则：

- `pump`：`pool_id` 必须为 `null`。
- `pump-amm`：`pool_id` 可选；有池地址时建议传。
- `bonk`：`pool_id` 必须为 `null`，服务用 `mint` 补参数。
- `stonkfun`：`pool_id` 必填，服务校验 StonkFun 配置、base mint 与 quote mint。
- `stonkfun-swap`：`pool_id` 必填，表示 StonkFun 迁移后的 Raydium CPMM 池。
- `meteora-damm-v2`：`pool_id` 必填。
- `meteora-dbc`：Meteora Dynamic Bonding Curve，`pool_id` 必填；当前支持未迁移池的 `swap2` ExactIn，报价币支持 WSOL、USDC。
- `raydium-amm-v4`：`pool_id` 必填，表示 AMM address。
- `raydium-cpmm`：`pool_id` 必填。

PumpAPI 交易方式不会在本服务校验层强制 `pool_id`，但如果策略进程已经拿到池地址，仍建议传 `pool_id`，可以减少 PumpAPI 选路歧义。

## 买入测试

建议先确认 `.env`：

```env
ENABLE_SIMULATE=true
```

然后发请求：

```bash
curl -X POST http://127.0.0.1:18787/v1/trades \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"side\":\"buy\",
    \"wallet_id\":\"main\",
    \"pool\":\"pump\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\",
    \"amount_sol\":0.001,
    \"slippage_percent\":15,
    \"tip_sol\":0.001,
    \"source\":\"manual\",
    \"idempotency_key\":\"manual-buy-test-001\"
  }"
```

返回会包含 `request_id`。

不传 `wallet_id` 时使用 `DEFAULT_WALLET_ID`。传 `wallet_id` 时，服务会使用对应钱包交易。

## 多钱包批量买入

```bash
curl -X POST http://127.0.0.1:18787/v1/batch/trades \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"wallet_ids\":[\"main\",\"wallet_2\"],
    \"trade\":{
      \"side\":\"buy\",
      \"pool\":\"pump\",
      \"mint\":\"$MINT\",
      \"pool_id\":null,
      \"quote_mint\":\"So11111111111111111111111111111111111111112\",
      \"amount_sol\":0.001,
      \"slippage_percent\":15,
      \"tip_sol\":0.001,
      \"source\":\"manual-batch\",
      \"idempotency_key\":\"manual-batch-buy-001\"
    }
  }"
```

批量交易会自动把幂等作用域变成：

```text
wallet_id:idempotency_key
```

所以同一个策略事件可以让多个钱包各买一次，但同一个钱包不会重复提交。

## 查询交易状态

```bash
curl http://127.0.0.1:18787/v1/trades/tr_xxx
```

把 `tr_xxx` 替换为买入请求返回的 `request_id`。

状态可能是：

- `accepted`
- `building_tx`
- `submitting`
- `simulated`
- `submitted`
- `confirmed`
- `failed`
- `rejected`
- `unknown`

## 幂等测试

重复发送相同的 `idempotency_key`：

```json
"idempotency_key":"manual-buy-test-001"
```

预期返回：

```json
"duplicate": true
```

并且不会再次提交交易。

## 卖出测试

```bash
curl -X POST http://127.0.0.1:18787/v1/trades \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOCAL_TRADE_API_TOKEN" \
  -d "{
    \"side\":\"sell\",
    \"wallet_id\":\"main\",
    \"pool\":\"pump\",
    \"mint\":\"$MINT\",
    \"pool_id\":null,
    \"quote_mint\":\"So11111111111111111111111111111111111111112\",
    \"sell_percent\":100,
    \"slippage_percent\":15,
    \"tip_sol\":0.001,
    \"source\":\"manual\",
    \"idempotency_key\":\"manual-sell-test-001\"
  }"
```

`sell_percent` 范围是 `1..=100`。

## 基础校验

第一版不做最大买入金额风控，只保留执行保护：

- 修改类接口必须带 Bearer token。
- 不允许请求体传私钥。
- `execution_method` 支持 `sdk` 和 `pumpapi`，默认 `sdk`。
- `pool` 支持 `pump`、`pump-amm`、`bonk`、`stonkfun`、`stonkfun-swap`、`meteora-damm-v2`、`meteora-dbc`、`raydium-amm-v4`、`raydium-cpmm`。
- Meteora/Raydium 第一版必须传 `pool_id`，不自动按 mint 选池。
- `quote_mint` 会按链上真实池记录校验；WSOL、USDC、USD1 和自定义 SPL Quote 均可通过 SOL 聚合器买入。
- 买入统一建议使用 `amount_sol`。WSOL 直池由 SDK 执行；非 WSOL Quote 由聚合器从 SOL 兑换到目标 Mint。只有明确需要池内 Quote 直买时才传 `amount_quote_units`，此时钱包必须持有该 Quote。
- sell 的 `sell_percent` 必须是 `1..=100`。
- `execution_method:"pumpapi"` 第一版不支持 `use_mev:true`，卖出只支持 `sell_percent:100`。
- `slippage_percent` 必须是 `0..=100`。旧字段 `slippage_bps` 仍兼容。
- SDK 交易方式需要 blockhash cache 可用且未过期；PumpAPI 交易方式使用 PumpAPI 返回交易里的 recent blockhash。

## SDK 说明

当前使用：

```toml
sol-trade-sdk = "6.0.2"
```

SDK 参数通过 RPC helper 补齐：

- `PumpFunParams::from_mint_by_rpc`
- `PumpSwapParams::from_pool_address_by_rpc`
- `PumpSwapParams::from_mint_by_rpc`
- `BonkParams::from_mint_by_rpc`
- `MeteoraDammV2Params::from_pool_address_by_rpc`
- `RaydiumAmmV4Params::from_amm_address_by_rpc`
- `RaydiumCpmmParams::from_pool_address_by_rpc`

SWQoS / MEV lane 通过 SDK 的 `SwqosConfig` 构造：

- `SwqosConfig::Jito`
- `SwqosConfig::Temporal`
- `SwqosConfig::FlashBlock`
- `SwqosConfig::BlockRazor`
- `SwqosConfig::Astralane`
- `SwqosConfig::Speedlanding`
- `SwqosConfig::Default`

PumpAPI 交易方式不经过这些 SDK DEX helper；它调用 `PUMPAPI_URL` 获取 unsigned transaction，再由本服务本地签名，并使用 Default RPC 进行模拟或提交。
