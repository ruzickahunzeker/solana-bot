# TS-S1-G3-C2 WSOL 保留与 unsigned 失败身份

日期：2026-11-13。接续 `WSOL_STATUS_FINALITY_AND_DEADLINE.md`。仅 Trade Service 源码、离线测试与文档；未升级 SDK/依赖、修改生产钱包/数据库/配置、构建 release、部署重启或执行真实 RPC、签名、广播。

## 已修正的边界

旧 reservation 只比较 V1 fingerprint，未独立校验读回行的钱包、公钥、owner、Quote 和证据形态；新插入后读回不在同一事务。unsigned failure 则只按 ID/reserved/signature=NULL 更新，不能发现孤立的交易字节、其他身份或更新后篡改。Application 还可能把错配失败回执作为本次失败返回。

本阶段复用原唯一 ConversionStore、表和索引，没有新建 reservation/journal，也不改变公开 URL、请求、七字段回执或 V1 字节。

## 保留流程

- V1 指纹仍由 `adapters/persistence/wsol_conversion.rs::reservation_fingerprint` 通过原 `(submission, quote)` serde 字节生成，含原 flags、literal key、Quote 字段和时间。没有新算法、trim 后替换、别名重解释或外部 fingerprint 参数。
- 生产 store 的 `reserve_bound` 接收项目 input/Quote snapshot 和可信 codec 的 fingerprint；旧 wire `reserve_live` 仅作 `cfg(test)` fixture，委托同一流程。
- 先以一次完整 SELECT 检查历史，避免所有重复读取争抢 writer lock；必须同时匹配 fingerprint、请求/Quote 的 key、Quote ID、wallet ID、公钥、owner type/ID，且记录有非空操作 ID/指纹、已知状态及一致证据。
- signed/unknown/submitted/confirmed 必须有非空签名和完整字节；reserved 必须无签名/字节/error；failed 可为 unsigned failure 或同时保留签名和字节的签后终态。半份/空证据、未知状态和脏 reserved 拒绝，不重建或修复历史行。
- 未找到历史时，在 `BEGIN IMMEDIATE` 中再次检查 key，再校验新 Quote 时效，插入并在同一事务读回。必须符合完整绑定、原生成 ID、clean reserved、创建/更新时间等于本次值；插入被忽略、记录删除或篡改时回滚，不提交孤立操作。
- 对相同有效历史仍先读记录、后考虑新 Quote 时效，过期重放只返回原 key，不重新执行。global key、每 wallet 一个 pending（跨 owner）、FULL/WAL 和旧 key 长度/UTF-8 规则保持。

Application 在钱包获取或返回 duplicate 前再次检查 input/Quote 与回执身份/状态；新创建回执必须 clean reserved。Application 不重新实现 V1 hash，精确指纹比较仍由唯一 codec/store 保证；这不是在不可信插件与被同时篡改的整套期望值之间提供密码学证明。

Store 不接管 confirmed/Live/禁播授权；这些门槛仍由 Application 在 reservation 前处理。部分内部调用曾允许的存储级 flags 语义不被偷偷改写，flags 继续参与原 hash。

## unsigned 失败事务

`fail_before_sign_bound` 接收原 reservation 快照，不再只收 ID：

1. 同一事务读取当前行，检查八身份、双方 clean reserved、signature 与 signed_transaction 都为 NULL，且旧 error 为空。
2. 完整身份和状态 CAS 写入 failed、脱敏错误及更新时间；影响行必须为一。
3. 同一事务读回检查原身份、failed、原错误的脱敏值、无签名/字节、创建时间未在事务内变化及本次更新时间，然后提交。
4. Application 验证端口回执，再按原 ID 读回并再次验证身份、状态、错误和无签名证据；异常不能伪造成一条有效的本次失败回执。

失败事务不能覆盖并发产生的 signed/其他状态，不丢弃已签字节，不重开原 key。旧 `mark_failed_before_sign` 仅为 `cfg(test)` fixture，生产没有旧按 ID 的 SQL 旁路。正常错误文本、脱敏与重复失败回执保持。

取消、提交回执丢失或异常回执之后，可能保留 reserved，也可能已提交 failed；同 key 只读真实历史，不自动重试准备/签名/发送。只有已提交的合法 unsigned failed 才按原 pending 索引允许另一个明确新操作；这不是自动补单许可。单纯 HTTP 400 仍不能证明没有保留记录，或判断当前状态。

## 离线证据

相对 C1 新增 16 项，只有临时 SQLite、无效合成 artifact、fake ports 和进程内 Router：

- 存储 6 项：19 组插入拒绝/忽略/删除/篡改；14 组保留旧 hash 的历史身份/证据异常；原 V1 黄金 hash、flags、UTF-8 与过期历史优先；unsigned 失败八身份/脏行/已签阻挡；18 组失败写入/读回故障及 URL 脱敏；双 pool 16 竞争的 sign/failure 唯一赢家与文件重开。
- Application 6 项：14 组 reservation 回执错误在钱包前拒绝；reservation 提交回执丢失后文件重开；失败端口回执/最终读回各 13 组错误；失败写前/提交后取消及回执丢失的文件重开；失败 CAS 遇到并发 signed；错误脱敏跨写入/读回稳定。
- HTTP 4 项：真实 Router 的 reservation 身份拒绝、reservation 回执丢失同 key 恢复且新 key 仍受锁、失败错身份拒绝、失败请求取消/回执丢失后 key 不重开；原七字段正常回执与 legacy 400 均有检查。

Gate 使用 Notify 确定阶段，没有用 sleep 猜竞态。既有同 key 双 pool 竞争、全状态文件恢复、签名落盘/模拟和 3 秒原签名查询测试继续回归。本阶段不提供真实钱包签名、链上、断电或生产迁移证据。

新校验会拒绝旧库中字段错配、半份证据等不一致记录，但不自动改写、删除或释放它们。不能把“能返回一个匹配的 hash”当作整行身份正确，也不能宣称本次代码自动修复了所有历史损坏数据。

历史 artifact 在保留时只检查成对存在/非空，不重新反序列化或验签；正常来源仍由受控签名、精确落盘和后续 CAS 保证。此校验不是任意全库篡改后的密码学完整性证明，也不替代发布前的历史数据来源审查。

最终全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                       # 625 passed
cargo test --locked --offline --quiet -- --test-threads=1   # 625 passed
cargo test --locked --offline --quiet http_contract_tests   # 126 passed
cargo test --locked --offline --quiet wsol                  # 135 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍仅 models/persistence 两条既有兼容重导出警告，没有以 allow 隐藏。Cargo.toml、Cargo.lock、既有 release SHA-256 仍分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。前一阶段的 609 项保留历史，不替作本批证据。

## 下一执行顺序

1. C3：存储/保留/签名/发送结果不明的错误分类和调用方合同，保留同 key/原签名查询能力，不让 generic 400 诱导换 key 重发。独立审查兼容影响，不暗改 Bot。
2. 审查 G4 的证据清单与剩余 quote/store 解码、失败诊断、恢复/取消交叉路径；不能凭局部测试通过宣称完整 WSOL 安全完成。
3. 继续 PumpAPI 外来交易审计、普通交易发送前账本、VerifiedRoute/协议扩展、基础设施/依赖规则、SDK 来源/升级演练与发布回滚。真实节点、钱包库、发布与资金操作仍需独立授权窗口。
