# TS-S1-G3-C1 WSOL 原签名查询与明确最终性

日期：2026-11-13。接续 `WSOL_SIGNED_JOURNAL_AND_SIMULATION.md`。仅 Trade Service 源码、临时库/内存 RPC 测试和文档；未升级 SDK/依赖、修改生产配置/钱包/数据库、构建 release、部署重启或执行真实 RPC、签名、广播。

## 修正的旧判定

旧 WSOL status 只取 RPC 返回数组第一项，以 confirmations=None 判断 confirmed，且任意 err 直接判 failed。它不能拒绝结果数量异常、明确确认等级与 confirmations 矛盾、legacy status/err 不一致或错误 slot。processed/confirmed 阶段的 fork/error 还可能提前释放 pending wallet，之后终态保护又阻止更正。

本阶段只修新查询的证据入口，不重写历史记录，不改变其他普通交易、聚合或 native transfer 的确认政策。

## 接入的边界

- `adapters/rpc/wsol_status.rs` 继续绑定构造时的原 RPC Arc 和原 SDK Signature，只调用一次 `getSignatureStatuses`，参数为一个原签名及 searchTransactionHistory=true。不换节点、不重签、不发送。
- RPC 返回必须恰好一项，context slot 非零；单项 null 才表示未找到。空数组、额外项或无效响应属于查询失败，不能凭第一项推进状态。
- status.status 与 err 必须表示同一个错误；confirmationStatus 必须明确为 processed/confirmed/finalized。finalized 必须 confirmations=None，非最终等级必须有 confirmations；不调用 SDK 的旧节点回退推断方法。非最终的具体确认数不用于推断最终性。
- 交易 slot 必须非零且不超过 context slot。缺字段/反序列化异常/不一致信息一律不生成可写入观测。
- `domain/wsol_progress.rs::WsolSignatureObservation` 复用项目 `ChainObservation/ChainCommitment`，另带 context slot。领域政策再次核对原签名和 slot 因果关系，不让 Application 直接信任一个 rooted 布尔值。
- Application 在 lookup 外增加固定 3 秒 deadline；失败、无结果、超时或领域拒绝后只读回权威记录，沿用原身份/字节校验。原 owner/签名证据/SDK 签名解析/loaded-wallet 的构造顺序保持。

3 秒仅限制已构造 reader 的 RPC lookup 阶段，不是整个 HTTP 请求的硬 SLA；SQLite 等待和读回另有原连接策略。没有新增环境参数或修改现有 RPC 配额/配置。超时会丢弃本次查询 future，不发起备用查询或定时任务。

本阶段不改变 SDK 底层 HTTP 已存在的只读重试实现；“一次”指项目调用一次原签名查询，不能据此承诺真实节点的 HTTP 请求数恒为一，外层 deadline 也不会撤销节点已收到的只读请求。

## 状态与错误语义

| 明确且有效的观测 | 可写入结果 | 钱包 |
| --- | --- | --- |
| processed 成功/错误 | submitted，错误加 non-final processed 前缀 | 保持 pending |
| confirmed 成功/错误 | submitted，错误加 non-final confirmed 前缀 | 保持 pending |
| finalized 成功 | 既有 confirmed 字符串 | 可按原规则结束 pending |
| finalized 错误 | failed，保留最终错误文本 | 可按原规则结束 pending |
| RPC/解析/证据无效/超时/未找到 | 不写观测、不清空旧 error | 保持原记录，或返回其他请求已推进的权威状态 |

WSOL 原表没有中间确认等级及重组元数据，因此不能照搬独立 Quote 的“confirmed 错误先 failed，后续允许 finalized 修正”政策。本阶段明确保留公开状态名：WSOL 的 confirmed 现在必须由明确 finalized 成功推进，不新增 finalized 字符串或表列。

写入仍经过 G3-A 单向 CAS。已是 submitted 时，重复/较弱 submitted 观测不重写时间或 error；新 provisional error 不能覆盖终态。signed/unknown 首次推进 submitted 时可以保存带前缀的临时错误；finalized 成功推进 confirmed 时清除临时错误。error 不应被调用方单独用作终态或补单许可。

终态/unsigned 记录仍不请求 RPC。同 key、签名、完整交易字节、owner/wallet、七字段回执和 HTTP 200/legacy 错误映射保持；正常查询不会生成新操作。构造 reader 的错误仍按原路径返回，RPC lookup 错误暂不新增响应字段或覆盖交易 error，诊断/错误分类留独立兼容工作包。

## 信任与兼容限制

这是原节点响应的形态、绑定和明确最终性校验，不是链上密码学证明、多节点共识或节点诚实性证明。没有新增独立 freshness oracle，也不凭“查不到”推断签名过期、释放钱包或自动重发。旧节点不提供明确 confirmationStatus 时保守保持 pending；真实节点兼容性需另行窗口验证。

旧代码已保存的 confirmed/failed 不自动回退、重新查询或迁移；本阶段不能证明那些历史终态当时满足新准入。发布时需要单独审查历史来源，不可仅靠本次源码修复宣称历史记录全部可信，更不能自动重开旧 key。

公共 URL、Query、鉴权、七字段 JSON、V1 Quote/fingerprint、原表/索引/global key/FULL/WAL 均未改。processed 错误不再 failed、畸形数据不推进、明确 finalized 才结束 pending，属于有意安全修正，不是纯目录移动；Bot/操作者必须在受控发布前理解新语义。本轮没有修改 Bot 或开启兑换。

## 离线证据

相对 G3-B2 新增 16 项：

- 领域 2 项：三种确认等级 × 有/无错误，以及签名/零 slot/未来 slot 拒绝；完全无 SDK 类型。
- RPC 4 项：结果数/slot、缺失/未知/矛盾确认信息、status/err 不一致、捕获唯一原签名/history 请求。既有两项旧映射测试改为正确语义，旧 processed+null、finalized+非空确认数改为拒绝样本，不当成兼容要求。
- Application 6 项：signed/unknown/submitted 在非最终成功/错误下保留钱包锁，finalized 后才释放；坏身份/slot；3 秒超时保留旧 error 且取消 future；超时后回读另一请求终态；调用方取消不写/不自动查询；迟到 provisional error 不覆盖 finalized。
- 真实 Router 4 项：七字段 provisional→finalized、坏证据不改回执、3 秒 timeout/Drop、经真实 RPC adapter 处理八组原始 JSON/确认形态。后者使用空钱包注册表、公钥/合成签名、内存 RpcClient，禁播且 Live 关闭，不初始化真实钱包。

pending reader 使用 Drop 计数证明 future 被取消，gate 用 Notify 明确同步，不用 sleep 猜阶段。原签名参数用只允许 GetSignatureStatuses 的内存 sender 捕获；没有签名器或资金传输方法。并发/文件恢复等既有测试继续保留。

最终全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                       # 609 passed
cargo test --locked --offline --quiet -- --test-threads=1   # 609 passed
cargo test --locked --offline --quiet http_contract_tests   # 122 passed
cargo test --locked --offline --quiet wsol                  # 119 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅 models/persistence 两条既有兼容重导出警告，未加 allow 隐藏。Cargo.toml、Cargo.lock、既有 release SHA-256 仍分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。G3-B2 的 593 项保留为历史检查点，不替作本批证据。

## 继续队列

1. unsigned reservation 与失败：完整身份/快照的 reserve 读回、fail_before_sign CAS 和失败最终回读，防止按 ID 更新/读取另一意图；保持 V1 格式和原 key 不重开。
2. 查询/存储/结果不明的错误诊断与调用方兼容，不能把 legacy 400 当成确定未签/未发送；不在此处暗改 Bot。
3. G4 的真实节点、磁盘/断电和部署验证仅可在独立授权窗口进行。当前离线通过不等于生产或全部 WSOL 安全验收。
4. 之后继续 PumpAPI、普通交易发送前账本、VerifiedRoute/协议扩展、基础设施/依赖方向、SDK 来源/升级演练与发布回滚；整体目标不缩小。

后续执行更新：G3-C2 已完成 typed reservation 的完整身份/证据校验、事务内新建/读回，以及 unsigned failure 的完整 CAS/失败回执绑定，见 `WSOL_UNSIGNED_RESERVATION_AND_FAILURE.md`。本文件保留 C1 历史计数；下一队列以新文档和 `EXECUTION_STATUS.md` 为准，错误分类、真实节点/恢复验收仍未完成。
