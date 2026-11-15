# TS-04-F2 独立报价提交编排边界

状态：源码与离线回归完成；未部署、未读取生产钱包/数据库、未调用真实 RPC/聚合器、未发送交易。

## 实施内容

- `application/sol_buy_submit.rs` 接管原 Handler 中的钱包解析、请求校验、惰性 journal 获取、历史查询和缓存读取，继续调用原 `quote_submission::execute`。
- `application/ports/quote_submit.rs` 定义既有运行实例解析和 journal 获取接口；已解析钱包只暴露项目公钥与既有受控提交能力，不暴露 raw RPC、SDK client 或 Keypair。
- `adapters/sdk/quote_submit.rs` 私有保留原 wallet Arc 与 persistence Arc，向现有 `SdkQuoteSubmission` 转发 prepare/sign/broadcast。不改准备、审计、签名、MEV 或发送实现。
- `AppState` 注入对象安全的 `SolBuySubmissions` facade；内部使用带关联 Wallet/Journal 类型的 runtime，以复用原 RPIT future ports，不为本次提取重写提交核心或持久化接口。
- bootstrap 和 HTTP 测试状态复用原钱包注册表、缓存与持久化实例，不额外初始化 SDK、创建缓存副本、执行迁移或切换持久化模式。
- HTTP 只保留鉴权、无副作用的 DTO 转换、响应和错误映射；公开路径、必填字段、金额与 HTTP 状态保持。

## 保持的执行顺序

```text
鉴权
→ 一次已加载钱包解析
→ key / confirmed 校验
→ 获取可选既有 journal（simulate 也保持这一调用）
→ Live 要求 durable
→ Live 先查询历史；命中即返回，不读报价缓存
→ 用原钱包公钥读取同一报价缓存
→ 原核心 execute：重复校验、第二次历史查询、政策、准备
→ Live 保留 key → 时效检查 → 签名 → 签名落盘 → 一次发送 → 结果落盘
```

钱包解析先于 key/确认检查；Live 的历史查询先于短期缓存，故报价过期或进程重启不能遮蔽原签名。核心中的重复校验和查询没有删减。

simulate 不使用 Live 历史结果、不占 Live key、不签名、不发送；无 journal 仍可模拟。旧缓存的 V1→内部转换已由同一 QuoteCache 直接读取替代，TTL、身份核对、原始整数、错误与共享实例保持。

准备失败不 reserve；reserve 或签名记录写入失败不发送。发送结果丢失保持原签名和 unknown/待对账语义；不换 key、不重新签名、不重发。对账接口尚未在本批迁移。

## 离线证据

| 层次 | 数量 | 覆盖 |
| --- | --- | --- |
| Application | 8 | 完整时序、simulate/Live key 隔离、丢失缓存后的历史重放、冲突、两次查询故障、无 journal、钱包替换、过期/政策/准备失败、写入失败、8 并发及 literal 身份/键 |
| SDK runtime | 2 | 空注册表不初始化钱包；复用 durable 临时内存库，重复获取 journal 不执行启动恢复 |
| 实际 Router | 10 | 迁移前缺钱包基线、完整 6 字段响应/32 字段报告、历史重放、状态码/脱敏、签名写入失败、unknown、鉴权/提取器/方法、禁播模拟、8 并发、文件重开恢复 |

fake backend 的签名/发送只是计数和合成字节；其发送入口还检查临时数据库已保存同一签名和字节，不等于真实链上验收。文件恢复测试只在 `/tmp` 唯一目录创建 SQLite；重开后缓存不含原 Quote，仍可返回原签名，测试结束关闭连接并清理该目录。

迁移前基线初版误把缺少 confirmed/simulate 的请求当成合法 JSON，实际旧提取器先返回 422；已在修改业务代码前修正测试请求，没有改变提取器或原接口合同。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet sol_buy_submit
cargo test --locked --offline --quiet adapters::sdk::quote_submit
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

2026-11-13：Application 8 项、SDK runtime 2 项、HTTP 总计 75 项；全套默认并发/串行各 414 passed、0 failed，无测试构建警告。manifest、lock、现有 release SHA-256 与上个检查点一致；vendor/原迁移 diff 为空。

## 后续 TS-04-F3 工作包（已完成）

源码/离线执行记录见 `QUOTE_RECONCILIATION_BOUNDARY.md`；以下保留 F2 结束时的工作包。当前队列以 `EXECUTION_STATUS.md` 为准。

独立报价对账 Handler 仍直接获取 SDK 钱包和 RPC，下一批迁至只读 runtime/Application：

1. 保留原顺序：key 校验 → 一次钱包解析 → durable journal → reader 快照 → 原钱包公钥 → 原 reconciler。
2. key 错误优先于缺钱包；缺钱包优先于 durable 错误。不能套用本批 submit 的 wallet-first 顺序。
3. 私有保留原 wallet Arc，reader 只提供签名状态查询，不公开 signer、发送、reserve 或重试能力。
4. 原整体 3 秒超时、RPC lane 顺序/每节点超时、CAS 确认升级、原签名核对与 no-store 响应保持。
5. 补真实 Router 的无签名/finalized 零查询、身份冲突、故障/脱敏、并发确认、保留原签名与 SDK 空实例回归。既有 reconciler 和 migrations 保持。

普通资金发送前账本、VerifiedRoute/可信报价、协议身份与资产扩展、SDK provenance/补丁账本、升级和发布演练仍属于完整主计划，不能因为本批接口隔离完成就判定整体完成或允许上线。
