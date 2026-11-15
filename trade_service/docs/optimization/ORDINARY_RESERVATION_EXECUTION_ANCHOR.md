# NP-01-C1 原保留身份与执行输入

日期：2026-11-13。状态：本阶段源码/离线验证通过，未部署。范围：为唯一普通签名账本接通原 reservation 的身份来源和实际消费者；不是广播前账本本身，也不是最终消息或资金授权。只做源码/离线验证，不升级 SDK/vendor/依赖，不部署/重启，不读生产配置、钱包或数据库，不执行真实 HTTP/RPC、签名或广播。PumpAPI 交易对接开发保持暂停。

## 实际接线

1. 原 `ValidatedAdmissionPayload` 增加项目 `execution_command` 读取，真实 V1 adapter 借用 API 原来已经生成的命令，仅新建成功后复制；不重复校验、改默认值或获取钱包资源。
2. `trade_admission::accept` 的 Created 结果携带私有、无 Clone/Serde 的 `AdmittedTrade`。Duplicate、Conflict、保留失败和 rejected 分支不产生该输入。Memory/Shadow 携带同一次内存保留所用 hash；Durable 携带同一次 `reserve_record` 输入 snapshot 的 hash，不再调用单独 fingerprint 或重序列化请求。
3. `OrdinaryReservation` 保存原 request_id、wallet_id、公钥、字面幂等键、V1 request_hash 与原存储模式；不是新 key、交易消息 hash 或第二套幂等权威。完整项目命令被复制并以只读借用使用，保持整数、Option、浮点 bits 和已解析模式。
4. 新输入构造核对钱包/key/付款分支/pool/side/method 与 Created 元数据，并要求原记录为 Accepted、非 duplicate、ID/hash 非空。内部不一致时 fail closed，不生成执行输入，也不删除或释放已经保留的原 key；此异常仍需按原记录核查，不自动重新下单。
5. API 只将这一 owned 输入交给生产 `execute_admitted_trade`，不再分别传递可错配的 request_id 与 command。仍只在 Created 后 spawn，原等待、响应状态与日志时点保持；启动日志增添原 request_hash 和 reservation_mode，未加 API 字段。
6. ordinary SOL wallet port 接收同一输入；SDK adapter 保留 resolve 时的 wallet_id，并在任何 quote/build/sign/send 之前核对同一 client 公钥与 admission 公钥。映射改变时返回既有 failed/原错误字段，不执行，不释放原 key。没有将此钱包核对自动扩展到其他 SDK/PumpAPI 执行消费者。

## 保持与限制

- 原 memory/durable 权威、duplicate/conflict、惰性 hash/JSON、V1 字节、默认/校验规则与 SQL/schema 不变；Shadow 镜像失败仍可按旧内存策略继续，锚点仍明确是 Shadow，不能冒充 Durable。
- `AdmittedTrade` 是 syntactic admission 的不可编辑输入，不是独立的完整金额、路由/池、附加指令或 min_out 审计。V1 hash 的承载和命令冻结不代表已经证明最终可签消息匹配该 hash。
- 当前消费者只把身份用于原任务及钱包核对；没有新增执行次数 CAS、签名前 journal、签名字节落盘/读回、原字节恢复或完整取消/崩溃保障。后续必须在原数据库权威上再次核对父记录并原子 claim，不能把 owned 类型本身当作跨进程 exactly-once 保证。
- 旧三参数 executor 入口和 `unreserved_fixture` 只在 `cfg(test)` 存在，供原孤立 executor 用例构造无资金样本；不据此证明真实 reservation。新 HTTP 合同实际经过生产 admission 和 owned 入口。
- 首轮兼容回归有两个“先删除订单”的孤立用例触发了新测试包装器的 expect；已修复为记录缺失则停止，原测试期望不变。生产入口始终使用实际 admission 输入，原 BuildingTx 缺记录/写失败拒绝执行的行为保留。

## 离线验证

新增七项：五项 Application 测试（全命令/大整数/浮点 bits 冻结、非 Created 不取命令、六类内部错配、公开钱包身份核对、Shadow 镜像失败），两项进程内 HTTP 合同（原 V1 hash/临时数据库父行、钱包公钥变化在报价前拒绝）。后者复用实际 admission/executor/ordinary SOL/legacy 编排，只将末端钱包/报价/发送替换为无密钥 fake；不建立真实连接或密码学签名。

专项已通过：admission 14、聚合 HTTP 四项（含 E0）、两个旧缺记录回归。新增七项后总数 883→890，最终结果：

```bash
cargo test --locked --offline --quiet                         # 890 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 890 passed
cargo test --locked --offline --quiet http_contract_tests     # 169 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

check/fmt/diff 通过，仍只有原两组兼容导出警告（models 和 persistence request_hash）；没有新增抑制。原 V1 hash/JSON/参数/消息/98 组状态黄金摘要随全套通过，未重写期望；旧迁移、vendor、SDK 普通调用 adapter、PumpAPI client/assets 未变。manifest/lock/release 保持本阶段前指纹：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

本阶段为实际接线有意修改 executor 的内部入口、普通聚合参数及启动日志，不再将旧 executor 文件 SHA 当作本轮不变项：由 `e232ecb1d46726de2db85407e3a302c69a113efe413ffdc05c438937c6d71677` 变为 `0b4f47de9a96483efbd54d321939053073ffd85f0afd856d4b9a52502cc5fb61`。98 组原分类指纹及实际执行回归保持，未借此修改 vendor、SDK 费用/发送或 PumpAPI 实现。

## 后续 C2 与剩余门槛

以本输入中的原身份为锚点，继续唯一 ordinary signed journal：父记录/key/hash/钱包核对与原子执行 claim，最终计划/报价及不可变消息绑定，签名前意图保存，签名后精确字节/签名/hash 落盘读回与 CAS，随后仅发送原字节。Memory/Shadow 的策略及兼容需明确，不能静默使用独立 quote journal 或降级执行。

还需覆盖写回执丢失、取消、并发、重启、签名后中断、原签名只读对账和 legacy 迁移；普通 opaque SDK 组合入口仍没有这些能力。不能因为 C1 通过而宣称 C/D/E 或全服务优化完成。运行配置、生产迁移、真实节点、发布/回滚仍单独验收；持续任务保持 active。
