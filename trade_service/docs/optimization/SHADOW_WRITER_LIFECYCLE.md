# NP-03 原 Shadow Writer 生命周期

日期：2026-11-15。沿用原 NP-03 / TS-09，不新增队列、数据库、迁移或幂等权威。
源码与受限 test check 已完成；新增 17 项 Rust 测试尚待本批独立源码 CI 实际运行。

## 实际收口

- 原 `TradePersistence` 的单个 1024 项 mpsc writer 移入其私有 `persistence/shadow.rs`。
  同一个原 pool、原 Insert/Update 及 FIFO 调用，仍同步 `try_send`，不新增后台 worker、
  等待队列、批处理或重试。私有组件不进入 API/Application/Domain。
- 原 SQL helper、字段/hash/JSON、字面 key、初始化/recovery、Shadow 的 WAL/Normal、
  Durable 的 Full、连接上限与原错误前缀不改。health 仍为原最后写入结果，不改为实盘总闸门。
- 关闭和 enqueue 共用进程内闸门；sender 不外借或 clone。关闭后只处理此前接受的 FIFO，
  关闭本身即最终入队边界，不需要第二个 flush 队列或另发一次写入。
- JoinHandle 留在原 completion 中按引用等待。超时、取消等待者或竞争等待锁的预算到期，
  都不丢弃原 handle、不 abort writer、不重开队列；之后仍可等待同一任务。
- 正常退出及任务异常结果会保留；重复等待不重复写入，错误不能在 handle 取走后变成成功。
  Drop 外部 Store 会关闭 sender 但不 abort 已接受写入；进程结束仍可能中断，不承诺断电安全。
- accepted/completed/failed/rejected/overflowed 只为进程退出诊断，不是订单状态机。
  历史入队拒绝/SQL 失败及计数溢出均不能报告完整排空，即使后继成功使原 health 恢复。
  writer 异常退出后已知不可用会保持 health=false，不将错误写成订单 Failed 或发起补单。

## 原服务退出接线

main 留住原 `state.persistence` Arc；普通任务 drain、原刷新/观测停止并 join 后，
同一 shutdown helper 等待原 Shadow writer。普通任务未完成或其他资源报错也会执行该步骤，
再返回错误；不把某一清理错误当作跳过其余资源的许可。

新增 `TRADE_SHADOW_DRAIN_TIMEOUT_MS`：默认 5000，范围 1..300000 毫秒，
与原普通任务退出预算独立，包含异步 join 锁的等待。它不是整个 HTTP/进程总截止时间，
也不能抢占同步短临界区或操作系统调度。README 已解释，未改线上配置。
Memory/Durable 时该步骤为 no-op：不创建队列、不关闭 pool、不改变原 reserve/duplicate 行为。

## 针对性合同

`persistence/shadow/tests.rs` 十二项：

1. FIFO 及最终更新、原 key/hash/JSON/签名字段逐项一致。
2. 实际单连接池占用造成超时，原 handle 留存，释放后只处理原写入一次。
3. 取消等待者后原 writer/handle 保持，之后完成原写入。
4. 竞争等待的预算包含原 join 锁；没有创建第二次执行。
5. SQL 写失败后原 health 可恢复，但退出仍报 failed，不能掩盖历史失败。
6. 原 1024 项满队列 `try_send` 拒绝、错误前缀及后继 health 成功不改变 rejected 证据。
7. Drop 最后 Store 后原 FIFO 仍可继续，未主动 abort。
8. 从未 poll 的原 writer 被取消后重复等待仍失败，不假装干净完成。
9. 关闭后新 Insert/Update 拒绝，无新 SQL 工作。
10. Memory 不碰数据库、Durable 原 reserve/duplicate 在 no-op 后保持。
11. 隔离临时文件重开保留终态行、key/hash/JSON；不恢复或重放任何交易。
12. 诊断计数溢出不能报告 clean。

原 production shutdown helper 四项新增：等待 Shadow 完成才成功、写失败不改变原父记录、
普通工作未完成仍清理 Shadow、原 refresher 失败仍清理 Shadow。
另有配置一项，验证独立默认值/范围；不改全局 env，不读取真实钱包、节点或生产库。
CI 新增 `shadow-drain`（十三项含配置）/`shadow-shutdown`（四项），保留原全部专项、
完整并发/串行、类型/别名 87 字段门禁和 source provenance；不套用父快照的 1498。

## 明确未覆盖

成功只证明旧 SQL 调用成功并排空此前接受的队列，不等于额外 fsync、断电/进程强杀、
Shadow 具有 durable/exactly-once、所有生产者和 SDK 内部 worker 都已停止，或所有业务已最终成功。
若普通任务 drain 已超时，后继生产者仍可能被关闭队列拒绝；原普通任务报告继续使退出失败，
不能据此释放 owner、换 key/路线/backend 重签或补单。Shadow 仍仅镜像，不成为资金决策权威。

NP-01 全 backend/mode/协议/方向授权与签名账本、NP-03 全局资源/配额与其他任务、
NP-04 其他业务合同适用性、NP-05 可信来源/升级演练及真实生产退出/恢复仍按原计划保留。
没有 SDK/依赖/vendor 升级、release、部署/重启、生产数据操作、真实签名广播，
不修改 Copy/Bridge/sdk9，不恢复 PumpAPI 新交易对接。
