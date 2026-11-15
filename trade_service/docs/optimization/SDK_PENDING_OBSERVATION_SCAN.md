# C3d2 SDK 有界后台只读扫描

日期：2026-11-15。源码及完整离线 CI 已通过：`6c50aca` / Actions `43690435964`，
artifact `14152368482` 下载核对，sdk-scan 16、六项独立 DDL 及全部原专项通过。
完整并发/串行各 1437 零失败/忽略/过滤，含实际 SQLx/worker 和后续 C3e 串联。
不升级 SDK，不部署/重启，不操作生产钱包/数据库，不发送交易。PumpAPI 对接仍暂停。

## 源码接线

原 ObservationPlan/OrdinaryObservationScanner 不增加任务或发送能力，只扩展原 pending port 的
候选查询。原聚合 PAGE_SQL/007 partial index 字节不改；另有 SDK_PAGE_SQL/010 索引，
限定 SDK 直接 DAMM v2 卖出、四种有签名阶段、完整产物列和父行 pending/签名匹配。
SQL 只读两个游标字段，损坏的 nominal metadata 仍由 C3d1 capture 完整拒绝，不把寻得候选当授权。

- 每次两条 SQL 在同一只读事务快照，各使用明确的 INDEXED BY，按同一
  (created_at_ms,request_id) 和同一年龄 cutoff 各读最多 limit+1 个候选。
- 合并最多 2*(limit+1)（最大 130）个轻量游标后排序/查重，再截为全局 limit（最大 64）；
  has_more/cursor/环回语义保持。不用 OFFSET、全表读取或并行加倍 RPC。
- 仍使用一个 scanner/worker、原共享 read runtime/HTTP client/观测 profile、节流/429 冷却、
  每笔只读 deadline、20 秒批量预算及 shutdown。取消不跳过未完成项，也不新建执行 owner。
- claimed/prepared/signing、无签名/半份证据、已最终化、错误父行/协议/方法及过新的行不进入
  SDK 候选。损坏候选 capture 拒绝且零 RPC，游标推进后继续处理本批后续原订单。
- 原默认 ORDINARY_OBSERVATION_ENABLED=false、配置字段和合法区间不改，不修改 .env。
  部署后仍需明确启用原开关；本轮没有启用或运行生产 worker。

## 010 迁移

仅新增 `idx_ordinary_sdk_observation_scan`，不重建表、不改原值/存储类型/键/签名/密文，
也不替换 007。初始化按项目既有规则执行迁移，即使 worker 关闭也会检查已标记索引，
因此未来部署必须包含独立生产迁移窗口，不能把开关关闭误认为不访问 schema。

专有迁移 guard 在原事务/迁移锁内检查标记与 schema，使用临时 SQLite 解析可信 DDL，
核对实际 type/table/SQL、非唯一 partial 属性及 index_xinfo 的列/顺序/排序/排序规则/rowid。
未标记而索引已存在、已标记却缺失/结构变化/错误表均拒绝，不静默采用或修复。
创建与 marker 同事务，marker 失败回滚；扫描缺索引直接报错，不退回全表或不完整的单类结果。

## 验证归属

本批 16 Rust：迁移 5、分页 4、SDK scanner/worker 6（含子进程入口）、两类真实原库混合 1。
覆盖旧行/类型/父行/007 保持、幂等/并发初始化、异常标记/索引、marker 回滚、两类分页同毫秒
排序/共享上限、hint 排除、EXPLAIN 两个索引与无临时 DB 排序、损坏候选无 RPC、四类 SDK 原
阶段最终化且不重复查询、取消/文件重开以及真实 worker factory→共享 runtime→回环 HTTP。
混合测试在原聚合与 SDK 的执行对象/owner 销毁后，每页一个，按同一游标依次最终化；原产物
和后端事件不变。真实 factory 的子进程去除代理，只接受原签名的 getSignatureStatuses；
默认关闭无连接，启用后一次原签名查询、无重签/广播、停机后关闭测试服务器。

最终 test check 约 8 秒/RSS 1,188,120 KiB；bin check 约 5 秒/RSS 710,956 KiB，原五组生产
警告保持，无新增抑制。独立 Python SQLite 3.37.2 六项、workflow policy 七项通过；
实际 Rust 用例与完整两线程/串行套件必须在现有源码专用隔离 GitHub Actions 执行。
Shared runtime/RPC 配额/线上配置、SDK/manifest/lock/vendor、旧迁移与 release 未改。

本批初轮 `57c75c5` / Actions `43688840398` 的专项 16 已通过，完整并发 1431/1，
串行未运行；旧 005 schema 夹具未先删除新 010 索引便删除其引用列，导致历史基线重建失败。
后继 `6c50aca` 仅修测试的索引/marker 清理并补恢复断言，没有修改生产迁移验证或跳过测试。
该后继的 observation-migration 1、sdk-scan 16、完整并发 1437（128.56 秒）/串行 1437
（235.46 秒）已通过，HTTP 200/参数 3 与全部 steps exit=0，原身份指纹匹配。

## 剩余门槛

本批与 C3e 的 Default 完整准备/受控执行离线链路已验收，不重复实施或重跑旧任务；
继续剩余 SDK 方向/协议/模式、实际 provider、资源/依赖隔离和升级/新协议演练。
真实节点格式/配额/延迟、生产迁移/空间/锁/刷盘/断电、调用方及二进制回滚读取兼容仍需独立
授权验证。不将本子路径的源码或离线结果当全普通交易/全协议/实际发布完成。
