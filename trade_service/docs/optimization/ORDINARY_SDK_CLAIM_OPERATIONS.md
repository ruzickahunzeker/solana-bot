# NP-01-C3a 原执行 owner 的 SDK 操作分型

日期：2026-11-15。接续 B4c1，不新增另一套订单/执行 owner，不伪造聚合 Quote。只修改主 Trade Service 源码并验证隔离测试库；不升级 SDK、不构建 release、不部署/重启、不访问生产钱包或数据库、不签名/广播。PumpAPI 交易开发继续暂停，Copy/Bridge/sdk9 不改。

## 本批边界

1. 原 claims 区分 `ordinary-sol-buy` 和 `ordinary-sdk-trade`。新 SDK 占用仅接受 Durable、Live、直接 DAMM v2 卖出 1–100%；它是接续账本的内部操作类型，不是新增协议能力、开放交易或完整消息授权。
2. 原聚合提交 `ensure_scope` 和 quote journal 保持只接受聚合 SOL 买入。SDK owner 不得进入旧 quote 的 begin/prepare/sign/send 路径；Memory/Shadow、simulate、其他协议/方向/两跳仍不能获取本批 SDK claim。
3. owner 绑定原 request ID/hash/mode、操作类型、钱包 ID/公钥、字面幂等键及完整内部命令。金额、各个路由字段、执行方式、MEV、滑点、小费及 PumpAPI priority fee 都参与比较；浮点按原 bits 比较，不改 V1 hash/JSON 或修剪原键。
4. Durable 仍在同一原父行上事务插入/CAS/核对/读回。只有首次插入成功才返回原 owner；并发或文件重开只返回 `AlreadyHeld`，不发放恢复 owner，不自动释放/重试。未完成 SDK metadata 前，该类型只接受 `claimed`、合法原 nonce 和全部后续 metadata 为空。

正式 SDK 调用尚未切换到此 claim。当前正式 Live 仍是 B4c1 所记录的 opaque SDK 调用；不能把新枚举、迁移或 fake 占用成功写成 SDK 已有签名前账本/原字节发送。

## 008 兼容迁移

旧 004–007 内容不改。008 在原迁移事务中创建 staging 表、复制旧行、双向逐字段比较值及 SQLite storage class，再替换旧表并重建原 007 索引；标记只在整个事务成功后提交。原 schema_version 仍为 1，已有幂等/订单/hash/JSON/签名/字节/错误/终态证据不重写。

迁移前核对外键开启、staging 不存在、列定义/主键/外键/索引及入向引用、触发器。SQLite PRAGMA 不暴露全部 CHECK/表选项，因此在独立内存连接重放可信旧 DDL，严格比较原表定义；未知定义（即使看似等价）保守拒绝，不尝试自动规范化。原索引的 SQL 在替换前后逐字比较；view/外部 trigger 依赖由 SQLite schema 验证触发失败并回滚，隔离用例验证原对象/行/标记保留。

这是一项真实待发布 migration，不是纯目录整理。未来上线须另外验收数据库规模、迁移临时空间与耗时、写锁窗口、异机备份恢复、旧二进制回滚与含新操作行的兼容性；没有在生产运行，也未验收断电/刷盘或真实钱包恢复。

## 验证与证据

本批新增用例覆盖：

- 旧多个阶段/NULL/文本零字节/二进制/错误/终态字段及 storage class、父记录和原索引逐项保留，重复 initialize 不重发迁移。
- 额外列/触发器/索引/入向外键/staging/坏索引/视图与外部触发器依赖、关闭 FK 和未知 CHECK 拒绝，原 schema/行/迁移标记回滚。
- staging 丢行、改字节、改 storage class 或改整数值时，在删除原表前拒绝。
- SDK 独立操作的范围拒绝、原父身份/键/hash/JSON/状态异常、无 Quote/签名 metadata、旧聚合 journal 不接纳 SDK。
- 单 pool 16 路及双独立 pool 16 路竞争恰一个 owner；隔离文件重开后保留原 nonce/Unknown 且不再授权执行。
- owner 各身份与完整命令字段、float bits、字面 key、nonce 格式/独立身份与 Debug 脱敏。

最终源码并发全套 1304 项通过（145.68 秒）、串行全套 1304 项通过（251.95 秒），相对 B4c1 新增 15 项。最终 `ordinary_claims` 专项 26（5.02 秒）、HTTP 合同 200（29.68 秒）、原参数合同 3（2,016 组）、DAMM 候选 37、production check/fmt/diff 全部通过，仍五组既有生产警告，无新增警告。首轮 24 项是两项补充测试之前的中间源码，不额外计数。测试仅使用临时库/fake/内存 RPC/隔离 loopback 或测试子进程，不运行真实 SDK signer 或发送交易。

首次沙箱全套为 1265 通过/39 失败：38 项是既有 loopback 绑定或信号子进程权限相关失败，另 1 项为新严格构造器让旧 fake executor 的非法命令夹具提前失败。仅修正 `cfg(test)` fixture 保留负例错误顺序，正式 constructor 仍严格；构造器用例也核对非法金额/simulate 拒绝。获得允许本机回环/测试子进程的权限后重跑上述并发全套成功；没有跳过、缩短或放宽原测试，也没有操作服务进程。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet -j 1 ordinary_claims
cargo test --locked --offline --quiet -j 1
cargo test --locked --offline --quiet -j 1 -- --test-threads=1
cargo test --locked --offline --quiet -j 1 http_contract_tests
cargo test --locked --offline --quiet -j 1 ordinary_param_contract_tests
cargo test --locked --offline --quiet -j 1 damm_sell_message
cargo check --locked --offline --quiet -j 1
cargo fmt --check
git diff --check
```

## 不变项与内容指纹

本轮接续没有修改正式 executor、SDK lane/参数/准备与 DAMM candidate、PumpAPI、manifest/lock、vendor 或 release 二进制。原普通聚合的提交/恢复/索引代码保留原操作限制；测试专用 `AdmittedTrade` 复制辅助不进入正式构建。既有其他服务与 sdk9 工作树改动不回滚，也不纳入本轮交付。

| 文件 | SHA-256 |
| --- | --- |
| `migrations/008_ordinary_claim_operations.sql` | `70a583e407c81a2309ec31b4f8b3a9cab8f0be10ef5255e52627c6726a3c7701` |
| `src/application/ordinary_claims.rs` | `08cd5a746c401ba3e2c79041337e97976ade7c36e83c11724de9a4e9046c753c` |
| `src/application/ports/ordinary_claims.rs` | `5e511d360b9754561a02c1e7a6031573d61107b7fcb33ca0aaa9fee63f5654f7` |
| `src/adapters/persistence/ordinary_claims.rs` | `7bff9e5fde5c2740a0a15ab2746922970d2fcfbaa32532e8540c2e33313bc353` |
| `src/adapters/persistence/ordinary_claims/operation_upgrade.rs` | `9d9dcb3427748972758fb074d4c5cd708cdf6309d8c4c9ef92ed84b3c7c40386` |
| `src/idempotency.rs` | `3410db761c181e50f9bded8b673675b85dfada4fbe7936746d7a4739473d8093` |

原 release 仍为 `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`。指纹仅用于本地续作核对，不冒充上游来源、已部署版本或链上验收。

## 下一主线与完整剩余目标

1. 独立 SDK prepared metadata：原意图/唯一正式 candidate/账户与整数金额/最小输出/费用、具体 tip recipient、程序权限和有效期，混合/未知类型拒绝；不能套 BoundQuote 或只信任消息 hash。
2. 原 ordinary owner 的 SDK prepare/authorize/signing/signed/receipt CAS 和独立读回，绑定原父身份/完整 metadata；不从 `AlreadyHeld` 重建 owner。签后落盘失败不广播，未知结果保留原证据。
3. 正式产物按完整证据受控签名，只发送已经落盘并独立读回的原字节，取代 opaque SDK 内部重建/签名/发送；原签名只读恢复与全签名证据分开验收。
4. 持续覆盖 DAMM 买入/其他协议/多跳、其他 backend/mode、Memory/Shadow、worker 取消/多签名、剩余共享 RPC/资源/依赖方向、通用双 fake backend、可信 SDK 来源与隔离升级演练。生产/真实节点与调用方兼容另设验收；PumpAPI 暂停项仍保留为完整目标缺口。

本阶段只有操作分型/原 owner 绑定/兼容迁移；不替代这些后续任务，不因此标记完整优化完成。
