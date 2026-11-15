# NP-03 普通后台交易任务生命周期

日期：2026-11-15。沿用原 NP-03 / TS-09 资源所有权任务，不另立项目或资金权威。
当前源码、受限类型检查及独立源码 CI 验收完成；新增 17 项 Rust 测试已实际运行。
验收提交 c6f2745 / Actions 43718342603 / artifact 14163942665；不是全服务或生产退出验收。

## 原问题与接线

原 Handler 在 Created 后直接 `tokio::spawn`，HTTP 返回/取消后 main 无法知道任务是否仍在执行；
原 signal 又先停止 blockhash/观测资源，早于全部普通后台执行结束。

- `bootstrap` 为原 AppState 创建一个 `infrastructure/trade_tasks::TradeTasks`；
  Handler、AppState clone、main 使用同一内部 Arc，没有新 worker、运行时、缓存或连接池。
- 单笔和批量接受共用原 Handler。鉴权/空批校验后、第一次接受 await 前登记线性 ticket；
  错误、拒绝和 duplicate 只释放此进程内登记，不新增 executor。
- Created 将同一 ticket 移交原任务及原 request_id，只 spawn 原 executor 一次。
  ticket 不含 TradeCommand、钱包、Keypair、RPC、ExecutionOwner 或 journal。
  原金额、规范化/hash/JSON、保留、签名/发送和恢复实现不改。
- task 完成后去掉进程内登记；panic/异常丢弃（含从未被 poll 的任务）记为 abandoned，
  不伪装干净排空，不改订单为 Failed，不释放资金占用或发起补单。
- `Notify` 先登记再检查任务表，避免最后一项完成造成丢唤醒；多个等待者共享同一表。
  Drop 外部持有者、超时或取消 drain 等待者均不会主动 abort 交易任务。

## 停机顺序与兼容

1. 原 Ctrl-C/SIGTERM 信号关闭登记闸门，Axum 使用原 graceful shutdown；
   新的普通交易工作在原接受/写库前返回 `503 {"error":"trade service is shutting down"}`。
   这只改变停机窗口；正常运行的校验顺序、请求/响应、状态等待与鉴权不变。
   批量仍为原 202/逐项错误形状，GET 查询不增加登记闸门。
   503 不能证明同 key 的先前请求未接受/未发送；恢复后继续使用原 key，只读原结果，不能自动换 key 补单。
2. 信号前已取得 ticket 的接受流程仍可派发原请求；登记到派发之间计数不归零。
   `close` 不重开、不释放 owner；重复 POST 的正常幂等合同不改。
3. `serve` 返回后 `service_shutdown::finish` 等待已登记普通工作，期间保留原刷新/观测资源。
4. 干净结束或预算耗尽后停止/await 原 blockhash refresher 与可选 observation worker，
   任一返回错误仍 join 另一个。超时/abandoned 会报告并返回退出错误，不声称交易未发送。

新增启动配置 `TRADE_TASK_DRAIN_TIMEOUT_MS`：默认 30000，范围 1..300000 毫秒。
该预算只从 HTTP graceful shutdown 返回后开始，不是总进程截止时间或交易有效期。
README 已解释；不修改线上 .env、systemd 超时或服务状态。
报告只含 active/admitting/abandoned 计数及最多 32 个原 request_id，完整跟踪表不因截断而丢项。

## 验证范围

- 内核八项：未派发登记释放、停止前 ticket 的延迟派发、外部句柄释放不 abort、
  最后完成唤醒多个等待者、取消 drain、不把 panic 当干净退出、运行时销毁未 poll 任务、
  有界报告但不遗忘其余在途任务。
- 真实生产 shutdown helper 四项：原刷新/观测晚于普通任务停止、超时仍不 abort 原任务、
  refresher 错误仍 join observer、abandoned 不能报告干净退出。
  仅必需的回环 RPC 候选、公开地址 fake reader、无签名记录的隔离内存 SQLite；无钱包、真实节点或广播。
- 原 HTTP Router 四项：停机鉴权/批量/只读、Memory/Shadow/Durable 的 202 后继续执行、
  请求取消不取消原 executor、duplicate/拒绝不新增后台执行。使用原 fake SDK port，不恢复 PumpAPI 开发。
- 配置一项：默认值及上下限；无全局 env 变更，不读取生产配置。
- 现有三层类型门禁增加 TradeTasks/DispatchTicket 两个具体运行时类型：共 28 个选定类型，
  加 outside-alias 后预期三个层 87 字段拒绝。不能因此宣称 API/全部依赖/不透明能力检查完成。

本地初次有效 test check 38 秒/RSS 2,266,392 KiB，最终 test check 9 秒/RSS 1,241,036 KiB，
production bin-check 12 秒/RSS 994,552 KiB 通过；
后者仍为七组既有警告，未压制或删除原兼容代码。初轮新增测试的 recent(limit) 参数错误已
修为项目 API 所需 Option；没有改变生产 API 或缩减断言。完整 Rust 运行只在获准的源码/
公开数据、仅回环 GitHub Actions，新专项加在原全套/门禁之前，不借用父快照 1481 的结果。

### 本批验收证据

- `c6f2745e2fee315d454b0c7327d1469f6c1fb441` /
  [Actions 43718342603](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43718342603)
  终态 success，artifact `14163942665` 下载核对；48 步骤全部 exit=0。
- 生命周期专项 12、shutdown helper 4、配置 1 全部通过；完整并发 1498（134.03 秒）/
  串行 1498（244.95 秒）零失败/忽略/过滤。独立比较完整原始测试名集合一致，父批 1481
  全部保留，新增集合恰好等于三项专项的 17 个入口，不只比较通过数字。
- 原源码/恢复 Clippy JSON 均 build-finished success；负例为预期 exit=101，独立核对
  原始 spans 确认 87 个不同字段拒绝，包含新 TradeTasks/DispatchTicket 和 outside_alias
  在 Application/Domain/Protocol 各一组；未把任意编译失败当作门禁通过。
- identity 与 service manifest/lock/vendor manifest SHA 匹配；源码发布审计绑定
  1029 文件/12,654,972 字节、25 变更路径，其他原源码和依赖不变；真实 main/索引不变。
- 发布后只更新验收记录和 503 的原 key 说明；不为文档单独触发完整 CI，不合并或部署。

## 明确保留的缺口

- 不是全服务任务 supervisor：SDK 内部 opaque worker、Shadow writer、靓号任务、独立资金操作等
  生命周期仍按原 NP-01/03 处理；本计数不能证明 SDK 取消、链上未执行或所有后台任务排空。
- 不处理接受请求在 durable commit 与 Created 交接之间取消的自动恢复，不恢复原 owner；
  签后只跟踪原签名，不能因为超时、panic、HTTP 取消或未查到结果而换 key/backend/路线重签。
- 不提供 HTTP graceful shutdown 的全局 deadline，不调整系统服务超时；进程强杀、刷盘/断电、
  生产迁移/恢复和真实节点另行验收。Memory/Shadow 不因此获得 durable 保证。
- 未升级 SDK/依赖/vendor、构建 release、部署/重启、操作生产密钥/数据库或发送交易。
  Copy/Bridge/sdk9 不变，PumpAPI 新交易对接仍暂停。
