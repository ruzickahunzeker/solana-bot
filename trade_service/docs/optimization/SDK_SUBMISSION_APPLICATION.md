# C3b6 原 SDK 提交的 Application 编排

日期：2026-11-15。源码/受限类型检查完成，新增十项 Rust 用例仍待云端实际运行。
正式 executor 和真实 SDK signer/sender 未切换，不声明完整业务授权或真实节点验收完成。

## 上轮失败与修复

C3b5 提交 `3bd979b` / Actions `43676492857` 终态 failure，artifact `14146766179`
已下载核对：类型检查/测试构建通过，sdk-candidate 专项 14 通过/1 失败，后续专项及完整套件未执行。
失败项是原 prepared-only reader 在新增 signing 阶段仍返回 nominal 候选。

本轮生产 reader 明确要求 `submission_stage=prepared`；签名阶段只能走签名证据读取。
原测试保留 signing 不得读取 prepared 的断言；合法 signing 的 claim 检查改为必须
`AlreadyHeld`，不能重新发 owner。其余七类损坏/部分字段仍必须拒绝，未放松验签或消息绑定。
新增五阶段用例再验证 signing/signed/broadcasting/submitted/unknown 均不能读取 prepared
或降级为 pre-sign failed。上轮 11 项账本用例尚未运行，不能宣称通过。

## 实际边界

- `application/ordinary_sdk_submission.rs` 消费原不可 Clone 的 `ExecutionOwner` 和
  后端私有 Candidate；不新建 Quote、订单键、恢复 owner 或备用后端。
- 新 application ports 只引用项目 command/metadata/record/bytes；没有 SDK 类型、
  Keypair、RPC client 或任意消息 signer。候选与签后 artifact 的内部类型由适配层拥有。
- Sqlite 实现接原 C3b5 的同一账本方法，不复制 SQL 或添加第二个持久化权威。
  增加原表上的 pre-sign failed 与只读 observe，无 migration/配置/公开 API 变化。
- 未进入 signing CAS 的可证失败只能在原 claimed/prepared 阶段记录 failed；
  CAS 已尝试、可能丢提交确认或 signer 已进入后，任何错误不走这个降级方法。
- 聚合结果结构提为项目共有 `JournaledTradeOutcome`，旧 `JournaledSolBuyOutcome`
  仅兼容 re-export，原聚合编排/DTO/状态逻辑不改变。

## 固定顺序

1. 原 owner/命令/operation 绑定，Live 政策和原候选授权。
2. prepare 保存，独立读回逐项比较原 metadata/完整 message。
3. 再查 Live/原证据授权，单次 signing CAS；CAS await 后再次检查。
4. 消费原候选进行受控签名，保存报告签名诊断，核对实际签名字节。
5. 签名落盘、父行同步更新，独立读回比较原签名/原字节。
6. 再查 Live 与保留的原签后 artifact，单次领取已落盘发送字节。
7. 领取 await 后再次检查 Live/原 artifact，仅调用一次 sender；不重建签名。
8. 仅匹配原签名的回执为 submitted，错误/不同签名为 unknown 并保存原签名。
   最终只读取原账本，错误父行或签名后的 Failed 不能覆盖原签名结果。

保存/读回/领取/回执任一步失败或取消均没有内部恢复、循环或自动重发。
读回 metadata 只在已逐项等于原不可变候选后用于证据比较，实际 signer 仍消费内存原候选，
不是从数据库重建可签对象。真实 backend 必须重新检查完整 live 授权与期限；
不能用 metadata hash、存储阶段或 fake backend 的接受代替完整业务政策。

## 验证

十项离线测试用真实隔离 SQLite 账本与两种不同 Candidate 表示的假后端，包含：
完整阶段顺序及发送点原库核对、八类签名前失败、五类 signing CAS/后置授权失败、
十三类签名后失败、三种独立读回变化、错误/不同回执、五个实际取消边界、
错误父行观测、错 owner/候选、五阶段禁止降级。
假后端仅生成临时 Keypair 并离线签合成消息，无真实 RPC/广播；共用一部分测试引擎，
因此不能冒称 NP-04 的所有独立后端/能力/协议矩阵已经完成。

最终受限 test check 约 9 秒、RSS 1,178,828 KiB，bin check 约 5 秒、RSS 677,464 KiB；
fmt/diff、policy 七项、SDK 来源检查通过。生产目标 15 组警告，包含尚未接正式入口的代码，
不压制警告或以假调用掩盖。完整 Rust 测试只在 source/public-fixture CI 执行，不在本机大链接。

已推送独立验证提交 `bd39f0487f8578aefa9fecda43db8cc120358c43`，
[Actions 43678564856](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43678564856)
已终态 failure，artifact `14147911636` 核对：candidate 15、prepared 21/1，
失败是故障夹具先被原 CHECK 拒绝，尚未执行控制器与全套。修复及新增真实签名核心
见 [C3b7](SDK_CONTROLLED_SIGNING.md)；不再轮询或重启旧失败任务，不把类型检查当运行通过。

## 后续

下一为实际 SDK 适配层的完整业务授权、保留原证据的受控 signer、固定 sender 语义与
原 executor 接线，不能直接把现有 opaque worker 接在新 port 后。还需独立观测/恢复、
其他方向/协议/backend/mode、SDK/新协议演练和真实发布验收。现有 PumpAPI 开发暂停，
无 SDK 升级、部署/重启、生产钱包/数据库或实盘交易操作。
