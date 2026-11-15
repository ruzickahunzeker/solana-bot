# TS-S1-G3-C3-A WSOL 提交错误合同

日期：2026-11-13。接续 `WSOL_UNSIGNED_RESERVATION_AND_FAILURE.md`。仅 Trade Service 源码、离线测试及文档；没有 SDK/依赖升级、release 构建、部署重启、真实 RPC、签名、广播或生产数据访问。

## 范围与兼容变化

仅修改 `POST /internal/conversions/wsol/submit` 的应用错误映射；鉴权、JSON 提取器和禁止广播的既有响应不变。成功回执仍为原七字段，包括正常返回的 failed/unknown/duplicate。错误 JSON 仍只有 `error`，不增加必填字段，不改 V1 hash、数据库格式或金额含义。

| 类型 | HTTP | x-wsol-error-code | 含义 |
| --- | --- | --- | --- |
| 明确参数/准入拒绝 | 400 | wsol_rejected | 校验失败，不代表其他并发请求没有操作 |
| 明确幂等冲突/钱包已有 pending | 409 | wsol_conflict | 查原操作，不能自动换 key |
| Quote 存储读不可用 | 503 | wsol_unavailable | 无法完成读取，先只读检查 |
| 身份、证据、回执不一致 | 503 | wsol_integrity_error | 停止执行，保留原记录等待核对 |
| 写入/提交回执或其他结果不明 | 503 | wsol_outcome_unknown | 不能从 HTTP 判断是否已保留、签名或广播 |

以上 503 消息在脱敏原错误后追加：

```text
; WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key
```

分类使用阶段及内部 typed marker，不匹配错误文本。未标记的 reservation 错误保守归 unknown；即使文本恰好是过期/参数错误也不降为 400。签名持久化后的 typed rejection 同样不能降级不确定结果。尚未迁移的 quote、unsigned simulate、旧 status 错误仍采用 legacy 映射，不能将本阶段规则推广到所有入口。

后续更新：G4-B 已单独迁移 status 的读取/观测错误与恢复上下文，见 `WSOL_STATUS_RECORD_AND_ERRORS.md`；G4-C 又单独迁移 quote/unsigned simulate，见 `WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md`。上段及 637 项命令记录 C3-A 本身的范围，不代表这些后续入口仍使用旧映射。

## 错误响应头

- `Cache-Control: no-store`、`x-wsol-auto-retry: false`，不发送 `Retry-After`。
- `x-wsol-error-stage` 表示 admission、quote_lookup、reservation、unsigned_failure、signed_persistence、simulation_receipt、pre_broadcast_read、broadcast_receipt、final_read 或 unclassified。
- `x-wsol-recovery` 为 review-request / inspect-original-operation / retry-read-only / reconcile-original-operation。只读重查不等于允许重提 POST。
- `x-wsol-submission-id` 只有 reservation 完成请求/Quote 绑定校验后才能出现。
- `x-wsol-signature` 只有 signed persistence 回执的身份、原签名字节及 clean signed 状态全部校验通过后才能出现。生成签名、尝试写入或提交回执丢失均不足以公开为已验证上下文。

错误上下文不保留 owner/key/交易字节；URL 脱敏，非法 HeaderValue 跳过，不产生注入或 panic。响应头是调用方合同，不会自动禁止外部代理/客户端重试。上线前仍需验证调用方不会把 409/503 当成自动补单信号；本阶段没有修改 Bot。

## 离线证据

相对 C2 新增 12 项：错误模型 4、API mapper 3、Application 2、真实进程内 Router 3。十故障矩阵覆盖 Quote 读取、保留回执丢失、unsigned 失败写/回执丢失、signed 写/回执丢失、模拟回执、广播回执，以及伪装参数文本和晚到 typed rejection。损坏 reservation/失败回执不能污染恢复 ID；签名头只在干净持久化回执之后出现。旧七项 HTTP 合同按上述显式兼容变更更新。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                       # 637 passed
cargo test --locked --offline --quiet -- --test-threads=1   # 637 passed
cargo test --locked --offline --quiet http_contract_tests   # 129 passed
cargo test --locked --offline --quiet wsol                  # 147 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

以上全部通过；生产 check 仍只有 models/persistence 两条既有兼容重导出警告。manifest/lock/release SHA-256 分别保持 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。所有测试仅使用临时 SQLite、fake ports 或内存 RPC；不提供链上、断电或生产兼容验收证据。

## 后续

1. C3-B：提供带 owner/wallet 绑定的原 key 只读恢复。现有 ID GET 可查已知 ID，但 reservation 提交回执丢失时可能尚不知道 ID。不得建议用 submit POST 作为只读查询；没有历史行时它可能执行。
2. G4：继续审查 quote/store 解码、诊断与恢复交叉路径。原签名 status 可能更新观测账本，但绝不自动重签/发送。
3. 后续 PumpAPI、普通交易发送前账本、VerifiedRoute、依赖规则与 SDK 来源工作照原队列推进，不因本阶段通过而宣称整体优化或发布完成。
