# C3d1 SDK 原签名只读恢复核心

日期：2026-11-15。源码与受限类型检查通过，18 项新增 Rust 用例待 CI。
不升级 SDK，不部署/重启，不访问生产钱包/数据库，不发送真实交易。PumpAPI 对接保持暂停。

## 范围与实际入口

- `RecoveredOrdinaryObservations::capture_observation` 按原 claim 的 operation 严格分发。
  ordinary-sol-buy 仍使用原 SOL/Quote 校验；ordinary-sdk-trade 只接受原已支持的
  Durable Live SDK 直接 DAMM v2 卖出，不伪造 BoundQuote 或重建执行 owner。
- SDK 恢复只允许 signed/broadcasting/submitted/unknown 且原签名/完整 packet 已落盘。
  claimed/prepared/signing、签前 failed/rejected、半份证据或其他协议/方向均拒绝。
- 复用原 `OrdinaryObservationContext`，新增私有操作类型绑定；只保留身份/签名/证据指纹，
  不携带 nonce、交易字节、wallet registry、Keypair、发送器或执行许可。
- 通过既有 `reconcile_captured` 查询一次原签名。GET/列表/重复 POST 不增加 RPC/资金副作用。
  007 索引、扫描 SQL 与默认关闭的原 worker 尚未扩大，SDK 自动持续扫描是下一 C3d2，
  不能把本恢复核心或测试调用宣称为 SDK 后台恢复已启用。

## 原身份与历史回放

原库同一事务核对 parent/claim/schema/operation/key/hash/nonce，以及 canonical SDK candidate、
原 unsigned message/digest、已签 packet/digest/规范编码与真实公钥签名。捕获后指纹绑定
原请求 JSON 字节、literal key、钱包、公钥、方向/池/Mint/Pool ID、时间戳和全部不可变产物；
receipt/error/finality 可以推进，不能替换已捕获身份。SQL 与 finality 写回时重新核对同一指纹。

项目 TradeValidator 的只读 replay 方法不接受当前 Config，不重放现行协议/功能开关准入：
显式金额方向、百分比、滑点、费用、MEV、模拟/SDK 方法及多跳字段与候选绑定；历史缺省滑点和
小费只取原签名候选，原 simulate Option、settlement 与规范化地址/MEV 保持 V1 语义。
用原 V1 codec 重算 request_hash，不更新 hash/JSON/oracle，也不返回可执行命令或新 owner。

## 最终性与并发

- 只接受原签名、非零 slot、finalized；复用 006 的原 FinalObservationV1 结构和字段顺序，
  SDK 新记录要求 canonical JSON。原聚合 validator 的校验/顺序、原阶段过滤保持。
- finalized 成功为 confirmed，finalized 错误为 failed，错误先脱敏并受元数据体积上限约束。
  未找到、429/错误、超时、取消、processed/confirmed 以及非最终错误均保持原签名和 pending 状态。
- 原 claims 表 final_observation_json 与父行 status/error 在同一事务 CAS、读回、提交后独立读回。
  不改 stage/nonce/metadata/已签字节；重复相同最终证据幂等，冲突最终证据拒绝。
  无新 DDL/索引/配置。写 parent 失败必须连同 finality 回滚，丢回执不产生重签/重发。
- SDK parent validator 仅有合法最终证据时接受签后 confirmed/failed。迟到 receipt 的原
  Submitting/CAS 门槛仍在，不能覆盖终态。原 owner 已消费，重复 claim 仍 AlreadyHeld。
- Application 原来拒绝签后 failed 的保护保留；只有 journal 的独立 `finalized_failure` 读回
  通过原 owner、完整历史回放与最终证据验证，才可采纳该 failed。端口默认返回 None，
  异常/缺证据不按错误文本或单个 Failed 状态推断链上失败；不改 executor 的 no-SQL-overwrite 规则。

## 新增验证与限制

本批 18 项（16 persistence/recovery + 2 Application）：四类签后阶段只读、五类无签名阶段拒绝、
原默认值/literal key、逐项原身份/请求/候选/签名/最终性损坏、捕获后合法字段变化、
错误签名/slot/commitment/过大 metadata、成功/失败与冲突、非最终/429/None/超时 drop、
事务回滚、receipt 竞争、两 SQLite pool 冲突、写回执丢失、原签名对象销毁后四类文件重开，
以及 Application 采纳合法最终失败/读证据失败时保持未知。
测试只用临时 Keypair、公开 fixture、内存 reader 和临时 SQLite；恢复后通过触发器禁止
新增 claim、修改 owner 或原交易产物。没有运行真实 RPC/供应商或生产数据库恢复。

最终受限 test check 约 9 秒/RSS 1,293,972 KiB、bin check 约 6 秒/RSS 764,880 KiB，
生产原五组警告保持。完整专项/并发/串行测试须在 GitHub-hosted、仅源码/公开 fixtures 的
既有隔离验证链实际执行；当前通过类型检查不代表运行通过。

最近已验收是 C3c2 `fd1c7cb` / Actions `43685088919` / artifact `14150028402` 下载核对，
完整并发 1398（132.54 秒）/串行 1398（235.13 秒）零失败/忽略/过滤，sender 14
含四项真实 signer/ledger/RpcClient 串联通过。该 1398 不包含本批新增 18 项。

## 必须接续

C3d2 新增独立 SDK partial index 与严格迁移验证、两种原 operation 有界归并扫描，
保持同一共享 runtime/观测预算/默认关闭/停止规则；补实际 scanner/worker 的 SDK 临时库恢复。
后续仍需其他方向/协议/模式、完整准备 RPC 集成、全局资源边界、SDK 升级/新协议演练和发布门槛。
生产 migration/刷盘/断电/调用方/回滚、真实节点与 provider 接受需独立授权验收。
旧优化版本的 validator 不识别 SDK finalized 记录；上线/回滚演练必须验证版本读取兼容性，
不因为没有新 DDL 就默认任意旧二进制可回滚读取这些新阶段证据。
