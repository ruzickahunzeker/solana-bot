# TS-S1-G3-A WSOL 单向状态与签名绑定

日期：2026-11-13。接续 `WSOL_QUERY_AND_API_BOUNDARIES.md`；仅 Trade Service 源码、测试与文档。未升级 SDK/依赖、构建 release、部署重启、读取生产钱包/数据库或执行真实 RPC、签名、广播。

## 本阶段实现

- `domain/wsol_progress.rs` 定义项目状态与两类事件：发送只能报告 submitted/unknown，原签名观测可以报告 submitted/confirmed/failed。纯政策不依赖 SQL、SDK 或 API。
- `WsolSignedOperation` 绑定原记录的八项不可变身份，以及精确、非空的原签名和完整字节。状态可以是旧快照，身份和已保存证据不能变。
- `ConversionStore::apply_signed_update` 在 `BEGIN IMMEDIATE` 中读原行，核对绑定和状态，再以身份、旧状态、签名和字节执行 CAS。更新后在同一事务读回，验证状态、身份、证据、错误和时间；触发器篡改或写失败会回滚。
- 弱结果/重复结果不执行 UPDATE，不清除错误，不改变更新时间，也不触发更新 trigger。矛盾终态观测显式报错并保留现状。
- 原泛化 `mark_broadcast_result` 仅作为 `cfg(test)` 兼容 fixture 保留，内部委托同一个安全更新核心，生产入口没有旁路 SQL。
- Application submit/status 与真实和测试 journal 均改用受限事件与绑定。发送保存结果后，回读必须匹配本次 artifact；状态查询回读必须匹配原身份和已有证据。signed/submitted/unknown 的缺失证据在请求 RPC 前报错。
- reserved 的并发回读允许前进到已签名状态，不要求状态逐字相同；不能将一个新签名强加给仍处于 reservation 快照的身份。已存在的签名/字节不得丢失或替换。
- `get_by_id` 直接按原 ID 取完整行，不再先读 key 后进行第二次 lookup；共享解码使用 fallible `try_get`。

## 单向政策

| 当前状态 | 接受的前进 | 重复或迟到处理 |
| --- | --- | --- |
| reserved | 不接受已签结果事件 | 缺签名证据，拒绝 |
| signed | unknown/submitted/confirmed/failed | 必须匹配原签名/字节 |
| unknown | submitted/confirmed/failed | unknown 原样返回 |
| submitted | confirmed/failed | unknown/submitted 原样返回 |
| confirmed | 无 | confirmed/弱结果不写；failed 冲突报错 |
| failed | 无 | failed/弱结果不写；confirmed 冲突报错 |

终态仍不重新开放原 key。其他 key 可以在终态之后按原索引申请新操作，不等于系统自动补单；迟到结果不能把旧操作改回 pending、抢占新操作的钱包锁。

## 兼容性与有意安全修正

四个 URL、DTO、七字段回执、金额/手续费/租金算法、V1 fingerprint、表结构、global key、pending-wallet 索引和 FULL/WAL 连接策略保持。原 V1 黄金 JSON/SHA 继续作为回归；不重写历史行。

这是安全行为修正，不是纯目录移动：迟到 unknown 不再降级已观测 submitted；终态不会被迟到回执覆盖；损坏身份、缺失证据及最终回读异常会明确失败。旧缺陷基线改名为 `wsol_late_unknown_no_longer_downgrades_observed_submission`，不能为保持旧缺陷让新实现降级。

错误仍映射原 HTTP 400，本阶段不将其重新定义为“肯定未签名/未发送”，也不新增自动重试、回退后端、改 blockhash、重签、换 key 或后台任务。RPC 确认映射沿用 G2-C，仅状态写入防倒退，不宣称最终性可信度已验收。

## 离线证据

相对 G2-C 新增 16 项测试；只使用临时 SQLite、合成无效签名/字节、fake ports 和进程内 Axum Router：

- 领域 1 项：六状态、五事件的 30 组政策，以及非法状态。
- 存储 7 项：实际 SQL 矩阵、八身份/签名/字节绑定、无 UPDATE trigger 的 no-op、写失败与更新后篡改回滚、缺失/空证据/异常时间、终态不抢占其他 key、双 pool 16 竞争与文件重开。
- 提交应用 3 项：发送暂停时观测先落库、成功/不确定回执迟到后的权威结果；最终回读 10 类身份/证据篡改；reservation 并发进展和不完整证据。
- 查询应用 3 项：弱 submitted 观测迟于另一请求的 confirmed/failed；有/无 RPC 观测时最终回读 10 类篡改；缺签名字节先于 RPC 拒绝。
- HTTP 2 项：真实 submit 路由保留先落库结果与重复语义；真实 status 路由并发弱查询不能覆盖终态。查询 fixture 的禁播/Live 关闭保持，不具有提交方法。

Gate 使用 Notify 明确同步，不靠 sleep 猜竞态。重复请求核对无新增 fake 签名/发送；这是调用计数证据，不是真实钱包签名或链上验收。

## 验证与后续

最终验证全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                       # 568 passed
cargo test --locked --offline --quiet -- --test-threads=1   # 568 passed
cargo test --locked --offline --quiet http_contract_tests   # 116 passed
cargo test --locked --offline --quiet wsol                  # 78 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅 models/persistence 两条既有兼容重导出警告，没有以 allow 隐藏。Cargo.toml、Cargo.lock、既有 release 的 SHA-256 分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`，与前置阶段一致；vendor/原迁移无 diff。不得把前一阶段 552 项替作本阶段证据。

下一阶段继续 G3-B：先完成 canonical 指令/金额/预算与 Quote 绑定审计，再明确 sign/record_signed/simulate 的证据顺序。当前 Live 仍在签名后、持久化前模拟，失败/取消可以丢失本地已签证据；本阶段没有解决它。随后覆盖取消/文件恢复、原签名查询 deadline/可信度及消费者兼容错误语义。

后续执行更新：G3-B1 已完成标准消息与原 Quote 的签名前授权及离线验证，见 `WSOL_PRE_SIGN_AUTHORIZATION.md`。该更新不修改本文件 G3-A 的历史计数，也不表示签名落盘顺序已修复。

后续还包括 PumpAPI 外来交易审计、普通交易发送前账本、VerifiedRoute/协议扩展证据、基础设施/依赖规则和 SDK 来源/升级演练。通用后端扩展门槛已补入主计划 4.3；未添加 Router 或扩大交易许可。本阶段不代表整体优化或生产发布完成。
