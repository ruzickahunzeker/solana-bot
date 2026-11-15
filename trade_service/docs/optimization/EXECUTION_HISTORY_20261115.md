# Trade Service 独立执行队列

更新日期：2026-11-15。主计划：`../TRADE_SERVICE_OPTIMIZATION_PLAN_V1.0.md`。

## 当前续作检查点（覆盖下方历史状态）

用户已恢复持续目标，当前继续原 TS-04/08/09、NP-02/03 的普通执行编排与资源隔离；
计划 3.3 的五项设计补充不新增 TS/NP 编号，RouterLeg 不引入实现或依赖。
PumpAPI 开发暂停，SDK 升级及生产边界保持不变。

当前唯一待验快照 `e3be4420fe68049f5f5ce399405682620d44ee17` /
Actions `43711006653` 已推送并确认 in_progress；1017 文件/12,572,454 字节路径/常见
敏感字面量/diff 审计通过，真实 main/索引不变。相对首轮只改验收脚本/测试/文档，
81 负例和完整服务回归仍待本批结果；不重跑旧失败或已通过快照。

首轮类型门禁 `8bd7796` / Actions `43710108477` 已终态 failure，artifact
`14159823162` 下载核对：此前所有步骤/服务类型检查通过，AST 工具三项通过；原源码
Clippy 检查因子命令未接全局 --config 而无法离线解析原 zap 来源，负例/完整回归未运行。
仅将参数传到 clippy 后，并补零依赖 Cargo fixture 的真实转发负例，十项 Python/七项
policy 回归通过；不改 SDK/服务依赖、源替换、网络隔离或断言。修正待新快照，不重跑旧失败。

最新完整验收普通执行编排 `26445c597c75d273bf7a8a7264da9cc33192fb16` /
Actions `43707853050` 已终态 success，artifact `14159283766` 下载核对：完整并发
1473（112.34 秒）/串行 1473（209.87 秒），零失败/忽略/过滤；ordinary-runtime 七项、
ordinary-intake 八项与所有步骤 exit=0，源码身份/manifest/lock/vendor 匹配。不重跑旧任务。

后继 NP-03/TS-09 类型门禁已接源码及 CI：三个生产层 forbid、26 类明确类型和外层别名
解析、原源码/81 个预期负例/恢复源码检查；尚未运行本批完整验收。十项 Python 回归和
七项 workflow 回归通过，含仅 metadata 的真实别名编译负例；开发用 syn 工具 512 MiB
类型检查通过（5.66 秒/RSS 232,864 KiB），三项工具 Rust 测试已在首轮 CI 通过。
服务依赖/SDK/交易路径不改，门禁不在小 VM 运行，详见 `DEVELOPMENT_CHECKS.md`。

前序完整验收普通接受流程 `26be71781406be7f912d53bd26e04fb549dce363` /
Actions `43706312318` 已终态 success，artifact `14159195795` 下载核对：完整并发
1466（129.94 秒）/串行 1466（238.29 秒），零失败/忽略/过滤；ordinary-intake 八项、
HTTP 202 与全部步骤 exit=0，manifest/lock/vendor 身份匹配。只推独立 CI 分支，main/
真实索引未变；旧终态任务不重跑，不冒充已部署。

后继源码 `application/ordinary_runtime.rs` 已接普通 executor 的全部编排，根入口借用
原资源，新增状态更新/原 SDK journal 投影/cache 读 port 及四值设置，七项新合同仅受限
类型检查后已由上述 26445c5 完整验收；不在父批 1466。test check `20261115T110647Z-210265` 35 秒/RSS 2,195,968 KiB，
bin check `20261115T110752Z-210600` 14 秒/RSS 980,732 KiB，exit=0，原七组生产警告；
格式/policy 七项通过。ordinary-runtime 专项及完整回归已在 26445c5 验收。
见 `ORDINARY_EXECUTION_PORT.md`；任务启动/取消和发送语义、原 hash/JSON/数据库/SDK 不变。

首轮普通接受快照 `c58eb8c` / Actions `43705256866` / artifact `14158646075`
已终态失败并下载核对：所有前置/原专项及 HTTP 202 通过，新专项 5/3，完整两套未执行。
三处新测试输入/观测时机已修正，保留旧 fixture/全部断言；生产实现未因此改变。
最终受限 test check `20261115T105239Z-204758` 9 秒/RSS 1,272,116 KiB，格式通过；
修正已在上述 26be717 完整验收，不重复触发旧失败任务。资源所有权核对另补原文档，未改运行生命周期。

前序完整验收：readiness `c220ba879db0b6fee766b22919cdfce6f8803f5d` /
Actions `43696813225` / artifact `14154584905` 已下载核对，所有步骤 exit=0；
完整并发 1458（143.31 秒）/串行 1458（252.76 秒），零失败/忽略/过滤，readiness-settings
六项通过。此前路径/敏感字面量审计只绑定该快照，不覆盖后续未发布改动；不重跑旧 CI。

本地后续 `application/trade_intake.rs`、对应 port/adapter/AppState/Handler 接线已通过
受限 test/bin 类型检查；本轮补成功日志原 target 及第八项回归，原校验/钱包 lease/
hash/JSON/保留顺序不改。test check `20261115T103713Z-198978` 9 秒/RSS 1,331,668 KiB，
bin check `20261115T103808Z-199318` 4 秒/RSS 575,448 KiB，均 exit=0，原七组警告。
新增七项 Application 与一项 HTTP 用例已发布到上述 c58eb8c；未计入已验收的 1458。
ordinary-intake 专项已接 CI runner，policy 七项/格式通过，保留所有原专项/完整回归。
上述接受流程八项及完整回归现已由 26be717 验收，见 `ORDINARY_TRADE_API_HANDLERS.md`；
不宣称合并 main 或部署完成。

下方 readiness 的“仅类型检查/待验”和 active 描述保留为历史，不覆盖本检查点，
不覆盖本次用户恢复的非 PumpAPI 队列，也不授权重复发布旧任务。

前序完整验收：bootstrap `970d8350a6508bbebcf2ebbbbae7b09bebfa7b8c` /
Actions `43695116074` 已终态 success，artifact `14154666415` 下载核对。完整并发 1452
（143.42 秒）/串行 1452（253.37 秒）零失败/忽略/过滤；bootstrap-assembly 4/HTTP 201/
参数 3 及全部步骤 exit=0，manifest/lock/vendor 指纹匹配，原七组生产警告。
999 文件/12,438,464 字节审计通过，真实 main/索引未变；旧终态任务不重跑。

本批 readiness 窄资源边界已发布源码：三值 Settings 与项目钱包诊断 port，SDK 查询留在
SdkReadinessWalletObserver，原 Registry/runtime/cache/persistence Arcs 复用；observe_health
helper 字节不变，V1 ready/诊断/采样顺序不改。六项新合同仅类型检查，未纳入 970d835。
test check `20261115T084206Z-166970` 33 秒/RSS 2,130,684 KiB、bin check
`20261115T084403Z-167501` 13 秒/RSS 978,984 KiB 均 exit=0；原七组生产警告。
policy 七项通过，后批 readiness-settings 专项及完整回归验收；原 bootstrap 构造器仅该
接线有意变化，不冒用前序 byte-identical 结论。见 `READINESS_RESOURCE_BOUNDARY.md`。

前序完整验收：`cb94bdd1d5b54ffd6566e722d92529e06d2f3718` / Actions `43693519994`
已终态 success，artifact `14153732230` 下载核对。完整并发 1448（142.77 秒）/串行 1448
（251.64 秒）零失败/忽略/过滤，api-access-policy 4/validation-settings 4/HTTP 201/参数 3
与全部步骤 exit=0，manifest/lock/vendor 身份匹配。996 文件/12,421,585 字节审计通过，
真实 main/索引未变；原七组生产警告保持。五项新增已实际执行，不重跑旧终态任务。

本批 bootstrap 已发布源码：原初始化/服务组装两段字节相同地移入 bootstrap.rs，main 保留
监听/日志/任务 start/停机协调。Resources/Bootstrapped 交接原 Arcs 和 refresher owner；
四项新合同现已在 970d835/43695116074 实际验收，不冒用更旧 cb94bdd，也不重复触发。
test check `20261115T082114Z-159090` 约 14 秒/RSS 1,551,888 KiB、bin check
`20261115T082256Z-159553` 约 4 秒/RSS 658,692 KiB 均 exit=0；原七组生产警告。
policy 七项通过，后批保留 bootstrap-assembly 专项及全部回归。临时 runtime_wallets Vec
在初始化函数返回时释放，不改变 SDK 身份/签名权威，不声称清零。见 `BOOTSTRAP_ASSEMBLY_BOUNDARY.md`。

前序完整验收：NP-03 `449731ce21b557421891d82d355a6e2a0a0f1f8a` /
Actions `43691952757` 已终态 success，artifact `14153850007` 下载核对。完整并发 1443
（133.72 秒）/串行 1443（236.47 秒）零失败/忽略/过滤，command-defaults 4/param-cache-port 2
及全部步骤 exit=0，manifest/lock/vendor 指纹匹配。994 文件/12,403,750 字节审计通过，
真实 main/索引未变，七组生产警告保留；旧成功/失败任务均不重跑。

本批已发布源码：NP-03 API 鉴权只接借用 access policy，31 个 Handler 调用已接；普通校验只接
已有 TradeValidationSettings，执行政策用原 Application 函数，原 Config 兼容 helper 仅测试
保留。五项新增合同现已由 cb94bdd/43693519994 验收，不在更旧的 449731c。
test check `20261115T080153Z-150488` 约 10 秒/RSS 1,338,228 KiB、bin check
`20261115T080309Z-150888` 约 4 秒/RSS 670,156 KiB 均 exit=0；原七组生产警告。
policy 七项通过，后批增加 api-access-policy/validation-settings 专项，保留全部回归；
详见 `API_CONFIG_BOUNDARIES.md`。当前运行有实际失败时优先修复，不抢先重跑/触发重复 CI。

前序完整验收 `6c50aca628313a8683b84800adbba1b72e751cd3` / Actions `43690435964`
已终态 success，artifact `14152368482` 下载核对：完整并发 1437（128.56 秒）/串行 1437
（235.46 秒）零失败/忽略/过滤，原五组生产警告。sdk-original-runtime 5、sdk-scan 16、
observation-migration 1、HTTP 200、参数 3 与全部步骤 exit=0，manifest/lock/vendor 身份匹配。
原 C3d2 旧夹具问题已验证修复，不重跑旧失败任务；当前真实 main/索引保持不变。

等待该 CI 时继续 NP-03：普通 DTO 的命令构造只接两项项目默认值快照，不再导入 Config；
原校验/政策/保留/序列化顺序不变，原合同仅改调用签名不改期望。四项新合同现已由上述 CI 验收，
已纳入待验的 449731c（不在旧已验收的 6c50aca）。test check `20261115T073633Z-140454` 35 秒/RSS 2,177,116 KiB，
bin check `20261115T073740Z-140817` 13 秒/RSS 979,788 KiB 均 exit=0，原五组生产警告。
command-defaults 专项及完整回归已通过，不重跑；见 `COMMAND_DEFAULTS_BOUNDARY.md`。

接续 NP-03 参数缓存：after_trade 复用已有 ParamCacheWriter，PoolQueryService 与 executor
仍持同一原缓存，生产 AppState 不再有具体 params 字段（仅测试保留），原键/写入时机不变。
新增两项 actual executor/原 fake backend/三个存储模式及服务释放合同，现已通过上述 CI。
最终 test check `20261115T074510Z-143335` 9 秒/RSS 1,229,316 KiB、bin check
`20261115T074419Z-143028` 5 秒/RSS 701,196 KiB 通过；生产七组警告（原五组加既有
缓存字段/get 未读两组），不抑制、不假读取。当前 449731c/43691952757 含 param-cache-port
专项及完整回归均已通过，详见 `PARAM_CACHE_RESOURCE_BOUNDARY.md`。没有生产操作或 SDK/依赖变化。

C3d2 `57c75c5` / Actions `43688840398` 已终态 failure，artifact `14152165576`
已下载核对：全部前置步骤通过，sdk-scan 16 通过；完整并发 1431/1（128.17 秒），串行未运行。
唯一失败为旧 005 schema 测试未删除新 010 索引就删除其引用列。本次只改测试基线重建与对应
marker 清理，保留原数据/父行/claim 断言并补 010 重建/唯一标记断言；生产迁移策略不变。

C3e 新增实际 execute_original_damm 的完整准备/模拟/签名账本/固定发送器测试，四个父用例
及一个隔离子入口，覆盖成功、三种签前拒绝、两种签后异常和两个取消点；仅临时 Keypair、
隔离 SQLite、回环 HTTP 和内存 RpcSender。初始七个类型错误已修复，受限 check 通过：
`target/dev-checks/20261115T072730Z-136412`，36 秒/RSS 2,154,948 KiB，exit=0。
没有本地链接/执行 Rust 测试；新增 observation-migration/sdk-original-runtime CI 专项，
保留完整并发/串行。新串联现已由上方实际 CI 验收，不冒用父批结果，详见 `SDK_ORIGINAL_RUNTIME_INTEGRATION.md`。

下方 in_progress 是当时发布记录，已终态任务不再等待或重跑。当前目标 active，所有 SDK/
部署/生产数据/真实广播边界及 PumpAPI 暂停不变。

## 历史发布记录

最新唯一运行句柄：C3d2 `57c75c5f39b59a55685dbc4fefcc7b906ab246d8` / Actions `43688840398`
已推送并确认 in_progress，987 文件/12,348,411 字节经白名单/常见敏感字面量/diff 审计，
真实 main/索引不变。16 项新 Rust（含回环入口）、010 实际 SQLx/worker 与完整套件待终态。
先核对该运行并优先修复实际失败，不重跑已成功的 C3d1，也不重新实施当前已接线的索引/扫描。
之后继续实际完整准备 RPC→受控 signer→固定 sender 的离线集成及其他方向/协议/模式，
保留资源边界/通用后端/SDK 演练与发布门槛；所有生产边界和 PumpAPI 暂停不变。

当前 C3d2 源码已接 010 专有 partial index/严格初始化 guard、原聚合与 SDK 两索引同只读快照
有界分页归并、同一 scanner/worker/共享 RPC 预算与默认关闭开关。16 项新增 Rust（含回环子进程
入口）仅类型检查（最终约 8 秒/RSS 1,188,120 KiB），bin check 约 5 秒/RSS 710,956 KiB、
原五组生产警告保持；独立 SQLite DDL 六项、policy 七项通过，完整运行待 CI。
实际 factory→共享 runtime→仅回环原签名 RPC、四阶段 SDK、文件重开和两类真实原订单共享游标
已有测试源码，不把它们当已运行或生产恢复。010 不重建表/改旧行或 007，异常标记/索引拒绝、
marker 失败回滚；未修改线上配置或启用 worker，详见 `SDK_PENDING_OBSERVATION_SCAN.md`。
最近完整验收为 C3d1 `87f8677` / Actions `43686886493` / artifact `14150979478`
下载核对，完整并发 1416（129.44 秒）/串行 1416（229.33 秒），恢复 16、controller 12、
sender 14、HTTP 200、参数 3、全部步骤通过，不覆盖当前新增 16 项。下方旧运行中记录为历史，
不重跑已终态 CI；目标 active，不升级 SDK/部署/操作生产数据/发送交易，PumpAPI 暂停保持。

最新运行句柄：C3d1 `87f86775c533ab18306abed634c1827eb02e98ec` / Actions `43686886493`
已推送独立源码分支并确认 in_progress；本批 18 项与全部完整套件待其终态结果。
978 文件/12,295,075 字节经路径白名单/常见敏感字面量/diff 审计，真实 main/索引未改，
不包含其他服务改动、运行配置、钱包、数据库、日志或二进制。不重新等待已成功的 C3c2。
主线继续 C3d2 索引/有界扫描/worker，保留本批 CI 失败时优先修复的门槛，不把类型检查当运行通过。

当前 C3d1 已在源码接普通 SDK 原签名的显式只读 context/历史 replay/原库 finality/CAS/读回，
typed operation 与原请求/hash/候选/签名绑定，不恢复 owner、不取 signer、不重新发送。
只允许 signed/broadcasting/submitted/unknown，只有合法 finalized 证据可推进 confirmed/failed；
迟到 receipt 不覆盖终态，Application 签后 failed 需独立验证最终证据，否则保持未知。
新增 18 项仅受限类型检查（最终 test 约 9 秒/RSS 1,293,972 KiB；bin 约 6 秒/RSS 764,880 KiB），
生产仍原五组警告，完整运行待 CI。007/扫描/worker 未扩至 SDK，无新 DDL/配置/API，
后续 C3d2 必须接有界扫描与严格迁移/worker 合同；详见 `SDK_ORIGINAL_SIGNATURE_RECOVERY.md`。
前序 C3c2 `fd1c7cb` / Actions `43685088919` 已终态 success，artifact `14150028402`
下载核对，完整并发 1398（132.54 秒）/串行 1398（235.13 秒），sender 14 及全部 steps 通过。
下方 C3c2 运行中及待验状态保留历史，不再轮询旧终态任务，不把 1398 套给当前新增测试。
目标 active，SDK/依赖/vendor/release/生产边界及 PumpAPI 暂停保持。

最新运行句柄：C3c2 `fd1c7cb8869d670cffbd3c5485744b7e8666f773` / Actions `43685088919`
已推送且确认 in_progress；只轮询这个新任务，不重跑已终态成功的 C3c1 `43683583741`。
972 文件/12,234,449 字节快照经路径/字面量/diff 审计，通过独立临时索引发布，真实 main/索引未动。
本批仅四项串联测试和已验收结果/恢复实施文档，无生产实现、SDK/依赖或运行数据变化。
四项新测试及完整套件仍待此 CI，原 1394 只证明其父提交 C3c1，不提前宣布新批通过。
下方 C3c2“尚未推送”保留为发布前历史；下一实际源码任务是 SDK 原签名只读恢复。

2026-11-15 C3c1 `dd946aaa961f821a000db492fd8c74381180695f` / Actions `43683583741` 已终态成功，artifact `14149574413` 核对，完整并发/串行各 1394 通过，详见下方权威验收。快照 970 文件/12,211,016 字节，白名单、敏感字面量与 diff 审计通过，真实 main/索引及原 manifest/lock/vendor/release 指纹保持。之后 C3c2 新增四项 `sdk_original_sender_journal_integration_*`，真实受控 signer、原 SQLite 账本与实际 RpcClient/sender 串联，底层内存 RPC 在 egress 核对 broadcasting 原字节；成功、错回执、receipt 写失败和取消的测试源码已通过受限 check（约 9 秒，RSS 1,315,280 KiB），尚未推送/运行，不借 C3c1 结果声称验收。先前测试侧 unwrap_err 需要 Debug 的编译错误只改测试匹配，不扩大生产类型 Debug 暴露。SDK 原签名恢复的实际缺口及实施门槛已记录于 `SDK_ORIGINAL_SIGNATURE_RECOVERY_PLAN.md`；没有恢复 owner 或放宽旧聚合 scope。目标 active，PumpAPI 暂停及所有生产边界保持。

仅实施 Trade Service；其他服务既有改动保留，不纳入本队列。已按用户要求创建对话内持续执行任务，本文件记录可续作检查点；不代表额外安装了 systemd/cron 或可脱离运行环境永久执行的守护进程。

## 最新范围与执行指令

当前权威验收：C3c1 `dd946aa` / Actions `43683583741` 已终态 success（14 分 32 秒），
artifact `14149574413` 下载核对，身份和全部 steps exit=0；完整并发 1394（124.26 秒）/
串行 1394（225.17 秒），零失败/忽略/过滤。prepared 22、签名账本 11、controller 10、
受控签名 5、sender 10、executor 8、observer 2、HTTP 200、参数 3、候选 54、probe 59
均通过。当前新增 C3c2 四项串联测试仅受限 check，尚未实际运行；保持源码、离线、生产验收分离。
下一任务按 `SDK_ORIGINAL_SIGNATURE_RECOVERY_PLAN.md` 继续，不能把此次全绿当 SDK 恢复或全协议完成。

以下逐条保留发布及开发当时的历史状态；旧 in_progress/待验数字不覆盖上方已验收结果。

2026-11-15 当前 C3c1 已在源码接通 Durable Live 直接 DAMM SDK 卖出的原 owner→实际后端→受控 signer→原库字节→固定 sender，原 executor 不再对该子路径调用 opaque SDK/补写最终父行；原钱包 getter/唯一 sender 真实接线，其他模式/协议不改。Default typed 回执必须匹配，provider trait Ok 不能冒充回执，只查询原签名/选定注册 RPC；缺少成功证据为 unknown，不换 lane/重签/重播。缺钱包/准备失败及旧 rejected 状态由原 owner 的无签名前事务保存，无新 DDL/配置/API。新增 20 项仅类型检查，完整回归待 CI；最终 test/bin check 约 8/6 秒、RSS 1,242,528/779,480 KiB，生产原五组警告、fmt/diff/policy/来源检查保持。另核对锁定 solana-rpc-client 4.2.2 的 send_transaction_with_config，仅调用 SendTransaction，没有隐含节点版本查询；内存发送器保留单次原字节及配置断言。详见 `SDK_ORIGINAL_DAMM_RUNTIME.md`。这是源码接线，不是部署或全协议/生产恢复完成；SDK 持续只读观测/恢复仍待接，全部生产边界/PumpAPI 暂停保持，目标 active。

C3b7 `83036a4` / Actions `43680602908` 已终态 failure，artifact `14147664555` 核对：candidate 15、prepared 22、signed journal 11、controller 10 通过，受控签名 4/5，完整套件未跑。过期夹具先被生产构造器拒绝；本轮仅 cfg(test) 保留原字段 backdate 注入，不放松生产时效或原测试。下方“运行中”均为历史，不再轮询旧终态任务；最新完整全绿仍 C3b4 1348，不套给当前源码。

2026-11-15 C3b7 已推送独立验证提交 `83036a4ca03a6b66d8ad3623a2e8afa450a0f63e` / Actions `43680602908`，确认 in_progress；只轮询这个新句柄，旧 C3b6 `43678564856` 已终态失败且 artifact 已核对。快照 965 文件/12,143,363 字节，路径白名单/常见敏感字面量/diff 审计通过，不含配置、钱包、数据库、日志或二进制。最终受限 test/bin check 约 10/4 秒，RSS 1,216,508/616,236 KiB，21 组生产警告保留；新五项、前序 controller 与完整套件仍待该运行结果。真实 SDK 签名核心已有源码，真实钱包 getter/唯一 sender/原 executor 仍未接，不能用受限 check 或 fake sender 宣称正式链路完成。主线继续发送证据/原签名观测与实际接线，不升级/部署/触碰生产数据/广播，PumpAPI 暂停、目标 active。

C3b7 补充收口：新增五项（含十组规范化 MEV/provider 与两项原 wire 冲突拒绝），签名核心只复用原 provider 映射的 crate 可见函数，不改原选择器/校验；新临时钱包 Default 夹具声明 use_mev=false，旧 nominal/384/576 oracle 不改。最终受限 test check 约 10 秒、RSS 1,216,508 KiB，生产 bin check 通过，21 组警告保留；新门槛/前序未跑的 controller 与完整回归待 CI。正式 executor/真实 getter/sender 和全方向仍未接完，不能把真实核心的离线验证当作发布完成。

2026-11-15 当前 C3b7 新增一次性 SdkSigningPermit（仅原 signing CAS 后构造）及真实 DammSellCandidate 受控签名/私有签后 artifact；原 controller 第一测试后端源码改为调用实际签名核心，尚待 CI 执行。原 owner/metadata/message、raw/报价期限、固定完整消息及公钥先检查再取 key，签后保留原候选和 request ID/hash；编码/核验不确定保留 typed 已知签名。新增五项 key 获取/替换/跨订单/过期/MEV 负例类型检查通过，正式 executor/真实 getter/sender 未切换，供应商回执/原签名观测与完整政策验收继续。见 `SDK_CONTROLLED_SIGNING.md`。

C3b6 `bd39f04` / Actions `43678564856` 已终态 failure，artifact `14147911636` 核对：candidate 15 通过，prepared 21/1，后续 controller/全套未运行。失败为故障夹具非法 Quote UPDATE 被原 CHECK 提前拒绝；本轮先断言约束拒绝，再仅在隔离连接临时忽略 CHECK 注入坏行测试 reader 并恢复 OFF，生产约束/负例不放松。C3b5 prepared-only 修复已执行通过，不重复实施。下方旧“运行中”保留历史，不再轮询或重启旧提交；当前新四项和前序未运行门槛待新 CI，最新全绿仍 C3b4 1348。用户恢复后目标 active，所有生产边界不变。

2026-11-15 C3b6 已推送独立验证提交 `bd39f0487f8578aefa9fecda43db8cc120358c43` / Actions `43678564856`，确认 in_progress；只轮询这个句柄，不再等待已失败的 `43676492857` 或重跑原源码。新快照 963 文件/12,116,998 字节，路径白名单、常见敏感字面量及 diff 检查通过；真实 main/索引、manifest/lock/vendor/release 身份未改。当前十项新控制器测试与 C3b5 的 11 项均待这次实际验收，不把受限类型检查当运行通过。正式 executor/真实授权 signer/sender 仍待接，任务 active，无生产动作。

2026-11-15 当前 C3b6 完成原 SDK Application 编排/项目 ports 与原 SQLite 方法接线，十项新增离线用例及 C3b5 修复待 CI；受限 test/bin check 约 9/5 秒、RSS 1,178,828/677,464 KiB，fmt/diff/policy 七项/来源检查通过，15 组生产警告保留。原 owner/Candidate 被消费，prepared 与签名两次独立读回、CAS await 后 live 授权重查、原字节一次领取、回执只认原签名、取消无重试；共有 JournaledTradeOutcome 仅替代原聚合内部结果结构定义/兼容 re-export，不改 API。新增 pre-sign failed 仅可在 claimed/prepared，signing 后不降级；正式 executor/真实 signer/sender 未切换，完整政策仍待。见 `SDK_SUBMISSION_APPLICATION.md`。

C3b5 `3bd979b` / Actions `43676492857` 已终态 failure，artifact `14146766179` 核对：类型/测试构建通过，candidate 14/1，后续专项/全套未执行。失败定位为 prepared reader 对 signing 阶段未拒绝；本轮 reader 加精确阶段门槛，原 signing 读取拒绝断言保留，合法 signing 的 claim 要求 AlreadyHeld，其他损坏字段拒绝不放松。下方旧“运行中”是历史，不再等待或重新触发同一提交。最新全绿仍 C3b4 1348，不套给 C3b5/C3b6；中断后用户已恢复 active，全部生产边界/PumpAPI 暂停保持。

2026-11-15 C3b5 已推送独立源码快照 `3bd979bca10aaced601b4544db35252e80384862` / Actions `43676492857`，确认 in_progress；后续只轮询此句柄，不在小 VM 大链接、不重复触发。快照 958 个文件/12,066,027 字节，白名单/常见敏感字面量/差异检查通过，不含运行配置、钱包、数据库、日志或产物；main 仍 `00c9b7a`、真实索引为空。当前 11 项 Rust 用例及完整回归仍待此运行验收，不套前序 1348；下一核心为原 owner 的实际 Application 编排/完整业务政策/受控 signer/固定 sender 与只读恢复，不能从 nominal metadata 恢复授权或调用旧 opaque worker 补单。目标 active，全部生产边界与 PumpAPI 暂停保持。

2026-11-15 C3b4 已云端验收：`b1fbc86` / Actions `43675181497` / artifact `14145658342` 下载核对，所有步骤 exit=0；完整并发 1348（114.52 秒）/串行 1348（213.36 秒）零失败/忽略/过滤，compiled-binding 4、候选 54、prepared 11、HTTP 200、probe 59、参数 3 通过。下方 C3b4“运行中”保留为历史，不能重启或混为当前 C3b5 通过；当前 11 项 SDK 原 owner 签名字节账本用例待独立 CI。目标 active，正式路径未切换，无生产操作。

2026-11-15 当前推进 C3b5 原 SDK owner 签名字节账本：复用原 005/008/009 authority/列，新增 signing/signed/broadcasting/receipt CAS、严格原消息/签名核验、独立读回及一次性原字节领取；11 项离线用例待云端运行，受限类型检查及双 pool 增补复检通过（test 约 9 秒/RSS 1,189,688 KiB，bin 约 5 秒/RSS 652,524 KiB），fmt/policy 七项/来源检查通过。正式 executor/签名/发送未切换，完整业务政策及实际编排仍是主线，不把存储阶段当授权。生产目标当前 12 组警告（原七组 + 未接正式路径五组）保留，不抑制；无 SDK/依赖/迁移/配置/生产操作。详见 `SDK_ORIGINAL_SIGNED_JOURNAL.md`。前序 C3b4 已推送 `b1fbc86` / Actions `43675181497`，确认运行中，不能把前序 1344 套给新代码；目标 active，PumpAPI 暂停。

2026-11-15 C3b2/C3b3 云端验收完成：`3f37b3e` / Actions `43673491613` / artifact `14145199925` 已下载核对，全部步骤 exit=0；完整两线程 1344（112.97 秒）/串行 1344（209.88 秒）零失败/忽略/过滤，输出证据 11、预览 13、候选 50、probe 59、HTTP 200、参数 3 通过。原 384 ABI 比较及新增 128 拒绝断言实际执行，先前夹具溢出已修复，不再等待该终态任务。当前 C3b4 编译绑定新增四项仅类型检查，不套用 1344；下一主线仍是完整政策、原 owner SDK 签名账本/受控签名/原字节发送，未完成不因本次全绿剔除。目标 active，无 SDK 升级/发布/生产数据/签名广播，PumpAPI 开发仍暂停。

2026-11-15 C3b3 修复已提交独立验证分支 `3f37b3e` / Actions `43673491613` 运行中；当前还在开发 C3b4 编译消息绑定，不覆盖该 CI 的源码或把它当四项新用例通过。C3b4 将 SDK 编译结果与标准 Solana 从冻结指令构造的完整消息核对，实际接候选/full guard/模拟/metadata 重构；四项新用例覆盖 96 组和消息/权限/顺序/上下文/wire 变化。受限 test/bin check 约 8/3 秒、RSS 1,163,716/547,048 KiB、原七组警告及 fmt/diff 通过，待运行验收。原参数/准备/executor/依赖/vendor不改，无新 RPC/配置/迁移或生产操作；详见 `DAMM_COMPILED_MESSAGE_BINDING.md`。下一核心仍是完整政策与原 owner 的签名前账本/受控签名/原字节发送，不把编译器核对当完整授权；目标 active。

2026-11-15 用户暂停后已明确恢复 active，继续非 PumpAPI 源码队列。当前 C3b3 补原报价 slot 到预览的 minContextSlot/响应核对、拒绝替换 blockhash/非空 loaded addresses，并要求正式/预览共享原 quote Arc；无新 RPC/SDK/配置/迁移。另修 C3b2 的 u64::MAX 合成夹具溢出，保留 384 ABI 比较、增加原 15% MAX 的 128 拒绝断言及零滑点合法对照，不改生产 min_out。新增九项用例；test/bin 类型检查通过（约 8/5 秒、RSS 1,183,732/629,056 KiB）、原七组警告与 fmt/diff/来源检查保持，尚待完整 CI。详见 `DAMM_PREVIEW_CONTEXT_BINDING.md`；正式 SDK owner 签名账本仍未接，不做生产/部署/交易操作。

首轮 C3b2 CI `80bd7c1` / `43671447982` 已终态 failure，artifact `14144671626` 下载核对。类型/测试构建、candidate metadata/prepared、参数、HTTP 200 通过；DAMM candidate 46/1，失败为测试夹具 TryFromIntError，后续专项及全套未运行。下方旧“运行中”只保留历史，不再等待该终态句柄或以 1321 声称当前通过。

C3b2 已推送独立验证分支，commit `80bd7c1` / Actions `43671447982` 运行中，新增输出证据专项及原完整回归保留；这是当前须读取的 CI 句柄，不重复触发或在小 VM 链接测试。真实 main/索引与生产数据/服务未改动。

2026-11-15 当前 C3b2 开发已接本笔 DAMM 净输出证据至原准备/profile/candidate：保留原检查/模拟的上下文，沿余额 slot 指定 minContextSlot，正式/preview 共用不可续期 Arc，精确绑定池/资产/输入/blockhash 与整数 min_out；无新 RPC/依赖/迁移/配置。新增 14 项 Rust 用例待云端运行；受限 test check 约 9 秒/RSS 1,234,780 KiB，bin-check 约 5 秒/RSS 673,900 KiB、原七组警告及 fmt/diff/来源检查通过。原准备/参数/executor/vendor/manifest/lock/release 不改，不自动接 signer 或宣称原 owner 签名账本已完成；详见 `DAMM_OUTPUT_QUOTE_BINDING.md`。目标 active，PumpAPI 开发暂停，无生产/部署/交易操作。

前序 NP-05-A 来源 gate 已云端验收：`6397b0e` / Actions `43669338197` / artifact `14144266219` 已下载核对；源码 checker/八项负例、policy 七项与完整两线程 1321（123.73 秒）/串行 1321（222.43 秒）均通过。后续旧“运行中”是历史记录，不重复执行来源调查或把旧结果套给当前 C3b2。

来源 gate 已推送同一独立分支：commit `6397b0e` / Actions `43669338197` 运行中。其 Rust 源码/迁移/manifest/lock/vendor 与已验收 `35e4362` 相同；新增 checker/八项负例及 policy 第七项的云端结果仍须核实。后续只汇总文档的变更不应无限触发同一源码的大构建；任何源码/脚本/来源清单或 CI 行为变更仍需真实验证。

2026-11-15 C3b1 云端验收完成：commit `35e4362` / Actions `43667516925`、artifact `14143511883` 已实际下载核对，所有步骤 exit=0。完整两线程 1321（123.08 秒）/串行 1321（221.71 秒）零失败/忽略/过滤；prepared 11、candidate 15、DAMM candidate 43/probe 48、HTTP 200、参数 3、lanes 19 通过，七组原生产警告保留。新增 17 项不再是“仅类型检查”，但正式 SDK prepared/signing/签名落盘/原字节发送仍未接，009 生产演练未完成。前述三次失败是设施诊断，不重复实现 C3b1；见 `GITHUB_ACTIONS_VALIDATION.md`。

同轮 NP-05-A 取得固定公开 SDK 6.0.2 commit `dc97b56c602a13d8967973c6eadcf79ab1fcddaa`（tag/commit 均 unsigned），完整比较上游 372/本地 375 文件：364 相同、8 修改、3 新增，全属既有 DBC 扩展，486 行补丁隔离重放后完全一致。新增只读来源 checker、八项负例及 CI 步骤，本机通过，尚未声称这些新增 Python 项已在上述旧 CI 执行。未改 vendor/manifest/lock/Rust 生产代码、release 或其他服务，无生产/签名/部署操作；目标 active，PumpAPI 开发仍暂停。来源/补丁账本及后续门槛见 `SDK_PROVENANCE.md`。

2026-11-15 当前云端验证为 `35e4362` / Actions `43667516925`，独立分支 `ci/trade-service-validation-20261115-c3b`。此前三次 CI-only 环境缺口（原 Git 依赖 404、缺 rg、Anchor 包外 IDL 缺失）已修正；原源码/迁移/manifest/lock/SDK vendor 与快照一致，尚待完整结果。用户环境选择和设备授权均已完成，不再重复询问/本机大编译。main 与真实索引未变，未部署/重启/操作生产数据或签名发送。

2026-11-15 独立验证分支 `ci/trade-service-validation-20261115-c3b` 已推送，首个提交 `ffc506e`，Actions `43666176316`。源码/资源检查通过；在依赖下载阶段，原锁定 zap-program 返回 404/认证失败，尚未编译或测试。CI-only 源替换保留原 Git 提交，使用 Cargo vendor 三包及逐文件校验，详见 `GITHUB_ACTIONS_VALIDATION.md` 与 `../../ci/README.md`。不改 SDK/manifest/lock、不注入本机登录凭据、不改真实索引/main、不部署。

2026-11-15 用户已选择 GitHub Actions 完成完整 Rust 验收，并完成设备授权读取私有仓库 CI。现准备独立源码快照分支，不合并或改写远端 main，不改变工作区 main/真实索引；只发布 Trade Service 源码/原 vendor/文档/公开 fixtures 与验证设施，排除所有运行配置/钱包/数据库/日志/产物和 Copy/Bridge/sdk9 修改。工作流固定 Rust 1.95.0，下载依赖后以仅回环网络 namespace/env-i/普通 runner 身份执行 offline 类型/专项/完整并发及串行套件，无 release/部署/交易。policy 五项和本地隔离 probe 通过；实际 CI 尚待启动/验收，见 `GITHUB_ACTIONS_VALIDATION.md`。目标 active，PumpAPI 开发仍暂停。

2026-11-15 用户已明确恢复持续目标为 active。本轮接续 C3b1，补 SDK 行自身 schema/operation 检查、持久化规范编码校验及三项 Rust 恢复/拒绝用例，并新增实际 009 DDL 的六项轻量 SQLite 检查，全部 DDL 检查通过。受限 test 类型检查约 15 秒/RSS 1,169,156 KiB、正式 bin 类型检查约 11 秒/RSS 972,892 KiB 通过；bin 为七组警告（原五组 + prepared helper/方法尚未接生产的两组），未抑制。详见 `SDK_CANDIDATE_PREPARATION.md`。当前 17 项新增 Rust 用例仍未运行，不能套用旧 C3a 1304 全套结果。未再次链接大测试目标，构建环境选择已询问，继续可安全离线的源码/合同工作；正式 SDK 发送入口未切换，不伪造 Quote/恢复 owner，不升级/部署/重启、不操作生产钱包或数据库、不签名广播，PumpAPI 开发仍暂停。

此前诊断窗口曾按用户设置暂停目标，只处理“卡在验证阶段”。当时查到主机 08:48/09:05 重启、旧编译句柄/进程消失，未查到解释此前重启的 OOM 记录；受控编译则触发开发 cgroup 内存上限及超时，未开始测试。该资源条件仍未改变，禁止据旧 running 标记重复启动。开发入口与日志见 `DEVELOPMENT_CHECKS.md`；上面的新 active 指令只恢复非 PumpAPI 源码任务，不自动扩大资源或生产授权。

2026-11-15 最新 NP-01-C3a：原 claims 增加 SDK 操作分型，008 事务迁移逐列/SQLite storage class 保留旧行并保持原 FK/索引，未知旧 DDL/列/触发器/依赖拒绝并回滚；owner 绑定完整命令、钱包/字面键及原身份。仅 Durable Live 直接 DAMM 卖出可取得新类型占用，仍不进入旧聚合 quote journal，正式 SDK 入口未切换。详见 `ORDINARY_SDK_CLAIM_OPERATIONS.md`。

新增 15，最终专项 26、完整并发 1304（145.68 秒）/串行 1304（251.95 秒）、HTTP 200、参数合同 3、候选 37 与 check/fmt/diff 通过，仍五组原警告。首轮沙箱全套 1265/39，38 为 loopback/信号权限，另 1 为 strict constructor 与旧非法命令 fake fixture 的冲突；仅修正 cfg(test) fixture 并补严格构造器负例，获准环境重跑后全绿，无跳过/放宽原测试。008 仅写隔离库，生产迁移/空间/写锁/备份恢复/回滚仍另行验收；manifest/lock/vendor/release、正式 executor/SDK/参数/准备/candidate/PumpAPI 保持。不部署/重启、不签名发送、不动生产数据/其他服务，PumpAPI 开发继续暂停，完整目标 active。

下一为 C3b：独立 SDK prepared metadata 与完整消息证据、原 owner 下 prepare/authorize/signing/signed/receipt CAS/独立读回，再接受控正式签名及已落盘原字节发送。不能伪造 Quote、新建恢复 owner 或把新分型当作正式 SDK 已有 signed journal；完整方向/协议/backend/mode/worker/RPC/来源/真实环境门槛继续。C3a 已验证项不重复实施。

2026-11-15 前序 NP-01-B4c1：完整零签名 `DammSellCandidate` 已由公开参数构建、私有持有原 profile/指令/交易，无 signer/RPC/sender 或可写交易出口；原正式 guard 也持该产物验消息/返回签名。真实 DAMM 卖出的 `simulate=true` 与 Live preview 改为固定只读模拟，不调用 SDK executor/sign；成功模拟 `simulated/signature:null`，失败预检/超时/取消/过期不释放正式闭包，原 3 秒余额不续期。原 SDK 方法禁播策略不放宽，其他方向/协议不变；正式 Live 仍 opaque SDK。详见 `DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md`。

新增 20 项（SDK 12、RPC 7、HTTP 1），最终 readonly/HTTP 专项 8、候选专项 37、完整并发 1289（142.51 秒）/串行 1289（246.58 秒）、HTTP 200、probe 48、lanes 19、原参数合同 3（2,016 组）与 check/fmt/diff 通过，仍五组既有生产警告。旧 576/384 oracle、参数/hash/JSON 不改，无 migration/owner/Quote/配置/生产数据/签名发送/部署操作；manifest/lock/vendor/release、原准备/参数/input guard、executor/钱包 registry/PumpAPI、root SDK/lane preparation 指纹保持，必要接线/产物文件见验收页。不改 Copy/Bridge/sdk9，PumpAPI 开发仍暂停，完整目标 active。

B4c1 当时的下一主线是正式交易的完整证据授权与原 ordinary owner 的签名账本/原字节发送。当时 claims operation CHECK 及 prepare metadata 仅适用 aggregator SOL buy；操作分型与兼容事务迁移现由顶部 C3a 接续，独立 SDK metadata/CAS/readback/恢复仍须继续，不能伪造 BoundQuote 或另造恢复 owner。当前产物实际用于模拟/正式 guard，但未从它直接受控签名发送；原 RPC 的内部重试/缓冲、全方向/其他 backend/mode、worker/多签名和真实发布门槛仍未完成。

以下 B4b 及其当时队列为历史；其 SDK preview 调用已经由上述零签名分支接续，不重复实施或混用旧数字/指纹。

2026-11-15 NP-01-B4b：实际普通 DAMM 卖出的正式/preview nominal profile 已分别冻结并接原 prepare/pre-invoke/invoke。纯公开参数生成原业务指令，原输入 snapshot 以同一 Arc 共享；项目私有 middleware 逐项检查 SDK protocol/full 指令、原 Normal 费用/具体 provider tip、编译单 payer 与 1232 wire 上限，并用冻结 bytes 核对返回签名。取消/重复/乱序回调拒绝，原 3 秒期限不续期；原任意 middleware 仍拒绝，其他协议/买入不变。共用 swap2 ABI 提取后原 576 probe 消息摘要保持。详见 `DAMM_FINAL_CANDIDATE_GUARD.md`。

新增 25 项，最终专项 25、完整并发 1269（139.95 秒）/串行 1269（239.16 秒）、HTTP 199、damm_probe 48、ordinary_lanes 19、原参数合同 3（2,016 组）与 production check/fmt/diff 通过，仍五组既有生产警告。原准备/参数/input guard、manifest/lock/vendor/release、executor/钱包 registry/PumpAPI、root SDK/lane preparation 指纹不变；lane execution/原 probe 投影/新模块有意变化，见验收页。无新迁移/配置/生产数据/签名发送/部署操作，不改 Copy/Bridge/sdk9，PumpAPI 开发仍暂停，完整目标 active。

这仍是 nominal candidate 一致性检查，**不代表独立可信全协议授权或普通 SDK 广播前账本完成**；SDK 内部仍可签名发送，取消无法撤销已经通过 callback 的 worker。下一主线 B4c/C/D 收敛唯一不可变可签产物与完整证据授权，接原 ordinary owner 的 journal/CAS/独立读回/原字节 sender，取代 opaque 内部重建签名广播；扩展全部方向/协议及其他 mode/backend，不继续用中间检查代替这条主线。剩余全局读取/资源、通用 backend 合同、可信 SDK 来源/演练与真实环境门槛均保留。

以下 B4a 及其当时队列为历史；B4b 当时实际交付以上述 B4b 检查点为准，最新进度见顶部 C3a。

2026-11-15 NP-01-B4a：已核对主 vendor 6.0.2 的普通 buy/sell -> instruction -> 模拟/worker -> compile -> sign -> send 实际调用链。高层 Prepared/without_rpc/simulate 均不能当作无 signer 最终构建；公开 `compile_message` 可复用，但不验证额外 signer、权限提升、费用/min_out、blockhash 时效或最终 wire 大小。新增九项无 Keypair/RPC 的编译/费用合同，记录 Legacy/V0/V1/ALT 与共享可变 fee clone 的实际行为；没有添加平行生产 builder 或放宽 middleware。详见 `ORDINARY_UNSIGNED_BUILD_AUDIT.md`。

最终专项 9、完整并发 1244（138.95 秒）/串行 1244（238.19 秒）、production check/fmt/diff 通过，仍五组既有生产警告。此批仅新测试模块及文档，A3c 的生产源码/配置/迁移/依赖/vendor/release 指纹不变；未部署、未访问生产数据、未签名发送，不改 Copy/Bridge/sdk9，PumpAPI 交易开发继续暂停，整体目标 active。

B4a 当时下一项为 **NP-01-B4b 最终协议指令/消息 profile**，现由上述 DAMM nominal 子项接续，其余全方向/完整授权与账本仍未完成：不能使用 quote-only probe 或重新构建的 preview 代替正式消息；之后扩展全部现有方向/协议并进入原 authority 的 signed journal。完整任务及无 signer/真实接线门槛见审查页，不把入口审查当作最终授权完成。

以下 A3c 是已收尾的前序检查点，其最终专项与检查已补齐：

2026-11-15 NP-01-A3c：实际准备余额 reader 已接 main 原 SharedReadRuntime 的第六类 profile；只持公开钱包/当前已注册 lane URL/同一 runtime Arc，不持 SDK RpcClient，也不扩大候选。scan -> raw 原账户与参数/模式绑定保留；一笔操作容量、250 ms HTTP 起点、单步含等待 2 秒/整次 4 秒、原 3 秒新鲜度不续期，scan 512 KiB/raw 64 KiB 流式体积上限和共享 429 冷却/取消已接真实 HTTP。旧 reader/17 项测试完整移至 cfg(test) oracle，不重复计新增。相对旧逻辑读，慢 scan 在新单步超时后可由合法 raw 在原有效期内恢复，这是明确记录的预算变化。

新增 19 项（runtime/codec 12、实际 HTTP 6、原 Arc 复用 1），preparation_ 90、完整并发 1235（139.07 秒）/串行 1235（237.37 秒）、HTTP 199、shared_read 89、quote_balance 25、原参数合同 3（2,016 组）与 production check/fmt/diff 通过，仍五组既有警告。manifest/lock/vendor/release、executor/钱包 registry/PumpAPI、原准备/参数与 SDK input guard 指纹不变；必要 wiring 变化与 oracle 证明见 `PREPARATION_SHARED_RPC_RUNTIME.md`。无 production/SDK 升级/签名/发送/部署操作，Copy/Bridge/sdk9 不动，PumpAPI 开发仍暂停。

A3c 当时下一主线是 **NP-01-B 普通 SDK 不可变构建与最终消息授权**：入口核对现由上述 B4a 接续；不能把 Prepared 命名、模拟、只读 probe 或中间参数绑定当作已取得最终可签消息。其后完成全方向的池/账户/程序/费用/min_out/权限授权、原 authority 广播前唯一 signed journal、受控发送及其他 mode/backend；worker 取消/多签名、剩余读取/资源治理、双 fake backend、可信来源/升级演练和真实环境验收继续计入完整目标。

以下 A3b2 及其当时 A3c 队列为历史；A3c 实际交付以上述检查点为准。

2026-11-15 NP-01-A3b2：raw 输入 account/program/精度随项目 snapshot 保留，metadata-only 构造不能通过新的身份检查；真实 LaneExecution hook 保留 payer/Mint/数量/比例，再核对实际 SDK 输入资产、DexType/params/方向/default、program/canonical 账户/明确声明精度、simulate 和实际 seed 标志。覆盖九类普通卖出及原谓词内直接 Quote 买入，Pump default/后缀、反向池、StonkFun meme/hop 连续性均检查；DAMM final preview 也绑定同一证据并强制模拟，模式不符/缺失/意外 preview 在 SDK 前拒绝。无新 RPC，不改原准备/参数投影或最终 SDK 执行行为。

新增 17 项（domain 2、SDK 输入 15，其中 controller 两项不额外重复计算），最终 preparation_ 71、input 15、balance 38、完整并发/串行各 1216、HTTP 199、shared_read 71、quote_balance 25、原参数合同 3 与 production check/fmt/diff 全部通过，仍五组原警告。最终并发 127.60 秒、串行 217.33 秒，详见 `PREPARATION_SDK_INPUT_BINDING.md`。首轮 fixture constructor 名称错误已修正，随后一次 SIGKILL 无测试结果；-j 1 首检查点 1215 通过后发现 preview/模式检查需补齐，已在最终源码重跑，不混用旧结果。未更改 profile/缓存/系统配置，内核日志读取无权限，不猜测 SIGKILL 原因。

依赖/vendor/release、原准备/参数文件、root SDK、lane preparation、executor/钱包 registry/PumpAPI root 与 A3b1 一致；实际 lane execution 接线、domain/raw read 的身份传递与新增 SDK input helper 为必要变化，详见验收页指纹。不访问生产钱包或库、不签名发送、不部署/重启，不改 Copy/Bridge/sdk9，PumpAPI 开发继续暂停，目标 active。fixture 不创建 SDK wallet/signer，不把 fake invoke、源代码参数检查或单服务离线测试当链上/最终消息验收。

A3b2 当时下一项为 **NP-01-A3c / 准备读取的共享 HTTP 治理**，现已交付上述子项，原要求保留：

1. 冻结当前每 lane 的 scan -> raw 请求顺序、精确 keys/processed/jsonParsed/base64/minContextSlot，及坏扫描不 fallback、全未解析/不可用只能 raw 核验、零/缺失/坏证据与数量/程序/精度差异；不强塞 UI supply/SOL 或扩大原节点候选。
2. 接 main 原 SharedReadRuntime 的独立准备 profile，明确容量/起点间隔、逐笔和整次 deadline、流式 body/账户上限、共享 429 冷却与取消，不以 SDK 两次逻辑方法调用冒充 wire 数量/无内部重试。
3. 用无 signer 的实际 read port/生产 transport + 隔离 loopback 验证真实请求次数、ID/响应、redirect、headers/body timeout/超限、429/冷却/忙时、取消及 blockhash 名额不饥饿；保留原新鲜度、输入绑定与 SDK 后不回退，不靠续期/补读/重签通过。
4. 完整不可变消息/全协议与方向/费用/min_out/权限授权、opaque SDK 广播前唯一账本、其他 mode/backend、worker 取消/多签名与恢复、其他读取/资源/依赖隔离、NP-04 双 fake backend、NP-05 可信来源/升级演练与真实环境验收仍保留。A3c 也不代替这些完整门槛。

以下 A3b1 与当时 A3b2 队列为历史，实际输入绑定现由上述子项接续，不重复实施已验收内容；未读余额的其他买入与最终 SDK message 仍未因本项获得完整授权。

2026-11-15 最新 NP-01-A3b1：真实准备 read port 原 owner scan 之后必须批量读取原始 Mint、classic/Token-2022 两个规范 ATA；按实际 Mint owner 选择输入，核对 slot 不回退/数量/位置、初始化/精度/权限/扩展、raw 整数、native reserve/lamports 与 supply。正常 parsed 数据须与 raw 的数量/程序/精度一致；坏扫描不能被 fallback 隐藏，不可用或全未解析扫描只能以合法 raw 账户恢复。未使用的另一程序 ATA 合法系统空账户预存款不计入数量，不因灰尘 SOL 误判程序冲突。原准备/参数文件与实际 lane/controller 不改，3 秒新鲜度从整次开始，scan + raw 共用 4 秒逻辑 deadline，不恢复数字余额回退。

新增 16 项（raw 10、RPC 6），preparation_ 54、preparation_assets 10、preparation_balance 36、完整并发/串行各 1199、HTTP 199、shared_read 71、quote_balance 25、原参数合同 3 与 production check/fmt/diff 全部通过，仍五组原警告；完整并发 129.08 秒、串行 214.07 秒，详见 `PREPARATION_RAW_ACCOUNT_EVIDENCE.md`。内存 RpcSender 仅证明逻辑读/配置/错误分类/取消，不是 wire、真实节点或签名证据；仍有 SDK 内部重试/HTTP 缓冲，解码后 1000 账户和 raw 8192 字节上限不冒充流式 body 或共享预算。尚未部署，无 migration/配置/依赖/vendor/release/生产钱包或库/签名发送操作，PumpAPI 开发继续暂停，完整目标 active。

A3b1 当时下一项为 **NP-01-A3b2 / 实际 SDK 输入与原始证据绑定**，之后 **A3c / 共享准备读取治理**，以下保留当时要求：

1. 逐项核对九类普通协议及 StonkFun wrapper 的实际输入 Mint/Token Program/方向/default，尤其 Pump builder 的 default/`*.pump` 强制 Token-2022 行为；把 raw program/精度/canonical 输入在真实 pre-invoke hook 与 SDK 参数绑定，不改 owner、vendor 或旧参数投影来迎合结果。
2. 用无 signer fixture 验证有效/错误/默认/反向/重复身份、过期与不回退合同；原始账户合法不自动保证 SDK 最终使用同一账户/程序，余额 port 尚未覆盖的其他买入/SOL-hop 资金验证不能遗漏。
3. 接 main 原 SharedReadRuntime 的第六类真实只读 profile，明确候选/独立容量/总 deadline/HTTP 请求数与流式体积/共享 429 冷却/取消；不能以两次逻辑方法调用替代实际 wire 测试。
4. 其余完整最终消息/费用/min_out/权限授权、opaque SDK 广播前 journal、其他 mode/backend、worker 取消/多签名、剩余读取治理、双假构建后端、依赖/可信 SDK 来源/隔离演练与真实环境验收均继续，不以余额子项改写完整目标。

以下 A3a 与当时 A3b 队列保留为历史；raw 核验/安全 fallback 已由上述 A3b1 接续，但其完整 SDK 输入绑定和预算要求未完成。

2026-11-15 最新 NP-01-A3a：普通 SDK 准备余额已经通过公开地址 read port、parsed metadata snapshot 与 SDK 入口 guard 接入真实 LanePreparation/LaneExecution。核对钱包/Mint/program/统一 decimals、initialized、重复/混合与 canonical 正余额；3 秒期限从读开始，慢结果不刷新时间，解码后最多 1000 账户。原准备/参数文件原字节保持，不增加 UI supply/SOL 读取；旧余额 helper/数字 ATA fallback 仅测试保留。SDK 前校验实际 payer/Mint/输入、卖出百分比的 u64 乘法与零值；Invalid/Expired 不换 lane，仅 Unavailable 允许原准备回退，SDK 后拒绝保留已知 preview 签名与原 unknown/不回退政策。未解析/非 canonical 正余额/seed 输入暂时 fail closed，不代表原始账户或完整签名授权。

新增 30 项（domain 4、SDK 规则 7、内存 RPC 11、真实 controller + fake read 6、入口后快照/签名 2）。最终 preparation_balance 30、完整并发/串行各 1183、HTTP 199、shared_read 71、quote_balance 25、ordinary_preparation 8、ordinary_param_contract_tests 3，production check/fmt/diff 全部通过，仍五组既有警告。完整并发 125.65 秒、串行 208.46 秒。详见 `PREPARATION_BALANCE_ENTRY_GUARD.md`。内存 RpcSender 只证明逻辑调用，当前仍有 SDK HTTP 缓冲/内部重试，4 秒逻辑 timeout 与解码后数量上限不是 wire/内存/共享配额控制，尚不是第六类共享 runtime profile。

manifest/lock/vendor/原 release、executor/钱包 registry/PumpAPI 指纹与 C2 一致；root SDK 只删除旧私有余额转发，在只读内存流补回它精确重现 C2 SHA。ordinary_preparation/ordinary_params 与旧整文件 helper oracle 原字节保持。无新迁移/配置/发布/生产钱包或库/签名交易操作，既有其他服务和 sdk9 工作树改动不动；PumpAPI 开发继续暂停，整体目标 active。

A3a 当时下一项为 **NP-01-A3b 原始账户/扩展证据与安全回退**，之后 **A3c 准备读取共享治理**，以下为当时完整要求：

1. 复用既有库解析 raw Mint/Token Account，明确实际 program owner、初始化/decimals、钱包/Mint/canonical 输入与扩展支持范围；先冻结错位/缺失/错误身份/冻结/TLV/溢出 fixture，不实例化 signer。
2. 将 raw 证据接真实准备 port；若恢复 ATA fallback，必须读取并验证原始账户，不能只查余额数字；坏/过期证据不换 lane，取消不续查，原有效金额/顺序/参数和 SDK 入口后不回退合同不变。
3. 准备读取接 main 原共享 HTTP runtime，明确独立容量/候选范围、逐笔/总 deadline、流式体积/请求数、429 冷却与取消；实际 wire/无隐藏重试的证据不得由逻辑 RpcSender 测试替代。
4. 完整最终消息授权、opaque SDK 广播前原 authority 签名账本、其他 mode/backend、剩余读取治理、NP-04 双假后端与 NP-05 可信来源/隔离演练仍继续，不把 A3a 或 A3 代替完整目标。生产/发布与暂停 PumpAPI 边界不变。

以下 C2 及其当时 A3 队列是历史快照，不重复实施已验证的 A3a，也不删除其剩余原始证据/预算要求。

2026-11-15 最新 NP-03-RPC-01C2：余额查询 WalletReadPort 的 Quote 操作已接同一个 SharedReadRuntime，第五类独立容量/250 ms 起点、单步 2 秒/总计 8 秒，保留原首个 Default 节点、不扩大备用。通常三笔、两个 ATA 路径最多五笔，processed/jsonParsed/null 配置与 SDK 序列化对照，raw u64/decimals/饱和及原空/ATA 零与错误语义保持。新增钱包/Mint/owner/程序/精度/重复/混合不完整拒绝；scan 512 KiB/1000 账户、其他方法仍 64 KiB，类型化超限错误禁止转 ATA 隐藏，正常不可用仍保留旧回退。Reader 构造时将 SDK RPC handles 投影为 URL 后释放，Wire Weak/实际请求验证；交易准备调用的旧 token_balance helper 原字节不改。新增 23 项（codec 6、runtime 10、旧 helper 差异 oracle 1、wire 6），最终 quote_balance 25（含两项既有合同）、shared_read 71、完整并发/串行各 1153、HTTP 199，production check/fmt/diff 通过，仍五组原警告。见 `QUOTE_BALANCE_SHARED_RPC_RUNTIME.md`。SDK/registry/executor/PumpAPI、manifest/lock/vendor/release 指纹与阶段前一致，无新迁移/配置/发布/真实签名资金操作；PumpAPI 开发继续暂停，整体目标 active。

C2 当时下一项为 **NP-01-A3 / 普通交易准备余额证据**，现 A3a 已完成上述子项，以下仅保留当时完整要求：

1. 核对 `SdkTradeClient::token_balance` -> `LanePreparation` -> `ordinary_preparation::buy/sell` 实际调用与钱包/Mint/程序、百分比/整数/零/ATA 规则，冻结参数摘要、调用顺序、错误与 lane fallback 时点；不得把 Quote Snapshot 的 supply/SOL 查询强行加进准备热路径。
2. 先建立受控、无 signer 的余额证据/读取边界，复现错误 Mint/owner、重复/混合未解析、扩展/精度/溢出与旧 ATA 回退的差异；有效请求的 SDK 参数/金额/hash/JSON 保持，坏证据不能伪造可用余额。
3. 在准备和 opaque SDK invoke 之间验证证据身份、预算与取消；明确不可用/坏证据是否允许下一 lane，任何进入 SDK 后的超时/错误/未知均不授权换 lane、重签或补单。源码/离线测试仍不调用真实 signer 或节点。
4. 继续 NP-01 完整最终消息授权、普通广播前签名账本/跨模式与恢复；其余寻池/账户/模拟/SDK 内部读预算、NP-04 双假后端、NP-05 可信来源/隔离演练仍保留，可独立推进，不把一个余额子项代替全部资金安全或整体完成。

以下 C2 当时队列与 C1/B2 等为历史检查点，不重复实施已验收子项。

2026-11-14 最新 NP-03-RPC-01C1：原生 SOL 批量余额已接 main 的同一 SharedReadRuntime，第四类独立容量/250 ms 间隔，按原 Default 候选串行读取；每笔 100 地址、整次 1000、节点整批 4 秒/总计 8 秒，HTTP 仍 2 秒/64 KiB/无重试重定向。任何页失败丢弃整批、备用从头读取，不拼接节点结果。Decoder 核对 ID/slot/数量、整数及账户/空切片形态；仅明确 null 按缺账户补零。实际 Application 数量异常丢行先失败复现，修复后逐行保留全部可见钱包/错误和原 200；另先复现迟到响应越过 read deadline，现响应后复核，迟到 429 仍共享冷却。新增行为回归 21（Application 1、HTTP 1、codec/runtime 14、wire 5）；原整文件八项 oracle 保留、现行 Quote/token 六项兼容测试继续覆盖，故计数净增 27，不当成 27 个全新独立场景。最终 native_balance 21、shared_read 55、完整并发/串行各 1130、HTTP 199，production check/fmt/diff 通过，仍五组原警告。见 `NATIVE_BALANCE_SHARED_RPC_RUNTIME.md`。manifest/lock/release、executor、PumpAPI、vendor 不改；SDK/registry 仅只读 runtime 注入，还原局部变动精确重现阶段前 SHA，Quote/token 方法字节不变。没有新迁移、配置、发布、真实钱包/库操作或签名交易；PumpAPI 开发仍暂停，整体持续目标 active。

C1 当时下一项为 **NP-03-RPC-01C2 / Quote 余额读取**，现已完成上述源码/离线子项，下列仅保留历史任务快照：

1. 冻结 literal 钱包/可见性、原首个 Default 节点、supply -> token scan/ATA -> SOL balance 的顺序与错误、整数 decimals/amount 和空/未解析/fallback 行为；不能把原首节点规则偷偷扩大为全部节点，或用新读政策重写既有金额语义。
2. 先明确 Token/Mint/owner/账户形态的校验、空与不完整响应的区别、最大响应/账户数和失败不返回部分合计；保留原有效整数与精度，不能将 `parsed.program` 或上游 supply 文本等同可信链上执行授权。
3. 在实际 quote read port 后接 typed 单次只读请求/独立非关键容量和总预算，覆盖 supply、owner scan、ATA 及 getBalance 的真实 wire 次数/429/取消；不抢占 hash 名额，不将五次 SDK 内部重试当成一笔请求，也不新建独立生产 runtime。
4. 寻池、独立 quote/WSOL、模拟及 SDK 初始化/内部读仍另列；最终消息授权、opaque SDK 广播前签名账本、其他 mode/backend、双假构建后端及 SDK 来源/隔离升级演练仍属完整目标。单项依赖缺失时继续可做的独立离线任务，不触碰生产或恢复 PumpAPI。

以下 C1 当时队列与 B2/B1 等为历史检查点，不重复实施已经验收的子项。

2026-11-14 最新 NP-03-RPC-01B2：readiness 已作为第三个独立容量 profile 接 main 的同一个 SharedReadRuntime。保留原 execution_lanes 候选子集/顺序及 metadata，不借全局健康非候选节点兜底；canonical 去重、429 共享冷却、250 ms 起点间隔、忙时立即 false、500/1500 ms 节点/总 deadline，探测本身不写 hash cache。生产 source 的旧采样顺序先复现两项失败，再改为 RPC/metadata/DB 后一次缓存条目/时钟读；原 ready 公式、17 字段、鉴权及 HTTP 200 不变，paused/DB 仍独立。新增 20 项（source 5、cache 1、runtime/probe 10、真实 HTTP 4），原 SDK readiness/三项 oracle 原字节保留。最终 readiness 33、shared_read 39、完整并发/串行各 1103、HTTP 198，production check/fmt/diff 通过，仍五组原警告。见 `READINESS_SHARED_RPC_RUNTIME.md`。manifest/lock/release、executor、PumpAPI 与 vendor 不改；root SDK 仅 probe 的 RPC clone 改 URL 投影，还原这一行后精确重现 B1 指纹。main/source/cache/runtime/probe 为本批必要接线，旧 age-only 方法仅为测试保留。无新 migration/配置/部署/真实交易，PumpAPI 开发仍暂停，持续目标 active。

B2 当时下一项为 **NP-03-RPC-01C1 / 原生 SOL 批量余额读取**，现已完成上述源码/离线子项；下列仅保留当时任务快照：

1. 先冻结可见钱包/地址顺序、缺账户为零、原 metadata/错误行为、Default 候选和串行备用；单独复现响应数量不匹配可能经 zip 丢行的风险，不把异常账户/响应伪造为零。
2. 在实际 WalletReadPort 后接共享只读 transport/独立非关键容量，先明确单次/总 deadline、请求/响应体积、最多账户数/分批及取消合同，再迁移 getMultipleAccounts；不抢占刷新容量，不用 SDK 逻辑调用数替代 HTTP 请求证据。
3. Quote 余额的 supply/token scan/ATA fallback、寻池、独立 quote/WSOL、模拟与 SDK 内部读取保留独立后续任务，不能以原生余额子项宣称全服务限流。
4. 完整消息授权、opaque SDK 广播前签名账本、Memory/Shadow 等模式、NP-04 双假后端与 NP-05 可信 SDK 来源/隔离演练仍未完成；不改变完整目标或生产授权边界。

以下 B1/D1B3 等为历史检查点，不重复实施已完成子项。

2026-11-14 最新 NP-03-RPC-01B1：blockhash 刷新与可选普通聚合状态观测已接 main 创建的同一个共享只读 runtime。两类独立保留容量与起点间隔、共享 endpoint 429 冷却；刷新单节点 500 ms/总计 1500 ms，状态读取保持单节点单次查询。HTTP 不重试/重定向，64 KiB 上限及错误脱敏；坏响应不更新时间。hash 解码保留 SDK 44 字符/固定 32 字节限制，过期 admission 的失败复现已修复，刷新任务显式 abort/join。新增专项 25 项，完整并发/串行各 1083、HTTP 198，production check/fmt/diff 通过，仍五组既有警告。原 SDK blockhash 实现/三项 oracle 原字节保留为 legacy_tests；manifest/lock/release、executor、root SDK、PumpAPI 与 vendor 不改，main/cache/钱包无用 getter/观测 transport 的必要接线已记录。见 `SHARED_READ_RPC_RUNTIME.md`。未部署/启用扫描，不改线上配置、不接触生产钱包或库、不签名发送；本批无新迁移，但整个尚未部署树已有 004–007，未来发布仍需完整备份/迁移门槛。

B1 当时下一项为 **NP-03-RPC-01B2 / readiness**，现已完成上述源码/离线子项。当时就绪探测仍使用 SDK 的内部 429 重试；余额、寻池、quote/WSOL、模拟与 SDK 内部读仍未全部共享预算，资金安全、NP-04/05 与 PumpAPI 暂停项仍属完整目标。

以下 D1B3/RPC-01A 与原 RPC-01B 队列为历史检查点；RPC-01B1 已完成的内容不重复作为待做项。

2026-11-14 最新 NP-01-D1B3 / NP-03-RPC-01A：生产观测 Reqwest transport 的九项 isolated loopback 测试通过（八个父用例与一个 exact helper），覆盖原签名/ID、429 单次与后续健康节点、Retry-After、302、Content-Length/chunked 64 KiB、headers/body timeout、在途并发/取消和坏响应/断开无泄漏。复用 cfg(test) 子进程 helper，代理环境只在 child 清除，原信号测试保留。RPC 配置先提取保留旧合并行为并复现七项本应四项，再修为跨别名精确稳定去重；四项纯测试保留不同 key/优先级/原空白语义，不访问生产配置。最终源码并发/串行各 1058、transport 9、配置 4、HTTP 198，production check/fmt/diff 通过，仍五组原警告。transport/gate、executor、root SDK、PumpAPI、manifest/lock/release 指纹与本轮开始一致，唯一新增生产行为为配置精确重复项合并，未部署/启用。见 `ORDINARY_OBSERVATION_HTTP_TRANSPORT.md`；上一轮 D1B2 为已验证实际进展，不重复作为待做项。

D1B3/RPC-01A 当时下一项为 **NP-03-RPC-01B**，下列为当时队列快照，剩余资金大项和 NP-04/05 依赖保留：

- 先冻结 blockhash 的 commitment、备用顺序、默认 400 ms refresh / 2000 ms max-age、失败保留旧 hash/readiness 合同；在既有 port 后接真实单次只读 transport 与预算，不用 SDK 方法调用数冒充 HTTP 请求数。
- 已核对 Solana RPC 4.2.2 的 HttpSender 有五次内部 429 重试；共享政策需覆盖实际请求、关键刷新优先级、后台公平性、并发/配额/冷却、全部冷却时拒绝与取消不返还已用间隔。不能把普通观测 1 秒间隔直接套给全部请求造成 blockhash 饥饿。
- 从一个实际只读 profile 开始接线并留明确覆盖范围，不建立未接入的平行框架；其余 SDK 内部读仍须逐项收口，不能因此声称全服务预算完成。新读路径不得改变发送 client、重签/补单规则、SDK 或 PumpAPI 暂停范围。

前检查点验证（D1B2）：专项 27、完整并发/串行各 1045、HTTP 198，production check/fmt/diff 均通过；production check 仍五组既有警告，未抑制。Cargo.toml、Cargo.lock 和原 release 指纹与当轮开始一致，vendor 无变化，详见 D1B2 验收页。外部 Copy HTTP、Bridge 事件与 PumpAPI backend 的兼容约束已集中到 `SERVICE_INTEGRATION_BOUNDARIES.md`，未改外部服务代码/消息/API。

D1B2 当时队列快照（已由顶部最新队列覆盖，不改变完整目标、不因记录计划暂停）：

1. **NP-01-D1B3 / transport 验收**：先对生产 Reqwest transport 做隔离本地 HTTP 夹具，覆盖实际请求次数、429/Retry-After、redirect、分块超体积、超时/取消及原签名/请求 ID。当前 fake transport 证据不能替代这些测试。测试必须固定 loopback、避免继承代理导致外联，只运行测试入口，不启动服务 main；不能为测试覆盖全局环境制造并发竞态。
2. **NP-03 / RPC-01**：在既有 read ports 后定义共享只读节点/预算边界，先稳定去重全部配置入口并捕获 serial fallback/deadline/429 合同，再逐路径接实际只读调用；明确 SDK 内置 HTTP 重试与跨钱包/后台竞争，不能用逻辑调用数冒充请求上限。不得把 fallback 扩展到重构/重签已进入执行的交易。
3. **NP-01 剩余资金边界**：完整最终不可变消息/协议权限/费用审计、opaque SDK 原 authority 的广播前 signed journal，以及其他模式/backend 的明确拒绝/能力与取消恢复；不以 D1B2 的子路径完成代替这些大项。
4. **NP-04/05 与 TS-09 后续**：两种假构建后端共享合同、依赖方向检查、可信 SDK revision/补丁账本与隔离演练准备。生产升级、迁移、发布及 PumpAPI 对接继续保留授权边界和未完成门槛。

2026-11-14 用户再次将持续目标设为 active，本轮继续非 PumpAPI 队列。NP-01-D1B2 新增七项回归并修复“前两项 URL 重复就丢失第二独立备用”的问题（先失败复现后修复）；专项 27、完整回归 1045 项通过。文件重开 signed/submitted/unknown 的最终成功/错误及无结果/故障均保留原证据，无新 claim/签名/发送；两个自建 exact-filtered 测试子进程的 SIGTERM/SIGINT 已在宿主实跑，沙箱超时未被跳过冒充通过。默认关闭的扫描、007 索引、局部请求预算/429 冷却和 bootstrap/关闭接线详见 `ORDINARY_PENDING_OBSERVATION_WORKER.md`。README 五项参数和 API 状态说明已补；优化未部署/启用，本轮未操作生产配置、数据库或服务，不升级 SDK 或恢复 PumpAPI 对接。以下暂停审查属于本轮恢复之前的历史记录。

2026-11-14 本轮审查：持续目标实际为 paused，以下 active 表述属于历史检查点。本轮没有恢复优化实施。现有 D1B2 源码（默认关闭的只读扫描、007 索引、局部 HTTP 预算/429 冷却、bootstrap/关闭接线）全套 `cargo test --locked --offline --quiet` 为 1038 passed；fmt/diff 通过，真实节点、进程信号及生产迁移尚未验收，未部署。用户另明确授权 RPC 运维：移除返回 429 的主 Helius 和返回 403 的 PublicNode，以正常 Helius 主用、Chainstack/Alchemy 备用；仅重启原同哈希 release，health/ready 正常，Bot/Bridge 不重启，不发送订单。完整现状与风险见 `../../../docs/optimization/PROJECT_REVIEW_2026-11-14.md`。历史源码检查点的“未联网/未重启”仅指各自当轮，不可扩展到这次授权运维。

2026-11-14 最新 NP-01-D1B1 源码/离线验证通过：实际普通聚合观测核心改接无 owner 的不透明 context，恢复 port 不持 AdmittedTrade/ExecutionOwner/钱包或可发送字节，原身份/请求 JSON/claims/报价与签名证据摘要逐次核对，证据变化拒绝。原 live 捕获同一事务校验原 owner，显式恢复可核对已有 signed 中断，不重签发 owner。新增九项，专项 9、并发/串行各 1018、HTTP 198、production check/fmt/diff 通过，仍五组警告；无新 migration。见 `ORDINARY_RECOVERY_OBSERVATION_CONTEXT.md`。main/executor/root SDK/钱包/PumpAPI/manifest/lock/004-006/release 不改；尚无自动启动扫描/持续确认/全局 RPC 预算，GET/重复及原准入/hash/JSON 合同保持。未部署/联网/真实签名；持续目标 active。

2026-11-14 最新 NP-01-D1A 源码/离线验证通过：Durable 普通 SOL 聚合在本次回执保存后按原签名单次观测，专用只读 journal/reader port，原钱包 Default RPC、3 秒 lookup deadline；只有明确 finalized 在原 claims 的 006 nullable 证据/父行同事务 CAS 并独立读回。旧 004/005 不改，旧行不补 owner。新增 20 项（18 Application/存储、2 Router），专项 20、并发/串行各 1009、HTTP 198、production check/fmt/diff 通过，仍五组原警告。见 `ORDINARY_POST_SEND_OBSERVATION.md`。GET/重复仍不发 RPC/重发；并非跨重启/持续对账，signing/signed 中断与其他 backend/mode、全量最终授权仍未完成。SDK/vendor/manifest/lock/release、executor/主工厂/钱包/PumpAPI 指纹不变，原 controller/quote adapter/journal 和迁移初始化的必要接线有记录。未部署、真实签名或联网；持续目标 active。

2026-11-14 最新 NP-01-B3a 源码/离线验证通过：共享聚合 prepared 保存原到期快照，实际释放待签消息前沿用原身份/权限/形态检查并重查有效期，新增拒绝先于 signer 获取。先新增过期消息回归在原实现失败，再修复并完成三项新增测试；adapter 专项 8、并发/串行各 989、HTTP 196、production check/fmt/diff 通过，仍五组警告。见 `QUOTE_SIGNING_FRESHNESS.md`。不改 API/DTO/hash/SQL/消息或 controller 的错误映射，不自动刷新报价/重签/发送；只影响共用该 prepared 的聚合路径，不恢复 PumpAPI，也不替代完整 NP-01。持续目标 active，未部署。

2026-11-14 最新 NP-03/TS-09-A 源码/离线验证通过：生产 AppState 不再暴露四个冗余 wallet registry/repository、quote cache、conversion store 字段，旧字段只为测试夹具保留。实际服务仍持有原 Arcs；替换夹具不改变请求权威、关闭服务不能被夹具启用，释放 Router 后四资源均释放。新增两项，最终并发/串行各 986、HTTP 196、production check/fmt/diff 通过。本轮 production check 实际为五组兼容/未使用警告，未抑制，详见 `APP_STATE_RESOURCE_OWNERSHIP.md`，不沿用历史两组数字。生产业务/factory 顺序、executor、SDK/PumpAPI、manifest/lock/release 指纹不变。下一批复现并收紧共享聚合签名边界的 Quote 过期重查，再继续 NP-01/其他资源；持续目标 active，未部署。

2026-11-14 最新 NP-02/TS-08-H 源码/离线验证通过：三靓号 Handler 接项目 service/任务 port；创建/取消纯政策归 App，真实 worker 归 adapter，根路径兼容。AppState 移除直接 wallet_ops 句柄。生成/保存/加载主函数字节未变，还原 VanityJob 定义可精确重现阶段前仓储 SHA；原 SQL/密文/流程未改。新增十三项并迁移一个原测试；最终并发/串行各 984、HTTP 194、vanity 筛选 17、check/fmt/diff 通过，仅两组原警告。初轮串行暴露旧 C2b 立即读内存竞态，现以 fake 返回/Drop 门闩及 Confirmed 后零写审计补强，单项连续十次和最终完整回归通过；生产 executor/SDK 未改。见 `VANITY_APPLICATION_AND_WORKER_BOUNDARIES.md`。持续目标 active，计划更新不暂停。

2026-11-14 最新 NP-02/TS-08-G 源码/离线验证通过：六个钱包 Handler 接项目管理服务/仓储及运行 port，创建/导入返回材料为 adapter 私有 owned 类型，只有显式导出返回字符串；原加载补偿/删除部分失败顺序保持。WalletInfo/ManagedWallet 归 domain，旧名兼容；还原唯一 ManagedWallet 类型定义后精确重现原仓储 SHA，证明 SQL/密文/方法字节未变。新增十七项（11 Application+2 adapter+4 HTTP），最终并发/串行各 971、HTTP 191、管理筛选 18、check/fmt/diff 通过，仅两组原警告，无新抑制；后补空串/空格导入也已在最终回归通过。见 `WALLET_MANAGEMENT_APPLICATION_BOUNDARIES.md`。持续目标 active，计划更新不暂停。

2026-11-14 最新 NP-02/TS-08-F 源码/离线验证通过：两个余额 GET 已接项目 WalletBalanceService、纯 metadata/runtime port 与 DTO 映射，bootstrap 复用原仓储/registry Arcs；API 不再解析 SDK Pubkey 或获取 runtime。共享 WalletMetadata 归 domain，旧 WSOL 名称仅兼容别名；models 的 WalletBalanceItem 桥收为测试专用，非新增警告抑制。新增十五项，最终源码并发/串行各 954、HTTP 187、余额筛选 14、metadata 两项、check/fmt/diff 通过，仅两组原兼容警告。get/list 关闭库错误预期已按两条原错误链修正，不改生产查询。原 SQL/RPC/配置/SDK/release/资金流程未改；main/Handler/DTO/类型别名的必要接线有记录。见 `WALLET_BALANCE_APPLICATION_BOUNDARIES.md`。持续目标 active，计划更新不暂停。

2026-11-14 最新 NP-02/TS-08-E 源码/离线验证通过：钱包管理/余额/靓号共 11 Handler、十个 DTO/OwnerQuery 及三个路由组迁入 API，http.rs 从 412 行收为 17 行组合；七项合同在原位置及移动后各通过，再补三项组合同后专项十项通过。最终并发/串行各 939、HTTP 183、check/fmt/diff 通过，仅两组原兼容导出警告。不将 Handler 归位当作钱包 Application/资源隔离完成。原仓储/registry/ops/SDK/RPC 实现、executor、配置、库格式/release 未改，保留旧生命周期错误；普通 GET 不隐式加入 RPC 对账。见 `WALLET_API_HANDLERS.md`。持续目标 active，不因修改计划暂停。

2026-11-14 最新 NP-01-C2b 源码/离线子项通过：Durable Live 普通 SOL 聚合已接原 owner/005 签名证据扩展、消息绑定、原签名字节提交/独立读回/只发送读回字节，executor 不覆盖父行权威结果。新增 20 项（16 Application、3 HTTP、1 SDK 纯拒绝验证），并发/串行各 929、HTTP 173、ordinary 专项 87、production check/fmt/diff 通过，仅两组原兼容警告；最终测试导入收紧后另做 tests 类型检查。旧 004 NULL owner 不补发，签名写失败不靠外层补写冒充成功。Memory/Shadow legacy、opaque SDK、普通只读链上对账及真实验收仍未完成。见 `ORDINARY_DURABLE_SIGNED_JOURNAL.md`；持续任务 active，计划更新不暂停。

2026-11-13 最新完成 NP-01-C2a 普通聚合执行占用：任何状态更新/报价前，Memory/Shadow 原幂等行原子标记，Durable 原父行关联表/事务核对/占用精确读回；重复或异常只读停止，不覆盖原结果或自动释放/retry。新增 19 项，最终并发/串行各 909、HTTP 170、check/fmt/diff 通过，仅两组原警告；见 `ORDINARY_EXECUTION_CLAIMS.md`。有意新增 004 占用表及 executor/idempotency/persistence 接线，旧 migration/V1/SDK/vendor/配置/release 未改。仍没有签名字节账本、全量授权或其他 backend 占用；C2b/完整 D/E 待续。持续任务 active，修改计划不暂停。

2026-11-13 最新完成 NP-01-C1：原 Created reservation 的 ID/公钥/字面 key/V1 hash/模式与完整项目命令收为不可编辑 owned 输入，实际 API→executor→普通聚合 port 已接通；聚合 quote 前核对原钱包身份。无新幂等权威/SQL/校验政策，Duplicate/失败保留不生成输入。新增七项，最终并发/串行各 890、HTTP 169、check/fmt/diff 通过，仅两组原警告；见 `ORDINARY_RESERVATION_EXECUTION_ANCHOR.md`。本阶段有意更新 executor 入口指纹，不修改 SDK/vendor/PumpAPI 实现或 release。C1 不是签名字节账本或全量授权，C2 父记录核对/原子 claim/签名字节落盘读回等仍待续；任务 active，修改计划不暂停。

2026-11-13 最新完成 NP-01-E0 普通 SOL 聚合不确定回执保护：本地签名后广播 Err 或不同回执保留本地原签名/Unknown，明确 Live Unknown 经项目 typed 分类，不凭 failure 字串降级 Failed；错误 URL 脱敏。原正常成功、模拟、key/hash/SQL、98 组 untyped 分类保持。新增七项，最终并发/串行各 883、HTTP 167、check/fmt/diff 通过，仅两组原警告；见 `ORDINARY_AGGREGATE_UNCERTAINTY.md`。这不是广播前 signed journal 或完整 C/D/E 迁移，最终写库前取消/崩溃风险仍在。持续任务 active，不因记录计划暂停。

2026-11-13 最新完成 NP-02/TS-08-D：Pool/预热/SOL hop 和 health/ready/capabilities 六个 Handler、两个路由组已迁入 API，三项合同先在旧位置通过，再补独立组验收两项。最终并发/串行全套各 876、HTTP 165、check/fmt/diff 通过，见 `POOL_AND_SYSTEM_API_HANDLERS.md`。不改查询/缓存/发现算法或能力；普通交易最终授权、唯一签名字节账本及 legacy 迁移仍待续。持续目标保持 active，修改计划不暂停。

2026-11-13 恢复记录：用户已将持续目标重新设为 active，继续非 PumpAPI 源码/离线队列。NP-01-B2b 已补齐本笔模拟净输出、19 项专用测试及当阶段回归；当阶段并发/串行各 871、HTTP 160 通过，见 `DAMM_SIMULATED_SWAP_OUTPUT.md`。以下 paused 描述保留为上次中间检查点，不再作为停止指令。更新计划和进度不会自动暂停；不升级 SDK、不部署/重启、不接触生产钱包及数据库、不发送交易的边界保持。

2026-11-13 执行约定更新：用户明确“以后修改计划不用暂停持续运行”。更新/记录/调整计划、询问完成度和阶段汇报都不再作为暂停指令；在持续任务处于运行状态时，继续已授权的非 PumpAPI 队列，只有用户明确要求暂停/停止才暂停。越界操作仍需单独授权，不能借“持续运行”升级 SDK、部署或操作生产资金。本次为只读盘点及记录更新，不改源码或运行配置；用户此前显式设定的 paused 状态不被这份文档冒充为 active。完成/未完成总览见 `PROGRESS_OVERVIEW.md`。

2026-11-13 历史暂停检查点：当时 B2b 测试模块尚未创建，只完成 dev-profile check，有四条警告，未构成可交付检查点。该状态已由顶部明确恢复及 B2b 完成记录覆盖；中间状态和原恢复清单保留在 `DAMM_OUTPUT_EVIDENCE_PENDING_PLAN.md`，不能按该历史暂停自动停止。

2026-11-13：用户在“记录修改计划，先不动”后明确将持续任务恢复为 active，现恢复非 PumpAPI 源码及离线测试。暂停时的中间状态、后续步骤和限制记录在 `SDK_LANE_SAFETY_PENDING_PLAN.md`；不升级 SDK、不构建 release、不部署/重启、不接触生产钱包及数据库、不发送交易。

NP-01-A2b lane 安全、B1 公开参数编译、B2a 身份快照和 B2b 本笔模拟净输出均已完成各自源码/离线回归，B2b 当阶段并发/串行各 871、HTTP 160 通过；证据见 `DAMM_SIMULATED_SWAP_OUTPUT.md`。下方数字只证明各自检查点，不把局部离线验证等同于完整资金授权、生产迁移或实盘验收。PumpAPI 交易对接仍暂停，不停止 ZMQ 或修改线上配置。

- PumpAPI 交易对接开发线暂停于 H2-B2-C，H2 剩余协议/完整授权以及 PumpAPI 对应的 H3-H5 不继续。已有代码、测试、缺口和最后验证数字全部保留，不作为完整授权或上线证据。
- 不停止 PumpAPI/ZMQ 数据接收，不改线上 backend 或已有 SDK 协议能力；开发暂停与运行停用分开处理。
- 普通 SDK/legacy 安全缺口、Handler 分拆、基础设施、双假后端合同和升级准备按主计划 1.1 的 NP-01 至 NP-05 推进。NP-01 初轮复核、A1 最终参数/费用及 A2a 准备顺序/DAMM 零签名 probe，NP-02/TS-08-B/C 普通交易 API/查询等待已完成各自源码/离线验证；其他未完成项不自动记为完成。
- 下方状态表、验证数字和“下一执行顺序”保留历史检查点含义；凡要求继续 PumpAPI 的旧指令均由本节覆盖。完整优化目标不缩水，不因暂停而提高完成度。

## 当前状态

| 任务 | 源码/文档 | 验证 | 部署 |
| --- | --- | --- | --- |
| NP-01-B4c1 DAMM 无签名产物/模拟 | 私有完整零签名候选接实际 simulation/Live preview，正式 guard 复用产物；同 snapshot/intent 及前后期限、固定 readonly RPC/4 秒逻辑预算/不释放失败 submit | 新增 20；候选 37、readonly+HTTP 8、并发/串行各 1289、HTTP 200、probe 48、lanes 19、参数 3 与 check/fmt/diff 通过，五组原警告；见 `DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md` | 未部署；正式仍 opaque SDK，无普通 SDK signed journal/完整授权或真实节点验收 |
| NP-01-B4b DAMM 正式候选 | 公开参数/原余额/完整 intent 冻结与 SDK protocol/full 私有 guard 已接实际卖出/preview；共用 swap2 ABI、返回验签/取消/单次核对；见 `DAMM_FINAL_CANDIDATE_GUARD.md` | 新增 25，含 384 实际 SDK 指令对照；专项 25、并发/串行各 1269、HTTP 199、probe 48、lanes 19、参数合同 3 与 check/fmt/diff 通过，五组原警告 | 未部署；仍 opaque SDK，不是完整协议授权/受控签名或 ordinary signed journal |
| NP-01-B4a 无 signer 构建入口 | 实际 SDK 编译/签名/模拟/worker 与高层 Prepared 审查，九项纯公开地址编译/费用合同；见 `ORDINARY_UNSIGNED_BUILD_AUDIT.md` | 专项 9、完整并发/串行各 1244、production check/fmt/diff 通过，五组既有生产警告；无生产执行代码变化 | 未部署；不是最终协议消息授权或普通 SDK 广播前账本 |
| NP-01-A3c 准备共享 HTTP | 同一 main runtime 第六 profile，当前 URL/两笔 scan/raw/原输入绑定/真实体积与 deadline/共享冷却/取消；见 `PREPARATION_SHARED_RPC_RUNTIME.md` | 新增 19，preparation_ 90、完整并发/串行各 1235、HTTP 199、shared_read 89、quote_balance 25、参数合同 3 与 check/fmt/diff 通过 | 未部署；其余 SDK 读取/最终授权/完整 signed journal 仍未完成 |
| NP-01-D1B1 无 owner 恢复上下文 | 不透明只读 context/port/core 已接实际发送尾部；原证据摘要与显式 signed 中断恢复，无新 migration，见 `ORDINARY_RECOVERY_OBSERVATION_CONTEXT.md` | 新增九项，专项 9、并发/串行各 1018、HTTP 198、production check/fmt/diff 通过，五组既有警告；销毁旧 owner/执行对象、禁重发/重占用触发器、16 双池竞争与 V1 样本 | 未部署；启动扫描、持续确认/关停/RPC 全局预算、全量最终授权与其他 backend/mode 仍待续 |
| NP-01-D1A 普通聚合原签名观测 | 本次回执保存后单次只读查询、3 秒 deadline、006 原账本 finalized 证据与父行 CAS/读回；见 `ORDINARY_POST_SEND_OBSERVATION.md` | 新增 20 项，专项 20、并发/串行各 1009、HTTP 198、production check/fmt/diff 通过，五组既有警告 | 未部署；跨重启/持续对账、签名中断恢复、全部 backend/存储模式与全量授权仍待续 |
| NP-01-B3a 聚合签名前时效 | 原 Quote 到期时间快照归私有 prepared，实际释放消息时重查；见 `QUOTE_SIGNING_FRESHNESS.md` | 三项新增，过期回归先失败后通过；专项 8、并发/串行各 989、HTTP 196、production check/fmt/diff 通过，五组既有警告 | 未部署；完整最终消息授权、普通原签名对账/全 backend 账本仍未完成 |
| NP-03/TS-09-A AppState 资源 | 四个生产冗余字段仅保留为测试句柄，服务复用原 Arcs，不改初始化次序；见 `APP_STATE_RESOURCE_OWNERSHIP.md` | 新增两项生命周期/权威测试；并发/串行各 986、HTTP 196、production check/fmt/diff 通过，实际五组警告已记录 | 未部署；其他 AppState 资源/接受入口、后台资源及完整依赖隔离待续 |
| NP-02/TS-08-H 靓号边界 | 原三 Handler 接项目服务/任务 port，私有创建命令承接原政策；取消政策在原 worker guard 调用，生成/保存/加载体字节不变，worker 从 AppState 移至 adapter；见 `VANITY_APPLICATION_AND_WORKER_BOUNDARIES.md` | 新增十三项、迁移一旧测试；最终并发/串行各 984、HTTP 194、vanity 17、check/fmt/diff 通过；C2b 竞态断言确定性补强十连过，无生产交易改动 | 未部署/运行真实生成；原恢复/取消并发/队列/owner 竞争、SDK 加载和钱包原子生命周期仍未完成 |
| NP-02/TS-08-G 钱包管理 Application | 六个原操作接项目服务/原仓储与运行 port，opaque owned 材料无 Clone/Serde，显式导出/no-store 保持；公开类型归 domain/原名兼容，原仓储其余字节精确不变，见 `WALLET_MANAGEMENT_APPLICATION_BOUNDARIES.md` | 新增十七项；最终并发/串行各 971、HTTP 191、管理筛选 18、check/fmt/diff 通过，仅两组原警告；无真实密钥/SDK/RPC 成功操作 | 未部署；vanity、跨仓储/SDK 原子生命周期、全局密钥日志/内存/真实加载与生产验收仍待续，原部分失败不被冒充事务 |
| NP-02/TS-08-F 只读余额 Application | 两个 GET 接纯项目 service/metadata/runtime/parser/clock，原仓储/registry Arcs 与 RPC 复用，DTO 只映射项目结果，WSOL 元数据名兼容同一 domain 类型；见 `WALLET_BALANCE_APPLICATION_BOUNDARIES.md` | 新增十五项；最终并发/串行各 954、HTTP 187、余额筛选 14、metadata 两项、check/fmt/diff 通过，仅两组原警告；首个错误文案测试预期已修正并全部重跑 | 未部署；不改变余额 RPC/SQL/配置/数据格式/节点，真实余额/整体 deadline 与钱包管理/vanity 分层等仍待续 |
| NP-02/TS-08-E 钱包 API 归位 | 钱包管理/余额/vanity 11 Handler、十个 DTO/OwnerQuery 及三个路由组迁入 API，http.rs 仅组合，无旧转发；见 `WALLET_API_HANDLERS.md` | 新增十项，七项原位置先通过；最终并发/串行各 939、HTTP 183、check/fmt/diff 通过，仅两组原警告 | 未部署；钱包 Application/运行资源/成功创建导出与真实余额/worker 验收仍待续；原资金/存储实现未改 |
| NP-01-C2b Durable 普通聚合签名账本 | 原 owner/同 authority 子表、消息/原签名字节落盘及独立读回，控制原字节发送，保留原签名/Unknown，不降级父行 | 新增 20 项，并发/串行各 929、HTTP 173、ordinary 专项 87、check/fmt/diff 通过，仅两组原警告，见 `ORDINARY_DURABLE_SIGNED_JOURNAL.md` | 未部署；仅 Durable Live aggregator_sol，Memory/Shadow/opaque SDK/链上对账及真实签名节点验收未完成 |
| NP-01-C2a 普通聚合原子占用 | 原 memory key 标记/新 004 父行关联表接 Live 聚合实际 guard，identity/mode/hash/branch 检查，重复/异常不改原状态；见 `ORDINARY_EXECUTION_CLAIMS.md` | 新增 19 项，11 authority/storage+7 executor+1 HTTP；并发/串行各 909、HTTP 170、check/fmt/diff 通过，旧 V1/参数/状态摘要不变，仅两组原警告；必要迁移/接线有记录 | 未部署；无原签名字节、完整授权及其他 backend 占用，C2b/完整 D/E 待续，Memory/Shadow 无跨重启保证 |
| NP-01-C1 原保留身份/执行输入 | Created 原 ID/key/hash/公钥/模式与冻结命令接实际生产入口，普通聚合 quote 前核对钱包；见 `ORDINARY_RESERVATION_EXECUTION_ANCHOR.md` | 新增七项，admission 14、聚合 HTTP 四项及旧缺记录回归；并发/串行各 890、HTTP 169、check/fmt/diff 通过，原 V1/参数/消息/状态摘要不变，仅两组原警告；executor 必要接线变更有记录 | 未部署；没有父记录执行 claim 或广播前 signed journal，完整 B/C/D/E 仍待续；不把 owned 输入当作资金授权或 exactly-once |
| NP-01-E0 普通聚合不确定回执 | legacy 广播 Err/不同签名保留原签名/Unknown，Live 明确 Unknown 由 typed 分支保留，URL 脱敏；原订单权威复用，见 `ORDINARY_AGGREGATE_UNCERTAINTY.md` | 新增七项，含三个旧 failure marker、缺签名/上下文、模拟、三存储模式 HTTP 重复/冲突/文件重开及最终写库失败；并发/串行各 883、HTTP 167、check/fmt/diff 通过，98 组旧分类与受保护指纹不变 | 未部署；无普通发送前 signed journal/完整 C/D/E，最终写库前取消/崩溃仍可能丢恢复证据，不自动补单 |
| NP-02/TS-08-D Pool/系统 API | 六个 Handler 与两个 route group 迁入 api，原 Application/DTO/validation/cache/runtime 保持；旧 http 无转发层，见 `POOL_AND_SYSTEM_API_HANDLERS.md` | 新增五项，三项先在旧位置通过，覆盖独立/总路由、方法/提取/鉴权顺序、HEAD、凭据/禁播独立、成功/失败/缓存与组隔离；并发/串行各 876、HTTP 165、check/fmt/diff 通过，仅两组原警告，受保护指纹不变 | 未部署；不改变 RPC 性能/协议能力，不是完整 TS-08 或普通 signed journal |
| NP-01-B2b 本笔模拟净输出 | 生产报价移除跨 slot 相减；固定消息八账户/innerInstructions、两笔转账角色/金额、同次模拟 epoch 费用及可选 pre/post balances 交叉核对；见 `DAMM_SIMULATED_SWAP_OUTPUT.md` | 新增 19 项，含 96 组编码/方向/资产/profile、u64/fee 边界、两次 RPC 间 ATA 新建、坏证据/取消；probe 46、并发/串行各 871、HTTP 160、check/fmt/diff 通过；原摘要/受保护指纹不变，仅两类原警告 | 未部署；仅 DAMM 卖出 probe 的 RPC 模拟输出证据，不是最终消息授权/买入或全协议保护、节点共识或 ordinary signed journal |
| NP-01-B2a DAMM probe 身份快照 | 固定八账户单 bank 读取/Clock/Pool/Mint/Vault/用户 ATA 与扩展校验接入卖出 probe，原 RPC/原消息/slot 私有绑定，五秒读取 deadline；见 `DAMM_PROBE_ACCOUNT_EVIDENCE.md` | 新增 14 项，整个 probe 27；并发/串行各 852、HTTP 160、check/fmt/diff 通过，旧摘要/受保护指纹及共享 token checker 未改，仍两类原警告 | 未部署；新增操作级拒绝需调用方验收。不是跨 bank 可信增量、池开放/费用版本审计、买入保护、最终授权或 ordinary signed journal |
| NP-01-B1 DAMM 公开参数 probe | 替换前冻结 SDK 576 组消息摘要；固定 quote-only profile 仅接受公钥/公开池参数/整数/blockhash，私有零签名产物接原模拟；不再构造含 payer 的 SwapParams；见 `DAMM_PUBLIC_PROBE_COMPILATION.md` | 新增七项：SDK 摘要/逐组完整交易对照 2、纯计划 3、实际内存 RPC/取消 2；并发/串行各 838、HTTP 160、check/fmt/diff 通过，旧摘要/受保护指纹不变，仍两类原警告 | 未部署；仅只读 profile，不是最终 SDK 无 signer 构建、可信池/Token 扩展授权、完整 min_out 或 ordinary signed journal |
| NP-01-A2b SDK lane 安全 | 单 sender 执行视图处理 SDK 追加 Default，typed unknown/原签名保留、禁止 SDK 调用后换 lane、总核验 deadline 已接 buy/sell；见 `SDK_LANE_ENTRY_AND_UNCERTAINTY.md` | 新增 25 项：lane/metadata 13、RPC 7、分类 3、HTTP/临时库恢复 2；并发/串行各 831、HTTP 160、check/fmt/diff 通过；旧参数/分类摘要及受保护指纹不变，仍两类原警告 | 未部署；无普通 signed journal/完整授权/真实 SDK 或节点验收，后台 worker 取消风险仍在 |
| NP-01-A2a SDK 准备与 DAMM probe | 原 buy/sell 准备顺序接有限 source/私有原 lane 适配器；DAMM 报价用无 signer 参数的零签名 RPC helper；见 `SDK_PREPARATION_AND_DAMM_PROBE.md` | 新增八项准备（含 144 组合与六取消点）、六项专用 probe；A1 摘要不变；最终并发/串行各 806、HTTP 158、fmt/diff/check 通过，仍两条原兼容警告；manifest/lock/release/受保护源码指纹不变 | 未部署；builder 仍接 payer 资源，最终 SDK 模拟/回退与 ordinary signed journal 未完成 |
| NP-01-A1 普通 SDK 最终参数 | 最终 buy/sell 请求、费用策略归 SDK adapter，ExecuteTrade 定义归 command adapter，旧路径兼容；见 `SDK_FINAL_PARAMETER_BOUNDARIES.md` | root 三项/2,016 组摘要冻结后移动，固定摘要保持；最终并发/串行各 792、HTTP 158、fmt/diff/check 通过，仍两条原兼容警告；manifest/lock/release 与 PumpAPI/executor wrapper 指纹不变 | 未部署；不代表 unsigned builder、协议成功或 ordinary signed journal |
| NP-02/TS-08-C 普通查询/等待 | 项目只读 source、Application query/wait、原存储借用 adapter 与请求作用域组合已接原 GET/接受等待，见 `ORDINARY_TRADE_QUERY_BOUNDARIES.md` | 新增三项旧 HTTP 基线先通过，连同 Application 八项共 11 项；pending HTTP 合同扩展六组；最终并发/串行各 789、HTTP 158、fmt/diff/check 通过，仍两条原兼容警告，manifest/lock/release 指纹不变 | 未部署；没有新增 signed journal、最终性证据或查询失败回退 |
| NP-01 首轮普通 SDK/legacy 复核 | 当前 SDK 组合调用、带签名模拟、lane 回退、多签名映射及 legacy 丢失签名已核对，见 `ORDINARY_SDK_SUBMISSION_SAFETY_WORKPACK.md` | 仅源码证据；未运行真实 SDK/签名/节点，安全实现与准入仍缺 | 未部署，不改变现有执行行为 |
| NP-02/TS-08-B 普通交易 API | 四个 Handler、原接受/等待 helper 与 route group 已迁至 api；Query DTO 归 api；无旧转发层，见 `ORDINARY_TRADE_API_HANDLERS.md` | 拆分前全套 776 与新增两项旧位置合同通过；最终并发/串行各 778、HTTP 155、fmt/diff/check 通过；仅两条原兼容导出警告，manifest/lock/release 指纹不变 | 未部署；不是普通 signed journal 或完整薄 Handler |
| TS-00 本地版本、SDK、入口清单 | 已记录 `BASELINE.md` | manifest/lock/vendor 指纹已核对；上游 revision/补丁来源待核实 | 无运行改动 |
| TS-01 DTO/快照合同 | 22 项测试与 trade/sol_buy V1 fixtures，含内部转换与临时账本恢复 | 全部通过；不是真实 Handler 成功下单/链上验收 | 不需部署测试代码 |
| TS-01 参数校验 | 5 项测试：金额、Quote、Pool ID、MEV | 全部通过，不读取生产配置 | 不需部署测试代码 |
| TS-01 HTTP 合同 | 当前 194 项（含序列化基线），默认 legacy 保护操作、separate/禁播及 readiness/pool-query/普通买卖/原 reservation/聚合占用与 Durable 签名账本/Pending/钱包管理/余额/靓号任务控制/不确定回执/独立 quote、SOL 转账与 WSOL、独立路由组合同；仅公开 fixture/fake runtime/非密钥元数据，SDK builder oracle 仅测试样本身份且不签名 | 全套通过；最新并发/串行各 984 项；C2b 强状态合同以确定性门闩和零写审计补强，fake 成功不等于真实签名/链上验收 | 未部署 |
| TS-00 vendor 来源补查 | `SDK_PROVENANCE.md` 记录 5.0.7 历史 revision 与 6.0.2 导入差异 | 本地证据已核对；不能用旧 revision 冒充当前来源 | 无运行改动 |
| TS-S1/TS-S2 边界审查 | `SUBMISSION_BOUNDARIES.md` 记录聚合提交、转账、WSOL、PumpAPI 的风险与后续任务 | 独立聚合已补离线故障测试；其他入口仍待处理 | 未部署 |
| TS-S1 A/B/C | 独立聚合 submit 已接入 domain、journal port、SQLite 与 SDK adapter；新 migration 002 | 新增 17 项应用/账本测试与 2 项领域测试通过；见 `QUOTE_SUBMISSION_JOURNAL.md` | 源码完成、离线通过；未部署/实盘 |
| TS-S1 D | 已接入带鉴权的 GET 原签名对账、查询 port/RPC adapter、确认级别 CAS 与 migration 003；无后台定时器 | 新增 21 项测试，含 RPC 内存 mock、并发/回执竞态、迁移回滚、文件重开；见 `SUBMISSION_RECONCILIATION.md` | 源码完成、离线通过；未部署/真实 RPC |
| 纯函数提取试点 | DAMM v2 `minimum_output` 移至 `src/protocol/meteora_damm_v2/mod.rs` | 拆分前后 3 项边界/错误测试通过；首轮全套 66 项 | 未构建 release、未部署 |
| TS-02-A 聚合提交内部类型 | API wire DTO、Domain quote、内部 command/result/outcome、执行 port 已分离；私有预检使用内部类型 | 新增 10 项合同/快照测试通过；见 `QUOTE_DTO_BOUNDARIES.md` | 源码/离线通过；未部署 |
| TS-02-B 普通交易内部命令 | 普通/批量 wire DTO、共享身份、整数金额和模式、内部执行命令、SDK/PumpAPI 参数映射已分离；接受/校验和持久化仍有旧 DTO 耦合 | 新增 8 项回归；18 成功/19 错误样本及旧 hash 联合指纹不变；见 `TRADE_DTO_BOUNDARIES.md` | 源码/离线通过；未部署 |
| TS-03-A Registry | 能力响应、Quote/Pool ID 校验和持久化 Pool 名称已读取中心注册表；旧 wire DTO 路径兼容 | 新增 7 项回归；旧能力与 108 组校验指纹不变；见 `PROTOCOL_REGISTRY.md` | 源码/离线通过；未部署 |
| TS-03-B 协议证据校准 | DBC Token-2022/no-hook、min_out=1 与旧能力差异已记录，完整 Program/扩展/模拟证据未接入 | 不把现有 adapter 或旧 live 声明当作本轮实盘验收 | 未部署 |
| TS-04-A 只读余额端口 | 两个余额 Handler 改取不含 signer 的 WalletReadPort；RPC adapter、项目 Address/QuoteBalance 与 SDK 余额 helper 已分离 | 新增 8 项内存 RPC 测试，含 32 组 ATA 推导对比；见 `WALLET_READ_BOUNDARY.md` | 源码/离线通过；未部署 |
| TS-S1-E 普通交易旧 hash 歧义 | durable 以旧 hash + 已存请求分支/身份核对；memory/shadow 原子比较 hash + 分支，旧记录不重写 | 1 项历史证据 + 新增 11 项修复测试，含两种资产并发/别名/损坏 JSON/原签名重启；见 `ORDINARY_TRADE_HASH_REVIEW.md` | 源码/离线通过；未部署 |
| TS-04-B1 独立报价签名句柄 | SDK backend 的 Prepared 改为模块私有构造的不可编辑对象，签名前再核对钱包/payer/unsigned 单签名与预检授权 | 新增 4 项无 signer/RPC 测试，旧禁播/恢复合同通过；见 `PREPARED_QUOTE_SIGNING.md` | 源码/离线通过；未部署 |
| TS-04-B2 signer 获取与 WSOL 模拟 | 原始 signer getter 已移除，聚合签名收回 SDK，legacy 聚合走 application/backend；WSOL 分离 unsigned prepare/simulate 与 Live 签名 | 新增 12 项 fake/内存 RPC/Message/禁播回归；见 `SIGNER_ACCESS_AND_UNSIGNED_WSOL.md` | 源码/离线通过；未部署 |
| TS-04-C Blockhash | 缓存不持有 SDK/RPC，项目 hash + 只读 port + RPC adapter，bootstrap 复用原 handles | 3 项旧行为基线 + 6 项刷新/RPC/采样测试；SDK 映射字节不变；见 `BLOCKHASH_READ_BOUNDARY.md` | 源码/离线通过；未部署 |
| TS-04-D Readiness | 只读 query/source、SDK execution-lanes probe、项目状态与 DTO 转换已接入；Handler 不取 SDK | 1 项旧响应基线 + 7 项真实路由 fake 测试（含 48 组合）+ 3 项 RPC mock；见 `READINESS_QUERY_BOUNDARY.md` | 源码/离线通过；未部署 |
| TS-05-A Pool 查询 | resolve/warmup/hop 的项目提示、查询 façade/backend/cache port 与 DTO 转换接入；SDK resolve 返回不再引用 API DTO | 4 项旧入口基线 + 9 项真实路由 fake 测试；保留错误/gate/别名/缓存顺序；见 `POOL_QUERY_BOUNDARY.md` | 源码/离线通过；未部署 |
| TS-05-B1 SDK 参数 | warmup/dex_params、Quote token、成员/平台/hop helpers 已集中至 SDK adapter，仅接收 RPC 与公开参数 | 3 项旧基线（90 组 token/error 指纹）+ 7 项本地/RPC/账户拒绝测试；见 `SDK_PROTOCOL_PARAMS.md` | 源码/离线通过；未部署 |
| TS-05-B2 实际 loader 样本 | 严格按地址的只读内存 sender；CPMM/StonkFunSwap、StonkFun 曲线/显式 SOL hop、DAMM v2、DBC 实际 loader 成功/拒绝及费用/方向/扩展回归 | 新增 19 项，全套 255 项；见 `SDK_PARAMETER_FIXTURES.md`；不是报价/链上成功证据 | 仅测试与文档，未部署 |
| TS-05-B3 九类 Pool 参数回归 | 补 Pump/PumpSwap/Bonk/AMM v4 实际 loader，当前九类 Pool 均有基础成功样本；完整 discovery/缓存/身份/交易仍待验收 | 新增 12 项，全套 267 项；见 `SDK_LOADER_COVERAGE.md` | 仅测试与文档，未部署 |
| TS-04-E0 普通执行结果 | 项目执行结果与原分类归 Application，SDK 原 ExecuteResult 路径重导出兼容；Executor 日志保持 | 迁移前 98 组指纹 + 2 项边界测试，全套 270 项通过；见 `ORDINARY_EXECUTION_RESULT.md`；完整 port 尚未接入 | 源码/离线通过；未部署 |
| TS-04-E1 普通执行 port | SDK/PumpAPI 参数与调用收回 adapter，普通编排只用项目 command/context/result；解析后保留原 wallet Arc，legacy 聚合保持原分支 | 新增 11 项真实编排 fake/临时 SQLite + 1 项空真实 adapter 测试，全套 282 项；见 `ORDINARY_EXECUTION_PORT.md` | 源码/离线通过；未部署 |
| TS-04-E2 接受身份与 HTTP | 已加载身份 directory/只读 lease，保留接受作用域的原 client Arc 与执行前再次解析语义；真实 Router 可以注入无钱包源 | 新增 11 项 HTTP 成功/默认显式 ID/重复冲突/批量/并发/pending/故障 + 1 项真实空 adapter，全套 294 项；见 `WALLET_ADMISSION_AND_HTTP_TRADES.md` | 源码/离线通过；未部署 |
| TS-04-E3 普通 SOL 聚合 | 单次聚合钱包 port、项目 intent/阶段 pipeline、原 SDK/quote adapter 接入，Executor 不再取 SDK 钱包或 wire DTO | 先冻结 448 组旧映射，新增 4 项纯 pipeline + 1 项 adapter + 7 项真实 Executor + 1 项真实 HTTP，全套 359 项；见 `ORDINARY_SOL_BUY_EXECUTION.md` | 源码/离线通过；未部署 |
| TS-02-C1 普通持久化输入 | typed validated/rejected V1 快照与 codec，核心 reserve/mirror 不再接收 wire/NormalizedTrade；旧入口及 hash 路径桥接兼容 | 先捕获 37 组真实入库字节指纹，新增 7 项，全套 301 项；见 `ORDINARY_PERSISTENCE_INPUT.md`；record/调用方/port 继续迁移 | 源码/离线通过；未部署 |
| TS-02-C2 Record 与 journal | TradeRecordV1 归内部 read-model，旧 models 路径兼容；项目 journal port 已接 codec reserve/mirror、执行写入和 durable 查询，不新增状态副本 | 迁移前 720 组记录字节指纹 + 3 项 fake 写入策略 + 3 项真实 adapter 测试，全套 308 项；见 `ORDINARY_RECORD_AND_JOURNAL.md` | 源码/离线通过；未部署 |
| TS-02-C3 已校验接受 | 内存 key/index port、惰性 V1 factory、Application admission decision/error 已替换 HTTP 内的保留编排，保留启动/日志/等待时点 | 新增 6 项纯接口事件顺序/故障 + 1 项 18 样本 adapter 回归，全套 315 项；见 `VALIDATED_TRADE_ADMISSION.md` | 源码/离线通过；未部署 |
| TS-02-C4 校验结果模型 | 普通 ValidatedTrade 归 Application，四类地址用项目 Address；保留原 wire Pubkey parser，DTO→command/codec 不再依赖结果内的链类型 | 新增 3 项地址/解析顺序/精度三态回归，全套 318 项；见 `VALIDATED_TRADE_MODEL.md` | 源码/离线通过；未部署 |
| TS-02-C5 设置与 builder | 5 字段校验设置、项目 command context/builder、执行策略已收窄，旧 Config/DTO 入口桥接；不新增配置 | 新增 3 项设置/惰性/边界 + 4 项纯项目 builder/policy 测试，全套 325 项；见 `TRADE_SETTINGS_AND_COMMAND_BUILDER.md` | 源码/离线通过；未部署 |
| TS-02-C6 输入与 parser | 无 serde 的 22 字段借用输入、纯 parser port 与原 SDK adapter，普通政策迁至 Application，旧入口/Warmup/replay 兼容 | 原 5 项参数测试迁移，新增 6 项纯政策轨迹 + 2 项 SDK parser + 1 项 DTO 映射，全套 334 项；见 `TRADE_INPUT_AND_PARSER_BOUNDARY.md` | 源码/离线通过；未部署 |
| TS-02-C7 拒绝接受 | rejected 的记录/两次写入/时间与惰性 mirror 归 Application，原 DTO/codec 留 V1 adapter；旧桥接仅测试 | 先通过 2 项真实 Router 基线，再新增 3 项接口轨迹/故障与 1 项 19 样本 V1 对比，全套 340 项；见 `REJECTED_TRADE_ADMISSION.md` | 源码/离线通过；未部署 |
| TS-02-C8 Pool 输入 | Warmup 借用输入与项目地址结果、raw resolve/hop parser 边界；Pool DTO/兼容 validator 不再引用 SDK 地址 | 新增 3 项纯政策、1 项 DTO、1 项真实 Router；精准 warmup 16 项、全套 345 项；见 `POOL_INPUT_AND_WARMUP_MODEL.md` | 源码/离线通过；未部署 |
| TS-02-D QuoteStore | 缓存保存内部 BoundQuote，经项目 QuoteCache/绑定元数据读写；旧 insert/get 仅 V1 映射且共享原实例，普通聚合已接内部端口 | 先冻结 108 组字节与 3 项时钟/容量/去重基线，再补 2 项内部端口回归；缓存模块 7 项，全套 365 项；见 `QUOTE_CACHE_BOUNDARY.md` | 源码/离线通过；未部署 |
| TS-06-A 只读报价 | 项目借用输入、source、已加载身份/lease 与缓存编排接入 POST/GET，Handler 不再取 SDK；复用原校验/网络/共享缓存 | 保留旧基线 1 项，新增 service 5、source 2、DTO 1、真实 Router 6 项；默认并发/串行各 380 项，HTTP 59 项；见 `SOL_BUY_QUOTE_QUERY_BOUNDARY.md` | 源码/离线通过；未部署 |
| TS-04-F1 独立报价预检 | 受限 wallet/backend port、Application 两次缓存检查与 SDK adapter 接入；保留一次钱包解析及原实例，Handler 不再取 SDK | service 7 项、空 adapter 1 项、HTTP 6 项（含迁移前基线）；全套 394 项通过；见 `QUOTE_PREFLIGHT_BOUNDARY.md` | 源码/离线通过；未部署/真实 RPC |
| TS-04-F2 独立报价提交 | runtime 解析既有钱包/journal，Application 保留 wallet-first、历史先于缓存及核心重复校验；SDK adapter 转发原受控提交，Handler 不再取 SDK | service 8 项、runtime 2 项、HTTP 10 项（含基线/8 并发/文件重开）；默认并发/串行各 414 项通过；见 `QUOTE_SUBMIT_HTTP_BOUNDARY.md` | 源码/离线通过；未部署/真实 RPC |
| TS-04-F3 独立报价对账 | key-first Application、私有原 wallet/RPC 快照和仅观测 journal view；原 reconciler/SQL/3 秒 deadline 保留，Handler 不再取 SDK | service 10 项、runtime/view 2 项、HTTP 9 项（含基线）；针对性 21 项、全套 435 项通过；见 `QUOTE_RECONCILIATION_BOUNDARY.md` | 源码/离线通过；未部署/真实 RPC |
| TS-08-A 报价 API | 五个 Handler 与 route group 迁至实际 api 目录，Query DTO 与旧通用 error helper 归 API；原总路由 merge，无旧转发层 | 先通过 435 项再迁移；增加 1 项独立/集成五路由对照；最终默认并发/串行各 436 项、HTTP 85 项通过；见 `QUOTE_API_HANDLERS.md` | 源码/离线通过；未部署 |
| TS-S1-F1 SOL 转账基线/政策 | V1 DTO、项目金额选择与 key/fee/rent 政策接入原 Handler/SDK 位置；原账本/组合发送未改，已复现幂等只比 owner 与 finish 错误忽略 | 原实现 HTTP 7 项、金额 2 项先通过；1,890 组指纹不变；新增政策 3/DTO 2 项；默认并发/串行各 450 项、HTTP 92 项通过；见 `SOL_TRANSFER_CONTRACT_AND_AMOUNT.md` | 参数隔离/离线通过；安全修复待 F2；未部署 |
| TS-S1-F2-A Native SOL 阶段 | 原 RPC prepare、受限 sign、不可编辑 signed artifact、同字节 send 和原 commitment confirm 分离；原 HTTP 暂经兼容编排，未接 journal | 新增 10 项内存 RpcSender/篡改/guard 测试；全套默认并发/串行各 460 项、HTTP 92 项；见 `SOL_TRANSFER_SDK_STAGES.md` | 阶段分离/离线通过；发送前账本待 B/C；未部署 |
| TS-S1-F2-B 转账账本 | 旧全局 key + 原子 attempt/projection，版本化整数 intent/快照、签名落盘、CAS、只读旧记录与 Unknown 恢复；钱包 pool 初始化 Full/FK，工厂仅复用 handle | B 历史检查点新增 18 项；全套 478、HTTP 92、生产 check 7 条警告；后续 C 已接入并消除五条未用接口警告，见 `SOL_TRANSFER_JOURNAL.md` | 账本离线通过，API 后续 C 已接；未部署/生产迁移 |
| TS-S1-F2-C Native API/Application | metadata/runtime/journal ports、原钱包 Arc、完整意图、snapshot 与签名落盘已接原 API；薄 Handler 迁至 api，移除无 journal 组合入口，旧仓储写入只作测试 oracle | 新增 Application 11/runtime 2/真实 Router 9 项，五处取消/16 竞争/写失败/文件恢复；全套并发/串行各 500 项、HTTP 101、转账筛选 65；生产 check 仅剩两条既有兼容重导出警告；见 `SOL_TRANSFER_API_EXECUTION.md` | 源码/离线通过；未部署/真实签名/生产迁移 |
| TS-S1-G1 WSOL 合同与风险基线 | 已审查现有四个 API、quote/fingerprint、唯一 pending 账本、公开 artifact 与多次 wallet 获取；补 wire/SQL/HTTP 基线及状态回退缺陷样本 | 新增 storage/wire 8、HTTP 5 项，全套 513 项通过；后续 G2-A 封装 artifact，G2/G3 runtime/单向 CAS 与取消恢复未完成；见 `WSOL_CONVERSION_SAFETY_WORKPACK.md` | 源码测试/文档通过；未部署，不改变兑换逻辑 |
| TS-S1-G2-A WSOL 已签 artifact | 签名实现与私有 Message/签名/bytes 收回 SDK 子模块；模拟/发送前一致性与禁播/钱包检查，原 API/SQL/金额/RPC 选择不变 | 新增四项 snapshot 篡改/内存模拟/空 client 禁播测试；全套并发/串行各 517、HTTP 106、wsol 筛选 24 通过；生产 check 两条既有兼容警告；见 `WSOL_SIGNED_ARTIFACT.md`；不等于完整 runtime/journal 隔离 | 源码/离线通过；未部署/真实签名/广播 |
| TS-S1-G2-B WSOL submit/simulate | 内部无 serde 输入/快照/read-model、metadata/runtime/journal ports 与 Application 已接原入口；保留原 wallet Arc 和 prepare 选择的 RPC；唯一旧 store/hash/SQL 复用，unsigned prepare 移回 SDK | 新增 15 项：Application 8、journal/runtime adapter 各 1、真实 prepare 内存 RPC 2、Router 3；并发/串行各 532、HTTP 109、wsol 筛选 40；生产 check 两条既有兼容警告；见 `WSOL_SUBMISSION_RUNTIME.md` | 源码/离线通过；Quote query/status 与完整 G3 待续，未部署/真实签名/广播 |
| TS-S1-G2-C WSOL query/API | quote/status 编排、只读 reader、原 RPC/原签名绑定已接入；DTO/四个 Handler/route group 迁至 api，builder 收回 SDK、唯一 V1 hash 留 codec；旧业务包装移除，原 store/schema/SQL 逐字不变 | 新增 20 项，迁移前黄金 JSON/SHA 与 551 项先通过；最终并发/串行各 552、HTTP 114、wsol 筛选 62；fmt/diff/check 通过，生产仅两条既有兼容重导出警告；见 `WSOL_QUERY_AND_API_BOUNDARIES.md` | 源码/离线通过；完整签名审计/状态安全/取消恢复与真实验收待续，未部署 |
| TS-S1-G3-A WSOL 单向状态 | domain 事件/政策、八身份与原签名字节绑定、事务内 SQL CAS/读回、弱结果 no-op、终态保护；submit/status 最终回读校验与并发真实 Router 回归 | 相对 G2-C 新增 16 项，全套并发/串行各 568、HTTP 116、wsol 78；fmt/diff/check 通过，生产 check 仍仅两条既有警告；见 `WSOL_MONOTONIC_PROGRESS.md` | 源码/离线通过；签名审计/证据顺序、取消恢复、可信对账和错误兼容仍待续；未部署 |
| TS-S1-G3-B1 WSOL 签名前授权 | 原 Quote/余额/租金/费用/blockhash 证据保留，单一 canonical builder 与完整 Message 比对；sign/unsigned simulate 显式接原 Quote，长期 signer 获取前核对；准备后过期拒绝 | 先捕获四种原消息 SHA，再新增 10 项；全套并发/串行各 578、HTTP 116、wsol 88、授权 9 项；fmt/diff/check 通过，仅两条既有警告；见 `WSOL_PRE_SIGN_AUTHORIZATION.md` | 源码/离线通过；签名落盘/模拟顺序、完整 record_signed CAS、取消恢复与可信对账仍待续；未部署 |
| TS-S1-G3-B2 WSOL 签名账本/模拟 | 完整身份的签名事务/CAS/精确读回后才做已签模拟；失败保留 unknown/签名/锁；广播前权威回读；原 key 不重签/重发 | 新增 15 项，含八个取消点/文件重开、提交回执丢失、双 pool 16 竞争、锁等待取消及真实 Router；全套并发/串行各 593、HTTP 118、wsol 103；fmt/diff/check 通过，仍仅两条既有警告；见 `WSOL_SIGNED_JOURNAL_AND_SIMULATION.md` | 源码/离线通过；确认可信度/deadline、unsigned 失败身份与错误兼容、断电及生产验收仍待续；未部署 |
| TS-S1-G3-C1 WSOL 明确最终性 | 原签名/slot 领域证据，严格结果数/明确确认等级/错误一致性；3 秒 lookup deadline；仅 finalized 推进原 confirmed/failed，非最终错误保留 pending | 新增 16 项，含原始 RPC JSON 经真实 Router、history 参数捕获、超时 Drop/取消与竞态；全套并发/串行各 609、HTTP 122、wsol 119；fmt/diff/check 通过，仍仅两条既有警告；见 `WSOL_STATUS_FINALITY_AND_DEADLINE.md` | 源码/离线通过；非节点共识/历史终态重判；unsigned 失败身份、错误兼容与真实节点/生产验收待续；未部署 |
| TS-S1-G3-C2 WSOL 保留/unsigned 失败 | typed reservation 绑定 V1 codec/完整身份/证据，新建事务内读回；unsigned failure 完整身份 CAS/回执/最终读回，旧按 ID helper 仅测试 | 新增 16 项，含 insert/failure trigger、并发签名竞争、保留/失败回执丢失、文件重开、真实 Router；全套并发/串行各 625、HTTP 126、wsol 135；fmt/diff/check 通过，仍仅两条既有警告；见 `WSOL_UNSIGNED_RESERVATION_AND_FAILURE.md` | 源码/离线通过；不自动修复历史损坏数据，错误分类/G4/真实节点与生产验收仍待续；未部署 |
| TS-S1-G3-C3-A WSOL submit 错误 | typed 阶段分类，明确参数 400/冲突 409/读不可用及完整性或结果不明 503；保持单字段错误 JSON，新增禁止自动重试及已验证恢复上下文 headers | 相对 C2 新增 12 项；并发/串行各 637、HTTP 129、wsol 147，fmt/diff/check 通过；仍仅两条既有兼容警告；见 `WSOL_SUBMIT_ERROR_CONTRACT.md` | 源码/离线通过；仅 submit 映射迁移，quote/simulate/status 与 by-key 恢复待续；未改 Bot/部署 |
| TS-S1-G3-C3-B WSOL 原 key 恢复 | 新增带 loopback/Bearer 及 key/wallet/owner 绑定的内部 GET，复用原 store，只读七字段快照；无 RPC、签名、广播、观测写入或解锁；原四入口不变 | 相对 C3-A 新增 14 项；并发/串行各 651、HTTP 136、wsol 161、recovery 14，fmt/diff/check 通过；仍仅两条既有兼容警告；见 `WSOL_READ_ONLY_RECOVERY.md` | 源码/离线通过；含真实只读临时 SQLite、回执丢失文件恢复及处理中/取消查询；未部署/改 Bot |
| TS-S1-G4-A WSOL quote 存储 | 先复现损坏列 panic/未分类错误，再接 try_get、行/JSON ID/expiry 绑定；INSERT/全部 13 字段/原 JSON 字节事务读回，错误回滚、不改坏历史 | 相对 C3-B 新增 8 项（storage 5、Router 3）；并发/串行各 659、HTTP 139、wsol 169、quote_storage 8，fmt/diff/check 通过；仍仅两条既有兼容警告；见 `WSOL_QUOTE_STORAGE_INTEGRITY.md` | 源码/离线通过；旧过期/未知字段与 V1 字节保持；损坏 quote 下仍可原 key 恢复，未部署 |
| TS-S1-G4-B WSOL status 记录/错误 | 请求 ID/完整身份/记录形态核对，最终单向读回及终态证据/error 不可变；typed status 503 与初始已核对的恢复头、成功 no-store；by-key 复用形态检查仍纯 SELECT | 相对 G4-A 新增 17 项（domain 1、分类 1、Application 8、Router 7）；并发/串行各 676、HTTP 146、wsol 186、status_boundary 17，fmt/diff/check 通过；仍仅两条既有兼容警告；见 `WSOL_STATUS_RECORD_AND_ERRORS.md` | 源码/离线通过；包括 36 状态矩阵、21 坏记录、终态补签拒绝、观测回执丢失文件恢复；未改 Bot/部署 |
| TS-S1-G4-C WSOL quote/unsigned | fallible 无私钥 metadata 投影、metadata/估算绑定、quote/unsigned typed 错误与 no-store；原四/13 字段和实盘 key 保持；完成局部取消/文件恢复交叉清单 | 相对 G4-B 新增 19 项：应用 9、metadata 2、分类 1、native/memory RPC 1、Router 6；并发/串行各 695、HTTP 152、wsol 205，fmt/diff/check 通过；仍仅两条既有兼容警告；见 `WSOL_QUOTE_AND_UNSIGNED_BOUNDARIES.md` | 源码/离线通过；零/错额不保存、缺失/坏 metadata 区分、Quote 回执丢失不伪造成功、预览不改 Live 记录；未部署/改 Bot |
| TS-S1-H0 PumpAPI 后续审查 | 核对外来 Message 解码即签名、simulate 在签名后、二进制 trim/数量转换及普通发送前 journal 缺口；源码行与三文件 SHA 已记录 | H0 为历史只读源码审查，非修复/签名/节点测试；见 `PUMPAPI_SIGNING_SAFETY_WORKPACK.md`，后续 H1/H2-A 见续作 | H0 未调用该资金路径，完整 H2-B/H3-H5 仍待实施 |
| TS-S1-H1 PumpAPI 请求/格式 | 公开项目请求输入、无 signer HTTP/文本响应 adapter、SDK mapper/严格 codec 已接原执行；修正 raw binary trim，16 KiB body/1232 packet/1644 base64 限制与 sanitize；不改金额/默认后端 | 新增 16 项；先冻结 80 请求与 Legacy/v0 SHA 并复现 binary 失败，最终针对性 16、并发/串行各 711、HTTP 152、fmt/diff/check 通过；仅两条既有兼容警告；见 `PUMPAPI_REQUEST_AND_CODEC_BOUNDARIES.md` | 源码/离线通过，未部署/真实 HTTP/RPC/签名/广播；结构校验不是消息授权，后续 H2-A 单独修复 unsigned；Live 授权和普通 signed journal 仍待处理 |
| TS-S1-H2-A PumpAPI unsigned | SDK 分发在 Live 入口之前选择公开 Address/RPC 模拟；HTTP 准备无 signer，复用 packet 结构检查并绑定单一零签名槽/钱包；原消息/config 保持，模拟 signature=None | 新增 10 项：helper/入口/memory RPC 8、空 SDK 分发 1、真实 Router/fake/临时库 1；针对性 10、并发/串行各 721、HTTP 153、fmt/diff/check 通过；生产仍两条既有警告；见 `PUMPAPI_UNSIGNED_SIMULATION.md` | 源码/离线通过；成功 simulated/程序失败 failed 均不带签名，同 key 不重执行；未部署/真实 HTTP/RPC/签名/广播。不是 Live 协议/支出授权或普通发送前 journal 完成 |
| TS-S1-H2-B1 PumpAPI 完整上下文 | opaque PumpApiTrade 持有原完整项目 command 自有只读快照与独立 config/wait；请求从快照投影，原 SDK 公钥有损 mapper 移除；literal 身份、协议、金额种类、hop/MEV/fee 保留 | 新增 7 项；实际 snapshot -> request 的 80 样本与 H1 旧 SHA 相同；并发/串行各 728、HTTP 153、PumpAPI H1/H2 筛选 33、fmt/diff/check 通过；见 `PUMPAPI_OWNED_EXECUTION_CONTEXT.md` | 源码/离线通过，未部署/真实 HTTP/RPC/签名/广播；仍两类兼容重导出警告，models::Side 也变为仅兼容/测试用；完整快照不等于可信 Quote/VerifiedRoute/授权摘要或 signed journal |
| TS-S1-H2-B2-A PumpAPI 消息/ALT | 模拟与 Live 签名前已接单零签名/钱包、消息摘要绑定及自有账户视图；ALT 一次有界逻辑批读、owner/state/Base64/限额/slot/库 lookup 前缀与正确顺序/去重；后续 RPC 带 minContextSlot | 新增 16 项含 375 组库对照、真实 memory RPC 模拟参数/原 bytes、篡改/坏表/取消/5 秒超时；并发/串行各 744、HTTP 153、PumpAPI 49、fmt/diff/check 通过；仍两类原兼容警告；见 `PUMPAPI_MESSAGE_ACCOUNT_VIEW.md` | 源码/离线通过；无 ALT 不新增账户 RPC。未部署/真实节点/签名/广播；不是池/Token/完整指令/支出授权，不锁住 ALT/分叉；普通 signed journal 待 H3 |
| TS-S1-H2-B2-B PumpAPI CPMM 子集 | 仅 RaydiumCpmm/StonkFunSwap 直接路线接真实 pool/两 Mint/vault/config/Clock 只读检查、角色与精度、Token-2022 白名单/必需扩展和坏 hook payload；两阶段共同 5 秒 deadline，最终 bank slot 进入模拟/send | 新增 17 项；并发/串行各 761、HTTP 153、PumpAPI 66、fmt/diff/check 通过；无新增弃用警告，仍两类原兼容重导出警告；见 `PUMPAPI_CPMM_ACCOUNT_EVIDENCE.md` | 源码/离线通过，未部署/真实节点/签名/广播；其他协议显式 None 未验证，普通 SDK 未接此检查；未解析完整 observation、用户账户生命周期、整数预算/min_out/全部费用副作用，尚非完整授权或 journal |
| TS-S1-H2-B2-C PumpAPI PumpSwap | PumpAmm 直接路线单 GMA 同 bank 池/两 Mint/vault/Global/Fee/Clock；方向/fees/Creator/Cashback/pool-v2/buyback/tail 角色核验；与 CPMM 共用有界 read/token/扩展校验，原 CPMM 策略/label 保持 | 新增 15 项含 64 组合、config 轮换/旧布局/两种 buy/Token-2022/ALT/取消/超时；并发/串行各 776、HTTP 153、PumpAPI 81、fmt/diff/check 通过；仍两类原兼容警告；见 `PUMPAPI_PUMPSWAP_ACCOUNT_EVIDENCE.md` | 源码/离线通过，未部署/真实节点/签名/广播；非零 disable flags 保守拒绝且语义待审计；Pump 曲线及其他池未完成，不代表金额/min_out/用户账户生命周期/全指令授权或 journal |
| TS-S2 | 已审查资金入口接入禁止广播策略；钱包管理可选 legacy/separate，鉴权移至 API middleware；默认兼容 | 新增 4 项配置、4 项无 signer SDK 包装层、6 项 HTTP/入口、2 项聚合策略测试通过；见 `EXECUTION_SAFETY_AND_ADMIN.md` | 源码/离线通过，未部署或启用新配置；能力类型硬隔离/调用方迁移待做 |

## 本轮变更边界

- 新增 `src/contract_tests.rs`，通过 `main.rs` 的 `cfg(test)` 挂载；无新依赖。
- 新增 `tests/fixtures/contracts/trade_v1.json`，仅包含虚构钱包身份和测试数据，不包含私钥。
- 首轮 `validation.rs` 只增加参数测试；安全轮新增独立执行策略校验，不改变原金额/协议归一化。
- `sdk_client.rs` 首轮提取 DAMM 纯函数；安全轮新增服务端策略与发送入口检查，SDK vendor 不变。
- `protocol/meteora_damm_v2` 暂时仅有 min_out 纯政策函数；不是新的报价器、协议适配器或能力声明。
- 首轮未修改依赖；HTTP 测试轮仅增加 dev-dependency Tower 0.5.3（原 lock 已有）及一条锁文件依赖边，无包升级。
- vendor、生产配置、钱包、数据库、运行二进制均未修改；没有加载真实钱包或启动测试 RPC。
- 新增 `src/http_contract_tests.rs` 与测试专用空 WalletRegistry 构造函数，生产构造逻辑不变。
- 启动前已有的 `persistence.rs` 重启测试改动保留，不覆盖。
- 账本轮新增实际使用的 domain/application/ports/adapters；TS-02-A 进一步移除了聚合提交应用编排对 API DTO 的直接依赖，但尚未完成全服务 DTO 迁移。
- 原 HTTP 路径和字段不变；独立聚合 Live 现在要求 durable，新增 submitting/unknown 恢复及更严格错误语义，调用方需按文档验收。模拟不签名、不广播、不占 Live key。
- 新表只在 durable 启动时初始化；本轮仅临时库执行迁移。durable 共享连接设置 synchronous=FULL，发布前需测磁盘刷盘延迟。
- 账本轮未改其他资金路径；安全轮仅为普通交易/内部聚合、转账、WSOL 和 PumpAPI 增加策略检查，原构建和发送逻辑保持。不代表它们都具备新的发送前签名账本。
- 对账轮新增 `api/dto/quote_submissions`、`application/submission_reconciler`、只读 ports、RPC adapter 和领域观测类型；只保存原签名的确认元数据，不提供重新签名/广播能力。
- 新 GET 为兼容增加；旧 POST 路径和字段不变，对账后的重复提交可读到 confirmed/finalized。旧调用方需处理新状态；不会自动修改 Bot。
- migration 003 在事务中扩展旧表，标记与数据复制原子提交；仅在临时数据库验证，生产库未执行。跨请求限流/去重、后台巡检及真实节点证据仍未验收。
- 新参数 `TRADE_BROADCAST_DISABLED=false`、`WALLET_ADMIN_AUTH_MODE=legacy` 默认保留旧行为；显式 separate 才隔离管理凭据与 loopback，尚未替调用方迁移。没有修改生产 `.env`。
- 禁止广播模式拒绝 SDK 完整买卖流水线（含 SDK simulate）；明确分离的模拟/只读/对账保留。SDK wrapper、PumpAPI submit 和 WSOL send 均有策略边界。READY 既有 paused 反映该策略，导出成功响应禁止缓存。
- 此保证限定已审查入口，不是 OS 沙箱/无密钥模式；raw RPC/signer 能力收口仍属于 TS-04。SDK 初始化 ATA=false 的源码已核对，真实初始化与故障演练未做。
- TS-02-A 新增 `api/dto/sol_buy`、`domain/quote`、`application/quote_submission_types`、`ports/quote_execution`；旧 aggregator DTO 路径保留重导出，客户端字段/状态/默认值无变化。
- 先捕获旧类型的合成指纹，再验证内部类型同值、预检 32 字段和旧格式文件账本恢复；没有迁移或修改真实订单数据。普通交易/API DTO、QuoteStore 及聚合网络实现仍是后续边界。
- TS-02-B 将普通/批量 DTO 移至 `api/dto/trades`，共享枚举保持 serde 名称并由旧 models 路径重导出；普通 Executor 改收无 serde 的 TradeCommand，Domain 以项目拥有的字节地址、整数金额和执行模式隔离链类型。
- 普通 validator/NormalizedTrade、旧 request hash/JSON/schema 保留；命令转换在保留幂等键前发生。SDK 与 PumpAPI 的金额、独立 tip 默认值、原始模拟三态及分支顺序已有离线映射回归。参数 mapper 不等于完整执行 port，尚未隔离 AppState/raw RPC/signer。
- TS-03-A 将已有能力声明与真实入口政策分开保存于 Registry；能力字段/列表顺序、SDK Pool ID/Quote 校验结果、原错误及存储名称保留。展示别名不自动成为 TradeRequest 别名；未适配协议继续 observe-only。
- DBC 文档/Token Program 声明、无 hook 验证与 vendor min_out=1 风险已记录但未改 vendor；完整协议身份、资产扩展和链上接受证据仍属 TS-03-B，不宣称注册表完成即全协议完成。
- TS-04-A 的余额 port 不公开 signer、交易或 raw RPC；adapter 私有持有现有 Default RPC，SDK token-balance 逻辑提取后复用。节点顺序、批量每节点 4 秒超时、原始整数、旧 fallback 和错误保留，未初始化真实钱包或 RPC。
- 普通接受流程、其他 HTTP 入口和 WalletRegistry 仍耦合 AppState/SDK，不宣称全 API/application 已隔离。Quote 余额查询原整体 deadline 缺失仍是独立后续风险。
- 普通旧 hash 仍可对 0.001 SOL 聚合买入和 1 USDC 直买生成相同整数指纹；reservation 现已追加分支核对，跨路径 Conflict，无法验证旧 JSON 时 fail closed。memory/shadow 同样校验参数/分支；不改变旧 hash、schema、订单或签名，不产生新 key 补单。
- TS-04-B1 将独立 Quote 的原始 VersionedTransaction 隐藏在 adapter 私有构造的句柄内；sign 不再接受外部任意交易，预检/钱包/签名形状二次检查保持既有执行流程。
- TS-04-B2 已移除公开 conversion_signer，原聚合/WSOL 消费点不再获取长期 Keypair。旧聚合分支经 Application/backend 复用但仍无独立签名 journal；WSOL simulate 现在使用零签名和 sig_verify=false，Live 仍走原保留/签名/模拟/记录/发送流程。raw RPC、SDK TradingClient 和其它资金风险继续单独审查。
- TS-04-C 将 blockhash 的 RPC 查询和 SDK Hash 转换移出缓存；只读 reader 复用 Default lanes 原顺序，失败保留旧值/时间，取快照后采时，精确年龄边界和回退规则不变。未实际启动刷新循环或改 RPC 超时/配额。
- TS-04-D 将 readiness 的完整响应与只读数据源分离，bootstrap 注入已有组件，实际 Handler 只做鉴权与转换；原 ready=hash&&rpc、paused/DB/mode 独立、执行 lane 顺序与全部字段保持。真实路由可注入无 SDK source，未把 fake 成功当作真实钱包/节点验收。
- TS-05-A 将 resolver/warmup/hop 的 Handler SDK 获取移入 backend，成功后元数据缓存由 Application 编排；SDK resolve 只转换原 Pubkey 字节，原寻池算法、lane/timeout/gate/错误保持。PoolRouteHint 不冒充 VerifiedRoute，ParamCache 也未升级成可执行路由缓存。
- TS-05-B1 将 9 Pool 的参数/Quote 类型与成员检查迁至 `adapters/sdk/protocol_params`，不再向参数加载函数传 TradingClient/signer。warmup/build 的原不同规则、sysvar、错误与 StonkFun 目标/hop 顺序保留；当前账户测试以拒绝为主，不宣称全部协议成功参数已验收。
- TS-05-B2 仅增加 `cfg(test)` 子模块与文档，调用真实 SDK loader，但传输只允许账户/epoch 内存读取；未知方法、send/simulate 拒绝并记 violation。CPMM 费用/Token-2022 epoch/hook、5 个优先 Pool 的参数/方向与 warmup/build 差异通过；DBC 曲线配置不是可交易报价样本。无生产参数/签名/广播行为变更，不将 loader 通过当作 VerifiedRoute 或实盘验收。
- TS-05-B3 继续仅测试/文档，补 Pump 曲线附加字段/Global/sharing、PumpSwap 批量快照/虚拟储备/当前配置、Bonk WSOL/USD1、AMM PnL/市场账户。九类基础成功不等于九类协议所有路径验收；PumpSwap 全局缓存使用相同配置避免并发污染，TTL/fallback/discovery 仍未覆盖。原 API、SDK 参数算法及支持许可不变。
- TS-04-E0 只提取 backend 报告与状态分类，SDK wrapper 保留原类型别名，Executor/PumpAPI 不再从 SDK 导入结果。先捕获原函数 98 组指纹后再移动；包括旧空签名、失败标记及模拟特殊分类，均不改变。不将 legacy 分类当作最终性证据，不改变持久化/钱包/blockhash/广播顺序；backend port 与调用计数故障注入仍待下一批。
- TS-04-E1 已接普通执行 port，参数 mapper 移回 SDK adapter 调用点，真实解析只读一次注册表并持有原 client Arc。fake 编排覆盖两 method/方向/模式及 durable update 错误、shadow、过期 hash、禁播、不确定签名/无自动重试；HTTP 接受阶段仍耦合真实 SDK 身份。最终写库失败仅保证内存留签名，发送前 journal 缺口不因此解决；没有改变旧资金路径或部署。
- TS-04-E2 普通接受改用已加载身份 port，私有 lease 保留原引用而不暴露签名能力；不能用 repository 的未加载项冒充可执行钱包。真实 Router 的接受/等待/GET/幂等/批量/503 通过 fake 执行和临时 SQLite 验证，auth/validator/policy 顺序保持。HTTP mock 成功不是链上成功，普通接受/持久化仍依赖 DTO/NormalizedTrade/AppState，发送前账本仍待做。
- TS-02-C1 普通存储改收不可变 V1 快照，DTO/hash/JSON/replay 解释集中 codec，旧 reserve/mirror 实现成为兼容桥；原 hash/JSON/key/schema/模式保持。新 typed 输入增加内部绑定防错，原调用按 record 构造无外部行为变化；memory 和存储错误优先级保持。37 组入库指纹、跨钱包/rejected/alias/shadow 回归通过，未解决普通发送前 journal 或全部接受层依赖。
- TS-02-C2 内部 read-model 与静态 ordinary journal 已实际接入，保留旧 aliases/serde/SQL 规则。Executor 写入 helper 可脱离 AppState 接 fake journal；原日志、durable/shadow 分支和次数保持。普通 row journal 不等于发送前签名账本，HTTP 接受仍耦合具体内存索引和 wire 输入；没有宣称完整 Application/Domain 重构完成。
- TS-02-C3 已校验接受保留由 Application 经项目 port 编排，内存 store/原子 key 复用旧实现，V1 factory 保持 hash/JSON/record 的惰性时点。应用不依赖 HTTP/SDK/AppState/任务，API 仅映射 decision/error 并保留原日志/启动/等待。原 wire validator、NormalizedTrade 链类型和 rejected 分支仍在边界，未把接受迁移冒充全部模型隔离或资金安全目标完成。
- TS-02-C4 普通 NormalizedTrade 旧路径重导出项目结果，原 parser/政策全部完成后才将地址转字节；四类地址保持独立，原整数与模拟三态保留。旧 SDK 参数相等测试只机械改为全字节比较，未改冻结指纹。wire Config/输入和 Warmup 链类型、rejected 编排仍待隔离，不宣称验证结果代表真实池或签名授权。
- TS-02-C5 校验仅取 5 个项目设置，DTO command 桥接到内部 builder，执行策略只取项目 policy/default；不让普通核心规则获取完整含钱包/RPC/凭据的 Config。原惰性默认、金额种类、模拟三态、独立 fee 和禁播差异保持；wire 输入/SDK parser/rejected 路径仍待分离，没有新增配置或路由许可。
- TS-02-C6 普通输入政策已脱离 DTO/SDK/Config，parser 继续使用原 SDK 算法且只作语法解析；22 字段映射保留字节/Option/借用。key/金额字段检查先于解析，原 Quote alias/错误链/校验次序保持，replay 不重新施加当前准入规则。Warmup 仅兼容调用原政策，rejected/发送前 journal/VerifiedRoute 和真实协议验收仍未完成。
- TS-02-C7 rejected 流程经项目 index/journal/惰性 payload，HTTP 仅保留原日志与 400 映射；初始 Accepted 插入及第二时间采样顺序、存储检查先于快照、Memory 不序列化、失败后保留拒绝记录均保持。无效请求不占正常 key；旧 DTO 构造/mirror 仅为测试 oracle，未改格式或生产数据，不等于普通发送前账本完成。
- TS-02-C8 Warmup 的 4 字段输入与三类 Address 结果归项目类型，旧 NormalizedWarmup 路径兼容；raw query 只解析语法，与 Warmup 非空/Quote alias/协议政策保持差异。原 gate/错误/后端与缓存顺序、108 组政策和字节合同不变，没有调整发现算法/RPC/loader 或把 hint 当 VerifiedRoute。
- TS-04-E3 普通聚合分支的 quote→bind→legacy-submit 已经项目 intent/port 收入 adapter，原 wallet Arc/QuoteStore 复用；分支先于 method、无本地 blockhash/Submitting、原模式/确认/错误/最终写入/缓存保持。448 组请求指纹与 fake 编排/自然 HTTP 分支通过；legacy unknown/签名前 journal 缺口没有在目录迁移中修正，也不允许因 Failed 而自动重发。
- TS-02-D QuoteStore 内容/绑定归项目类型，原 TTL、容量预检查、过期删除、clock/ID 顺序、literal 身份/错误与 V1 字节保持；原 re-export/insert/get 桥接同一实例，普通聚合实际走内部缓存端口。没有新增原子容量、过期竞态修复、去重或 Quote 可执行性许可。
- TS-06-A 独立 quote POST/GET 已通过 source/身份/缓存 port 编排，原参数先于 lookup、fetch 成功后取公钥、lease 与 literal ID、GET 不重新报价/校验、HTTP 400/410/502/503 和脱敏保持。只读禁播差异与旧 JSON/query 提取器行为保留，preflight/submit/reconcile 尚未迁移，不宣称真实 RPC 或可信报价/资金验收。

## 下一执行顺序

最新非 PumpAPI 检查点为 NP-01-C3a（操作分型专项 26、完整并发/串行各 1304、HTTP 200、候选 37、参数合同 3 与 check/fmt/diff 通过，见顶部/验收页）；前序 B4c1/A3c 的产物/模拟/共享读取保持。下一 C3b 正式产物的完整授权、独立 SDK metadata 与原 ordinary owner 的 journal/CAS/readback/原字节 sender，不能假借 Quote metadata 或恢复 owner 绕过旧聚合范围。新 SDK 类型只有占用、正式入口未切换；局部无签名分支不代表正式 SDK 已隔离、已部署、真实供应商/TLS/HTTP2、全 backend/mode 恢复或全服务限额完成。其余参数/ATA/寻池/模拟/SDK 内部读尚未迁入，GET/重复仍只读原存储，不重签或重发。全方向最终消息/池/程序/费用/权限授权、跨模式/backend 签名账本、SDK worker 取消/多签名证据、其他资源/接受入口与依赖方向、钱包原子激活/vanity 竞争、全局密钥日志/内存、通用 Backend 合同、SDK 来源及真实环境验收均留在完整计划。PumpAPI 对接保持暂停，release/生产验收须单独授权，计划更新与阶段报告不暂停 active 目标。

B2b 在原零签名消息上用严格本笔 CPI transfer 与同次模拟 post-account/Mint/Clock 净费用证据代替旧跨 slot 相减，可选同次模拟 pre/post balances 交叉核对，新增 19 项；经典 SPL legacy Transfer 与 Token-2022 checked 语义分离，不靠弃用告警白名单。旧 576 组消息、2,016 组参数、98 样本分类及受保护指纹均未改。安全主线继续 B 的最终消息/池与程序/费用版本/整数预算/权限授权和无 signer 边界，扩展买入及其他协议；不可把只读 probe 当作 SDK 最终可签交易。真实 RPC 的 metadata/格式能力需要单独窗口，不因节点缺证据而 fallback 到旧余额减法或自动补单。之后进入 C/D/E 唯一普通签名账本/受控原字节发送/legacy 迁移；SDK worker 取消及全部签名审计仍有缺口。以下 1-9 保留历史阶段依赖与风险，其中旧“继续 PumpAPI”顺序已被顶部范围覆盖，不按该历史指令续作。

1. TS-06-A 后 `cargo fmt --check`、service 5 项、source 2 项、HTTP 合同 59 项、全套 `cargo test --locked --offline --quiet` 与 `-- --test-threads=1` 各 380 项、`git diff --check` 通过，无测试构建警告。SDK vendor/原迁移 diff 为空，release、manifest/lock 指纹持续核对不变；未构建 release、部署或清理其他数据。
2. TS-01：HTTP 方法/鉴权/拒绝路径、真实 readiness/pool 查询及普通买卖/幂等/批量的 fake 成功、并发、durable reserve 故障已完成局部回归；真实 SDK runtime、签名/广播计数、完整 Handler fault/模拟矩阵和链上验收仍未完成。
3. TS-00：本地来源核查完成可查部分；取得可信 6.0.2 上游基线后再生成补丁账本，不阻塞离线安全边界工作，也不盲目覆盖 vendor。
4. TS-S1-D 的请求触发对账与离线回归已完成。真实 RPC、调用方状态兼容、迁移空间/延迟和 release 部署保持独立窗口；不因完成 mock 而自行上线。
5. TS-S2 的已知入口策略与可选管理鉴权已完成源码/离线验证，调用方凭据迁移、真实 SDK 启动验收和受控启用保留单独窗口；不自行修改生产。转账和 PumpAPI 的独立风险保持发布门槛。
6. TS-S1-E 兼容修复已完成源码/离线回归；调用方 409/503、真实 Handler 的成功钱包与故障注入、运行时启用保留独立验收。不会为验收读取生产钱包或重发历史订单。
7. TS-02-C6/C7/C8/D、TS-04-E3、TS-06-A、TS-04-F1/F2/F3、TS-08-A、TS-S1-F1/F2-A/B/C、TS-S1-G1/G2-A/B/C/G3-A/B1/B2/C1/C2/C3-A/C3-B/G4-A/G4-B/G4-C、TS-S1-H1/H2-A/H2-B1/H2-B2-A/B/C 已完成各自局部源码/离线验证。最新并发/串行各 776 项、HTTP 153、PumpAPI 81 通过；生产 check 两类原兼容警告，manifest/lock/release 指纹不变，vendor/原迁移未改。原合同与 CPMM 两阶段策略/label 保持，B2-C 新增 PumpSwap 单 bank 直接路线证据与实际共用的 read/token checker；实际 pool 分发明确保留未检查的 None。PumpSwap 非零 disable flags 仅保守拒绝，逐位语义/其他 feature flags 仍待审计；账户检查不是可信 Quote/完整用户账户/指令/整数预算/min_out/全部费用授权或 signed journal，普通 SDK 未接同样保护。下一批按 `PUMPAPI_PUMPSWAP_ACCOUNT_EVIDENCE.md` 继续 B2 Pump.fun 内盘、Bonk/StonkFun 曲线、DAMM v2/DBC/AMM v4 证据，不新增无界 GPA、不把 hint/None 当 VerifiedRoute；继而 B3/B4 全量授权、H3 唯一发送前账本、H4 SDK/legacy 缺口。WSOL 的真实节点/断电、生产历史/调用方兼容及发布证据仍缺。HTTP 取消/超时、任何状态或暂未查到结果不授权换 key 重签/释放钱包；剩余 Handler/基础设施、依赖方向/双假后端合同、SDK 来源/升级演练和发布窗口继续。仅临时库/fake/内存 RPC，不接触生产资金、不部署。
8. TS-02-A/B、TS-03-A、TS-04-A/B1/B2/C/D、TS-05-A/B1/B2/B3 已完成局部内部类型、Registry、只读余额、签名、WSOL 模拟、查询和 SDK 参数集中化的源码/离线验证；普通接受/校验、持久化、其它 raw RPC 与完整 SDK 后端仍未全部迁移。穿插 TS-03-B 协议证据校准与 graduated Via-SOL/config/discovery/缓存/扩展补测，不因发现 vendor builder 或九类 loader 基础成功就开放实盘。上游版本证据不足不阻塞这些离线任务。
9. 主计划 4.3 已补通用后端边界与验收，分配到既有 TS-02/03/04/05/07/09；先完成当前优化，未来 Router 单独接入。复用现有 ports，不新增依赖/空框架、不改公开 API 或默认后端；两种假构建后端合同和依赖方向检查仍属后续验收，不把本轮 WSOL fake 测试冒充整体完成。

按最新范围节及主计划 1.1 持续推进非 PumpAPI 队列；PumpAPI 开发须单独明确恢复，不能按上述历史顺序自动续作。生产部署、数据库/密钥操作、网络变化和实盘仍需要明确窗口或授权。各局部阶段完成源码/离线验证不代表整服务优化或发布完成。
