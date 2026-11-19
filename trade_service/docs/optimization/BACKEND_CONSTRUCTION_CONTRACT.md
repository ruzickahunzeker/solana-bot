# NP-04 双假构建后端合同

日期：2026-11-15。归属原 NP-04 / TS-04/07/09，不新增工作包、依赖或交易能力。
八项新增 Rust 测试已写入；最终本地受限 test check 通过（13 秒，RSS 1,334,436 KiB），
独立源码 CI 已验收：91b3d94 / Actions 43714527309 / artifact 14162686991。
类型检查与实际 Rust 运行分别记录，不相互替代。

## 原入口与权威

- 使用现有 `application/ordinary_submission::execute/submit`、
  `ports/quote_execution::{QuoteSubmissionBackend, OrdinaryQuoteBackend}`。
- 原 `SqliteOrdinaryExecutionClaims`、原 admission/claim、原表和原 owner 保持；
  测试复用已有 `ordinary_submission/tests.rs` 的 Journal 故障包装，不新建 reservation、
  Quote store、存储状态机或恢复 owner。
- 共有的原 Pipeline 仅供应冻结测试 Quote；不负责两个后端的构建/候选转换。
  共有 Probe 仅记录事件、阻塞取消点、核对发送点的原库字节，不构造或签名。
- 全部新实现仅在 `cfg(test)` 子模块内。生产执行、API、状态/错误规则、参数、hash/JSON、
  migrations、manifest/lock/vendor 和 SDK 版本均不改。

## 两种独立构建表示

| 后端 | 独立构建与表示 | 共同检验点 |
| --- | --- | --- |
| RecordBackend | 从 BoundQuote 显式生成字段记录，整数 debit/credit floor、资产/钱包/路线等写入 RecordPlan；serde JSON 编码 | 解码后的业务意图逐字段等于原 Quote；收据与原未签消息绑定 |
| TapeBackend | 从 BoundQuote 生成 Owner/Spend/Swap/Receive/Validity 有序操作；Box<[Step]>，bincode 编码 | 独立解析操作形状/资产/整数/路径顺序，归一后的意图等于同一原 Quote |

两者没有共用构建 Engine，不调用对方构造函数，不从另一后端的候选转换；编码确实不同。
它们的 fake sign 仅生成带明显测试标签的不可执行收据，没有 Keypair、密钥或密码学签名。
fake broadcast 仅读取隔离内存 SQLite 并返回固定收据/错误，不拥有网络 transport。
状态读取也只返回 None，并断言查询的是该后端原固定签名，不访问链上。

两个候选故意使用非 Solana 表示，检验 Application/账本不依赖 SDK 的具体 Prepared 类型；
不能将假的 preflight、签名或收据当作真实协议构建、授权、模拟或签名验收。

## 同一组合同

源码：`src/application/ordinary_submission/tests/backend_contracts.rs` 及其
`record_backend.rs`、`tape_backend.rs`。八个入口各对两个 concrete 类型运行：

1. 原始整数：1、1,000,000、2^53+1、u64::MAX，以及大整数 expected/min_out；
   直接验证 Prepared 构建不经过浮点。大整数构建用例不是 API 金额上限准入证据。
2. 相同 Quote 的不同候选编码与相同业务意图，避免两个类型实际复用同一候选构建。
3. 共同编排顺序、签后落盘/读回先于 fake send；相同 owner 重入与 claim 不产生第二次构建/收据。
4. 错金额、输入资产、钱包、零/弱 min_out、断开路线、未支持协议、失败 preflight 和过期拒绝。
5. 八类原账本写/独立读回/确认丢失故障，不能绕过存储进入发送；已有签名只作原签名证据保留。
6. 已知无收据的 fake 签名拒绝保持原 V1 Failed 规则；有收据但消息变化、发送异常或不同回执
   保持原不确定语义，重入不能构建/重签/重发。URL 中的测试 token 不出现在结果错误中。
7. 候选构建、签后落盘未确认、fake send 中实际 abort/join；原 claim 持有、原库阶段/签后字节保留。
   签后只读恢复查询原收据，不发送；没有“abort 撤销链上副作用”的声明。
8. Record -> Tape 及反向：原结果 Unknown 后换后端重入，仍只能读原结果，不能产生另一个候选/签名。

## 验收与边界

CI runner 新增 `backend-contracts` 专项，仍保留全部旧专项、完整并发/串行及三层类型/别名门禁。
本机只运行受限类型检查与轻量脚本，完整 Rust 执行只在已授权的源码/公开数据、仅回环 CI。
发布快照身份/终态日志/完整计数已单独核对，未套用父快照 e3be442 的 1473 项。

### 本批验收证据

- `91b3d94037a1d557ce2b255f4eb491bc308e01ac` /
  [Actions 43714527309](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43714527309)
  终态 success；artifact `14162686991` 下载核对，45 步骤全部 exit=0。
- 专项八项全过（1.87 秒）；完整并发 1481（129.52 秒）/串行 1481（236.54 秒），
  零失败/忽略/过滤。独立比较两套原始测试名集合相同，八项新增均存在于两套完整日志。
- 原源码/恢复 Clippy 成功，负例 exit=101 为预期；独立解析原始 JSON spans 确认三层
  81 个不同字段被拒绝，包括各层 outside_alias，不仅读取 checker 摘要。
- identity 与 service manifest/lock/vendor manifest 的 SHA 匹配；发布审计绑定 1022 文件/
  12,615,602 字节，只有测试/专项 runner/文档变更，其他原生产源码字节与父批一致。
- 真实 main/索引不变，不合并或部署。本地后续仅更新验收记录，不为记录单独触发完整 CI。

本合同的实际范围是现有 Durable 普通 SOL Quote 买入控制器和虚构单跳 `fake-amm`。
没有新增生产 Backend 抽象、RouterLeg 实现、Router SDK 或支持的协议/方向/存储模式。
普通 SDK 的 `DammV2SellV1` nominal profile 和受控 Durable Live 直接卖出范围不变；
原 `ordinary_sdk_submission` 的共用 Engine 两候选测试仍只算其原局部证据，不冒充本合同。

NP-01 的其他 backend/mode/方向完整授权与账本、NP-03 全局资源/退出/读取预算、
NP-04 其他业务路径的合同适用性和 NP-05 隔离升级/协议演练仍须按原范围验收。
本批不得把 fake 合同等同于真实 Router/SDK 可替换、全部协议准入或生产发布完成。
PumpAPI 新对接仍暂停；不部署、不重启、不接触生产钱包/数据库、不发送交易。

## Original NP-04 SDK Controller Follow-Up

Local continuation after the original Record/Tape Quote contracts, not a new
backend/protocol task. The original SDK First/Second tests still share Engine
and retain their old assertions. Seven independently accepted Rust contracts run
the actual ordinary SDK controller and original SQLite journal with Native
and an independently constructed Tape candidate/signed representation.

The shared Case creates only an admitted public fixture intent, original claim,
temporary SQLite and a synthetic test Keypair. It builds neither candidate.
Native uses its existing fixture builder; Tape constructs its own ordered
price/limit/output-ATA/swap/close recipe from that admitted intent and explicit
public pool fixture state, using standard Solana/SPL encoding rather than the
SDK builder or a conversion from Native. Candidate, signer and memory sender
do not call the original Engine. Control only records events/blocks; the
original semantic validators and original journal remain shared guards.

These seven tests require matching canonical message/nominal metadata without
signing, identical controller order and one signer/send, twelve storage faults,
three altered readbacks, two unknown sender outcomes with original-signature
read-only observation, four real abort/join boundaries with signed-row evidence,
and recipe/consistent-candidate/packet changes. A replacement candidate with
internally consistent metadata must fail the already-issued original permit
before signer access. Cancellation cannot continue after the blocked gate is
released, and neither implementation obtains a new owner after an unknown.

This is a deliberately narrow fake contract for the existing Durable direct
DAMM sell100/classic-SPL/WSOL/Default scenario. Synthetic Keypair signing makes
valid nominal packets for the actual journal, not a production wallet or live
permission. The sender only reads temporary SQLite and returns a fixture
receipt/error. It has no network, SDK client, worker or RPC capability.
It does not prove fresh chain/pool/output evidence, buy/other modes/providers,
Router replaceability, all protocols or an SDK upgrade. No profile/dispatch,
DTO/hash/JSON, signing permit, journal/DDL, manifest/lock/vendor is changed.

Read-only source audit reconstructs every old test after removing one module
marker, retains989 original source/vendor/migration/dependency blobs, and checks
independent imports/build/sign/send and original permit/persisted-byte ordering.
Six actually changed weaker sources reject; mutation generation failures do
not count as policy rejection. Format/workflow checks pass, not Rust execution.
The local runner adds one focused group while retaining both complete suites.
The fixture-correction e6c5a6e/run44375225006/artifact14463247130 independently
passed full2078 each (350.04s/681.34s),129steps and74types/225fields. It excludes
these seven tests, whose coherent successor cannot inherit that acceptance.

The exact successor `d5303e2e7c01414150102f42e115cd9cba748221` /
[Actions44382091879](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44382091879)
completed its first run successfully. Downloaded artifact14465763941 independently
passed full concurrent2085 (380.83s) and serial2085 (733.72s), all2078 parent
names retained plus exactly seven source-bound new names, focused7 and all130
ordered zero-exit steps. Original/restored compiler results and74 types/225 raw
field spans, locked source identity, original tests/controller/journal/SDK and
six source-policy negatives were checked. Synthetic signatures and same nominal
bytes remain limited evidence for the explicit fake scenario, not live pool
freshness, an actual Router, all modes/protocols or production permission.
Do not rerun this terminal snapshot or publish documentation-only CI. The later
original NP-03 wallet mutation changes are outside this result and need their
own compiler/focused/full validation.
