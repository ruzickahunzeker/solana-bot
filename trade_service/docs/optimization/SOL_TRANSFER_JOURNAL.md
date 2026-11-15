# TS-S1-F2-B SOL 转账发送前账本

状态：本文件记录 F2-B 账本检查点；后续 F2-C 已接入原 API/Application 并修正旧幂等/finish 缺陷，见 `SOL_TRANSFER_API_EXECUTION.md`。未部署、未重启、未操作生产钱包或数据库、未执行真实 RPC/签名/交易。下文测试计数及待接入说明为 B 阶段历史证据。

## 实施内容

- `domain/sol_transfer.rs` 增加项目 intent 和 Reserved/Signed/Submitted/Confirmed/Unknown/Failed 状态，不依赖 SDK。
- `application/sol_transfer_record.rs` 接管原 16 字段内部 V1 record，WalletRepository 保留原 SolTransferRecord 重导出；公开响应仍为 11 字段。
- `application/ports/sol_transfer_journal.rs` 只提供查询、保留、记录签名、签前失败、发送结果和确认状态，没有 signer/RPC/send。
- `adapters/persistence/sol_transfer_codec.rs` 负责新版本化 intent/snapshot JSON 和唯一旧 projection decoder；新格式使用整数与项目地址字节，不使用具体 Solana 类型。
- `adapters/persistence/sol_transfer_journal.rs` 实现原子保留、写前签名保存、状态 CAS、兼容读取和启动恢复。
- `migrations/wallet_001_sol_transfer_attempts.sql` 只供钱包库初始化引用，不是 TradePersistence 的 migration，也不改变旧钱包/交易列。
- WalletRepository 的 journal 工厂只克隆原 pool handle；不会在每次获取时迁移、恢复、建池或解密钱包。

## 唯一权威与兼容

旧 wallet_sol_transfers.idempotency_key 的全局唯一约束保持，不改成 wallet-scoped。新 attempt 以旧 transfer_id 为主键/外键；不是另一套 key 分配器。

同一事务同时写入旧 projection 与新 attempt。native 状态由 attempt 表达，旧表维持响应投影；读取时核对二者一致，不在冲突时挑一个覆盖另一个。所有更新要求精确状态和一行 CAS 成功，缺失或变动立即失败。

新 fingerprint 包含 schema/operation/live、literal key/owner、wallet_id、source、实际 destination 和请求固定金额/百分比，不包含 label、当前余额、费用或随机 transfer_id。相同完整内容重放，任何身份/金额变化冲突。

V1 旧 requested INTEGER 的 u64 as i64 往返编码保持，仅用于兼容投影；规范 intent JSON 保留完整 u64。新 signed projection 的实际金额/fee/balance 做 i64 范围检查，不能悄悄截断。固定/50/100 快照使用同一 F1 政策核对，不另写金额算法。

没有 attempt 的旧记录只读重放，绝不回填为新可执行操作。source/destination/owner/金额必须匹配；非规范或异常内容冲突。旧 processing/无签名数据不能证明未发送，F2-C 仍须对不能核实的旧结果 fail closed，不能重新签名。

## 状态与落盘

| 内部状态 | 旧投影状态 | 已保存的授权 |
| --- | --- | --- |
| Reserved | processing | 无签名/字节 |
| Signed | unknown | 原签名、字节、准备快照已在同一事务保存 |
| Submitted | submitted | 保留原签名/字节 |
| Confirmed | confirmed | 保留原签名/字节 |
| Failed | failed | 仅签前失败，无签名 |
| Unknown | unknown | 保留原签名/字节，或为重启时未签名的保留记录 |

签名记录、发送结果、确认和恢复都同时更新两种表示；任一 trigger/IO/状态错误回滚。未知发送结果不重新开放 key，不清空原签名，不允许 fail_before_sign 抹掉已有签名。

初始化在事务中创建增量表，并将新 Reserved/Signed 记录改为 Unknown，不签名或发送。旧无 attempt 行、Submitted/Confirmed/Failed/Unknown 不重写。恢复错误会回滚两种表示，不能带不一致状态继续初始化。

钱包 pool 的连接初始化显式使用 synchronous=FULL、foreign_keys=ON；journal 每个写事务只读核对此条件，遇到降级连接拒绝写入，不在请求中偷偷改 PRAGMA。该同步策略会影响钱包库所有写入，发布前仍需测延迟；没有修改运行中连接或生产配置。

## 离线证据

新增 18 项测试，包括：

- 版本化 JSON、全整数、未知 schema、全部身份/金额指纹差异、literal key 与全局 key。
- 保留两表的事务回滚；签名字节/旧 projection 原子提交；发送与确认更新故障和恢复失败的回滚。
- 状态 CAS、非法/迟到写入、签前终态、unknown 不再保留为新操作、错误 URL 脱敏。
- 快照身份/金额/租金、空签名/字节和实际字段范围错误；固定/50/100 成功快照。
- 旧有无签名/多种状态、不规范/损坏内容及 u64::MAX 编码；不回填 attempt，不自动重发。
- 新 Reserved/Signed 重启恢复；已提交和终态不变；文件重开保存原签名/字节。
- 两个独立 pool、16 个竞争请求仅一个 Created；projection/attempt 各一条。
- 生产构造路径的五个临时文件库连接均为 FULL/foreign_keys=ON；降级连接拒绝保留。

测试只创建内存或唯一 `/tmp` SQLite 目录，不提供 passphrase 或真实钱包；签名为合成字符串、字节为无效占位符，无资金执行后端。账本不代替消息/签名审计：真实字节仍须来自 F2-A 的不可编辑 SDK artifact。

这些证据证明 ledger 事务与恢复行为，尚未证明完整 Handler 在签名/发送/取消每个阶段都正确接入，后者属于 F2-C。不能把一次 Created 测试说成实际只发送了一笔交易。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet transfer_journal
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo check --locked --offline --bin pump-trade-service
git diff --check
```

2026-11-13：新增 18 项账本测试、HTTP 92 项通过；全套默认并发/串行各 478 项通过，无测试构建警告。F1 金额指纹继续通过，Cargo manifest/lock/release SHA-256 未变，vendor/原迁移未改。

生产配置的 cargo check 也通过，但有 7 条 unused/dead_code 警告：2 条兼容重导出，5 条为待 F2-C 接入的 journal trait、reservation/intent 字段及相关 helper。未加 allow 或伪造调用来掩盖未接入状态，下一批接通实际调用链后复核；不能宣称生产配置零警告。该检查未生成或替换 release 制品。

上线前仍需一致备份/迁移与 FULL 写入性能验证、调用方 unknown/409/503 兼容及制品验收。回滚保留新记录和原签名，不恢复旧数据库快照来清除 attempt。

后续 F2-C 已完成项目 metadata/执行 ports 与原 Handler 接入，先持久化再 send，显式处理写入错误和取消，去掉无 journal 组合入口；详见 `SOL_TRANSFER_API_EXECUTION.md`。当前生产检查仅剩两条既有兼容重导出警告，不再有五条待接入警告。
