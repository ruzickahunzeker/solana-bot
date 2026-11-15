# TS-02-C3 已校验交易接受编排

日期：2026-11-13。状态：新增 7 项离线测试，全套 308 → 315，通过，无编译警告。未升级 SDK/依赖/vendor、构建 release、部署/重启、连接真实 RPC、读取生产钱包/数据库或发送交易。

## 已实施

- `application/trade_admission.rs::accept` 编排已校验请求的内存幂等、临时记录索引和 journal 保留/镜像；返回项目 `AdmissionDecision/AdmissionError`，不包含 StatusCode、TradeResponse 或 AppState。
- `application/ports/trade_admission.rs` 定义 `MemoryTradeReservations`、`TradeRecordIndex` 和惰性 `ValidatedAdmissionPayload`。接口只使用字符串、旧分支位、内部 V1 record/snapshot，不导入 DTO、SDK、SQLx、Axum 或 Config。
- `idempotency::ReserveResult` 保留为项目 `MemoryReservation` 的旧路径别名，原 DashMap 的原子 tuple key/hash/分支核对算法不变。
- `adapters/cache/trade_admission.rs` 适配现有 IdempotencyStore/TradeStore；只有 TradeStore 的项目记录 insert helper，没有新增索引或 AppState 副本。
- `adapters/persistence/trade_admission.rs::AdmissionPayloadV1` 保存 wire/normalized 的不可变借用，仅在相应时点计算 hash、构造 record 或持久化 JSON。记录继续使用原 request 字面元数据、默认 method 和同一时点的 created/updated 时间，不从 command 的规范化地址重建旧行。
- journal 的 `ensure_snapshot_storage(SnapshotWrite)` 保留“存储存在检查先于快照构造”的原顺序；只检查已有 handle，不增加 SQL/RPC 或健康查询。
- HTTP 接受入口仍负责原身份/校验/策略/command、结果投影、原日志、tokio task 启动、响应等待与批量逐钱包处理。普通接受中的 validated 保留分支已实际调用新应用服务，不是未接入的平行实现。

## 保留时序

| 模式/结果 | 顺序与副作用 |
| --- | --- |
| memory 新请求 | 原 UUID 时点 → hash → 原子 key 保留 → record 时间/字段构造 → 索引 insert；不准备存储 JSON 或调用 journal mirror |
| memory/shadow 重复 | hash → key duplicate → 索引 get → 原记录 clone 标 duplicate；不构造新 record/JSON，不启动执行 |
| memory/shadow 冲突/缺失索引 | 原 409 原因；不创建新 record/JSON |
| durable | 不访问内存幂等；先构造/插入尝试记录，再检查存储、准备原 hash/JSON、执行 durable reserve |
| durable duplicate/conflict/error | 移除刚创建的尝试索引；duplicate 保留原记录/签名；冲突仍 409，失败仍 503；不启动执行 |
| shadow 新请求 | 内存接受后，按原顺序检查存储/快照/mirror；任何该阶段错误只返回 shadow_error 给原 HTTP 日志，保留 key/index 并继续原执行 |

真实 journal 模式是启动后不变的字段；新服务只采样一次模式，不增加运行时切换。UUID 仍在原入口、command 转换后生成；record 时钟仍在 V1 adapter 构造时取样，Application 自身不读取时钟或启动任务。

## 错误与 API

- fingerprint 失败保留原 500/错误文本；memory 冲突或孤立 key 保留各自原 409 文本。
- durable 的存储检查、快照或 reserve 错误统一回原 503，不把内部详情变成新的客户端接口；原 warning 仍在 HTTP 模块输出。
- shadow 失败仍在接受 info 之前输出原 warning；record/index 不回滚，保留旧模式行为。
- 原 Created/Duplicate/Conflict、TradeResponse 8 字段、GET/list、批量整体 202/per-wallet 结果、pending 202/完成查询保持。
- 旧 hash/原 JSON、字面 key、分支防碰撞、schema、拒绝记录路径均不改变；没有重写或重发历史订单。

## 新增测试

`application/trade_admission/tests.rs` 6 项，仅使用项目接口和 fake，不含钱包、SDK、数据库或 transport：

1. memory 成功的完整事件序列，0 snapshot/check/mirror。
2. 已有/缺失索引的 memory duplicate，只查原索引，不创建 record/JSON，不修改原 duplicate 标志。
3. fingerprint 失败与内存冲突都在索引改变前结束。
4. durable Created/Duplicate/Conflict/Error 的事件顺序、原签名保留和尝试索引删除；明确禁止额外 memory hash 路径。
5. durable 存储检查先于 snapshot，两阶段失败都清理索引且保持 PersistenceUnavailable 分类。
6. shadow 在成功/存储/snapshot/mirror 各阶段的顺序，错误保留内存接受，不回滚。

fake 的拒绝记录/update/query 方法全部是 panic 标记，保证应用接受不偷偷调用额外存储或执行状态操作。

V1 adapter 另有 1 项：18 个既有接受样本的 record 字面字段与旧 TradeStore 构造一致（时钟值独立采样，仅比较同 record 的 created=updated）；hash、分支位、字面 key 和 JSON 与旧 codec 保持。

原 48 项真实 Router 合同继续覆盖实际默认/显式身份、两 method/方向、memory/durable、成功/重复/冲突、批量、并发/pending 和 503；原 37/720 组字节与 normalization/hash/恢复/执行计数回归全部保留。

## 验证命令

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：315 passed、0 failed、无编译警告。没有新依赖/迁移；vendor 与现有迁移文件未改，原 release 未构建或替换。

## 下一批 TS-02-C4

2026-11-13 续作：普通已校验结果已改由 Application 拥有并使用项目 Address，原 parser 留在边界，新增 3 项、全套 318 项通过，见 `VALIDATED_TRADE_MODEL.md`。完整 Config/validator 输入和 rejected 路径继续后续；以下保留本批要求。

1. `validation.rs` 的 wire validator/NormalizedTrade 仍使用 SDK Pubkey，API rejected 分支也仍在旧入口；不能宣称所有校验/接受已完成模型隔离。
2. 先审查 NormalizedTrade 全部消费者/构造点，将普通已校验结果归项目 Application 并使用自有 Address，旧 validation 路径可重导出兼容。
3. 初步保留旧 SDK Pubkey 解析在 wire 边界，然后转换字节；不顺便替换解析算法或改地址长度/错误顺序，避免 SDK 升级隔离与行为修复混杂。
4. 用旧 18 成功/19 拒绝、hash/入库字节与真实 HTTP 合同保护转换；进一步收窄 validation 配置与拒绝编排另行分步，不扩大本批范围。
5. 普通资金路径发送前签名账本、完整 VerifiedRoute、协议扩展/报价、SDK provenance 和真实链上/部署仍独立推进；新接受服务不是资金执行/最终性证明。
