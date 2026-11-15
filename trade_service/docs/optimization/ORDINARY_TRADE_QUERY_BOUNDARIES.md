# NP-02 / TS-08-C 普通订单查询与结果等待

日期：2026-11-13。状态：本阶段源码/离线验证通过；未部署。只改 Trade Service，不恢复 PumpAPI 对接、不升级 SDK/依赖、不读取生产数据、不签名或广播。

## 实际边界

| 位置 | 职责 |
| --- | --- |
| `src/application/ports/trade_reads.rs` | 仅暴露按请求 ID 读取和最近订单读取，使用项目拥有的 V1 read-model；无 SQL、SDK、DTO、钱包、reserve、update 或执行接口 |
| `src/application/trade_queries.rs` | GET 查询编排、列表默认/边界、原提交结果等待与 pending 政策；不获取 AppState 或具体存储 |
| `src/adapters/persistence/trade_reads.rs` | 借用原 TradePersistence 与 TradeStore；Durable 复用既有 ordinary journal 读取，Memory/Shadow 读取原索引，不新增 SQL 或状态副本 |
| `src/main.rs::AppState::trade_queries` | 请求作用域组合工厂；返回项目 service/不透明只读 source，不给 Handler 暴露 adapter 类型。不创建连接、不复制账本、不长期捕获第二份存储句柄 |
| `src/api/handlers/trades.rs` | GET 仅鉴权、提取和响应映射；接受流程调用同一 service 等待结果，保留原 spawn、日志和响应选择 |

借用工厂直接使用当前请求 AppState 的原两个 handle，不增加全局 service 字段或第二个缓存。生产 AppState clone 与离线 fixture 替换 persistence 后均读取对应原状态，不依赖 Config 的 persistence mode 副本；按实际 TradePersistence 选择权威。

## 原语义保留

- Durable 的 DB 结果为权威；缺行返回 404，关闭数据库、坏行或其他查询错误返回原 503/prefix；不从内存补一个“成功”。Memory/Shadow 继续读原内存索引，不因数据库有不同快照而覆盖。
- 列表默认 20，范围仍为 1..100，排序与完整 V1 字段继续由原存储实现负责；不新增过滤、别名、ID 解析或脱敏规则。
- 等待值为 0 时只读一次，不先创建 deadline。正值仍先设 deadline，读取完成后判断状态/时间，每次 pending 间隔仍为 20ms；不改 200/202 选择。
- 仅 Accepted/WarmingParams/BuildingTx/Submitting 为 pending。Submitted/Unknown 等停止等待不等于链上确认，不能因此解锁、换 key 或重签。
- GET 查询错误向 HTTP 保留原错误；提交后的等待遇到错误/缺行仍返回 None，由原接受流程保留当时的 accepted 快照。不把两类错误行为强行统一，也不新增广播或日志重试。
- deadline 仍是轮询判断点，不是单次读取的强制超时。慢读取可超过 wait_ms，完成后仍使用该快照；本次不暗改延迟/失败策略。
- 取消等待会 drop 当前读取 future，不派生后台轮询、切数据源或改订单状态。已经接受的原 executor 任务不因 HTTP 等待取消而自动撤销；本轮没有为它增加取消、补单或恢复执行权限。

只读是项目端口的能力范围，不是 SQLite OS 权限沙箱；adapter 内部仍借用既有存储对象，原 executor 可以并发写入它。查询分层不等于普通广播前签名账本、可信最终性或完整 Handler/Application 重构已完成。

## 离线证据

1. 先在旧 Handler 位置运行新增三项 HTTP 基线：三种持久化模式下互相矛盾/独有的 DB 和内存行；105 条记录验证默认/0/1/100/101/usize::MAX 与排序/全部字段；关闭和损坏临时 DB 的 503、鉴权及不回退。
2. 新 Application 八项测试覆盖十种状态、literal ID、完整原行/原错误、列表边界、零等待单次读取、pending 轮询、读取后 deadline 判断、失败/缺行停止及取消 drop。source 不具备写入、sign/send 或替代账本能力。
3. 三项 HTTP 基线在新边界通过；共 11 项针对性测试通过。现有 pending HTTP 合同扩展为 Memory/Shadow/Durable 与 wait=0/1 的六组，继续验证 202、重复请求 200、同一签名/一次执行及完成后 GET。

测试仅使用公共合成记录、内存 fake 和临时 SQLite，没有真实钱包、RPC 或网络监听。最终验证：

```bash
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

默认并发与单线程全套各 789 项通过，HTTP 筛选 158 项通过；生产 check、格式和 diff 检查通过。生产仅保留两条原兼容导出警告（models 领域类型及 persistence 的 request_hash），无新增抑制或 ignored。vendor、原 migration 无 diff，本轮没有新增迁移或依赖，以下指纹与上阶段相同：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

## 后续

普通接受仍保留私有的 API orchestration 与 V1 payload adapter，钱包管理、池/状态 Handler、infrastructure/依赖规则、双假后端合同仍待收口。SDK/legacy 组合模拟/签名、lane 回退、多签名及发送前 journal 风险按 `ORDINARY_SDK_SUBMISSION_SAFETY_WORKPACK.md` 推进，不用本次只读测试冒充已修复。
