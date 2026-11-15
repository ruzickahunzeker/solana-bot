# 原签名只读对账验收

日期：2026-11-13。只针对独立 SOL quote journal。源码已接入，尚未部署、连接真实 RPC 或发送实盘。普通 TradeRecord、转账、WSOL、PumpAPI 和 SDK vendor 不在本轮改造范围。

## 接口与权限

```text
GET /v1/quotes/sol-buy/:quote_id/submission
  ?wallet_id=原钱包ID&idempotency_key=原幂等键

IP + Bearer -> 校验钱包ID/公钥/quote_id -> 读取账本
  -> 无签名或已 finalized：直接返回
  -> 查询原签名（含历史）-> 单向确认级别 CAS -> 重新读取当前记录 -> 响应
```

不存在的订单返回 404，不创建新记录；数据库不可用或非 durable 返回 503。钱包/quote 身份冲突返回 409。RPC 查不到或异常时返回已有订单及查询状态，绝不能据此自动构造第二笔交易。

链上“只读”不意味着本地数据库完全不写：只保存原订单的派生确认信息；不修改金额、幂等键、报价意图、原签名或交易字节。成功响应禁止缓存，不返回可执行交易字节。

没有后台扫描/定时器，也没有绑定 Telegram 刷新或自动重试。调用方主动查询才触发对账；原 POST 的重复请求只读回账本，不自动查询链上。

## 实际模块

| 文件 | 职责 |
| --- | --- |
| `domain/reconciliation.rs` | SDK 无关的确认级别、链上观测及状态政策 |
| `application/ports/reconciliation.rs` | 仅查询签名与保存观测的 ports；不提供 signer、signed bytes、reserve 或 broadcast |
| `application/submission_reconciler.rs` | 身份核验、查询超时、不降级处理和并发后读回 |
| `adapters/rpc/signature_status.rs` | Solana 类型/响应转换；最多两个现有 RPC 顺序查询，不新建实际节点配置 |
| `adapters/persistence/submission_journal.rs` | 签名绑定、确认级别 CAS；兼容迟到的广播结果 |
| `api/dto/quote_submissions.rs` | 新只读 DTO，明确分开交易错误与查询错误 |
| `http.rs` | 新 GET 路由、现有鉴权及 HTTP 错误映射 |

## 状态政策

- 正常成功观测：processed -> submitted，confirmed -> confirmed，finalized -> finalized。
- processed 执行错误 -> unknown；confirmed 执行错误 -> failed，但可被更强 finalized 结果修正；finalized 执行错误 -> failed 且不再查询。
- 任一 failed/unknown 状态都不授权自动重发。confirmed 不是 finalized，也不是最终买卖到账验收。
- 按 `confirmation_rank` 单向更新；同级或低级结果不覆盖。若同级节点互相矛盾，保留已保存证据，等待更高级观测，不声称已解决分叉争议。
- 只接受原签名的观测。RPC 缺失明确 confirmationStatus 或新旧错误字段不一致时拒绝推断 finality。
- NotFound/超时/错误不覆盖交易状态；交易错误和查询错误使用独立响应字段。
- 所有实际 RPC 查询完成后重新读账本，即使查询失败，响应也不会拿查询前的旧状态覆盖另一请求已保存的强确认。
- 对账若早于广播结果写回，迟到的 submitted/unknown 不覆盖已观测状态，提交响应也读取当前强状态。
- 已存 finalized 后不再查询节点。单个可信 RPC 的明确 finalized 仍是节点提供的证据，不是多节点共识证明；本轮未开放任何自动资金动作。

## 查询预算

沿用钱包已有 Default RPC，不改密钥、节点列表或全局 RPC 调度。最多两个节点顺序查询，每节点最多 1 秒，Application 的 RPC 查询预算 3 秒；数据库等待不计入此网络预算。没有并行请求或 getProgramAccounts 扫池。至少一节点返回 null 时报告 not_found，不代表全网不存在签名。

建议消费者以至少 1 秒的间隔轮询，RPC 异常退避。跨请求的限流/去重及后台巡检不在本轮实现，不能把预算当作无限刷新许可。

## 兼容迁移

新增 `003_quote_reconciliation.sql`：在事务内扩展状态 CHECK、增加 confirmation_rank/observed_slot，并复制原身份、指纹、幂等、签名和字节。专属迁移标记在同一事务内写入，避免重复重建。失败完整回滚，不把迁移标为完成。

初始化仍只在 durable 下执行。本轮仅临时库运行迁移；发布前需估算 quote journal 拷贝及 WAL 所需空闲空间，并预留受控窗口。不能直接覆盖旧 002 来假装支持既有数据库升级。

回滚保留原数据库与签名字节，不恢复旧库抹除订单。旧二进制不识别 confirmed/finalized 和新 GET，因此回滚前必须停止该独立入口的新请求、核对未决订单及调用方兼容；不自动降级 schema 或重提旧 key。

## 验证范围

离线测试覆盖确认/最终状态、processed/confirmed/finalized 错误、查询空/超时/异常、身份拒绝、缺失订单、无签名、并发延迟结果、迟到广播回执、落盘失败、旧表迁移与回滚、文件库重开、HTTP 鉴权与方法、DTO 不泄露交易字节。

RPC adapter 使用 RpcClient 内存 mock；Application 使用只有查询能力的 fake reader。没有生产钱包、签名器、真实 RPC 或 socket 服务。

本轮新增 15 项对账/迁移测试、4 项 RPC adapter 测试、1 项提交回执竞态测试及 1 项 HTTP 测试；全套 116 项通过，格式与差异检查通过。manifest、lockfile、SDK vendor 和原 release 指纹均未改变。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline submission_reconciler -- --nocapture
cargo test --locked --offline signature_status -- --nocapture
cargo test --locked --offline --quiet
git diff --check
```

剩余门槛：真实 RPC 验收、受控 release、迁移空间/延迟验收、调用方新状态兼容、全服务硬隔离与管理权限。构建/测试通过不代表这些门槛已完成，也不代表 SDK 已升级。
