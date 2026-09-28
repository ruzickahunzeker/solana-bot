# PumpAPI、sol-trade-sdk 5.0.5 与 StonkFun Via-SOL 总体修复升级方案

## 1. 文档目的

本方案用于一次性协调以下三个独立服务的兼容升级：

- `pumpapi_zmq_bridge`：只接收、规范化并转发 PumpAPI 数据流。
- `trade_service`：唯一持有签名钱包、构造交易、签名、提交和持久化交易的服务。
- `copy_trading`：Telegram UI、跟单策略、限价单和交易编排服务。

本次升级的直接问题是：Telegram 对 StonkFun 自定义报价池使用 `amount_sol` 下单，
`trade_service` 正确返回 `custom quote token requires amount_quote_units`。目标不是绕过校验，
而是引入 SDK 5.0.5 的 `StonkFunViaSol`，在明确满足条件时执行原子两跳：

```text
买入：SOL -> 股票报价 Token -> Meme Token
卖出：Meme Token -> 股票报价 Token -> SOL
```

## 2. 当前基线

| 项目 | 当前状态 |
|---|---|
| PumpAPI Stream | `wss://stream.pumpapi.io/` |
| 本机数据分发 | ZeroMQ PUB，`tcp://127.0.0.1:8939` |
| `sol-trade-sdk` | vendored `5.0.4` |
| SDK 本地扩展 | Meteora DBC、Solana 4.x 兼容、早期 StonkFun 支持 |
| Trade API | `http://127.0.0.1:18787`，Public API 保持 Bearer + IP 白名单 |
| 钱包归属 | `trade_service` 是私钥密文和运行时签名钱包的唯一管理者 |
| Telegram | 只使用 `wallet_id` 调用 Trade API，不持有可解密私钥 |

已确认失败样例：

```text
request_id: tr_0918c1dfaa8146379be7907ad0f11d68
pool: stonkfun-swap
mint: 2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL
quote_mint: Xsf9mBktVB9BSU5kf4nHxPq5hCBJ2j2ui3ecFGxPRGc
request amount: 0.001 SOL
result: rejected before chain submission
```

该失败没有产生链上交易，也不属于 RPC、MEV、滑点或钱包故障。

## 3. 强制边界

### 3.1 本次允许修改

- PumpAPI 新旧字段兼容和安全规范化。
- `sol-trade-sdk` 从 5.0.4 合并升级至 5.0.5。
- 将本地 Meteora DBC 扩展移植到 5.0.5 基线。
- `trade_service` 增加 StonkFun Via-SOL 路由解析和执行。
- Trade API 增加可选、向后兼容的金额和结算字段。
- Telegram 手动交易、跟单和限价单统一金额构造与前置拦截。
- 必要的缓存表、迁移、测试、文档和运行指标。

### 3.2 本次明确不做

- 不改用 PumpAPI 托管签名或 Lightning Trade API。
- 不向 PumpAPI 或任何第三方发送私钥。
- 不把通用 SOL/Token 转账加入 Public Trade API。
- 不取消 Bearer Token、IP 白名单、幂等键、readiness 或钱包公钥绑定检查。
- 不把 `amount_sol` 直接按 decimals 换算成自定义报价 Token。
- 不把未知或缺失的 `quoteMint` 默认视为 WSOL。
- 不因为 PumpAPI 已提供事件就自动启用尚未验证的交易协议。
- 不在本次启用 Meteora DAMM V1、Meteora DLMM、Raydium CLMM 或 Whirlpool 交易。
- 不重写现有 Telegram UI；只增加必要路径、提示和字段展示。
- 不改变既有钱包 ID、标签语义或默认钱包选择规则。
- 不删除现有数据库，不批量重放历史订单，不自动重试已 rejected 的订单。

### 3.3 服务职责边界

| 能力 | PumpAPI Bridge | copy_trading | trade_service |
|---|---:|---:|---:|
| 接收链上数据流 | 是 | 否 | 否 |
| 事件字段规范化 | 是 | 兼容解析 | 否 |
| 策略与 UI | 否 | 是 | 否 |
| 路由候选缓存 | 发布观测值 | 保存 UI/策略路由 | 保存可执行路由 |
| 私钥解密与签名 | 否 | 否 | 是 |
| 两跳交易构造 | 否 | 否 | 是 |
| 交易幂等与最终状态 | 否 | 保存关联 | 权威状态 |

## 4. 协议支持矩阵

| 协议 | PumpAPI 观测 | Trade API 单跳 | Via-SOL | 本次状态 |
|---|---:|---:|---:|---|
| PumpFun | 是 | 是 | 不适用 | 回归验证 |
| PumpSwap | 是 | 是 | 不适用 | 回归验证 |
| Raydium Launchpad/Bonk | 是 | 是 | 否 | 回归验证 |
| StonkFun 内盘 | 是 | 是 | 是 | 本次新增 |
| StonkFunSwap/CPMM 外盘 | 是 | 是 | 是 | 本次新增 |
| Raydium CPMM | 是 | 是 | 作为 SOL hop | 回归验证 |
| Raydium AMM V4 | 是 | 是 | 作为 SOL hop 备用 | 回归验证 |
| Meteora DAMM V2 | 是 | 是 | 否 | 回归验证 |
| Meteora DBC | 兼容识别 | 本地扩展 | 否 | 必须保留 |
| Meteora DAMM V1 | 是 | 否 | 否 | 仅透传/记录 |
| Meteora DLMM | 是 | 否 | 否 | 仅透传/记录 |

“PumpAPI 可观测”不等于“允许交易”。只有显式列入 Trade API 白名单并通过测试的协议才可执行。

## 5. PumpAPI Bridge 升级

### 5.1 连接层

保持单连接：

```text
wss://stream.pumpapi.io/
```

保留自动重连和原始 JSON ZMQ 广播。不得为每个下游服务创建独立 PumpAPI 连接。

增加运行指标或结构化日志：

- 最近成功接收时间。
- 重连次数和最近错误。
- JSON 解析失败计数。
- 未知 `pool` 计数。
- 缺少 `mint`、`poolId` 或 `quoteMint` 的交易事件计数。
- ZMQ 发布计数。

### 5.2 字段兼容

输入动作按以下顺序读取：

```text
action -> txType -> tx_type
```

规范化消息至少保留：

```text
signature
action
pool
pool_id
mint
quote_mint
tx_signer
platform
launchpad_config
pool_created_by
token_program
token_amount
quote_amount
tokens_in_pool
quote_in_pool
sol_in_pool
decimals
timestamp
programs_used
address_lookup_tables
```

### 5.3 quoteMint 安全规则

- PumpAPI 明确提供 `quoteMint`：原样使用。
- 只有经过协议规则确认的旧 SOL 事件才允许回退为 WSOL。
- StonkFun、Raydium CPMM、AMM V4、自定义 Launchpad 或未知协议缺失 `quoteMint`：
  不发布为可执行候选。
- 缺少 `quoteMint` 的事件仍可在 raw channel 广播和审计。

### 5.4 StonkFun 分类

按以下优先级识别：

1. PumpAPI 可信解析字段 `platform == "stonkfun"`。
2. 已知 StonkFun `launchpadConfig` 白名单。
3. 本地持久化的 StonkFun mint 索引。

`platformName` 不可信，不可用于授权交易。未知 Raydium Launchpad 平台保持未分类。

迁移后 `raydium-cpmm` 事件只有在 mint 已由可信 StonkFun 内盘事件确认时，才映射为
`stonkfun-swap`。`poolCreatedBy == "raydium-launchpad"` 本身不足以证明属于 StonkFun。

### 5.5 非交易事件

只有 `buy` 和 `sell` 进入跟单执行候选。

以下动作仅更新索引、Pool 或审计，不触发交易：

```text
create
migration
createPool
add
remove
createConfig
distributeFees
transfer
```

## 6. sol-trade-sdk 5.0.5 合并

### 6.1 合并原则

以 crates.io 官方 `5.0.5` 源码作为新基线，再移植本地扩展。禁止把 5.0.5 文件逐个覆盖到
现有 5.0.4 vendor 后直接上线，因为 5.0.5 包含 PumpFun V2、PumpSwap 储备、StonkFun、
Token-2022 和多个 DEX 的广泛变化。

### 6.2 必须保留的本地能力

- `MeteoraDbcParams`。
- `DexType::MeteoraDbc` 和 `DexParamEnum::MeteoraDbc`。
- Meteora DBC 指令构造和 RPC Pool 加载。
- Solana 4.x 的 RPC/UI account 兼容修改。
- 当前项目依赖的 SWQOS 通道行为。
- `METEORA_DBC_NOTICE.md` 和相关回归测试。

### 6.3 使用 5.0.5 官方能力替换的部分

- StonkFun 内盘参数和指令。
- StonkFunSwap 毕业外盘。
- `StonkFunViaSolParams`。
- `SimpleBuyParams::stonkfun_with_sol`。
- `SimpleSellParams::stonkfun_to_sol`。
- 5.0.5 的 Token-2022、transfer fee、ATA policy 和主网模拟修复。

不要继续维护与 5.0.5 官方 StonkFun 实现重复的本地代码。

### 6.4 SDK 合并验收

- Cargo 依赖、vendor manifest 和 lockfile 版本一致为 5.0.5。
- 不存在两套同名 StonkFun 类型或分支。
- Meteora DBC 编译入口仍存在。
- 不启用新的 DEX 枚举，除非 Trade API 明确完成映射和验证。
- SDK 官方测试和项目适配测试分开记录结果。

## 7. Trade API 合同

### 7.1 金额字段互斥

买入请求只允许以下字段之一：

```text
amount_sol
amount_lamports
amount_quote
amount_usdc
amount_quote_units
```

同时提供多个金额字段必须返回 `400 AMOUNT_FIELDS_CONFLICT`，不能静默选一个。

### 7.2 现有直连语义

- WSOL/SOL quote + `amount_sol`：现有单跳。
- USDC quote：使用 `amount_usdc`、`amount_quote` 或 `amount_quote_units`。
- USD1 quote：使用 `amount_quote` 或 `amount_quote_units`。
- 自定义 quote + `amount_quote_units`：使用钱包已有报价 Token 单跳交易。

### 7.3 StonkFun Via-SOL 语义

仅在全部满足时自动启用：

```text
pool in [stonkfun, stonkfun-swap]
quote_mint not in [SOL, WSOL, USDC, USD1]
buy request contains amount_sol or amount_lamports
settlement is absent or settlement == sol
可解析可信的 WSOL <-> quote_mint hop
```

此时：

- 买入调用 `stonkfun_with_sol`。
- 卖出且 `settlement == sol` 时调用 `stonkfun_to_sol`。
- API 中的 `amount_sol` 始终表示用户允许花费的原生 SOL，不表示 quote Token 数量。

以下情况不得自动 Via-SOL：

- 非 StonkFun 协议。
- quote 为 USDC 或 USD1。
- 请求明确提供 `amount_quote_units`。
- SOL hop 未找到、过期、不包含 WSOL 与目标 quote，或不在协议白名单。
- Mint/quote/pool 方向无法确定。

### 7.4 可选新字段

建议增加向后兼容字段：

```json
{
  "settlement": "sol",
  "amount_quote_units": null,
  "route_hint": {
    "sol_hop_pool": "raydium-cpmm",
    "sol_hop_pool_id": "..."
  }
}
```

`route_hint` 只能是提示。`trade_service` 必须从链上重新校验两个 mint、Pool 所有权和协议，
不能信任 Telegram 或 PumpAPI 提供的 Pool ID。

### 7.5 结构化错误

错误响应增加稳定 `error_code`，保留现有 `error` 文本：

```text
CUSTOM_QUOTE_AMOUNT_REQUIRED
SOL_HOP_NOT_FOUND
SOL_HOP_STALE
SOL_HOP_PAIR_MISMATCH
AMOUNT_FIELDS_CONFLICT
UNSUPPORTED_POOL
UNSUPPORTED_QUOTE
INSUFFICIENT_QUOTE_BALANCE
TRANSACTION_TOO_LARGE
```

HTTP 400 代表请求不可执行，copy_trading 不应自动重试。HTTP 429/502/超时按现有有限重试策略处理。

## 8. SOL Hop 路由与缓存

### 8.1 数据结构

建议在 `trade_service` 持久化可执行 hop：

```text
quote_mint
pool
pool_id
base_mint
pool_quote_mint
source
observed_at
verified_at
expires_at
```

唯一键至少包含：

```text
quote_mint + pool + pool_id
```

不得继续用“一 Mint 只保存一条路由”的模型覆盖不同 quote 或不同 Pool。

### 8.2 选择顺序

1. 已验证且未过期的 Raydium CPMM WSOL pair。
2. 已验证且未过期的 Raydium AMM V4 WSOL pair。
3. 受超时限制的 RPC 解析。
4. 全部失败则返回 `SOL_HOP_NOT_FOUND`。

选择时至少校验：

- Pool 账户属于预期程序。
- Pair 的一边为 WSOL，另一边为目标 quote mint。
- Vault、token program 和方向与链上状态一致。
- Token-2022 transfer fee 已由 SDK 支持。
- 路由年龄未超过配置上限。

PumpAPI 只能提供候选和提前预热，不能替代链上校验。

## 9. copy_trading 修复

### 9.1 请求模型

`TradeRequest` 增加可选字段：

```rust
amount_quote: Option<f64>
amount_usdc: Option<f64>
amount_quote_units: Option<u64>
settlement: Option<String>
```

所有调用必须经过统一构造函数，禁止手动交易、策略和限价单分别拼 JSON。

### 9.2 手动交易

- WSOL 路由：显示并发送 SOL 金额。
- StonkFun 自定义 quote 且 Via-SOL ready：显示两跳路径，发送 `amount_sol + settlement=sol`。
- StonkFun 自定义 quote 但 Via-SOL 未 ready：提交前拦截。
- 用户未来选择直接报价币模式：显示报价币余额并发送 `amount_quote_units`。
- 不能把 SOL 预设按钮的数值复用于 USDC、USD1 或自定义 Token。

### 9.3 自动跟单

- 固定 SOL：允许符合边界的 StonkFun Via-SOL。
- 比例买入：只有原事件可确定实际 quote 数量或可安全换算 SOL 预算时执行。
- 缺少签名、Pool ID、quote mint 或 SOL hop：记录 blocked，不提交。
- 幂等键继续使用策略 ID、源交易签名和方向。
- 400 业务拒绝不重试。

### 9.4 限价单

- 创建时保存 `quote_mint`、结算资产和金额类型。
- 触发时重新验证两个 Pool，不能盲用创建时缓存。
- 路由变化不能把 SOL 金额解释成自定义报价 Token。
- Via-SOL 未 ready 时订单保持 blocked/failed，不循环高频重试。

### 9.5 UI 最小变化

快速交易展示：

```text
Pool: StonkFunSwap
路径: SOL -> Xsf9...PRGc -> Meme
结算: SOL
```

确认页需要明确“两跳交易”。普通用户仍输入 SOL，不要求理解 raw quote units。

## 10. 滑点、手续费与账户边界

- 两跳分别进行滑点保护，并设置整体最小输出。
- 不把一个 `slippage_percent` 简单重复后宣称为整体同等滑点；UI 应说明它是路径容忍度。
- 小费和 MEV provider 仍属于整笔交易，不按 hop 重复收费。
- `AccountPolicy::Auto` 用于首版兼容验证。
- `HotPathMinimal` 仅在 WSOL、quote 和 meme ATA 已预建后用于极速跟单。
- 卖出到 SOL 时应 unwrap WSOL；股票 quote ATA 可保留复用。
- 交易体积、Compute Unit 或 ALT 不满足时拒绝，不自动拆成两个非原子交易。

## 11. 数据库迁移边界

- 所有 migration 必须向前兼容且可重复检测。
- 新列使用 nullable 或有明确默认值。
- 不重写旧订单金额含义。
- 旧订单仍按历史 request JSON 展示。
- 路由表升级使用新表或复合唯一键，不能无审计地覆盖旧表。
- 迁移前分别备份 `trades.sqlite`、`wallets.sqlite` 和 `copy_trading.sqlite`。
- 使用 SQLite online backup 或短暂停写一致性备份，不能只复制活跃 WAL 主文件。

## 12. 实施阶段

### 阶段 A：只读基线与备份

1. 记录服务版本、二进制 SHA256、systemd unit 和环境变量名称。
2. 备份源码、vendor、数据库和当前 release 二进制。
3. 执行 SQLite `quick_check`。
4. 保存当前 `/health`、`/ready` 和支持钱包列表结果。

验收：可以在不重新编译的情况下恢复当前运行版本。

### 阶段 B：PumpAPI Bridge 兼容升级

1. 新旧 action 字段兼容。
2. `platform=stonkfun` 与配置白名单双重分类。
3. 移除不安全的全局 WSOL 默认。
4. 补充规范化字段和未知事件指标。
5. 使用 fixture 回放，不立即改变交易服务。

验收：raw channel 字节透传不变；normalized channel 只增加字段或更严格拒绝不完整路由。

### 阶段 C：SDK 5.0.5 受控合并

1. 建立独立 5.0.5 vendor 基线。
2. 移植 Meteora DBC 与 Solana 4 兼容补丁。
3. 删除重复的本地 StonkFun 实现。
4. 适配 `trade_service` 编译接口。
5. 更新版本和 lockfile。

验收：旧协议单元测试和 DBC 测试通过，尚不启动 Live Via-SOL。

### 阶段 D：Trade API Via-SOL

1. 引入 SOL hop 候选缓存与链上校验。
2. 增加 StonkFun Via-SOL 构造。
3. 增加金额互斥、结构化错误和审计字段。
4. 增加 warmup/readiness 状态。

验收：模拟交易覆盖内盘、外盘、买入和卖出；错误边界均在签名前拒绝。

### 阶段 E：Telegram 接入

1. 统一 TradeRequest 金额模型。
2. 手动交易接入。
3. 跟单接入。
4. 限价单接入。
5. 增加两跳路径展示和 blocked 原因。

验收：失败样例不再产生 `amount_sol` 与自定义 quote 的错误组合。

### 阶段 F：灰度上线

1. PumpAPI Bridge 单独重启，观察 schema 和延迟。
2. 部署 `trade_service`，先保持 Via-SOL feature flag 关闭。
3. 执行模拟和 readiness 检查。
4. 仅允许一个钱包、一个 StonkFun Mint、极小金额 Live。
5. 完成买入和卖回 SOL 后再开放 Telegram 手动交易。
6. 最后开放自动跟单和限价单。

## 13. Feature Flags 与默认值

建议新增：

```env
ENABLE_STONKFUN_VIA_SOL=false
STONKFUN_VIA_SOL_MAX_BUY_SOL=0.005
STONKFUN_SOL_HOP_MAX_AGE_SECONDS=300
STONKFUN_SOL_HOP_POOLS=raydium-cpmm,raydium-amm-v4
STONKFUN_VIA_SOL_ACCOUNT_POLICY=auto
```

默认关闭。开启前必须满足：

- SDK 5.0.5 合并测试通过。
- Trade API 模拟通过。
- 数据库备份完成。
- 指定测试钱包余额和租金充足。
- PumpAPI 数据新鲜度与 RPC readiness 正常。

## 14. 测试矩阵

### 14.1 PumpAPI

- `action` 与旧 `txType`。
- StonkFun `platform` 分类。
- 已知/未知 `launchpadConfig`。
- 自定义 quote。
- 缺失 quote 不回退 WSOL。
- StonkFun migration/createPool 到 CPMM。
- DAMM V1/DLMM 只记录、不交易。
- 重连重复事件由签名去重。

### 14.2 Trade API

- Pump/WSOL 单跳买卖。
- PumpSwap 单跳买卖。
- Bonk/USD1 现有路径。
- Meteora DAMM V2。
- Meteora DBC。
- StonkFun 自定义 quote 直连 `amount_quote_units`。
- StonkFun 内盘 Via-SOL 买入/卖出。
- StonkFunSwap 外盘 Via-SOL 买入/卖出。
- SOL hop 缺失、过期、mint 不匹配。
- 金额字段冲突。
- Token-2022 transfer fee。
- ATA 首次创建与已存在两种情况。
- MEV provider 开/关。
- 相同幂等键不重复交易。

### 14.3 Telegram

- 手动买入完成后切换卖出页面。
- 卖出完成后切换买入页面。
- 开启和关闭确认交易。
- Dry-run 不调用 Trade API。
- Live Via-SOL 显示两跳路径。
- 不支持路由在提交前 blocked。
- 跟单固定 SOL、比例、跟卖。
- 限价触发时重新验证路由。

## 15. 上线闸门

以下任一项失败，不得开启 Via-SOL Live：

- SDK 或项目测试失败。
- Meteora DBC 回归失败。
- Pump/PumpSwap 基础回归失败。
- `/ready` 不健康。
- PumpAPI 心跳过期。
- 钱包公钥与 `wallet_id` 对不上。
- SOL hop 未经过链上 pair 校验。
- 模拟交易失败或交易大小超限。
- 无法确认卖出最终回到原生 SOL。
- 数据库没有一致性备份。

## 16. 回滚方案

回滚顺序：

1. 关闭 `STONKFUN_VIA_SOL_ENABLED`，不必先回滚全部服务。
2. 若 Trade API 异常，恢复旧 release 二进制和旧 vendor/source 快照。
3. 若 PumpAPI normalized schema 异常，恢复旧 Bridge；raw ZMQ 协议保持不变。
4. 若 Telegram 异常，恢复 `copy_trading` release，不回滚权威交易数据库。
5. 数据库 migration 只允许前向兼容；优先通过旧程序忽略新列回滚，不执行破坏性降级 SQL。

回滚后验证：

```text
四个 systemd unit active
Trade API /health == ok
Trade API /ready == ready
Telegram 可打开钱包和交易面板
PumpAPI ZMQ 持续有新事件
旧协议模拟交易正常
```

## 17. 编译与部署约定

按既有约定，代码修改阶段不自动执行完整编译。需要编译时打印命令，由维护者执行：

```bash
cd /root/http/sol-bot/trade_service
cargo check
cargo build --release

cd /root/http/sol-bot/copy_trading
cargo check
cargo build --release
```

编译完成后保留 release 主程序，可以清理 `target/debug` 和 release 中间依赖以回收空间。
重启顺序：

```text
1. pumpapi-zmq-bridge
2. trade-service-db，等待 /health 和 /ready
3. copy-trading-dev
4. wallet-watch-recorder（仅在其 schema 有变化时）
```

## 18. 完成定义

本次升级只有在以下全部成立时才算完成：

- PumpAPI 新字段与旧字段均可解析。
- 未知/缺失 quote 不会被错误当作 WSOL。
- SDK 已是 5.0.5，且 Meteora DBC 能力保留。
- StonkFun 自定义报价池可使用 SOL 原子买入并卖回 SOL。
- 非 StonkFun 自定义报价仍被安全拒绝。
- 手动交易、跟单和限价单使用统一金额构造。
- 失败样例通过模拟和小额 Live 回归。
- 没有破坏 Public Trade API、钱包安全、幂等或现有协议。
- 文档、配置参数和回滚步骤与实际实现一致。

## 19. 2026-09-28 实施状态

已完成：

- 升级 vendored `sol-trade-sdk` 到 5.0.5，并保留本地 Meteora DBC 支持。
- PumpAPI normalized 流补齐 action、数量、program、ALT 字段；raw ZMQ 保持不变。
- 修正 StonkFun 分类，并禁止未知自定义 Quote 缺失时回退为 WSOL。
- Trade API 增加显式 Via-SOL 请求字段、默认关闭的功能开关、单笔上限及链上第二跳 pair 校验。
- SDK 执行路径支持 StonkFun curve / graduated CPMM 的 Via-SOL 买卖。
- 幂等指纹包含结算资产和第二跳路由。
- copy_trading 请求模型预留统一字段；当前自动跟单仍安全拦截没有可信第二跳路由的自定义 Quote。
- Public Trade API 和 README 已同步配置与请求示例。

待编译后执行：

- Rust `cargo check` 与测试。
- 使用真实 WSOL/Quote CPMM 地址完成 Dry-run/模拟。
- 小额 Live 买入后原路卖回 SOL。
- 路由表增加可信第二跳来源后，再开启 Telegram 自动跟单 Via-SOL。

当前生产服务没有重启，`STONKFUN_VIA_SOL_ENABLED` 默认关闭。
