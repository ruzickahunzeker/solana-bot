# TS-02-C2 普通记录与 Journal Port

日期：2026-11-13。状态：新增 7 项离线测试，全套 301 → 308，通过，无编译警告。未升级 SDK/依赖/vendor、构建 release、部署/重启、读取生产钱包/数据库或发送交易。

## 已实施

- `application/trade_record.rs` 拥有 `TradeRecordV1`，保留原 14 个字段、字段顺序、serde/Clone 行为、u128 时间和全部旧状态。
- 它是冻结的普通订单行/读模型，不是新的领域交易聚合，也不是有效签名、资金授权或最终性证明。
- `models::TradeRecord` 重导出同一类型，旧调用方无需转换；API `TradeResponse` 的原投影保持，GET/list 继续使用原 V1 记录形状。
- 持久化核心、TradeStore、Executor 与 API 转换显式引用内部记录归属，不再从 wire models 取得记录定义。
- `application/ports/ordinary_journal.rs` 定义项目 `TradeJournalMode`、`TradeReservation` 和 `OrdinaryTradeJournal`：typed reserve、两类 mirror、update、read/recent。接口不导入 API DTO、NormalizedTrade、SDK、Axum、SQLx 或 Config。
- `persistence::DurableReserve` 保留为 `TradeReservation` 的旧路径别名，三个分支与语义不变。
- `adapters/persistence/ordinary_journal.rs` 将现有 TradePersistence 适配到 port，模式映射在 adapter，不泄露配置枚举或 SQLite handle。

没有仅放置一个未使用的 trait：旧 codec 的 reserve/mirror 已经走 port；Executor 的持久化 helper 只收 `&impl OrdinaryTradeJournal`；HTTP durable GET/list/等待查询调用 port。它们仍使用同一个现有 TradePersistence 对象，没有增加 AppState 的存储副本或新的缓存/后台状态。

采用静态 trait/impl Future，与既有 journal 端口方式一致，没有为这层新增 Box/dyn 调度或网络/数据库查询。

## 兼容证据

先在旧 `models::TradeRecord` 上捕获：9 Pool × 2 Side × 2 method × 10 status × 2 optional/时间形态，共 720 个字符串序列化样本；逐个直接反序列化再序列化，核对原时间值。

```text
10fcd4f5bc6dfb242d926533a0f6651728d6e73877a474bb9c4d935d58c89178
```

涵盖 null/有值、空签名、引号/换行、duplicate 标志、u64 边界以上和 u128::MAX。这里是 serde 读模型边界，不代表 SQLite INTEGER 接受这些极值；原 timestamp→i64 拒绝规则没有改变。

模型移动后该指纹保持。此前 37 组入库 hash/JSON/key 字节指纹、98 组结果分类指纹、旧 normalization/hash/API/恢复与协议参数回归都不重生成期待值。

## 新增测试

| 位置 | 数量 | 证据 |
| --- | --- | --- |
| `persistence/record_contract_tests.rs` | 1 | 迁移前后 720 组字节/roundtrip 指纹，继续从旧 models 路径调用以验证重导出 |
| `executor/journal_tests.rs` | 3 | 无记录时 0 写/0 mode 查询；成功写不探测 mode；durable 错误停止、shadow 错误按旧规则继续，失败调用不自动重试。fake 的 reserve/mirror/query 是 panic 标记，可发现意外副作用 |
| `adapters/persistence/ordinary_journal.rs` | 3 | 真实 memory/shadow/durable 模式与 reserve/mirror/update/read/recent；duplicate/conflict/拒绝 key 隔离及排序/limit；原签名 COALESCE、unknown/error、查询 duplicate=false 和关闭临时 pool 的原错误透传 |

数据库测试全部使用新的 `sqlite::memory:`；shadow 只运行原测试进程内 writer，并有界等待写入完成。fake journal 不持有数据库、SDK 或钱包。

## 保留行为

- memory 的无数据库读错误、mirror/update no-op，shadow 原异步行为、durable 原保留和查询不变。
- 执行前写入的原日志模块/文本和先后顺序保持；只有失败后才读取 mode，durable 返回 false、shadow 继续；结束写失败不自动再写/重发。
- SQL 仍保留已有 signature；未把读模型的 transient duplicate 字段写入数据库，原 read_record 的 duplicate=false 保持。
- 旧 hash、原 JSON、表/schema/迁移、时间校验、重启 unknown 规则均不变。
- 没有增加普通交易发送前签名账本。这个 ordinary row journal 与已有独立 Quote 的 submission journal 不同，不可混称安全目标已完成。

## 验证命令

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：308 passed、0 failed、无编译警告。Cargo.toml、Cargo.lock、现有 release SHA-256 与前批一致，vendor 与现有迁移文件不变；未构建/替换二进制。

## 下一批 TS-02-C3

2026-11-13 续作：已校验接受编排、内存端口和惰性 V1 factory 已实际接入，新增 7 项、全套 315 项通过，见 `VALIDATED_TRADE_ADMISSION.md`。wire validator 的模型与 rejected 路径仍按后续分步，以下保留本批执行要求。

1. 普通接受仍在 HTTP 函数内编排，使用 wire Request、NormalizedTrade、AppState、具体 TradeStore/IdempotencyStore。当前端口不等于这部分已经脱离 transport。
2. 为内存订单索引/幂等保留提供项目端口，再迁移“参数已校验后的接受/重复/冲突/保留”流程到 Application；鉴权、JSON 提取、身份先于校验的顺序继续保持。
3. 需要 V1 payload/draft adapter 或惰性 factory：保持 memory 原来只在需要时算 hash、不构造持久化 JSON，shadow/durable 原 record 创建、JSON/hash 和写入顺序；不能提前序列化来简化新 service 而改变错误/副作用顺序。
4. 接受结果与 HTTP StatusCode/TradeResponse 分离，应用返回项目 decision，API 再投影。日志、任务启动、响应等待和批量 per-wallet 语义分别核对，不把所有旧副作用一次混入重构。
5. 继续使用真实 Router、typed 存储、故障/call-count 和冻结字节合同守住迁移。wire validator 的链类型解耦、签名前账本、VerifiedRoute、协议/SDK 真实验收仍独立推进。
