# TS-S1-G WSOL 兑换合同与安全边界工作包

日期：2026-11-13。范围仅 Trade Service 的 SOL/WSOL 包装、全量/部分解包；不新增聚合兑换、不开放协议、不升级 SDK、不部署或访问生产资金。本文件区分现状基线与后续安全修正，不把测试旧缺陷当作保留缺陷的要求。

## G1：现状与离线合同

本节冻结 G1 时的顺序与缺陷，不要求保留已知缺陷。后续 G3-B2 已将签名落盘移到已签模拟之前，见 `WSOL_SIGNED_JOURNAL_AND_SIMULATION.md`。

现有四个 internal 路径保持：

| 方法/路径 | 请求/响应 | 关键顺序 |
| --- | --- | --- |
| POST `/internal/conversions/wsol/quotes` | wallet/owner/direction、可选 amount_lamports、all 默认 false；quote 13 字段 + executable | store → metadata/owner → all 互斥/方向 → runtime/RPC → 账户/预算 → 保存 quote |
| POST `/internal/conversions/wsol/simulate` | WsolSubmission；status/quote_id/network_fee_lamports/units_consumed | simulate 必须 true → store → quote → fingerprint → metadata/binding → unsigned prepare/simulate |
| POST `/internal/conversions/wsol/submit` | WsolSubmission；七字段提交回执 | 禁播 → simulate/confirmed/Live gate → store → quote → reserve/历史 → prepare/sign → signed simulation → record_signed → 再查禁播 → send → 结果保存 |
| GET `/internal/conversions/wsol/submissions/:submission_id` | Query owner_type/owner_id；同七字段，duplicate=false | store → record → owner → 必要时原签名查询 → CAS 保存 → 读回 |

WsolSubmission 的 simulate 为必填，confirmed 默认 false；key 按 UTF-8 字节长度 1..128，trim 只检查空白，原值参与 fingerprint。fingerprint 保留现有 serde `(submission, quote)` 的完整字节语义，包含 flags、Quote ID/时间/费用，不可悄悄升级 hash 绕过旧 key。Live 查询历史先于过期校验，但 API 仍先读取 Quote，不是仅凭 key 查历史。

报价 executable 当前始终 false；不是 Live 开关开启即能直接执行的能力声明。HTTP 默认业务错误为 400，JSON/Query 提取可先于鉴权失败；不能在目录迁移时顺手更改。

现有金额与交易语义：9 位原始整数；Wrap 可建 ATA + transfer + sync_native；全量 Unwrap 关闭原 ATA 并返租；部分 Unwrap 使用临时账户，不关闭原 ATA。预算保留 100,000 lamports，另加网络费和必要租金；不能套用 native transfer 的 10,000 margin 或 100% 排空算法。quote TTL 60 秒，来源余额要求完全相等，ATA/租金/费用增加需重新报价。

现有 ConversionStore 是唯一兑换幂等权威，全局 key + 每 wallet 一个 pending（跨 owner），FULL/WAL；模拟不占 Live key。不得为重构再建第二套兑换 reservation。

新增基线覆盖 wire defaults/响应字段、完整 Quote/flags/key fingerprint 差异、过期重复、存储写入故障、两 pool 16 并发、pending wallet、文件重开、HTTP metadata/模式/绑定优先级及失败重放。只用公钥、合成无效签名/字节、临时 SQLite 和空 runtime；没有签名/发送。这些测试不是完整 Live pipeline fault 注入。

G1 新增 13 项（storage/wire 8、HTTP 5），首次全套 513 项通过；后续 G2-A 再增 4 项，当时并发/串行各 517、HTTP 106、wsol 筛选 24 项通过。G2-A 的实际边界与检查命令见 `WSOL_SIGNED_ARTIFACT.md`。G2-B 已接入 submit/simulate 项目模型、metadata/runtime/journal ports 与原钱包/RPC 保留，新增 15 项离线测试，见 `WSOL_SUBMISSION_RUNTIME.md`。G2-C 又完成 quote/status 只读端口、DTO/Handler/路由和 builder 分拆，新增 20 项，见 `WSOL_QUERY_AND_API_BOUNDARIES.md`；完整 G3 和真实验收仍未完成。

## 复现和审查发现

1. G1 时 prepare/sign/simulate/send 多次从 WalletRegistry 获取 wallet；G2-B 已在 submit/simulate 保留同一原 client 和准备时选择的 RPC，不再重新 resolve。G2-C 为 quote/status 只读 reader 绑定原 RPC（status 再绑定原签名）；完整报价身份/链上证据核对仍待后续，不把局部接入说成全服务 runtime 已固定。
2. G1 时 `PreparedWsolTransaction` 的 Transaction/signature/bytes 字段公开可编辑；G2-A 移到 SDK 私有 artifact 并增加发送前一致性检查，见 `WSOL_SIGNED_ARTIFACT.md`。G3-B1 又接入完整标准消息/Quote 授权，G3-B2 接签名事务/精确读回，不能以这些局部完成冒充完整运行时、恢复或链上验收。
3. G1 时已 record_signed 才 send，但 Live simulation 在 sign 后、持久化前。G3-B2 已改为完整身份签名事务提交/读回后才模拟，模拟失败保留 unknown/原证据和锁；落盘前取消仍可能只留 reserved，未向 RPC 暴露 artifact，不自动重签。没有广播不能被简化为“从未签名”，见 `WSOL_SIGNED_JOURNAL_AND_SIMULATION.md`。
4. G1 已复现 `submitted → unknown` 的迟到状态降级；G3-A 已接单向 CAS，G3-B2 增加模拟不确定事件和发送前权威回读，不降低已有观测。原签名查询可信度仍未因此完成，见 `WSOL_MONOTONIC_PROGRESS.md`。
5. 文件重开保留 reserved/signed 原状态和原字节，没有重启恢复观测策略；reserved 没签名会长期占 pending wallet。禁止把这类记录自动释放/重签/补发。
6. G1 的 status 按 `confirmations.is_none()` 判断 confirmed，缺少查询外层 deadline。G3-C1 已作为独立安全修正加入 3 秒 lookup deadline、原签名/slot/形态校验，且只允许明确 finalized 推进既有 confirmed/failed，见 `WSOL_STATUS_FINALITY_AND_DEADLINE.md`。没有增加公开 finalized 字符串；查询错误仍不覆盖原记录，历史终态不自动重判。
7. G3-C3-A 已迁移 submit 的 typed 400/409/503、禁止自动重试及经过验证的 ID/签名上下文；G3-C3-B 新增原 key/wallet/owner 的内部纯账本 GET，可在不知道 ID 时恢复，不触发节点/签名/观测写入。status 后续已在 G4-B、quote/unsigned simulate 在 G4-C 单独迁移。调用方迁移未进行，不能把服务端响应头当成外部自动重试已关闭。见 `WSOL_SUBMIT_ERROR_CONTRACT.md`、`WSOL_READ_ONLY_RECOVERY.md`。
8. G4-A 已复现并修复 quote_json 损坏列导致的 row.get panic，加入 ID/expiry 行绑定与新写事务内全部字段/字节读回；旧过期/未知字段保持可读，不修复历史坏行。损坏 Quote 不再阻塞不依赖它的原 key 恢复，见 `WSOL_QUOTE_STORAGE_INTEGRITY.md`。
9. G4-B 补请求 ID/完整身份/已知状态与成对证据的初始核对、最终读回单向状态矩阵及终态证据/error 不可变，包含 unsigned failed 不能补入签名。status 存储/运行时故障改为 typed 503，明确 owner/不存在保留原 400；恢复头只用已核对的初始上下文，历史 URL 只在响应副本脱敏，见 `WSOL_STATUS_RECORD_AND_ERRORS.md`。不改变原签名/3 秒 deadline/有效 finalized 才终态的规则。
10. G4-C 复现并拒绝不匹配钱包 metadata/不可能的成功 Quote，补无私钥/fallible metadata 投影、金额/租金/预算估算校验、quote/unsigned simulate typed 错误及取消/文件恢复。正常四/13 字段与 V1 字节不变，原实盘 key 不被预览覆盖；交叉清单见 `WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md`。

## G2：输入、模型与受限 runtime

1. 先提取项目拥有的 amount/direction/binding/quote/submission/read-model；旧 wire JSON、hash、数据库字节单独留在 DTO/codec。不得把 SQL/serde/SDK 类型直接放入 Domain。
2. 提供只读 metadata、quote 查询/journal、受限 execution runtime 和原签名状态查询 ports；均复用现有 store/client，不在每次请求初始化钱包或 RPC。
3. runtime 接受时绑定原 wallet pubkey，后续 prepare/sign/simulate/send 保留同一 wallet/RPC snapshot。API 只做鉴权、DTO 和 HTTP 映射。
4. prepared/signed artifact 私有构造，只读提供授权摘要和原签名/字节。签名前后验证 payer、指令全集、账户、Mint/Token Program、数量、费用预算及临时 signer；不能授权外部任意 Message。
5. unsigned simulate 不获取长期 signer，也不持久化 Live key。部分解包的临时 Keypair 属于构建材料，不得泄漏、日志化或被误认为钱包私钥。

## G3：持久化、取消与对账安全修正

1. 保留原表、global key 和 pending wallet 索引；旧 fingerprint/read-model 兼容，不迁移或覆盖已保存操作来释放 key。
2. 明确签名授权、签名前失败、signed/unknown/submitted/confirmed/failed 的状态前置条件；record_signed 成功才允许一次广播。签名生成后的落盘失败不广播、显式错误，不自动重签。
3. SQL 更新使用真实 CAS，迟到 unknown 不覆盖已观测 submitted/confirmed；对账发现链上失败可以终止但不能重新开放原 key。说明 terminal 后允许新 key 不等于自动补单。
4. 五类取消点与写失败使用 fake 的确定性 gate/trigger 验证；重启对 reserved/signed 仅做观测/保守标记，不重新发送。保留原签名/字节、owner、wallet 与 fingerprint。
5. status 仅查原签名，设置外层 deadline/明确确认等级；网络未知保留原状态，更新竞态读回权威结果。绝不自动换 RPC 广播、改 blockhash、改金额或补单。
6. 错误码/unknown 改进作为独立安全行为修正记录，先补消费者兼容合同，不伪装成纯目录迁移。

## G4：验收和后续边界

执行更新：G3-B1/B2、G3-C1/C2/C3-A/C3-B、G4-A/B/C 已完成各自局部源码与离线验证；最新并发/串行各 695、HTTP 152、WSOL 205 通过，完整命令/指纹见 `EXECUTION_STATUS.md`。不声称任意全库篡改后的密码学完整性、真实节点共识、调用方兼容或断电/生产验收已完成，也不自动部署。下一批转入 `PUMPAPI_SIGNING_SAFETY_WORKPACK.md` 的 H1，无需等这些生产窗口才继续离线优化。

- 第一层：默认值、字段、整数/预算、指令、fingerprint 和存储字节保持；全量/部分 unwrap 分开验证。
- 第二层：fake 应用与真实 Router 覆盖 prepare/sign/simulate/signed 落盘/send/结果写入/status；并发和各处取消，临时文件库重开。
- 第三层：内存 RpcSender 验证 unsigned 无签名、受限签名授权、同字节 send、精确 deadline 和确认映射。禁止连公网或加载真实钱包。
- 第四层：独立生产窗口才允许一致备份、迁移/FULL 延迟、制品部署与真实模拟。实际签名/广播仍单独授权，不因本工作包存在自动执行。

G1 不等于 G2/G3 已完成。后续仍有 PumpAPI 外来交易审计、普通 SDK 组合买卖发送前账本、协议 VerifiedRoute/扩展证据、SDK 来源/升级演练、基础设施/依赖规则和发布回滚任务；不以此宣称整体优化结束。
