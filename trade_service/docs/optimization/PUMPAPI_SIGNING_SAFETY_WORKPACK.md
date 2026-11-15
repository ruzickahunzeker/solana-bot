# TS-S1-H PumpAPI 签名与普通交易账本工作包

日期：2026-11-13。H0 为本地源码审查与后续执行基线，不是修复、RPC 或实盘验收。只针对 Trade Service，不改 Bot/Bridge，不升级 SDK、不改 vendor/依赖、不部署重启、不访问生产数据、不签名或广播。

## 暂停记录

2026-11-13 用户调整范围：暂停 PumpAPI 相关交易对接开发，保留 H1 至 H2-B2-C 的历史成果与未完成风险。随后要求仅记录计划、暂不实施；之后用户恢复持续任务，仅执行非 PumpAPI 队列，本工作包仍暂停。

本文件下方的阶段顺序为保留的技术计划，不授权自动续作；PumpAPI 剩余 H2 及其 H3-H5 需明确恢复后再推进。H3/H4 中属于普通 SDK/legacy 的独立缺口仍保留在主计划 1.1 的非 PumpAPI 队列，不能据此修改 PumpAPI 路径。不改线上 backend、ZMQ 数据接收、配置或服务状态，也不把暂停当作风险已解决。

## H0：源码证据

| 位置 | 已核实的行为 | 后续要求 |
| --- | --- | --- |
| pumpapi_client.rs:75-78 | HTTP bytes 解码后直接将 message 交给 VersionedTransaction::try_new，用 client.payer 签名 | 解码不是授权；签名前必须核对完整意图、消息及已验证协议账户 |
| pumpapi_client.rs:80 | 签名之后才判断 simulate | unsigned 模拟需与 Live 分开，不能因最终不 send 就允许先签外来消息 |
| pumpapi_client.rs:113 | 请求构造只需公钥，却持有整个 TradingClient/payer | HTTP/解码组件只接项目公开身份，不获取 signer/SDK client |
| pumpapi_client.rs:142-143、288 | 原始响应先 trim_ascii_whitespace，再决定 JSON/binary | 原始序列化交易的末尾字节可能是 ASCII 空白值；需 fixture 判断，不能随意删改 binary bytes |
| pumpapi_client.rs:129-135 | USDC/USD1 用 6 位，其他 Quote 默认 9 位；u64 转 f64 | 必须结合现有 validator 核对哪些 Quote 真可达；不能据此宣称任意 SPL 精度支持。审计仍以原整数预算为准 |
| pumpapi_client.rs:235-241 | 广播在 client RPC 内完成，含 SDK/RPC max_retries=3 | 区分原交易的 RPC 重播与生成第二笔交易；发送前 journal 必须保存同一原签名字节 |
| executor.rs:82-103 | durable 写 Submitting 后调用整个 PumpAPI backend，最后才拿到结果 | 当前普通 record 不是发送前 signed evidence，不能据此保证广播后写失败的恢复 |
| adapters/sdk/ordinary_execution.rs:64-72 | 已有项目 command/执行 port，但具体 backend 仍调用组合接口 | 提取时复用现有 port/唯一 reservation，不建第二套普通订单/幂等权威 |

本审查只证明这些本地调用关系，未执行外来交易签名，也未测试可利用性。decoder 或普通 fake port 的成功不是指令授权或链上成功证据。H0 当前文件 SHA-256：

```text
e923205217e3441c185a2e869a3324d1eb8ecf1f549a277a914dd97d73a11353  src/pumpapi_client.rs
e232ecb1d46726de2db85407e3a302c69a113efe413ffdc05c438937c6d71677  src/executor.rs
2246675f820b79d4939efa655ff0d8237456651c945f157059a54d4baf56b28f  src/adapters/sdk/ordinary_execution.rs
```

## 执行顺序

| 阶段 | 交付物 | 离线准入 |
| --- | --- | --- |
| H1 格式/请求边界 | 冻结请求 wire 和四类响应 fixture；纯解码器/公开钱包身份输入；修正有证据的 binary 空白问题并限制响应体/交易形态 | 所有旧合法 JSON string、单元素 array、binary 样本保持；畸形/多笔/超限失败；无 signer/RPC |
| H2 消息授权与 unsigned 模拟 | 项目意图、受控外来 message、协议账户验证、不可变授权摘要；unsigned simulate 独立 | 错 payer、额外 signer、未知/额外指令、错误资产/方向/数量/费用/收款账户、delegate/close 等副作用在签名前拒绝；模拟不调用 sign/send |
| H3 普通发送前账本 | 绑定现有 reservation、wallet、旧请求指纹、授权 message 与原签名字节的 journal；签名回执核对后才允许 RPC 暴露 | 原 API/旧 hash/schema 兼容；仅临时 migration；签名/广播/回执故障、并发、取消、文件恢复不重签/换 key/补单 |
| H4 SDK/legacy 路径及恢复 | 审查普通 SDK 组合调用与普通聚合 legacy 的相同缺口，统一受控编排；只读原签名对账与错误兼容 | 不能只修 PumpAPI 就声称 ordinary 全部安全；逐个 backend 证明独立 build/模拟能力，无能力时明确限制，不虚构支持或改默认路径 |
| H5 合同与发布证据 | HTTP/金额/Pool/Quote/MEV/幂等矩阵、后端差异与 SDK 升级回归清单 | 本轮仅离线；真实模拟、生产历史/刷盘延迟、调用方迁移、release/回滚与实盘分别授权 |

H2 的协议审计不能只有 Program ID 白名单。必须验证真实池身份、Mint/Quote/Token Program、买卖方向、精度、ExactIn/min_out、账户权限、ATA/WSOL 生命周期、费用和可写目的账户。v0 还需解析/核实 lookup addresses。尽量复用已有项目领域类型、受控聚合审计和协议 registry/loader；先确认能复用的语义，不能简单把任意交易伪装成聚合 Quote 或 VerifiedRoute。

只凭模拟成功或设置 min_out=1 不开放未验证协议。费用账户/tip、额外 SOL/SPL transfer、approve/delegate/authority/close 等均要按意图审查，不能假设远端服务始终只给合法 swap。请求 f64 与真实指令整数预算的关系也必须明确，不能把服务商回包的数量当作授权上限。

## 兼容与通用后端边界

- 不改变普通/批量的既有 key 作用域、V1 指纹、金额含义、钱包选择或默认执行方式；必要安全拒绝/状态改变先单独记录合同，不伪装成目录迁移。
- 新外来消息 backend 只提供未签名、受控计划；统一经过 Application 的意图审计、唯一 journal、签名证据持久化和发送策略，不开放任意交易签名接口。
- 若当前 SDK 没有独立 build 能力，先记录真实限制并查现有公开接口，不为赶进度改 vendor、升级依赖、绕过 journal 或双发比较。现有 opaque backend 不会因预留接口自动获得安全能力。
- 签名后不自动换消息、金额、池、后端或 blockhash；发送/提交回执不明只读原签名。原 RPC 同字节重播不等于可以生成新签名。
- memory/shadow 不能冒充跨重启 durable。回滚不能恢复旧 DB 来清空已发记录，历史坏行不自动删除或解锁。
- `TRADE_BROADCAST_DISABLED` 仍不是无 signer/无网络沙箱；模拟路径必须有独立的无签名证据，不因传 simulate=true 就信任组合函数。

## 下一项

H1 源码/离线通过，见 `PUMPAPI_REQUEST_AND_CODEC_BOUNDARIES.md`：16 项新增测试，80 请求/Legacy/v0 黄金字节冻结，raw trim 已修正，公开身份请求、HTTP/文本与 SDK 结构解码分离；全套并发/串行各 711、HTTP 152 通过，check 仅两条既有警告。严格限额/sanitize/v1 拒绝是已记录的格式安全变化，不把 decoder 当成意图审计。未做真实 HTTP/RPC/签名/部署。

H2-A 已拆 unsigned 模拟，见 `PUMPAPI_UNSIGNED_SIMULATION.md`：分发在 Live 入口之前选择无 signer 的公开身份/RPC 路径，核对单一零签名槽/钱包并保持原消息/config；模拟 signature=None，程序失败不再作为带签名的不确定提交。新增 10 项，全套并发/串行各 721、HTTP 153、fmt/diff/check 通过，未部署或做真实 HTTP/RPC/签名/广播。

H2-B1 已将有损 PumpApiTrade 参数替换为完整项目 command 的自有只读快照，保留 literal 钱包/key、协议、金额种类和 hop/fee/MEV；请求投影直接用项目地址，80 样本仍与 H1 旧 SHA 一致。新增 7 项，全套并发/串行各 728、HTTP 153、PumpAPI 筛选 33 通过；见 `PUMPAPI_OWNED_EXECUTION_CONTEXT.md`。原公开 DTO/hash/schema/default/backend 未变，不把模型矩阵当成九类协议交易成功。

H2-B2-A 已在模拟和 Live 签名前接入消息账户视图：单零签名/钱包、消息 hash 绑定、ALT owner/state/slot/库前缀/索引/顺序/去重、5 秒一次逻辑批读以及后续 minContextSlot。新增 16 项含 375 组库对照，全套并发/串行各 744、HTTP 153、PumpAPI 49 通过，见 `PUMPAPI_MESSAGE_ACCOUNT_VIEW.md`。未构造 signer 或测试真实广播，账户视图不等于完整池/协议/支出授权。

H2-B2-B 已接入 PumpAPI 的 RaydiumCpmm/StonkFunSwap 直接路线子集，见 `PUMPAPI_CPMM_ACCOUNT_EVIDENCE.md`：同 bank 池/Mint/vault/config/Clock、部分角色/精度和 Token-2022 扩展检查，坏 hook payload 显式拒绝，最终 slot 进入模拟/send；新增 17 项，全套并发/串行各 761、HTTP 153、PumpAPI 66。无真实节点/签名/广播，普通 SDK 未接同样检查，其他协议返回 None 不作验证。

H2-B2-C 已接入 PumpSwap / PumpAmm 直接路线：一次同 bank 八账户读取，核对池/Mint/vault/Global/Fee/Clock、当前方向与 fee/creator/cashback/pool-v2/buyback/tail。真实复用的 SDK assets/read helper 保持 CPMM 原 label/策略，补重复 vault 扩展拒绝。新增 15 项含 64 组合，全套并发/串行各 776、HTTP 153、PumpAPI 81；见 `PUMPAPI_PUMPSWAP_ACCOUNT_EVIDENCE.md`。无实际节点/签名/部署，非零 disable flags 仅保守拒绝，语义/其他 feature flags 未全面审计。

待单独明确恢复 PumpAPI 开发后，再继续 Pump.fun 内盘、Bonk/StonkFun 曲线、DAMM v2/DBC/AMM v4 的 B2 证据，继而 B3/B4 完整 observation/用户账户/指令/整数支出/min_out/全部费用与签名前绑定。普通 SDK/legacy 的独立调查按最新非 PumpAPI 队列及 `ORDINARY_SDK_SUBMISSION_SAFETY_WORKPACK.md` 推进，不恢复 PumpAPI 接入。当前 Live 仍缺完整授权和普通发送前 journal；不得把模型/预览/ALT/CPMM/PumpSwap 局部检查或 WSOL 回归当作 B2 全部/B3/B4/H3-H5 完成，不能缩减原资金安全目标。
