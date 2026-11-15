# TS-02-C7 拒绝订单记录与持久化边界

日期：2026-11-13。状态：新增 6 项离线回归，全套 334 → 340，通过，无编译警告。未升级 SDK/依赖、构建 release、部署/重启、接触生产配置/钱包/数据库、访问真实 RPC 或发送交易。

## 拆分前证据

先在原真实 Router 路径增加并通过两项基线，再修改生产源码：

- Memory/Shadow/Durable 下重复无效请求保持独立 request_id、原 8 字段响应和 400；无签名、不执行、不保留正常幂等键，修正后可用同一 key 成功接受并重放。
- 关闭临时 Durable SQLite 连接或注入无存储 Shadow，仍返回原拒绝 400 和输入错误；内存证据保留，不误返回提交失败或调用执行后端。

上述成功执行只使用 fake backend 与合成签名，不创建 SDK 钱包或访问节点。

## 已实施

- `application/ports/trade_admission::RejectedAdmissionPayload` 只提供旧记录工厂与惰性拒绝快照，不提供普通 key reservation 或执行能力。
- `application/trade_admission::reject` 经已有 `TradeRecordIndex` 执行原两次写入：Accepted 初始记录 → 设置原错误与 Rejected → 采第二次时间 → 覆盖最终记录。时钟由调用边界注入，不读取 AppState、DTO、SDK 或全局配置。
- `application/trade_admission::mirror_rejection` 独立执行：Memory 直接返回；其他模式先确认存储可用，再创建拒绝快照，再调用原 journal。错误交回 Handler，仍只记录日志，不删除内存证据或改变拒绝响应。
- V1 record/JSON/hash/key 的 DTO 映射留在 `adapters/persistence/trade_admission::RejectionPayloadV1`，接受与拒绝复用相同的原记录字段工厂；没有重新执行当前 validator。
- HTTP 保留身份先于校验、错误文本、日志字段及其相对位置；不再调用具体 `TradeStore::rejected` 或 `persistence.mirror(record, DTO, None)`。
- 原 DTO TradeStore 构造器、Persistence reserve/mirror 桥接已无生产调用，保留为 `cfg(test)` 旧行为 oracle；生产 TradeStore 不再依赖 wire TradeRequest。旧 HTTP 路径、序列化与持久化格式不变。

没有“修正”原短暂 Accepted 状态，也没有把拒绝订单变成有效交易的 reservation。原 `__rejected__:<request_id>` key、来源元数据和原始 JSON/hash 保持。

## 验证

新增 6 项：

1. 拆分前的真实 Router 重复拒绝/同 key 成功与重放，3 种模式，核对 SQLite key/hash/JSON 及零执行。
2. 拆分前的真实 Router 拒绝写库故障，保持 400 与内存记录。
3. 项目 fake index 的两次写入、第二时钟采样和 Handler 日志位置。
4. Memory 模式即使快照/存储均故障也不触碰这些入口。
5. Shadow/Durable 的存储、快照、mirror 故障优先级和留证，无正常 reservation、重试或状态删除。
6. 19 个原拒绝样本的新/旧 record、错误、默认执行方式与拒绝快照字节对比。忽略非确定的真实时钟值，但 fake clock 另验证顺序与精确值。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet rejection
cargo test --locked --offline --quiet rejected_v1_factory
cargo test --locked --offline --quiet
git diff --check
```

结果：精准 5 + 1 项、全套 340 项通过，无编译警告。既有 37 组入库、720 组记录、normalization/hash 与 loader/HTTP/恢复合同未更新期待值。manifest/lock/release 指纹保持；vendor、原迁移与运行配置未改。

## 下一批

TS-02-C8：Warmup 仍在兼容边界返回 SDK Pubkey，resolve/hop 的 DTO 仍直接调用 SDK parser。将剩余只读输入/结果改为项目类型并复用原 parser port，保持 Warmup 的非空/Quote alias/政策，与 resolve/hop 的纯语法解析不同；不改变寻池、缓存、超时、RPC 顺序或 SDK loader。

发送前签名账本、签名能力硬隔离、VerifiedRoute/报价/协议扩展证据、SDK provenance 和部署验收仍未完成。本批不是完整重构或资金安全目标的最终验收。
