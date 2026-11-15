# SDK 原签名只读恢复实施清单

日期：2026-11-15。接续 C3c1，不是实现完成或生产启用声明。
范围沿用新 Durable Live SDK 直接 DAMM v2 卖出；其他 SDK 协议/方向/模式后续扩展。
PumpAPI 暂停，不升级 SDK、不改公开 API、不部署、不读取生产钱包或数据库、不发交易。

当前 C3d1 已在源码实现显式 SDK context/replay/finality 与迟到 receipt 保护，新增 18 项
仅受限类型检查，未接 SDK 自动扫描；见 `SDK_ORIGINAL_SIGNATURE_RECOVERY.md`。
最近已验收 C3c2 `fd1c7cb` / Actions `43685088919`，artifact `14150028402` 下载核对，
完整并发/串行各 1398 通过；不覆盖本批恢复核心。下方缺口是制定本清单时的核查，
其中 context/validator 核心已由 C3d1 源码接续，扫描索引/有界归并/worker 及生产验收仍待完成。

后续权威进展：C3d1 `87f8677` / Actions `43686886493` / artifact `14150979478`
完整并发/串行各 1416 通过，18 项新增已运行。C3d2 另有 010 严格索引迁移、两类同快照
有界扫描与 SDK worker/回环接线源码，16 项仅类型检查、六项独立 DDL 通过，实际 Rust
运行待 CI，见 `SDK_PENDING_OBSERVATION_SCAN.md`。不把下方历史缺口重复执行或当作生产已启用。

## 已核对的缺口

- `ordinary_observation_scan.rs::PAGE_SQL` 及 007 partial index 仅覆盖 ordinary-sol-buy，
  不能因为共用 claims 表就宣称 SDK 已自动恢复。
- 原聚合 `observation_context` 严格要求 SOL 买入及 BoundQuote；不得通过伪造 Quote、
  放宽 ensure_scope 或重建 AdmittedTrade/ExecutionOwner 来容纳 SDK。
- SDK 现有 validate_sdk_row 禁止 final_observation_json，parent validator 也不接受签后终态；
  必须与最终性写入、竞争处理和读回一起扩展，不能只把 SQL operation 过滤改为 IN。
- SDK 的 broadcasting 与 signed/submitted/unknown 都有原签名证据，崩溃可能发生于任一阶段。
  claimed/prepared/signing 没有完整已落盘签名，不进入恢复发送，也不重新取得执行 owner。
- 现有 scanner、共享只读 runtime、可选关闭配置和 shutdown 可复用；恢复不增加一个后台 RPC client。

## 实施顺序

1. **只读 SDK context**：在 persistence adapter 按 operation 严格分发；SDK capture 校验原
   request JSON/key/hash、钱包、公钥、pool/mint/方向、候选参数、原无签名消息及已签 packet。
   校验不得重新应用当前默认配置，显式值与历史候选默认快照必须区分；必要时提取只读 replay
   helper，并保留 V1 hash/JSON oracle。context 只保留原身份/签名与不可变证据指纹，
   不暴露 owner_nonce、交易字节、Keypair、执行/广播方法或新的准入许可。
2. **SDK 最终性读写**：只接受匹配原签名、非零 slot 的 finalized；processed/confirmed、None、
   超时、429、错签名及格式错误保持 pending/unknown，不降级到“未发送”。
   原 claim 与父行在同一事务 CAS，保持已签 bytes/hash、候选、nonce、operation 及原提交阶段；
   finalized 成功推进 confirmed，finalized 错误推进 failed。签前失败与链上失败不得混淆。
   相同最终证据可幂等读回，不同最终证据拒绝，不覆盖其他权威终态。
3. **并发与读回**：每次 read/finality 必须重算同一证据指纹。SDK 写回 receipt 与恢复 finality
   并发时不得覆盖已保存最终状态；原 executor 仍用 finish_journaled，禁止旧快照 SQL 补写。
   签后 failed 只有合法最终证据才能作为订单结果，不把它误识别为签前错误。
4. **有界扫描**：保留 007 及原聚合查询，另增 SDK partial index/migration，不重建原数据表。
   两个 operation 各用索引、每次最多 limit+1 个候选；在同一只读快照按
   (created_at_ms,request_id) 归并后仍只返回全局 limit+1，保证 tie、游标、年龄 cutoff 和
   环回语义，不使用 OFFSET/全表读取。SDK 包含 signed/broadcasting/submitted/unknown，
   排除无签名、签前失败、已最终化、不匹配父行及损坏候选。
5. **运行接线**：复用默认关闭的原 ObservationPlan/scanner、同一 runtime 的观测容量、节流、
   deadline/429 冷却、单批 deadline 及停止信号。GET/列表/重复 POST 不新增 RPC 或任何资金副作用。
   配置开关不在本轮改为 true，Memory/Shadow 不伪装 Durable。

## 验收门槛

- 使用临时 Keypair、公开 fixture、内存/回环 RPC 和临时 SQLite；不读取 .env 或生产数据。
- 原执行对象、原 owner、signer、发送器销毁后，从原文件库重开捕获四类有签名阶段；
  恢复对象仅能 lookup 原签名、写合法最终证据，测试中不提供任何可签/可发端口。
- 对 request key/hash/JSON、钱包/池/方向/百分比/滑点/费用/MEV、候选消息、签名/字节/hash、
  operation/schema/nonce、最终性字段逐项篡改，要求先拒绝、零 RPC、零状态写入；
  同一合法 context 创建后证据变化也拒绝。
- 明确缺省参数历史回放、literal key 空白与原值兼容；不因当前默认值变化而错误拒绝合法历史。
- finalized 成功/失败及同证据重复、冲突证据、最终性写失败/丢回执/事务回滚、两个 SQLite pool
  竞争、receipt 与 finality 竞争和文件重开均须验证；不替换原 signed packet。
- None/429/超时/取消/错误 bank/签名/slot、不明确 commitment 及非最终链上错误均不重签/重发，
  原 pending 状态/签名证据保持。超时必须取消当前只读 future，不放任隐藏后台重试。
- 两类候选混排、同毫秒排序、分页边界、年龄 cutoff、取消游标、批量上限、损坏候选隔离及
  EXPLAIN 索引检查；迁移重复执行、旧行/类型/索引不变和异常索引明确拒绝。
- 专项和完整并发/串行测试在源码专用 GitHub Actions 实际执行，不能用类型检查替代。
  生产 migration/刷盘/断电、真实节点接受/配额、调用方兼容与上线回滚仍需独立授权和验收。

## 当前证据归属

- C3c1 源码提交 dd946aa / Actions 43683583741 已终态成功，artifact 14149574413 下载核对，
  完整并发/串行各 1394 通过；不据此声明下述 C3c2 或 SDK 恢复完成。
- 后续 C3c2 四项 journal integration 源码已写入，仅受限 check 通过：真实候选/受控签名、
  原 SQLite ledger、真实 RpcClient 与固定 sender 串联，底层 RpcSender 仍为内存实现。
  覆盖正常、错回执、receipt 写失败和取消；不代表准备 RPC→真实供应商或恢复已完成。
- 本文件是下一任务的实施/验收约束，不将任务拆小后声称全量 SDK 恢复或整个优化完成。
