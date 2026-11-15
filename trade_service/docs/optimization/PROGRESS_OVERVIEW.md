# Trade Service 已完成与未完成盘点

日期：2026-11-15。依据：当前源码路径、主计划与执行记录。用户已重新将持续优化目标设为 active，本轮继续非 PumpAPI 源码/离线任务；历史暂停及运维记录保留。修改计划本身不触发暂停或恢复。不再用缺少权重的百分比表达完成度。

恢复前的总体审查及获准 RPC 运维见 [历史审查](../../../docs/optimization/PROJECT_REVIEW_2026-11-14.md)：当时源码全套 1038 项通过、优化未部署，仅更换失效 RPC 并重启同一旧 release。本轮不做生产运维；最新源码数字以下列 NP-01-C3a 记录为准。外部 Copy/Bridge 已有兼容规则集中见 [对接边界](SERVICE_INTEGRATION_BOUNDARIES.md)，不是新增协议或对接功能完成声明。

## 统计口径

- 当前 C3b1 草稿见 `SDK_CANDIDATE_PREPARATION.md`：SDK 自有 metadata 与 prepared/CAS/只读回接口仍未完成验收；本轮补类型/规范编码拒绝及恢复用例，实际 009 DDL 轻量 SQLite 六项通过，test/bin 类型检查通过，七组生产警告如实保留。当前新增 17 项 Rust 用例未运行，旧 C3a 1304 不计为新源码通过；本机大目标编译资源问题未绕过，目标 active，完整构建环境待确认。不升级/部署/生产操作，PumpAPI 开发仍暂停。
- 最新 NP-01-C3a：原 claims 操作分型/008 兼容迁移/完整命令与钱包及字面键 owner 绑定已接源码，旧聚合 journal 不放宽。仅新 Durable Live 直接 DAMM 卖出可占用新类型，正式入口未切换，SDK metadata/CAS/签名/原字节发送仍未完成。新增 15，专项 26、完整并发 1304（145.68 秒）/串行 1304（251.95 秒）、HTTP 200、参数 3、候选 37 及 check/fmt/diff 通过，五组原警告。首轮沙箱失败及负例 fixture 修正、严格未知 DDL/依赖拒绝、旧行/storage class/索引/FK/文件重开/单与双 pool 竞争证据见 `ORDINARY_SDK_CLAIM_OPERATIONS.md`。未部署/操作生产库，不新增 Quote 或恢复 owner。
- 前序 NP-01-B4c1：完整私有零签名候选接实际 DAMM 卖出 simulation/Live preview，固定只读 RPC、不调用 SDK signer；失败/取消/过期不释放正式闭包，原 3 秒起点/准入保持。正式 guard 改持同类型产物，正式 Live 仍 opaque SDK。新增 20（SDK 12/RPC 7/HTTP 1），候选 37、readonly/HTTP 8、完整并发 1289（142.51 秒）/串行 1289（246.58 秒）、HTTP 200、probe 48、lanes 19、参数 3 与 check/fmt/diff 通过，五组原警告。见 `DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md`；当批未部署，无新迁移/Quote/owner。当时接续要求现由 C3a 操作分型部分接续，SDK metadata/CAS/readback/恢复、受控正式签名/原字节发送继续，不把无签名模拟当完整链路。
- NP-01-B4b 前序：实际 DAMM nominal profile 与私有 protocol/full guard 已接原路径，原参数/费用/消息/签名/单次核对；新增 25、完整并发/串行各 1269、HTTP 199、专项/check/fmt/diff 通过，见 `DAMM_FINAL_CANDIDATE_GUARD.md`。当时 preview 仍 SDK 调用，现由 B4c1 接续，历史数字/指纹不冒充最新。
- NP-01-B4a 前序：实际 SDK 高层 Prepared/without_rpc/simulate 与 compile/sign/send 调用链已核对，只有底层公开消息编译可直接不含 signer；编译不保证费用/权限/blockhash/wire/协议准入，fee clone 也不是不可变快照。新增九项公开地址合同，专项 9、完整并发 1244（138.95 秒）/串行 1244（238.19 秒）、production check/fmt/diff 通过，仍五组既有生产警告。该批仅测试/文档，未改实际执行路径/依赖/vendor/release 或生产服务，见 `ORDINARY_UNSIGNED_BUILD_AUDIT.md`。不把入口审查当作最终授权/完整 journal。
- NP-01-A3c 前序：准备 scan/raw 已接 main 原 SharedReadRuntime 的第六类独立 profile，单笔在途/250 ms 起点/单步 2 秒含 pacing/总计 4 秒，scan 512 KiB/raw 64 KiB 流式上限，只有当前注册 URL，429 共享冷却/取消/类型化坏证据不 fallback。原 3 秒新鲜度和 A3b2 输入绑定保留；不再经 SDK RpcClient 读取准备余额。新增 19 项，preparation_ 90、完整并发/串行各 1235、HTTP 199、shared_read 89、quote_balance 25、参数合同 3 与 check/fmt/diff 通过，详见 `PREPARATION_SHARED_RPC_RUNTIME.md`。旧 reader/17 测试原字节为 oracle，必要 Arc 注入/惰性取得保持零 signer guard；无依赖/vendor/release/配置/迁移或生产操作，未部署。其他完整门槛保留。
- NP-01-A3b2 历史：raw account/program/精度随 snapshot 接 SDK 入口，实际程序/variant/方向/default、canonical 输入/声明精度/模式/preview 检查；新增 17 项，preparation_ 71、完整并发/串行各 1216、HTTP 199、shared_read 71、quote_balance 25、原参数合同 3 与 check/fmt/diff 通过，仍五组原警告，见 `PREPARATION_SDK_INPUT_BINDING.md`。当时准备仍用 SDK RPC，现由 A3c 接续；完整最终消息授权和签名账本仍未完成，未部署。
- NP-01-A3b1 历史：实际准备 port 原 scan 后必须核验同次 raw Mint/两个规范 ATA，最小 slot/数量/程序/精度/状态/权限/扩展、raw 整数与 native reserve/supply 已检查；坏扫描不 fallback，不可用/全未解析扫描只能依据合法 raw 输入恢复。3 秒期限与 scan + raw 共用 4 秒逻辑 timeout，不新增 UI supply/SOL 请求。新增 16 项（raw 10、RPC 6），preparation_ 54、raw 10、balance 36、完整并发/串行各 1199、HTTP 199、shared_read 71、quote_balance 25、原参数合同 3 与 check/fmt/diff 全部通过，仍五组原警告，见 `PREPARATION_RAW_ACCOUNT_EVIDENCE.md`。当时尚缺实际 SDK 输入绑定，现由 A3b2 接续；不将 raw 元数据与逻辑调用当最终授权/wire 预算，未部署。
- NP-01-A3a 历史：普通 SDK 准备余额改用公开地址 read port、parsed metadata snapshot 与 pre-invoke guard；核对身份/程序/精度/状态/canonical 正余额，3 秒时效、卖出乘法/零值检查，坏/过期证据不换 lane。原准备顺序/参数文件不改，旧余额 helper 与裸数字 ATA fallback 仅测试保留。新增 30 项，preparation_balance 30、完整并发/串行各 1183、HTTP 199、shared_read 71、quote_balance 25、准备流程 8、参数合同 3 与 check/fmt/diff 全部通过，仍五组原警告，见 `PREPARATION_BALANCE_ENTRY_GUARD.md`。当时尚无 raw 验证，现由上述 A3b1 接续；未部署。
- RPC-01C2 历史：Quote 余额查询为第五类共享 profile，原首 Default 节点、250 ms 起点、2 秒单步/8 秒总计；raw 整数/ATA 旧语义保持，增加身份/完整性/上限拒绝，reader 不再持 SDK RPC handles。新增 23 项，quote_balance 25（含两项既有）、shared_read 71、完整并发/串行各 1153、HTTP 199、production check/fmt/diff 通过，仍五组原警告。见 `QUOTE_BALANCE_SHARED_RPC_RUNTIME.md`。当时 SDK/root registry/executor/依赖/release 不改，交易准备仍使用原 helper；其错误 Mint 缺口后由 A3a 元数据入口检查接续，不能以查询校验代替资金授权。下列 C1 等保留为历史证据。
- RPC-01C1 历史：原生批量余额为第四类共享只读 profile，100/1000 地址、整批 4/8 秒和独立 250 ms 起点预算，保留 Default 候选/正常字段/整数；半批失败不拼接节点，异常数量丢行与迟到响应先失败后修复。新增行为 21 项、原八项 oracle 与现行六项兼容覆盖令计数净增 27，最终 native_balance 21、shared_read 55、完整并发/串行各 1130、HTTP 199、production check/fmt/diff 通过，仍五组原警告。见 `NATIVE_BALANCE_SHARED_RPC_RUNTIME.md`。当时 Quote/token 方法字节不变，现查询 API 已由 C2 迁移；资金 helper 原字节仍保留，未部署。
- RPC-01B2 历史：readiness 在同一 runtime 有独立容量/250 ms 间隔、500/1500 ms 边界、候选子集/metadata 与共享冷却；原采样时序两项先失败后修复，等待后一次读取缓存新鲜状态/age，不写缓存、不改 V1 ready/HTTP。新增 20 项，readiness 33、shared_read 39、完整并发/串行各 1103、HTTP 198、production check/fmt/diff 通过，仍五组原警告。见 `READINESS_SHARED_RPC_RUNTIME.md`；SDK 版本/依赖/发送 client/release 不改，root SDK 当时只有只读 URL 投影一行，未部署。
- RPC-01B1 历史：刷新与可选普通聚合观测共享同一真实只读 runtime，两类独立容量/间隔与共享 429 冷却、单次 HTTP/体积/错误检查、关键刷新 500/1500 ms 边界及生命周期已接线。专项 25、完整并发/串行各 1083、HTTP 198、production check/fmt/diff 通过，仍五组原警告；见 `SHARED_READ_RPC_RUNTIME.md`。当时只覆盖两个 profile，现 B2 已迁移 readiness；没有部署。本批无新迁移，不代表整个未部署优化树没有迁移。
- 最新 D1B3 / RPC-01A：生产 Reqwest transport 的真实 loopback HTTP/1.1 九项、配置纯合并四项、完整并发/串行各 1058、HTTP 198，production check/fmt/diff 通过，仍五组原警告；配置去重先复现再修复，保持不同 key/原顺序与空白合同。实际 transport、executor、root SDK/依赖/release 不改，只有测试及配置入口的精确稳定去重；未部署，真实供应商/TLS/HTTP2 和全服务预算仍未验收。见 `ORDINARY_OBSERVATION_HTTP_TRANSPORT.md`，下一项 RPC-01B 不把 SDK 内部重试或 blockhash 优先级遗漏。以下为历史子项，不覆盖尚未完成的大项。
- 当前 NP-01-D1B2 源码已有默认关闭的 Durable 普通聚合只读扫描/007 索引、原证据恢复、局部请求预算/429 冷却与关闭接线；本轮补七项后专项 27、完整并发/串行各 1045、HTTP 198、production check/fmt/diff 通过，仍五组原警告，包括自建测试子进程的 SIGTERM/SIGINT 关闭。重复 RPC 入口的备用丢失先复现后修复，临时文件库三状态重开及失败/无结果不重签/重发。没有验收真实节点/HTTP transport 集成、生产迁移或全服务 RPC 预算，也没有上线或启用；见 `ORDINARY_PENDING_OBSERVATION_WORKER.md`。下列 D1B1 及更早数字保留为历史证据。
- 下文“完成”只指列出的具体子项已完成源码及离线验证，不代表对应整个 TS 阶段完成、链上模拟成功或已部署。
- 最新 NP-01-D1B1：实际观测核心改用只读不透明 context，销毁旧 owner/执行对象后可从临时原库显式恢复 signed/submitted/unknown；证据变化拒绝，无重新占用/签名/发送。新增九项，专项 9、并发/串行各 1018、HTTP 198、production check/fmt/diff 通过，仍五组警告，无新 migration。见 `ORDINARY_RECOVERY_OBSERVATION_CONTEXT.md`。尚无启动扫描/持续确认/全局 RPC 预算，未部署。
- 最新 NP-01-D1A：Durable 普通 SOL 聚合在本次回执保存后按原签名单次查询，3 秒 RPC deadline、006 原账本 finalized 证据/父行 CAS/独立读回已接实际控制器；新增 20 项，专项 20、并发/串行各 1009、HTTP 198、production check/fmt/diff 通过，仍五组警告。见 `ORDINARY_POST_SEND_OBSERVATION.md`。不提供跨重启/持续对账，GET/重复仍只读存储。
- 最新 NP-01-B3a：共享聚合 prepared 保存原 Quote 到期快照，在释放待签消息前重查；先复现过期消息仍能释放，再完成三项无 signer 测试。并发/串行各 989、HTTP 196、production check/fmt/diff 通过，仍五组警告；见 `QUOTE_SIGNING_FRESHNESS.md`。完整消息授权、普通对账与全 backend 账本未完成。
- 最新 NP-03/TS-09-A：四个生产 AppState 冗余资源字段仅为测试保留，服务复用原 Arcs；新增两项生命周期/权威合同，并发/串行各 986、HTTP 196、production check/fmt/diff 通过。本轮实际五组警告已记录，未新增抑制；见 `APP_STATE_RESOURCE_OWNERSHIP.md`。其他 AppState/接受入口和完整依赖隔离未完成。
- 最新 NP-02/TS-08-H：三靓号 Handler 接项目任务服务/port，创建/取消政策归 App，原生成主函数归 SDK adapter 且字节未改，AppState 移除直接 worker；新增十三项并迁移一项原测试。最终并发/串行各 984、HTTP 194、vanity 17、check/fmt/diff 通过，仅两组原警告。回归暴露的旧 C2b 测试竞态已用门闩/零写审计补强（单项十连过），不改交易生产实现。见 `VANITY_APPLICATION_AND_WORKER_BOUNDARIES.md`；不把 fake 管理/生成/签名当作真实 SDK/节点或链上验收。
- 不删除暂停的 PumpAPI 工作及其剩余门槛，也不把它们从完整计划中剔除以提高完成度。

## 已完成源码及离线验证的子项

| 类别 | 已验收交付 | 尚不能据此宣称 |
| --- | --- | --- |
| 内部模型/参数 | 普通及聚合 DTO/项目 command、整数金额/地址/模式、校验输入/parser/settings、record/V1 codec、接受与拒绝编排分离 | 全服务已无 SDK/旧状态对象耦合 |
| 普通持久化兼容 | 原 request hash/JSON/键/存储模式保持，旧 hash 歧义校验、重复/冲突/重启合同、journal port 接入；C1 Created 原身份/hash 与冻结命令已接实际执行，聚合拒绝公钥变化 | 所有普通交易已有广播前账本、完整授权或跨进程 exactly-once |
| 普通聚合执行占用 | C2a Memory/Shadow 原幂等行标记、Durable 原父行关联表/CAS/精确身份读回；报价前使用，重复不改原结果，取消/丢回执/文件恢复及两个 SQLite pool 竞争合同 | 所有 backend 已接入或跨重启 exactly-once；Memory/Shadow 不提供 durable 保证 |
| Durable 普通 SOL 聚合签名账本 | C2b 原 owner/005 同 authority 子表扩展、消息绑定、签名落盘/独立读回/只发原字节；旧 004 表迁移、竞争/取消/写失败/文件重开及真实 HTTP+fake backend 合同通过 | Memory/Shadow/opaque SDK 已迁移、完整协议授权、只读链上对账或真实 SDK/节点/发布已验收 |
| 聚合签名前时效 | B3a 共享 prepared 保存原到期快照并在取出待签消息前重查，过期拒绝先于 signer 获取，未过期 Legacy/V0 消息原字节保持 | 完整协议/费用/权限授权、真实节点或检查之后永不跨过期限；不自动重建、重签或重发 |
| 普通聚合观测/恢复基础 | D1A/D1B1 原观测/恢复核心；D1B2 默认关闭的扫描与局部预算/文件/信号恢复；D1B3 生产 transport 的本地 HTTP/1.1 请求/响应/超时/取消证据通过 | 全部 backend/mode、无签名 signing 中断、真实供应商/TLS/HTTP2/生产调度与全服务 RPC 预算验收；不是新的执行许可 |
| 协议与参数 | 中央 Registry、SDK 参数映射集中、九类基础 loader fixture、Pool query/warmup/hop 边界 | 九类完整报价/账户扩展/迁移/多跳或全协议实盘验收 |
| 只读基础边界 | RPC-01A 配置去重；B1 刷新/观测、B2 readiness、C1 原生余额、C2 Quote 查询、A3c 准备余额六类共享 HTTP/冷却，各有独立容量/间隔、真实 wire/体积/错误/取消合同；准备仅当前 URL，旧 oracle 独立保留 | 全服务 RPC/signer 已收口或供应商额度/真实节点验收；参数/ATA/寻池、独立 quote/WSOL、模拟与 SDK 内部读仍待续 |
| 普通准备余额入口 | A3a/B1/B2 的 raw/SDK 输入/模式与 A3c 实际 HTTP 预算，保留时效、整数/比例、原顺序/参数 oracle；坏/过期证据停止，SDK 入口后不回退 | 全方向/费用/min_out/全部账户权限与最终不可变消息授权、实际 signer/SDK 路径/真实节点或已部署 |
| 普通 SDK 构建入口 | B4a 实际调用链审查、九项无 Keypair/RPC 编译/费用合同，明确版本/ALT/权限提升/字节绑定与编译不提供的保证 | 无 signer 最终协议构建/不可变授权已接真实路径，或签名/广播前账本完成 |
| DAMM 卖出候选 | B4b 正式/preview nominal profile 接实际 SDK protocol/full callback、原请求/费用/消息/返回签名、取消/单次/时效核对；25 新测试含 384 SDK 指令对照 | 已隔离 SDK 签名能力、全量可信协议授权/全方向已覆盖，或 ordinary signed journal 已实现 |
| DAMM 不可变产物与模拟 | B4c1 私有完整零签名交易及不可写出口，实际 simulation/Live preview 只读 RPC、不进入 SDK signer；正式 guard 复用产物，API null/状态/重复保持 | 正式 Live 已受控签名/落盘发送，原账本已扩展 SDK，全 SDK simulate 无签名，或共享 RPC/真实节点已验收 |
| API Handler/DTO 归位及 Application 子集 | 普通交易/批量/查询/等待、独立报价/预检/提交/对账、转账与 WSOL、Pool/系统、余额、六个管理操作与三靓号任务控制均有 Application 子项；全部 Handler/路由组归位，http.rs 仅 17 行组合 | 全部资金/任务状态安全与资源隔离完成；AppState/接受入口、完整 SDK 构建/授权仍待续；管理/生成的原部分失败不等于原子生命周期 |
| 独立聚合提交 | durable 保留/签名与广播阶段、状态与原签名只读对账、临时库恢复/竞争/错误合同 | 普通 SOL 聚合 legacy 分支自动获得相同保证 |
| Native SOL 转账 | 阶段化准备/签名/发送、完整意图、签名账本/CAS、API/Application、取消/写失败/临时库恢复测试 | 真实签名、断电、生产数据库与节点验收 |
| WSOL 包装/解包 | 查询/模拟/提交分层，签名前消息绑定、签名落盘、单向状态、原 key 只读恢复、存储及错误完整性 | 已完成真实节点/调用方迁移/生产发布验收 |
| 普通 SDK 通道安全 | 单目标 sender 执行视图；仅准备失败可换 lane；SDK 调用后不再自动补单；typed unknown/原签名保留；核验总 deadline | SDK worker 已可取消，或已实现普遍恰好一次发送 |
| 普通 SOL 聚合回执 | E0 广播 Err/不同回执保留本地原签名，明确 Live Unknown 不因错误文字降级 Failed；C2b 进一步覆盖上述 Durable 子路径的发送前账本 | E0 本身证明广播前落盘，或 Memory/Shadow 取消/崩溃可恢复；完整 legacy 迁移未完成 |
| DAMM 卖出 probe | 公开参数固定零签名构建、576 组消息对照；八账户身份/扩展；本笔 CPI 净输出、模拟 epoch 费用和可选同次模拟 pre/post balances 核对，不再用旧跨 slot 相减 | 节点诚实性/真实节点兼容、买入/其他协议保护、最终消息授权或普通签名账本 |
| 安全策略/权限子项 | 已审查入口禁播策略、可选 legacy/separate 管理鉴权、离线拒绝合同 | 线上已启用新策略、调用方已迁移或 OS 级无密钥沙箱 |

## 部分实现，尚不能算完成

| 子项 | 当前状态 | 缺少的完成条件 |
| --- | --- | --- |
| 普通 SDK 完整授权 | DAMM probe、nominal candidate 与 B4c1 不可变零签名交易已实际使用；模拟无 SDK signer，但正式仍 opaque SDK | 独立完整证据授权且 signer 只接唯一已授权产物，原 owner 账本/原字节发送，全方向与其他 backend/mode |
| 完整架构隔离 | Handler 已归位，余额/管理/vanity 控制已接项目服务和 port；直接 worker 及四个冗余资源字段不再存在于生产 AppState | full Config、普通接受/index/journal、cache、原始 RPC/SDK 资源及 bootstrap 等继续收口；局部分层不等于全量资源隔离、实际生成验收或原子生命周期 |
| 全协议能力校准 | Registry 和基础样本存在，已有局部账户证据 | 全方向/Quote/Token 扩展/缓存/迁移池/多跳/权限/费用证据及拒绝矩阵不足 |
| SDK 来源 | 已整理本地 6.0.2 与历史证据 | 可信上游 revision、导入补丁账本和可复现来源证据仍不完整 |

## 必须继续完成的大项

1. DAMM 本笔输出的源码/离线子项已完成；仍需真实节点格式/能力验收及其他方向/协议扩展，不能把只读模拟当作最终交易授权。
2. 普通 SDK 的全量不可变构建与签名前授权：池/资产/Token Program、金额/min_out/费用、账户权限、附加指令和最终消息绑定；包括买入及其他协议。B4c1 已有实际使用的不可变零签名产物/只读模拟；下一操作分型扩展原 claims/签名账本与 SDK metadata/CAS/readback/恢复，使正式 signer 只接完整授权产物并原字节发送，取代 opaque 内部重建签名广播。既有 journal 仅支持 aggregator SOL buy，不能伪造 Quote/新 owner；全方向/协议仍待完成，不以模拟/guard 代替最终授权。
3. 普通交易唯一广播前 signed journal：C1/C2a/C2b、D1A/D1B1/D1B2 已有该 Durable 子路径的原 owner/签名字节/读回、可选跨重开只读扫描与局部预算源码/离线证据；全部 backend/mode、真实节点/生产恢复和 RPC 全局预算仍待续，不能宣称全普通交易完成。
4. 受控发送与恢复：只使用已授权并持久化的原字节，保留所有签名证据，不确定状态不换路线/新 key 重签；处理 SDK worker 的后台副作用。
5. 普通 SOL 聚合 legacy 路径迁移：C2b 仅迁移 Durable Live，Memory/Shadow 仍走原 legacy；独立 quote 账本不替代普通原 authority，最终订单写入不冒充广播前账本。
6. 剩余 Application/资源边界、基础设施、依赖方向、统一 RPC 并发/配额、缓存版本与观测/性能证据收尾；Handler 已归位不重复记作待移动，但不能据此删除剩余业务隔离任务。
7. 双假构建后端通用合同、取消/不确定结果合同及防新增 SDK 类型泄漏的依赖检查。
8. SDK 来源/补丁账本、隔离升级演练与新协议接入演练；当前授权不包含实际 SDK 升级，能先做的离线准备不因此停下。
9. 真实环境、生产迁移/刷盘/断电、调用方兼容、制品身份、部署/回滚验收。实际生产动作须另外明确授权。

## PumpAPI 暂停范围

既有 H1/H2 至 H2-B2-C 的代码与离线证据保留。剩余协议证据、完整消息/支出授权、普通签名账本及后续接入仍暂停，未当作完成；不停止 ZMQ、不改已有运行配置。普通 SDK 的 Pump.fun/PumpSwap 不因协议名相同就自动进入暂停范围。

## 后续持续执行约定

- 更新、记录或修改计划，以及询问进度/阶段总结，均不代表暂停。持续任务运行时继续已授权的队列，不要求用户反复说“下一步”。
- 只有用户明确暂停/停止时才暂停；已显式设置的暂停不能用文档改写或汇报措辞冒充已恢复。
- 仍保持不升级 SDK、不部署/重启、不接触生产钱包及数据库、不发交易的边界。单项需额外授权时，不做该越界操作，同时继续可做的独立离线任务。
- 后续报告固定列出已验证子项、当前未完成项、剩余验收，不凭测试数量或目录数量推算百分比。

详细历史证据见 `EXECUTION_STATUS.md`；B2b 恢复清单见 `DAMM_OUTPUT_EVIDENCE_PENDING_PLAN.md`。
