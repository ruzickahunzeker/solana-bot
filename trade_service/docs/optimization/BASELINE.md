# Trade Service 专属优化基线

日期：2026-11-12。只核对本地 Trade Service，不代表上游最新版本或实盘验收。

## 版本身份

| 项目 | 本次记录 |
| --- | --- |
| 仓库 HEAD | `00c9b7a47f08e2f66da32926744f09d1aeb36141` |
| 工作树 | 开始前已有其他服务改动、Trade Service persistence 重启测试及未跟踪目录；HEAD 不是完整工作树版本 |
| SDK manifest | `sol-trade-sdk = "6.0.2"`，`[patch.crates-io]` 指向本地 vendor |
| `Cargo.toml` SHA-256 | `dd30e76d5a065e1647c0f657d8d7254b3e8357676248af055b17f5329df3f1f8` |
| `Cargo.lock` SHA-256 | `f0f0421ca67903d630027b91e959b23a4bac8dcbe2da43e645cc51e5ecaeb535` |
| vendor manifest SHA-256 | `10862a3e951ab95e47234c7562a0f1bb63310d7d12e74d3ef1b35eddf40cd596` |
| vendor HEAD tree | `149a17f900a317e35bbce00e770c3b713aade3da`，本次无 vendor 改动 |
| 原 release 文件 SHA-256 | `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f` |
| 开始时测试 | `cargo test --locked --offline --quiet`：47 项通过 |
| 磁盘 | 开始时约 4.4 GiB 可用，不制作生产库副本，不做 release 构建 |

上游 revision 和本地 vendor 补丁来源尚未逐项证明。Git 历史存在 vendor 导入/升级提交，不等于可直接重放的上游补丁账本。

上表保留首轮基线。本轮 HTTP 合同测试仅添加 Tower 0.5.3 测试依赖及对应锁文件依赖边，未升级包版本。当前 `Cargo.toml` SHA-256 为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`，`Cargo.lock` 为 `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`。vendor manifest 与原 release 指纹不变。历史来源证据见 `SDK_PROVENANCE.md`，资金操作边界见 `SUBMISSION_BOUNDARIES.md`。

vendor 内容指纹为 `46bba5f356b729605ea03e76ebdc4ec6ebec6c83c6ba61a45a16d71ccaccabb6`，计算方式如下，需在仓库根目录执行；这是带路径的已跟踪文件清单指纹，不是上游 archive 哈希：

```bash
git ls-files trade_service/vendor/sol-trade-sdk | LC_ALL=C sort | xargs sha256sum | sha256sum
```

## HTTP 入口清单

以下来自 `src/http.rs`。进程内 HTTP 鉴权与拒绝路径已有测试（见 `HTTP_CONTRACTS.md`），仍不代表真实网络或签名成功路径验收。

| 方法 | 路径 | 当前保护/职责 |
| --- | --- | --- |
| GET | `/health` | 无 Bearer，进程健康 |
| GET | `/ready` | 普通保护，运行依赖状态 |
| GET | `/v1/capabilities` | 普通保护，静态能力响应 |
| GET / POST | `/v1/wallets` | 普通保护，列表/创建导入 |
| GET | `/v1/wallets/manage` | 普通保护，管理列表 |
| PATCH / DELETE | `/v1/wallets/:wallet_id` | 普通保护，更新/删除 |
| POST | `/v1/wallets/:wallet_id/export` | 普通保护，导出；需管理权限隔离 |
| POST | `/v1/cache/warmup` | 普通保护，预热 |
| POST | `/v1/pools/resolve` | 普通保护，寻池 |
| POST | `/v1/pools/sol-hop` | 普通保护，SOL 第一跳 |
| POST | `/v1/quotes/sol-buy` | 普通保护，聚合报价 |
| GET | `/v1/quotes/sol-buy/:quote_id` | 普通保护，绑定报价 |
| POST | `/v1/quotes/sol-buy/:quote_id/preflight` | 普通保护，不广播检查 |
| POST | `/v1/quotes/sol-buy/:quote_id/submit` | 普通保护，独立聚合提交 |
| POST / GET | `/v1/trades` | 普通保护，提交/列表 |
| POST | `/v1/batch/trades` | 普通保护，逐钱包普通交易 |
| GET | `/v1/trades/:request_id` | 普通保护，交易查询 |
| GET | `/internal/wallet-balances` | loopback 加普通保护 |
| GET | `/internal/wallets/:wallet_id/quote-balance/:mint` | loopback 加普通保护 |
| POST | `/internal/wallets/vanity-jobs` | loopback 加普通保护 |
| GET / DELETE | `/internal/wallets/vanity-jobs/:job_id` | loopback 加普通保护 |
| POST | `/internal/wallets/:wallet_id/transfer-sol` | loopback 加普通保护，独立转账流程 |
| POST | `/internal/conversions/wsol/quotes` | loopback 加普通保护 |
| POST | `/internal/conversions/wsol/simulate` | loopback 加普通保护 |
| POST | `/internal/conversions/wsol/submit` | loopback 加普通保护，独立兑换流程 |
| GET | `/internal/conversions/wsol/submissions/:submission_id` | loopback 加普通保护 |

普通保护同时检查来源 IP 和 Bearer token。此处不记录真实网络地址、令牌或密钥，也没有调用钱包导出。

## SDK 入口与提取方向

| 现有入口 | 必须保留的边界 | 后续归属 |
| --- | --- | --- |
| `new`、`wallet`、`conversion_signer`、`rpcs` | 钱包身份和敏感 signer 不泄露到 API | bootstrap、wallet service、signing/RPC adapters |
| `resolve_pool`、`resolve_stonkfun_sol_hop`、`warmup_params` | 池 owner/Mint 验证、缓存、备用节点及超时 | pool resolver 与 SDK 参数适配 |
| `buy`、`sell` | 资产单位、协议参数、min_out、模拟/发送语义 | SDK adapter，Application 掌握授权和 reservation |
| `pumpapi_trade` | 外部返回交易经审计、受控签名/模拟/提交 | PumpAPI adapter |
| `send_aggregator_transaction`、`simulate_unsigned_aggregator_transaction` | 两者能力隔离，不能由模拟路径进入发送 | aggregator、signing、RPC adapters |
| `aggregator_target_ata`、`resolve_aggregator_lookup_table` | Token Program、账户与 ALT 校验 | 对应 adapter，不能只依赖外部报价 |
| `wallet_balance`、`quote_balance`、`wallet_balances`、`transfer_sol` | 查询与资金操作隔离 | wallet service / RPC / signing |
| RPC/MEV lane 状态、`ping_rpc` | 诊断不暴露 token，不能默认代表可交易 | infrastructure 与 RPC adapter |

`WalletRegistry`、`BlockhashCache`、`pumpapi_client` 也引用具体 SDK 客户端，不能只移动 `sdk_client.rs` 就宣称升级隔离完成。

## 持久化入口

- 普通与批量交易：`accept_trade_for_wallet` 在 durable 模式下先调用 `TradePersistence::reserve`，再进入 executor；仍需 Handler 级副作用测试。
- 普通交易内部的聚合分支通过上述普通保留入口；不要与独立 quote submit 混淆。
- 独立 quote submit：目前依赖 QuoteStore 内存记录，durable 覆盖待 TS-S1 实现验证。
- 转账：钱包仓储的转账记录与 reserve 路径；兑换：ConversionStore 独立账本。均不能由普通交易测试代替。

## 后续升级必须保留的兼容行为

这些是本地行为清单，不冒充已核实的 vendor 补丁清单：

1. DAMM v2 模拟手续费 Default lane 与实际 Temporal 等提交策略分离。
2. DAMM v2 最小输出按整数计算，拒绝零报价及舍入为零；原错误文本保持。
3. CPMM 自定义 Quote 按输入/输出 Token 交易，不能静默改成 WSOL。
4. StonkFun Via-SOL 的池、开关和金额约束不能因新 builder 存在而失效。
5. RPC 限流与备用节点行为、聚合交易审计、转账租金预留需逐项回归。

TS-00 尚缺上游来源比对、完整补丁账本及签名/广播副作用图，当前只完成本地清单。

## 后续轮次进度

以上是初始基线，不覆盖后续实现记录。2026-11-13 后续检查点：

- 独立报价提交已改用 journal：见 `QUOTE_SUBMISSION_JOURNAL.md` 与 `SUBMISSION_BOUNDARIES.md`；原内存 submission 描述仅代表改造前状态。
- 请求触发原签名对账已接入：见 `SUBMISSION_RECONCILIATION.md`，无后台定时器。已审查入口禁播与可选 separate 管理鉴权见 `EXECUTION_SAFETY_AND_ADMIN.md`。
- TS-02-A/B 已分离聚合/普通执行内部类型，旧 V1 快照和 normalization/hash 指纹不变：见 `QUOTE_DTO_BOUNDARIES.md`、`TRADE_DTO_BOUNDARIES.md`。
- TS-03-A Registry 保持旧能力/108 组校验合同：见 `PROTOCOL_REGISTRY.md`；TS-04-A 只读余额端口见 `WALLET_READ_BOUNDARY.md`。
- TS-S1-E 修复旧 hash 的 SOL/Quote 分支歧义：见 `ORDINARY_TRADE_HASH_REVIEW.md`。旧 hash/schema/键/签名保留；durable 增加旧 JSON 核对，memory/shadow 也检查参数与分支。
- TS-04-B1 为独立报价增加不透明 Prepared 句柄与签名前授权复核：见 `PREPARED_QUOTE_SIGNING.md`。B2 已移除公开 signer getter、将聚合签名收回 SDK，并拆分 WSOL unsigned prepare/simulate 与 Live 签名：见 `SIGNER_ACCESS_AND_UNSIGNED_WSOL.md`。旧内部聚合发送前签名账本仍未迁移。
- TS-04-C 已从 blockhash 缓存移除 SDK/RPC，改用项目 hash、只读 port 和 adapter，刷新顺序/时效规则保持：见 `BLOCKHASH_READ_BOUNDARY.md`。
- TS-04-D 已提取 readiness 只读查询与 SDK probe，实际 HTTP 路由通过无 SDK source 注入验证完整成功合同，旧 ready/paused/DB 规则保持：见 `READINESS_QUERY_BOUNDARY.md`。
- TS-05-A 已提取 Pool 查询 façade/backend/cache port 与自有路由提示，实际 resolve/warmup/hop 路由通过旧合同和 fake 成功/失败回归：见 `POOL_QUERY_BOUNDARY.md`。SDK 实际参数构建与完整 VerifiedRoute 仍未迁移。
- TS-05-B1 已将 SDK warmup/dex_params、Quote token、成员/平台/hop 检查集中到 `adapters/sdk/protocol_params`，旧 90 组参数选择指纹保持；成功账户样本尚未覆盖全部协议：见 `SDK_PROTOCOL_PARAMS.md`。

当前全套 236 项离线测试通过，SDK vendor、manifest/lock 与 HTTP 合同轮相同，原 release 制品未替换；磁盘约余 3.4 GB。未部署，其他资金风险、真实 RPC、raw RPC/SDK 能力隔离与管理凭据的运行时迁移仍按各自门槛验收。下一批为严格只读脚本 RPC 与实际 SDK 参数成功样本。
