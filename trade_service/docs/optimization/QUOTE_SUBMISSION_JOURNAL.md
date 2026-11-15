# 独立聚合提交账本验收记录

日期：2026-11-12。针对独立 `POST /v1/quotes/sol-buy/:quote_id/submit`；后续新增只读 GET 对账，见 `SUBMISSION_RECONCILIATION.md`。源码接入及离线验证已完成，没有构建 release、重启服务、修改生产配置或访问生产数据库。

## 实施边界

```text
HTTP 鉴权、钱包映射、请求校验
  -> Live：先查账本，已有记录直接返回（不要求仍有内存报价）
  -> 新请求：原报价绑定 -> prepare / 指令检查 / 不签名模拟
  -> Simulation：返回 simulated，不保留 Live key，不签名、不广播
  -> Live：原子 reserve -> 签名 -> record_signed 提交成功 -> 广播一次
  -> 保存 submitted 或 unknown，始终保留原签名
```

实盘只在 `durable` 模式执行；数据库不可用不降级到内存。`memory`/`shadow` 的普通交易语义不改变，但不再允许独立聚合入口使用不可靠的内存提交缓存。路径与请求/响应字段不变，新增了更严格的状态和错误行为，调用方必须按更新后的 API 文档处理。

重复的 reserved 请求返回 `submitting`；已签名但最终结果尚未落盘时返回 `unknown` 和原签名。数据库写入失败可能返回 `503`。这些均不允许调用方自动换 key 重试。

`preflight` 是创建记录时的检查快照，不随重复读取刷新，不能据此再次下单。报价未保留前的预检失败不产生资金副作用；幂等键完成原子保留后不会自动释放或重新执行。

## 实际模块

| 文件 | 职责 |
| --- | --- |
| `src/domain/submission.rs` | SDK 无关的身份、完整报价指纹、状态、签名字节类型 |
| `src/application/ports/submission_journal.rs` | 账本查询、原子保留、签名保存和状态转换接口 |
| `src/application/quote_submission.rs` | 模拟/实盘编排；TS-02-A 后使用内部 command/result/quote，不再直接依赖 API DTO、SDK 或 Axum |
| `src/application/ports/quote_execution.rs` | 后端执行接口，使用内部类型与 opaque Prepared；具体 SDK 类型留在 adapter |
| `src/adapters/persistence/submission_journal.rs` | SQLite 唯一约束、状态 CAS、恢复；读取时校验身份及指纹一致性 |
| `src/adapters/sdk/quote_submission.rs` | 包装原预检、SDK 签名与广播；从落盘所用的相同字节恢复交易并校验签名 |
| `src/http.rs` | 认证、临时报价读取和错误到 HTTP 的映射；移除旧内存 submission 缓存 |
| `migrations/002_quote_submissions.sql` | 独立操作表，不伪造 Pool，不混用普通交易金额语义 |

TS-02-A 保持这里已有的 V1 快照、状态与签名字节格式；转换与固定指纹证据见 `QUOTE_DTO_BOUNDARIES.md`，没有新增数据库迁移。

只使用已有依赖，不修改 SDK vendor。`TradePersistence` 在 durable 启动时初始化新增表并恢复未决状态；本轮仅测试库执行了这个流程。旧 `persistence.rs` 测试改动保留。

## 一致性与安全属性

- 唯一约束为本操作账本的 `(wallet_id,idempotency_key)`，不是拼接字符串；公钥、报价 ID、完整报价内容、版本、操作和 Live 模式被指纹绑定。
- JSON 对象顺序规范化，不改变数组顺序；原始 u64 数量不会先转换成浮点数。
- 只有新 reservation 的调用能进入 signer；重复请求不再签名或发送。签名只能从 reserved 保存一次，不能被后续错误或不同结果覆盖。
- 先保存 signature 与 signed bytes，再允许广播。网络超时或 RPC 返回不同签名都记为 unknown，并保留本地原签名。
- durable 连接使用 WAL 与 `synchronous=FULL`；同库普通交易写入也受此刷盘设置影响，尚未做生产磁盘性能验收。
- 签名字节不进入 API、日志或 Debug 输出。它们属于敏感运行数据，不应提交 GitHub；备份与权限按生产交易库管理。
- 重启只把 reserved/signed 转成 unknown，不重新签名或广播。submitted 仍不等于 confirmed/finalized。
- 相同请求的恢复读取仍需现有钱包 ID/公钥映射及鉴权；未开放无认证的历史查询。

## 已有测试

测试使用 fake backend、SQLite 内存库及临时文件库，不加载真实钱包、SDK 客户端或 RPC。

账本实施轮新增 17 项应用/账本测试和 2 项领域测试，全套当时为 95 项；后续对账轮补充了状态观测与回执竞态，当前全套 116 项通过，格式/差异检查通过。

1. 16 个并发请求最多一次签名、一次广播。
2. 两个独立连接池竞争同一文件数据库，只能创建一个 reservation。
3. 同名 key 在不同钱包下隔离；钱包公钥/报价 ID 变化冲突。
4. 模拟不占用 Live key，不签名、不广播，也不会误读成已完成的实盘结果。
5. 缺确认、无 durable、非法 key、实际数据库关闭均被拦截。
6. reserve 失败不签名；签名落盘失败不广播；结果落盘失败保留原签名且不重发。
7. 超时、RPC 返回错误签名、广播中的任务取消均不触发第二次发送。
8. 真实文件数据库关闭重开保留幂等、签名与字节；不需要重新生成短期报价。
9. 状态 CAS 不允许覆盖签名，已签名记录不能被错误标成“签名前失败”。
10. 金额、最低输出、滑点、到期、目标 Mint、路线变化均影响指纹；对象键顺序不影响，数组顺序和超大整数差异会影响。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline application::quote_submission::tests -- --nocapture
cargo test --locked --offline --quiet
git diff --check
```

## 尚未验收

- 不代表真实 HTTP 成功下单、链上模拟、实盘、SDK 升级或所有协议已验收。
- 原 `aggregator_quote::submit` 仍供普通 `/v1/trades` 内部聚合分支使用，本轮未强行改变该路径；不能宣称所有发送入口都已统一。
- 转账、WSOL、PumpAPI 的既有风险和独立管理权限仍待处理。
- 原签名的请求触发对账已接入并用 mock 验证；后台定时对账、真实 RPC、部署及实盘尚未验收。查询层没有签名/广播能力，详见对账记录。
- 不把这些测试当作完整聚合交易指令审计；原 rollout 的其他安全门槛仍有效。
- 进程重启测试不是主机断电/文件系统损坏测试；磁盘与备份恢复需受控验收。

## 发布与回滚门槛

1. 核对线上实际 persistence 模式、调用方对 400/409/503 和 submitting/unknown 的处理；不要仅为绕过 503 自动修改线上配置。
2. 在隔离目录和临时数据库完成 release、升级旧表、故障注入及目标磁盘延迟验收；本轮未启动带生产密钥的“模拟实例”。
3. 明确禁止资金操作的隔离手段，再做链上只读/模拟验收；不能依赖一个不存在的通用 `TRADE_SUBMISSION_ENABLED`。
4. 受控窗口才部署、执行数据库初始化；任何小额实盘另行授权。
5. 回滚保留当前交易库、幂等和原签名，不恢复旧库抹除已发送订单。旧二进制不认识新 quote journal，因此回滚前必须停止该独立入口的新请求并核对未决操作；不能直接恢复旧内存 submit 路径后重提历史 key。
