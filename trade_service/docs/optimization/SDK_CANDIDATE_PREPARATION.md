# C3b1 SDK 候选元数据与 Prepared 源码验收

日期：2026-11-15。持续目标已恢复 active；接续 C3a，不重复操作分型。
当前源码/离线验收已通过，不是 SDK 已有完整授权/广播前账本或允许上线的声明。
不升级 SDK、不部署/重启、不访问生产钱包或数据库、不签名广播。PumpAPI 开发继续暂停。

## 源码与接线

- `domain/sdk_candidate.rs` 定义项目自有的版本化候选数据，不引用 SDK 类型或私钥。
  包含原池账户/Mint/程序、输入账户/精度、整数余额/比例/输入/min_out、具体 relay/tip 接收者、
  blockhash 与执行标志。消息摘要只证明字节绑定，不证明池账户、费用或余额仍然可信。
- 实际 `DammSellCandidate` 构建已生成该元数据；metadata adapter 复用原参数映射、协议指令、
  CU/tip 组装和消息编译重建原候选，拒绝仅更新摘要而不匹配账户/金额/费用/接收者的记录。
  这仍是 nominal consistency，不是独立链上授权；不包含 signer/RPC/sender。
- `application/sdk_preparation.rs` 绑定原 Durable SDK DAMM 卖出的钱包、命令、比例、滑点、tip bits
  与 MEV 选项。原 owner 不克隆/反序列化，不建立第二个执行或恢复 owner。
- 原 claims 增加 009 nullable `sdk_prepared_json`，不重写旧列/旧 hash/JSON。SDK 行只能为
  无 metadata 的 `claimed`，或完整 `prepared`；混合 Quote、部分字节/摘要、签名/终态字段拒绝。
  校验器自行检查 schema/操作类型，并要求持久化 JSON 为原编码器的规范形式。
- `prepare_sdk_candidate` 只在原 owner、原父行 BuildingTx、原 claimed 行上单次 CAS 写入；
  事务内重新校验及精确读回，另有只读 `read_sdk_candidate`。旧聚合 journal 和恢复扫描不扩展为 SDK。
  prepared 元数据不产生 signing、signed、发送许可或自动重试；Unknown 读回不恢复执行身份。

候选构建已使用元数据，prepared 写入/读回的隔离运行测试已通过，但尚未接正式 SDK 编排。
不能把这些局部证据当成正式签名授权，更不能通过 fake 调用消除未使用告警后宣称接线完成。

## 已验证证据

| 检查 | 结果与范围 |
| --- | --- |
| 云端完整 Rust 验收 | `35e4362` / Actions `43667516925`：完整两线程 1321（123.08 秒）及串行 1321（221.71 秒），零失败/忽略/过滤；无生产数据 |
| 云端专项 | SDK prepared 11、SDK candidate 15、DAMM candidate 43、DAMM probe 48、HTTP 200、参数 3、lanes 19 全部通过；新增 17 项已实际运行 |
| SQLite DDL 检查 | `python3 scripts/test_sdk_candidate_migration.py`：6 项通过；仅内存/临时文件库，CI 中也通过 |
| Rust test 配置类型检查 | `bash scripts/dev_check.sh check`：通过，约 15 秒，最大 RSS 1,169,156 KiB；没有链接/运行测试 |
| 正式 bin 类型检查 | `bash scripts/dev_check.sh bin-check`：通过，约 11 秒，最大 RSS 972,892 KiB；没有生成/运行 release |
| 验证入口 shell 测试 | 成功、失败、超时、重复锁、状态/旧 running 标记与 bin-check 路径通过 |

SQLite 检查读取实际 009 DDL，验证旧 16 列值/SQLite storage class、父记录、索引和外键保留，
nullable 默认 NULL、重复执行、marker 写失败时 ALTER 回滚、未标记同名列拒绝及文件重开。
其 008 形状只用于构造 fixture；Python 中的事务模型不等于 Rust 迁移器或 SQLx 调用。
特别是 `execute()` 与 `executescript()` 的提交行为不同，不能用后者冒充事务化 ALTER 验证。

日志分别保留于 `target/dev-checks/20261115T013226Z-11834/`（test 类型检查）和
`target/dev-checks/20261115T013508Z-12729/`（bin 类型检查）；它们及临时库不提交 Git。
正式检查当前有七组警告：原五组，另两组为 SDK prepared 检查 helper 与写入/读回方法未接生产调用；
没有新增 allow(dead_code)、假调用或自动接入 signer 来隐藏它们。

## 尚未通过的验收

- 当前完整源码已在仅回环/env-i/offline 环境运行，详见 `GITHUB_ACTIONS_VALIDATION.md`；
  GitHub-hosted runner 不是交易服务器、真实 RPC 或断电恢复环境，不能替代生产验收。
- prepared 写入/读回尚未接正式 SDK，完整独立授权、signing/signed/receipt CAS、受控签名和
  原字节发送仍未完成。现有七组警告未抑制，其中两组正说明这项接线仍待继续。
- 本机 3.6 GiB/无 Swap 不再重复大链接；后续里程碑沿用已授权的云端验证，不反复询问环境。
- 009 的生产规模/锁等待/临时空间、备份恢复、旧二进制兼容和调用方验收均未完成。

## 继续顺序

1. C3b1 源码及离线回归已验收；保留 run/commit 与原证据，不重复实现本子项。
2. 补最终原池/资产/Token 扩展/费用版本/整数预算/权限及有效期的独立证据；元数据与重编译结果
   不能代替它们。规范 JSON 也不认证 metadata 数值未被另行修改，读回不证明快照仍新鲜。
3. 在原 owner 下扩展独立 SDK 授权状态/CAS、受控签名、签后 durable 落盘/独立读回/原字节发送；
   原签名只读恢复，不从 AlreadyHeld 重造 owner、不以新 key/新路线补单。
4. 持续完成买入/其他协议与 backend/mode、SDK worker/多签名、共享 RPC/资源/依赖、双 fake backend、
   SDK 来源/升级演练与真实环境门槛。未完成项不因本机编译条件不足而删除。
