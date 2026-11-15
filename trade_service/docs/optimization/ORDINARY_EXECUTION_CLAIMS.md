# NP-01-C2a 普通 SOL 聚合原子执行占用

后续状态：C2b 已扩展原表及 owner，并接入 Durable Live 普通 SOL 聚合签名账本，见 `ORDINARY_DURABLE_SIGNED_JOURNAL.md`。下文无签名字段、bool 标记等描述仅为 C2a 当时的历史实现，不作为当前完整表结构说明；Memory/Shadow 仍无 durable 保证。

日期：2026-11-13。状态：本阶段源码/离线回归通过，未部署。范围：C1 原身份之后、任何状态更新/报价/构建之前的执行占用。仅源码和离线验证；不升级 SDK/vendor/依赖、不操作生产配置/钱包/数据库、不进行真实 HTTP/RPC、签名或广播。PumpAPI 交易对接开发保持暂停。本阶段不等于签名字节账本或完整 C2/B/C/D/E 完成。

## 实际实现

- `application/ports/ordinary_claims.rs` 定义项目占用结果和受限 port；`application/ordinary_claims.rs` 校验原模式、Live SOL 聚合操作及父订单身份/状态，不包含 HTTP/SDK/signer。
- `RuntimeExecutionClaims` 每次由原 AppState 的 persistence/index/idempotency Arc 组合，不新建业务 store。生产 executor 必须先获得 Acquired；AlreadyHeld 或 Err 直接只读停止，不先把订单改为 BuildingTx，也不查询执行钱包、报价或自动重试。
- Memory/Shadow 在原 `IdempotencyStore::Reservation` 上原子设置 `execution_claimed`，严格比对原 wallet/key/request_id/hash/付款分支；不在新字典上生成第二个 key。Shadow 镜像故障仍保留旧内存策略，不冒充 durable。
- Durable 在同一个 SQLite pool/事务内执行条件 INSERT、父记录核对、占用行精确读回和提交。首条写语句获取 SQLite 写锁；父行必须匹配原 ID/钱包/公钥/key/hash，首次占用要求 Accepted 且无原签名。再核对原父行元数据、V1 JSON 身份与 SOL 付款分支；不一致会回滚临时 INSERT。
- 已有占用只返回 AlreadyHeld，保留父订单原状态、签名和时间戳。占用的版本/操作/hash/时间异常会拒绝，不自动覆盖、释放、过期回收或降级到其他 authority。
- 原 `verify_stored_replay` 的 JSON 核对抽为同一纯 helper，保留原判断；未重新计算 V1 摘要、改变校验默认值或引入当前设置回放。

## 数据与入口范围

新增 `migrations/004_ordinary_execution_claims.sql`，只由 durable 初始化加载；未修改任何旧 migration。新表 `ordinary_execution_claims` 只有原 request_id 主键/父行 FK、固定版本/操作、原 request_hash、创建时间。没有 wallet/key 的第二套唯一约束，也没有签名、签名字节或最终消息授权。

FK 限制删除仍有占用证据的原父行。重复初始化不清空占用；旧进程重启恢复仍只把原非终态订单变为 Unknown，不重放。表是追加式源码迁移，本轮只在临时库执行，生产升级/历史兼容/回滚仍须受控验收，不能直接删除表来“释放”一笔订单。

仅普通 `aggregator_sol` 的 Live 执行接入占用。模拟不消耗 Live 占用；普通 opaque SDK/PumpAPI 路径未自动获得这项保护。旧孤立 executor 样本通过 cfg(test) 的假 claimer 继续测试旧编排；新占用/执行器及 HTTP 测试使用实际 admission 和原权威，不据孤立样本声称真实 CAS。

## 错误与恢复语义

| 情况 | 本阶段行为 |
| --- | --- |
| 首次成功占用 | 继续原政策检查、BuildingTx、钱包核对和原聚合流程 |
| 第二个执行者或已持有占用 | 不改原状态/签名，不重新进入后端 |
| claim 写失败/提交回执不明/身份损坏 | 记录脱敏错误并停止；不释放 key，不伪造成另一笔失败/未发送 |
| 占用后任务取消 | 保留占用，不自动恢复/补单；无论是否已有签名都不能据此认定未发送 |
| durable 重启 | 已有占用保留，原非终态订单沿用 Unknown 恢复合同，人工核查原订单 |

由于占用异常不会覆盖父行，本阶段新的 HTTP 请求可能保持 Accepted/202（等待预算耗尽）；同键重复仍按旧合同返回 200/duplicate 与原状态，异参仍为 409。数据库不可用时查询可能为 503。错误日志是诊断，不是链上结论；恢复数据库可用也不自动重跑原请求。不得换 key 自动提交。

## 离线证据

新增 19 项（890→909）：

- 11 项 authority/存储测试：三模式、16 个并发占用者、十类 Memory 父行异常、key/模式/操作拒绝、Durable 父行/hash/JSON/付款分支/状态拒绝及回滚、坏占用证据、关闭的库、文件重开、Shadow 镜像失败、两个独立 SQLite pool 的竞争、FK/重复初始化与旧父行保持。
- 7 项真实 executor 合同：重复不覆盖 Submitted、16 次并发仅一笔 fake 后端、提交占用后模拟丢回执、占用后取消、关闭 durable 库、已终态父行不降回 BuildingTx、模拟不使用 Live claim。
- 1 项真实进程内 HTTP/临时 SQLite 触发器故障：Accepted/202、原 key 重复与异参冲突保持，没有钱包 lookup/报价/fake 签名/发送。已有 E0/C1 的真实 HTTP 流程也经过新 guard 回归。

样本仅公开地址、手工配置、无密钥 fake 和临时数据库。Fake 后端回执不执行密码学签名；取消测试等待真实 claim 已完成的 Notify 后 abort 并等待任务退出。文件重开仅证明占用和原订单可恢复，不证明尚未实现的签名字节可恢复。

专项 `cargo test --locked --offline --quiet claim` 23 项通过（含历史匹配用例、后加 HTTP 用例之前）；最终结果：

```bash
cargo test --locked --offline --quiet                         # 909 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 909 passed
cargo test --locked --offline --quiet http_contract_tests     # 170 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

check/fmt/diff 通过，仅两组原兼容导出警告（models、persistence request_hash），无新增抑制。原参数/消息/V1 hash/JSON/98 组状态黄金摘要随全套通过，未重新生成期望。旧迁移及 SDK/vendor/PumpAPI 实现未改；本阶段有意修改 executor、idempotency、persistence 接线并新增 004 占用表，原 executor SHA 从 C1 的 `0b4f47de9a96483efbd54d321939053073ffd85f0afd856d4b9a52502cc5fb61` 变为 `982ab1ae6c19c1043336e380bba4145a9db2580be69bf28c4a5ba1d1ead335b3`，不把必要接线冒称文件不变。

manifest/lock/release 保持本阶段前指纹：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

## C2b 及剩余门槛

下一步把原占用与不可编辑最终计划/报价绑定，加入原 owner/CAS、签名前意图、签名后原字节/原签名/hash 的同步落盘和精确读回，再只发送这些原字节。签名后结果不明的恢复/只读对账、取消/写失败与所有 backend 的统一授权仍待完成。

当前 claim 结果不是签名授权或跨重启 exactly-once；Memory/Shadow 的标记随进程丢失，不能用于跨重启 Live 恢复保证。Durable 的占用也不包含最终交易与原签名，崩溃仍可能缺失发送证据。后续不得复用独立 quote journal 作为第二个普通幂等权威，或因已有占用就绕过完整消息/费用/min_out/账户权限审计。持续任务保持 active，更新计划不暂停。
