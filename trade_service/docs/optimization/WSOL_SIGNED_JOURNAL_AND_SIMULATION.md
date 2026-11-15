# TS-S1-G3-B2 WSOL 签名落盘与已签模拟

日期：2026-11-13。接续 `WSOL_PRE_SIGN_AUTHORIZATION.md`。仅 Trade Service 源码、离线测试和文档；未升级 SDK/依赖、构建 release、部署重启、访问生产钱包/数据库或执行真实 RPC、签名、广播。

## 实际顺序

新 Live 请求：

```text
原 gate / Quote / 唯一 reservation
  -> 原钱包与原 RPC 的 prepare / 受控 sign
  -> 完整身份 CAS 保存原签名和完整字节
  -> 事务内验证读回，提交后 Application 再验证签名回执
  -> 已签模拟（原 8 秒 timeout）
  -> 再查禁播与原记录/原证据/状态
  -> 只有 signed 才允许本次发送（原 12 秒 timeout）
  -> 原签名结果单向保存 / 绑定读回
```

相同 key 的历史记录仍在任何钱包/prepare/sign/simulate/send 之前返回，不自动重执行。独立 simulate 仍为 unsigned，不占 Live key、不获取长期 signer；本阶段没有改它的资金语义。

## 签名事务

`ConversionStore::record_signed_bound` 使用原表和 `BEGIN IMMEDIATE`。调用方传入原 reservation 快照及不可变 artifact 的签名/字节，不再只传 ID：

- 原记录和当前行必须匹配 submission ID、key、fingerprint、Quote ID、wallet ID、公钥、owner type/ID 八项身份。
- 两者必须都是干净的 reserved，签名、字节和 error 全为空；待保存的签名/字节必须非空。
- SQL CAS 包含完整身份和上述状态条件；影响行数必须为一。
- 事务内按原 ID 读回，验证身份、精确签名/字节、signed 状态、空 error、创建时间不变及本次更新时间。触发器拒绝/忽略/删除/篡改时回滚。
- 提交后返回已验证的记录；Application 再核对同一个 artifact 和 clean signed 回执，才允许把已签数据交给模拟 RPC。

存储层比较可信调用方提供的保留身份，不凭一条 read-model 重算缺少 flags/Quote JSON 的指纹。生产 reservation 仍经原 V1 codec 校验；没有新增外部 fingerprint 参数。旧按 ID 的 `record_signed` 仅是 `cfg(test)` fixture，委托同一绑定核心，生产没有旧 SQL 旁路。

存储拒绝、异常回执或提交回执丢失时不调用已签模拟/发送。已提交的签名字节仍可由同 key/ID 读取；不确定提交不能变成重新签名许可。

## 模拟失败语义

新增内部事件 `WsolUpdate::SimulationUncertain`，目标为既有 unknown；不是伪装成 broadcast 事件，也不新增数据库/公开状态字符串。沿用 G3-A 单向 CAS：迟到模拟失败不能降低已观测 submitted 或覆盖终态。

模拟返回程序失败、RPC 错误或超时后：

- 不广播，不调用 unsigned 的 `fail_before_sign`。
- 保存原签名/字节，保守记录 unknown，并保留 pending wallet；不把失败当作确定未签名/未发送。
- 回执仍为原七字段，error 明确提示 `signed evidence retained; do not resubmit with a new key`。其他请求已经推进状态时，返回权威新状态。
- 保存模拟回执失败时显式返回错误，原 signed 证据不清空；重复请求只读历史。

unknown 的含义是无法可靠判定链上结果，不表示本服务一定调用过 send。已签模拟把完整交易交给节点，不能再用旧的无签名失败回执丢弃证据或释放钱包。当前 HTTP 存储错误仍为 legacy 400，不得据此换 key 重试；错误分类兼容另列后续任务。

成功模拟后新增一次原 ID 查询，核对原身份/签名/字节和合法状态。若检查时已经是 unknown/submitted/confirmed/failed，直接返回该记录，不再调用发送；损坏或倒退为 reserved 则拒绝。此检查不是跨 RPC 的分布式锁，也不宣称消除检查之后的所有竞争；原字节、原签名和单向 CAS 仍是后续保护。

## 兼容和恢复限制

四个路径、鉴权、请求字段/默认值、七字段提交结果、Quote JSON/V1 指纹、金额/费用/租金算法、表/索引/global key/FULL/WAL 保持。新增的 unknown+签名回执、发送前身份拒绝属于有意安全修正；调用方上线前须兼容，不改变 Bot 代码或自动开启实盘。

请求取消和正常重开后，reserved/signed/submitted 保留原状态与已落盘证据，没有新增启动定时器、自动广播、清理、换 key 或重签。签名生成和磁盘提交不是同一原子操作：签后落盘前取消/故障可能只剩 reserved，但此时没有把 artifact 交给 RPC，原 key/pending 也不释放。不能声称磁盘不可写时仍能保证保存每一个本地生成的签名。

测试覆盖正常关闭/重开和故障注入，不等于主机断电、内核/文件系统损坏或生产磁盘刷盘验收。当前原签名查询的确认映射仍沿用旧行为，可信最终性和查询 deadline 未在本阶段修复，不能因签名账本完成就上线。

## 离线证据

相对 G3-B1 新增 15 项，既有领域/SQL 政策矩阵扩展到 SimulationUncertain：领域 36 组、SQL 30 组事件组合。

- 存储 5 项：八身份/空 artifact、脏 reservation/过期快照/越界时间、18 种写拒绝/忽略/删除/读回篡改、双 pool 16 竞争的唯一原签名赢家及重开、SQLite 锁竞争期间取消。
- Application 8 项：程序失败/RPC 错误/真实 8 秒 timeout 后保留证据和锁；模拟回执写失败；12 类签名回执篡改；12 类发送前读回异常；签名提交已成功但回执丢失后的文件重开；模拟失败迟于链上观测；成功模拟遇到既有观测/不确定状态不发送；八个确定性取消点的文件重开。
- HTTP 2 项：真实 Router 返回 unknown/原签名/原七字段/新 key 拦截；三处请求取消后同 key 不重签/重发。

八个取消点是 reservation 后、prepare 后、签名写前、签名提交后、已签模拟、发送内、发送结果写前、结果提交后。Gate 使用 Notify；没有用 sleep 猜测阶段。模拟 RPC 在 fake 中首先核对实际 SQLite 已有精确签名/字节；独立 unsigned 模拟依然无 Live reservation。所有签名/交易字节为合成无效 fixture，没有调用钱包签名器。

首轮曾有一项测试把“数据库指纹变更后重新读取的值”同时作为新的期望指纹，不能据此验证原身份绑定；已修正为使用原 reservation 快照，未放宽生产校验。

最终全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                       # 593 passed
cargo test --locked --offline --quiet -- --test-threads=1   # 593 passed
cargo test --locked --offline --quiet http_contract_tests   # 118 passed
cargo test --locked --offline --quiet wsol                  # 103 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅 models/persistence 两条既有兼容重导出警告。Cargo.toml、Cargo.lock、既有 release 的 SHA-256 仍分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。G3-B1 的 578 项保留为历史检查点，不替作本批证据。

## 下一执行顺序

1. 原签名查询：外层 deadline、RPC 返回长度/slot/确认形态/错误一致性，不能把 processed 错误或 malformed confirmations 当成可释放钱包的终态；保留查询失败不重发原则，并补真实 Router 兼容测试。
2. unsigned reservation/失败路径：审查 reserve 的完整身份读回，以及目前仍按 ID 的 fail_before_sign 和失败回读绑定；保持旧 V1 字节，不在安全更新中重开 key。
3. 存储/不确定错误的调用方合同：明确无法证明未发送，不自动重试；再做独立兼容修正，不能在目录迁移时暗改 Bot。
4. 真实节点、断电恢复、生产迁移/刷盘延迟和受控制品验收仅留单独窗口。本工作包不授权它们。

之后继续 PumpAPI 外来交易审计、普通交易发送前账本、VerifiedRoute/协议扩展证据、基础设施/依赖规则、SDK 来源/升级演练与发布回滚。整体目标保持，不因本阶段离线通过宣称全部优化完成。

后续执行更新：G3-C1 已完成原签名 lookup 的 3 秒 deadline、RPC 形态/签名/slot 校验与明确 finalized 才推进终态，见 `WSOL_STATUS_FINALITY_AND_DEADLINE.md`。本文件保留 B2 的历史计数和发现；当前继续队列以新文档/`EXECUTION_STATUS.md` 为准，unsigned 失败身份、错误兼容和真实验收仍未完成。
