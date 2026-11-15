# NP-01-C2b Durable 普通 SOL 聚合签名账本

后续 D1A 已接本次 record_broadcast 成功后的原签名单次观测，006 在同一 claims 行保存 finalized 证据并 CAS 推进原父行；并发/串行各 1009、HTTP 198 通过。见 `ORDINARY_POST_SEND_OBSERVATION.md`。它不提供跨重启或持续确认、不改变 GET/重复的存储读取合同；下文“普通对账仍待续”仍适用于这些剩余门槛。

后续测试补强：TS-08-H 全套串行回归暴露了既有 HTTP 测试“读到 DB Confirmed 即内存已更新”的时序假设；现用 fake 返回/Drop 完成门闩和 Confirmed 后零写审计确定性验证，单项连续十次、最终并发/串行各 984 与 HTTP 194 通过。不改生产 executor/journal/SDK/请求合同，不删除或放松状态断言；详见 `VANITY_APPLICATION_AND_WORKER_BOUNDARIES.md`。下文 929 等数字保留原检查点含义。

日期：2026-11-14（11-13 实施，续作验收）。范围仅为 **Durable + Live + 普通 aggregator_sol 买入**；未部署。不能将局部交付解释为完整 NP-01 或所有普通交易已完成。

## 实际接线

- 沿用原 admission reservation、业务 key、request_id、V1 hash/JSON、钱包 ID/公钥及同一个 SQLite pool，不创建第二套普通幂等权威，也不借用独立 quote 的提交账本。
- 首次 claim 返回不支持 Clone/Serde 的私有 ExecutionOwner；原占用保存 owner nonce，重复占用不重新签发 owner。日志 Debug 不打印 nonce。
- `005_ordinary_signed_submissions.sql` 扩展原 `ordinary_execution_claims`，保存阶段、报价/预检、原未签名消息及 SHA256、原签名/原交易字节及 SHA256。迁移通过同库 ledger 事务执行，不改旧 migration；旧 004 行保持 NULL owner，不能靠升级获得重新执行许可。
- `application/ordinary_submission.rs` 编排原 quote/bind 与受控 prepare/sign/persist/readback/broadcast；Durable 不调用 legacy submit，journal 缺失/异常不降级执行。
- 签名前绑定原钱包、公钥、SOL 整数数量、WSOL 输入、目标 Mint、滑点、正数 min_out、连续路由、报价时效及原预检权限。只允许经过原预检的 adapter 私有消息；这不是新协议的完整指令审计。
- 签名适配器验证钱包 payer、恰好一个签名、原签名的密码学有效性、规范交易字节，以及与授权未签名消息完全相同。
- 签名原字节/原签名与原父订单 `submitting` 在同一事务提交；随后独立读取、校验并逐字节比较，**只发送读回的原字节**，不重建、不换 lane 或 key。
- 提交结果 CAS 更新原父行；错误/不同签名回执保留原签名和 Unknown。不覆盖更晚的 Confirmed 等结果。外层 executor 不再用过期快照镜像覆盖该父行；无法取得权威结果时仅在内存保留 Unknown 及可取得的原签名。

## 状态与故障

```text
claimed -> preparing -> prepared -> signing -> signed -> submitted / unknown
                          签名前且无签名证据时 -> failed
```

| 情况 | 保留与停止规则 |
| --- | --- |
| 报价/预检/政策或 prepared 写入失败 | 不调用签名或发送；可安全证明未签名时记录 Failed |
| 签名消息变化、签名字节写入或读回失败 | 不发送；内存保留原签名/Unknown，不盲补写父行冒充 durable 提交成功 |
| 发送 Err、不同回执、结果写失败或观测失败 | 保留原签名，不重签/重发；数据库可能仍处于 Submitting，需核对原订单 |
| 签名后取消或文件重开 | 保留已提交的签名/字节；原 pending 恢复 Unknown，不能用旧 owner 自动发送 |
| 原父行已有更晚结果 | CAS 回滚或只读返回，不用 executor 旧快照降级结果 |
| 旧 004 占用无 owner | 保持占用，不生成新 owner，不重新报价/签名/发送 |

签名字节尚未成功提交时，数据库可能仍为 BuildingTx/null signature，而内存为 Unknown/原签名。HTTP Durable 查询以数据库为准，可能仍等待到 202；同键重复返回原订单，不重新执行。若签名事务已经提交，重开后保留原签名并恢复 Unknown。两种情况都不能用新 key 自动补单。

本 profile 的 unsigned/signed blob 为 1..1232 字节，单份 metadata 上限 64 KB；超限保守拒绝，不静默截断。将来扩展交易格式必须重新审查。

## 离线验证

样本只使用公开地址、固定不可执行消息、假签名/假发送及临时 SQLite；未调用真实 HTTP/RPC、未生成密码学签名、未广播。

- Application 故障合同覆盖原 owner/预约错配、竞争、prepared 元数据损坏、政策/签名失败、签名写失败/丢回执/读回损坏、错误回执、取消、重开、旧表迁移与重复初始化。
- 三项实际进程内 HTTP 合同连接真实 admission/claim/controller/SQLite，但使用 fake backend：八字段响应及重复查询保持；nonce/raw bytes 不外泄；签名写失败不能被 executor 补写掩盖；更晚 Confirmed 不被降级。
- SDK 纯验证器以零签名、错误 payer、多签名、错误签名和损坏字节检查拒绝；没有有效公开已签名样本的成功接受验证。fake 合同不证明真实 SDK 或节点会接受。
- 相对 C2a 新增 20 项：16 Application、3 HTTP、1 SDK 纯拒绝验证。最终结果如下；之前 927 是中间检查点。

```bash
cargo test --locked --offline --quiet                         # 929 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 929 passed
cargo test --locked --offline --quiet http_contract_tests     # 173 passed
cargo test --locked --offline --quiet ordinary                # 87 passed
cargo check --locked --offline --bin pump-trade-service       # passed
cargo check --locked --offline --tests                        # passed after cfg(test) import cleanup
cargo fmt --check                                            # passed
git diff --check                                             # passed
```

production check 只余原 models/persistence 两组兼容导出警告。初次 check 发现新增测试专用 owner import 警告，已通过 cfg(test) 导入收紧并再次 check/fmt/diff 通过；没有添加警告抑制。该收紧不改执行行为，完整回归是收紧前的同一逻辑，收紧后 tests 类型检查通过且无警告。

2,016 组 SDK 最终参数、98 组原始结果映射、576 组 DAMM 零签名消息及 V1 hash/JSON 黄金合同随全套通过，未重写期望。Cargo.toml/lock、release、PumpAPI 实现、普通 opaque SDK adapter 与 DAMM vendor 文件指纹保持前检查点。必要接线文件有意改变，不冒称 executor 未改：

```text
bfeebf9e5aa8ce78ef157406d954189a589d8e73d63802a62af6b25fdbfb9bec src/executor.rs
3021780891652cb2ec60240273ada6fc5502e4f9d16d9898ddb0b0fd3ae039cb src/main.rs
f9b9622080b7d145479f1ddad18e3691aa3356cb2c093fb9452b9a0dd1be43a8 src/adapters/sdk/quote_submission.rs
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

## 仍未完成

1. Memory/Shadow 仍为原 volatile claim + legacy 提交流程，这是明确存储模式分支，不是 Durable 异常 fallback；无 durable 字节恢复保证。
2. opaque 普通 SDK 和暂停中的 PumpAPI 未获得这份签名账本。完整池/程序/费用版本/账户权限与买卖方向授权仍未完成。
3. 普通原签名只读链上对账、所有签名来源收口、后台 SDK worker 取消控制和全部 backend 迁移仍需继续。
4. 正向 SDK 字节样本、真实节点、迁移/刷盘/断电、调用方兼容、发布及回滚须单独验收；不自动重提历史失败订单。

本阶段有意改变 executor/main/SDK quote adapter 接线；不升级 manifest/lock/vendor，不构建 release，不操作生产配置、钱包、数据库或服务。持续目标 active；更新计划或报告进度不暂停。
