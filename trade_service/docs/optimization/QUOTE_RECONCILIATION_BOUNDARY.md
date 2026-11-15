# TS-04-F3 独立报价对账编排边界

状态：源码与离线回归完成；未部署、未读生产钱包/数据库、未连接真实 RPC/聚合器、未签名或发送交易。

## 实施内容

- `application/quote_reconciliation.rs` 接管原 Handler 的 key 校验、一次已加载钱包解析、惰性 journal 获取、reader 快照和原 reconciler 调用。
- `application/ports/quote_reconciliation.rs` 只表达钱包身份、签名状态读取及既有观测账本能力，没有 prepare/sign/reserve/broadcast。
- `adapters/sdk/quote_reconciliation.rs` 私有持有原 wallet Arc；按原时点取得 RPC 列表，封装为只提供状态查询的 reader，不公开 raw RPC 或 SDK client。
- `adapters/persistence/quote_reconciliation.rs` 提供原 SQLite journal 的观测视图，仅 lookup/record_observation；不提供初始化、保留 key 或签名字节写入。
- 原 reconciler、签名状态 RPC adapter、CAS SQL、迁移和 schema 均未修改；bootstrap 复用原注册表和持久化 Arc。

## 保持的顺序与合同

```text
Query 提取 → 鉴权 → key 校验 → 钱包解析 → durable journal
→ reader 快照 → 原钱包公钥 → 原 reconciler（总超时 3 秒）
```

- 与 submit 的 wallet-first 不同，这里 key-first；空 key 仍先于缺钱包报错。
- journal 不可用时不取得 reader；取得 reader 的时点仍先于公钥和首次账本读取，即使最终无需 RPC 查询也不改变顺序。
- 原钱包删除/替换不改变进行中的查询身份；下次请求再使用当时注册表。
- 无签名或已 finalized 的订单不查询链上；只读状态接口在禁播配置下仍可用。
- 沿用原 RPC lane 顺序、每节点超时、整体 3 秒 deadline、原签名核对、CAS 确认升级及失败后刷新逻辑。
- 成功响应保持原 10 字段与 `Cache-Control: no-store`。缺钱包/key 400、记录缺失 404、身份/Quote 冲突 409、账本故障 503；RPC 故障/缺失保留原订单并在 200 中表达 reconciliation 状态。
- 未引入报价缓存、默认钱包解析、后台定时器、自动重签或重发。

## 离线回归

| 层次 | 数量 | 证据 |
| --- | --- | --- |
| Application | 10 | key/wallet/durable 错误优先、reader/pubkey 时点、完整记录和签名字节、无需查询、缺失/冲突、读写故障、缺回执/错误签名、3 秒 timeout、注册表变化、8 并发 CAS、literal 身份/键 |
| SDK runtime/观测视图 | 2 | 空注册表不初始化 SDK；重复获取临时 durable journal 不执行恢复，观测写入复用同一账本 |
| 真实 Router | 9 | 迁移前基线、完整响应/no-store/禁播只读、无需查询、HTTP 错误、RPC 故障、最终失败/脱敏、注册表变化、8 并发、鉴权/Query/方法拒绝 |

全部使用假查询源、合成签名和临时内存 SQLite；没有真实 signer、RPC 请求或广播。3 秒超时测试等待本地 Tokio timer，不修改依赖或测试时间配置。8 并发测试保证所有请求先读取同一 signed 状态，再竞争观测更新；仅一次 CAS 成功，原签名/字节及记录数量保持。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet quote_reconcil
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

2026-11-13：F3 针对性 21 项通过；之后连同 TS-08-A 路由提取，全套默认并发/串行各 436 项通过，HTTP 85 项通过，无测试构建警告。manifest/lock/release 指纹与 F2 一致，vendor/原迁移 diff 为空。

独立报价五个入口已形成 HTTP→Application→ports→adapter 的实际调用链，Handler 提取见 `QUOTE_API_HANDLERS.md`。这不等于全 API、普通资金安全、完整协议能力或升级演练已完成。
