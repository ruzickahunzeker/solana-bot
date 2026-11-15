# Trade Service 外部服务兼容边界

日期：2026-11-14。用途：将专用优化计划与三服务总计划已有约束集中为消费者验收清单。本文不新增 API 字段、ZMQ schema、跨服务数据库写入或 PumpAPI 功能，也不代表跨服务端到端验收完成。

## 职责与两种 PumpAPI 接口

```text
PumpAPI stream -> pumpapi_zmq_bridge -> copy_trading -> Trade Service -> chain
                  ingress/hints       strategy/UI     execution authority
```

- Bridge 负责数据接入/规范化与可观测性；不因此取得钱包、策略执行或签名权限。
- Copy Trading 负责 Telegram、策略/限价判断、本地订单意图及远端状态关联；通过 HTTP 对接 Trade Service，不引用其 SDK Rust 类型。
- Trade Service 负责执行准入、钱包身份、协议/资产核对、签名、发送和原签名恢复；内部 SDK 升级不能把这些规则转嫁给 Bot。
- Trade Service 内的 PumpAPI Local Transaction backend 是另一条交易构建来源，不是 ZMQ Bridge。其开发线仍暂停；普通 SDK 的 Pump.fun/PumpSwap 也不因名称相似而归入这条暂停线。
- 现有钱包/数据库兼容路径保留；不以共享数据库表作为新增跨服务 API，不在本轮迁移或删除 Copy Trading 钱包数据。

## Copy Trading HTTP 合同

依据当前 `copy_trading/src/trade_client/mod.rs` 与 Trade Service `src/api/routes.rs`，以下兼容要求已纳入现有计划，不新设平行执行 API：

| 边界 | 必须保持的语义 | 验收依据/未完成项 |
| --- | --- | --- |
| `/v1/trades` 与查询 | 字段/别名/默认值/可空行为/原状态码不因移动 Handler 改变；原钱包/字面幂等键与参数绑定 | V1 fixtures、HTTP/重启合同已有局部证据；部署后的调用方验收待续 |
| 金额与 Quote | 保留已有 `amount_sol`、`amount_quote_units`、卖出比例等互斥语义；边界内部转整数，不能把 Quote 数量当 SOL | 原 hash/JSON、精度及付款分支回归；显示简称不替代实际 Quote Mint 验证 |
| 钱包 | `wallet_id`、标签、公钥分别处理；标签修改不偷偷换执行身份，不把缺省钱包规则改成旧缓存选择 | 钱包身份/管理合同已有，远端实际 UI 与管理鉴权迁移仍需验收 |
| `/ready` | 保留持久化模式、数据库健康与可用性字段；健康不等于 Live 交易授权 | 现有 readiness 合同；不能让旧/缺字段响应授权自动恢复 |
| `/v1/capabilities`、Pool 与 warmup | 保留当前 schema/旧能力含义；候选池、缓存命中和 SDK builder 不自动升级为实盘准入 | Registry/Pool fixture；全方向/Token 扩展及真实链上证据待续 |
| SOL 聚合 Quote/preflight/submit | 保留报价绑定、期限、钱包、明确确认和幂等语义；不以普通交易或另一个 quote authority 代替本操作账本 | 独立与普通聚合合同分别验收；不宣称所有模式/backend 都有相同保障 |
| `/internal/*` 与钱包管理 | 内部接口不因远程 Bot 需要而取消 loopback；管理凭据不作为交易凭据扩散 | 现有 Copy 客户端仍是单 token 模型，启用 separate 前须另行迁移调用方/可信本机入口，不在本轮自动启用 |

超时、HTTP 取消、503、signature 未可见或 unknown 不证明未提交。Copy Trading 保留同一业务动作的原请求、幂等键和远端 request_id；允许的恢复必须满足既有 durable/healthy/ready 条件且保持原 key/参数，不能以新 key/新路线补单。SDK 已进入组合执行后不能把只读节点切换套到重建/重签交易。

## 事件与订单关联

| 身份 | 所属服务 | 规则 |
| --- | --- | --- |
| `event_id` / source event identity | Bridge/Copy | 目标须区分同一 signature 内多事件；现有缺少 index 的实现仍为缺口，不宣称无损去重 |
| `source_signature` | Copy -> Trade | 当前已有请求字段，表示被观察的源交易，不是本钱包执行签名 |
| 本地 `order_id` / strategy identity | Copy | 本地保存事件、策略、业务动作与远端请求关联，不要求 Trade 新增本地数据库字段 |
| `(wallet_id, idempotency_key)` / `request_id` | Trade | 原业务保留与远端订单身份；优化不改变旧 hash/JSON，也不重新发放执行 owner |
| 执行 `signature` | Trade/chain | 用于原交易状态查询；不能与 source_signature 混用；多签名 SDK 仍须单独补全证据 |

关联链是各自权威之间的映射要求，不是增加必填字段或重新编码旧幂等键的许可。Bridge 的 pool/Quote/协议标识是候选输入，Trade 最终按实际 backend/协议/账户/资产/金额证据判断，不信任一条事件的 `route_status=executable` 即放行。

## 未来修改的验收顺序

1. 先冻结当前消费者请求/响应、金额/幂等/错误、钱包与能力 fixture；内部重构不得强迫 Copy 使用某个 SDK 版本。
2. 新增字段或枚举另做旧消费者缺字段/未知值检查；不偷偷改变 v1 schema 或把 observe 改成 Live。
3. Bridge 后续 schema/游标/缺口告警/回放另开兼容工作包，保留 v1；暂停期间仅保留要求和既有代码测试，不实现新事件协议或交易接入。
4. SDK/Router 差异留在 adapter 和能力映射；最终消息授权、广播前 journal 与恢复合同不能被新 backend 绕过。
5. 三服务独立构建/发布/回滚；生产钱包恢复、调用方兼容与协议模拟分别留证，不把单服务离线测试当成端到端成功。

当前待完成的主要合同仍是：跨服务多事件身份、可靠交付/恢复、管理凭据迁移、生产兼容，以及完整通用 backend/授权/签名账本。上述边界已预留，不代表这些功能已经实现。
