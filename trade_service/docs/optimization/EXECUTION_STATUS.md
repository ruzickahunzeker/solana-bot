# Trade Service 当前执行状态

更新日期：2026-11-19（Asia/Shanghai）。唯一主计划：[Trade Service 优化计划](../TRADE_SERVICE_OPTIMIZATION_PLAN_V1.0.md)。
本页只保留当前检查点；[完整历史](EXECUTION_HISTORY_20261115.md) 是冻结证据，不是待执行队列。
目标由用户明确恢复为 active；计划或历史记录不能自行暂停/恢复目标。

## 当前任务

- 当前续原NP03的Provider转换边界：原配置/映射移入SDK adapter，去除
  adapter对根facade的反向调用，原初始化/选路/错误顺序可逐字还原。五项新合同
  仅源码/格式/策略检查，尚未Rust运行；不继承已验收父批的结果。
  见同一[资源边界](APP_STATE_RESOURCE_OWNERSHIP.md)，不新增任务/协议/实盘权限。
- 同批原NP03的[候选缓存](PARAM_CACHE_RESOURCE_BOUNDARY.md)补1024条元数据上限，
  保留原实例/键/字段与写入端口，不涉及SDK缓存、报价、授权或账本。容量/并发/
  三模式执行三项新合同待独立Rust验收；与上述合计八项，预期完整各2078、129步
  及74类型/225字段，不套用父批结果。
- 原NP03/05 [SDK配置边界](APP_STATE_RESOURCE_OWNERSHIP.md)及
  [公开SDK兼容门禁](SDK_PROVENANCE.md)已独立验收：`0530bed` /
  [Actions44363743712](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44363743712)
  / artifact14457878477。并发2070（394.39秒）、串行2070（749.17秒），原2065
  全保留、恰增五项；config3/surface2、Pump56、HTTP208、127有序步骤及68类型/
  207原始编译字段通过。源码/锁定依赖身份及原初始化/Provider/钱包行为重建通过。
  未实际共享SDK资源，不重跑终态或另发文档CI，记录随真实后继源码提交。
- 原 NP-01 [完整消息最终模拟](PUMP_SDK_FINAL_SIMULATION.md)已独立验收：
  `f519207` / [Actions44355235997](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44355235997)
  / artifact14454946722。并发2065（394.35秒）、串行2065（755.19秒），原2052全
  保留、恰增13；read4/wire3/SDK6、Pump56、HTTP208、125步及66类型/201字段通过。
  同次最终消息/净输出/fee/tip与原时效、旧无小费数学/账户解码/原发现/HTTP/夹具
  重建和22负例源审计通过。仍仅测试原型，无新签名权限或生产节点验收；原
  provider/ABI/接收者策略、真实费用/rent与协议专用原签名字节账本接线仍未完成。
- 原 NP-01 [最小输出与完整无签名消息](PUMP_SDK_SWAP_CONSTRAINTS.md)已独立验收：
  `a2a9f27` / [Actions44348156801](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44348156801)
  / artifact14450265978。并发2052（351.28秒）、串行2052（677.53秒），原2044全
  保留、恰增8；constraints4/envelope4、Pump50、HTTP208、122步及64类型/195字段。
  原SDK桥接/旧源码重建、27负例源审计、源码与锁定依赖身份通过。仅测试构建中的
  模板/预算验收，后继13项最终模拟不在该CI，不继承验收；未部署或发送交易。
- 原 NP-01 [模拟后账户](PUMP_SDK_POST_BANK.md)已独立验收：
  `9e1305f` / [Actions44339297404](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44339297404)
  / artifact14447005729。并发2044（389.05秒）、串行2044（743.91秒），原2039全
  保留、恰增5；post-bank5/Pump42、原read7/wire3/adapter3、HTTP208、120步与
  64类型/195字段及源码身份通过。原解码器/旧断言重建与十负例源审计通过。
  首批9029053仅超限夹具预期文案不匹配，修正只区分两道长度关卡，不重跑首批。
  同次账户/余额与Clock证据不等于接收者策略、可信报价或签名授权；未部署。
- 原 NP-01/03 [共享请求绑定](PUMP_SDK_SHARED_PROBE.md)已独立验收：
  `03299ad` / [Actions 44327646677](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44327646677)
  / artifact `14440796497`。并发2039（359.38秒）、串行2039（699.35秒），原2026
  名称全保留、恰增13；read7/wire3/adapter3、Pump37、HTTP208、119步骤及64类型/
  195字段通过。复用原配额/HTTP/冷却，固定消息/11位置/最低slot/时效；只证明请求
  绑定和返回形状，不证明账户owner/data或可信报价，未部署、不重跑。
- 原 NP-01 [无签名报价探测](PUMP_SDK_UNSIGNED_CANDIDATE_GATE.md)已独立验收：
  `45d18c0` / [Actions 44319842306](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44319842306)
  / artifact `14437675802`。并发2026（374.16秒）、串行2026（721.32秒），原2017
  名称全保留、恰增9（4+5）；Pump专项34、HTTP208、116步骤及62类型/189字段通过。
  同次响应整数净输出/资金守恒、消息与时效仅为测试构建证据；当批响应未绑定请求。
  首批9808224因两个同源E0624失败，后继只补原测试模块内pub(super)，未重跑首批。
- 原 NP-01 [Pump.fun 原生费用地址](PUMP_SDK_NATIVE_DESTINATIONS.md)已独立验收：
  `69e1230` / [Actions 44310987087](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44310987087)
  / artifact `14432369441`。并发2017（385.06秒）、串行2017（733.08秒），原2014
  名称保留、恰增3；Pump专项25、HTTP208、116步骤、62类型/189字段及源码身份通过。
  同 bank Global 主地址/buyback 集合、creator PDA 与账户别名拒绝仅为测试构建证据，
  不证明实际费率/可信报价或开放签名。首批4949bee唯一E0599为测试漏导入，未重跑。
- 原 NP-01 [Pump.fun 金额与角色](PUMP_SDK_AMOUNT_WIRE.md)已独立验收：
  `9bb5330` / [Actions 44303096331](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44303096331)
  / artifact `14429825256`。并发2014（372.34秒）、串行2014（709.76秒），原2009
  名称保留、恰增5；Pump专项22、HTTP208、116步骤、62类型/189字段及源码身份通过。
  区分精确输入/固定输出，绑定原快照账户/权限/cashback/固定角色；未接生产或签名。
- 原 NP-01 [Pump.fun 离线账户与资金证据](PUMP_SDK_ACCOUNT_EVIDENCE.md)已独立验收：
  `be136c0` / [Actions 44297259103](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44297259103)
  / artifact `14427210893`。并发2009（365.26秒）、串行2009（707.51秒），原1992
  名称保留、恰增17；专项17、HTTP208、116有序步骤、62类型/189字段及身份核对通过。
  固定四/八账户、真实 Clock/epoch 与整数预算只在测试构建；未接实盘或恢复 PumpAPI。
- 原 NP-03 [普通存储与登记权限](APP_STATE_RESOURCE_OWNERSHIP.md)已独立验收：
  `1d45d25` / [Actions 44289227980](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44289227980)
  / artifact `14421994464`。并发1992（384.69秒）、串行1992（733.56秒），原1989
  名称全保留、恰增3；HTTP208、115步、62类型/189字段及源码身份核对通过。
  首批 `7f6f3c7` 的 `E0277` 仅补原 SDK 日志关联类型 `Send`，运行逻辑和断言保持。
  首批终态未重跑；未部署，不把应用任务 drain 当成 SDK worker 可取消。
- 原 NP-03 [AppState 资源所有权](APP_STATE_RESOURCE_OWNERSHIP.md)已独立验收：
  `65ef56e` / [Actions 44280875076](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44280875076)
  / artifact `14419816862`。并发1989（385.91秒）、串行1989（735.10秒），
  原1984全保留、恰增5；HTTP208、113步/59类型/180字段通过。覆盖同一只读
  Blockhash、后台不持有请求状态、原资源关停交接；SDK 见证不等于发送任务可取消。
  首批 `3eef0d6` 的13个编译错误仅系旧测试缺入口，修正只恢复测试转接，未部署。
- 原 NP-03 / TS-09-A [AppState 请求配置](APP_STATE_REQUEST_SETTINGS.md)已独立验收：
  `839cd85` / [Actions 44272714261](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44272714261)
  / artifact `14415756815`。并发1984（383.21秒）、串行1984（729.06秒），
  原1981全保留、恰增3，HTTP207、109步/58类型/177字段通过。
  首批 `1453907` 唯一失败为测试误比较两次独立生成的时间戳；修正只改测试和记录，
  未重跑首批。生产请求不再持有完整启动配置，认证、默认值和等待期限不变。
- 原 NP-03 原生 SOL 转账广播所有权已独立验收：`f0c7a73` /
  [Actions 44262897322](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44262897322)
  / artifact `14410522852`。并发1981（344.02秒）、串行1981（667.31秒），
  原1978全保留、恰增3，108步/57类型/174字段通过。原选中 RPC/SDK 请求、
  预检/十二秒期限及账本顺序保留，1169文件/14,028,424字节/模式已核对。
  见 [转账广播所有权](SOL_TRANSFER_BROADCAST_OWNERSHIP.md)。
- 原 NP-03 转账确认读取已独立验收：`c9394e7` / [Actions 44257386302](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44257386302)
  / artifact `14408765651`。并发1978（360.21秒）、串行1978（697.06秒），原
  1960名称全保留、恰增18，107步/174字段及1166文件/14,016,617字节快照已核对。
  原签名/节点/十五秒和原确认语义保留，未部署，见 [转账确认读取](SOL_TRANSFER_CONFIRMATION_SHARED_READS.md)。
- 原 NP-03 转账准备读取已独立验收：`a5420ba` / [Actions 44252423493](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44252423493)
  / artifact `14405719189`。并发1960（353.02秒）、串行1960（683.20秒），原
  1939名称全保留、恰增21，104步/171字段及1159文件/13,977,673字节身份通过。
  首候选67d1d39因三个旧夹具漏参失败，未重跑；修正版保持原生产代码和断言，
  见 [转账准备读取](SOL_TRANSFER_PREPARATION_SHARED_READS.md)。未部署。
- 原 NP-03 靓号任务生命周期已独立验收：`3d72a14` /
  [Actions 44245588737](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44245588737)
  首次终态 success，artifact `14403371260` 下载逐项核对。完整并发 1939
  （333.66 秒）/串行 1939（645.91 秒），父批 1926 名称全保留、恰增十三；
  owner8/shutdown5、原 bootstrap/退出/Shadow 合同、101 步及55类型/168原始
  字段通过。原生成/import、同 Arc 交接、原清理与旧释放断言重建，一正九负
  源审计和全树 1152 文件/13,912,894 字节及模式/依赖身份核对通过，main/索引/
  冻结历史保持。未部署，不重跑终态或另发文档 CI，见 [靓号生命周期](VANITY_TASK_LIFECYCLE.md)。
- 原 NP-03 DAMM 报价 probe/账户证据链已独立验收：`903d646` /
  [Actions 44240714400](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44240714400)
  首次终态 success，artifact `14400459607` 下载独立核对。完整并发 1926
  （329.86 秒）/串行 1926（636.14 秒），零失败/忽略/过滤且原始名称相同，
  父批 1897 全保留、恰增二十九。shared/oracle14/实际 SDK quote adapter11/
  真实 HTTP4、旧 runtime5/buy runtime6、全部 99 步与 55 类型/168 原始字段通过。
  原八/九账户、完整同 bank/CPI/费用/净输出及 typed 终态校验保留，五秒账户/
  whole-buy、三十秒 probe 和三秒 freshness 不改；原方法/旧断言重建及一正十负
  审计通过。全树 1148 文件/13,883,013 字节及模式、identity/依赖哈希一致，
  main/真实索引/冻结历史保持。未部署，不本地 Cargo 重试、重跑终态或另发
  文档 CI；见 [报价读取验收](DAMM_QUOTE_SHARED_READS.md)。其余原门槛保留。
- 原 NP-03 DAMM 最终候选模拟及签名前 preview 已独立源码/离线验收，买卖及
  legacy/original 路径复用原 runtime，保留原消息、slot、四秒总限及前后 freshness。
  `26e4b8b97e424b0808a248c88fb5a5a30359e034` /
  [Actions 44234932246](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44234932246)
  首次运行终态 success（28m24s），artifact `14397918749` 下载独立核对：完整
  并发 1897（292.52 秒）/串行 1897（561.63 秒），零失败/忽略/过滤、原始名称
  相同，父批 1874 全保留且恰增二十三。shared/oracle 12/实际买卖候选方法 8/
  真实 HTTP 3、旧 actual runtime 5/buy runtime 6 及全部 96 步通过；52 类型/
  159 原始字段和原源码/恢复检查通过。原 oracle/旧方法/控制流/签名账本字节及
  旧 HTTP/wire 合同逐项核对，新增精确请求/ID/消息见证通过；全树 1141 文件/
  13,804,808 字节及模式、identity/依赖哈希一致，main/索引/冻结历史保持。
  三个失败候选及仅夹具修正留在 [候选读取记录](DAMM_CANDIDATE_SHARED_READS.md)，
  不重跑终态或另发文档 CI，不本地 Cargo 重试或做生产操作。未部署。
  后继 DAMM 报价 probe/账户证据链现已验收，见本页对应项，
  见 [报价读取边界](DAMM_QUOTE_SHARED_READS.md)；不新增任务或分母，其余原门槛保留。
- 原 NP-03 SDK 原签名观测及 Quote 提交观测本批已独立源码/离线验收：两条实际入口复用原共享状态读取，
  保留原 skip/钱包/RPC 顺序/总预算/Unknown 与签名；原 Raw reader/SDK observer
  及旧测试仅测试保留，签名发送和账本不改。SDK 网络轮询受既有一秒共享间隔约束，
  不延长总预算或补单；新增迟到结果的 caller-budget 检查。
  `e81dda4d4704747e43010d442565dfa1eade2de8` /
  [Actions 44223334225](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44223334225)
  首次终态 success；artifact `14392698134` 下载独立核对：完整并发 1874
  （292.68 秒）/串行 1874（549.27 秒），零失败/忽略/过滤且原始名称相同；父批
  1850 全保留、恰增二十四。SDK shared/oracle 12/Quote adapter 6/真实控制器假
  invoke 3/真实 HTTP 3、原全部专项、50 类型/153 原始字段及全部 93 步通过。
  原 reader/parser/SDK observer 和旧测试、原 guard/prepare/invoke/Quote 签名发送/
  消息及共享 runtime/HTTP/预算/CAS/旧 wire 合同逐字核对，精确请求/观测/预算和
  Unknown/原签名/不进入下一 lane 的见证通过。全树 1134 文件/13,748,624 字节
  及模式、identity/依赖哈希一致，main/索引/冻结历史保持。未部署，不本地 Cargo
  重试、不重跑终态或另发文档 CI，不新增任务/分母；下一原入口为 DAMM 零签名
  模拟/报价读取，SDK 内部确认器/任务、全局配额及其他原门槛仍保留，
  见 [SDK 签名共享读取](SDK_SIGNATURE_SHARED_READS.md)。
- 原 NP-03 独立 Quote 状态读取本批已独立源码/离线验收：实际对账 reader 仅快照原前两 RPC 的 URL，
  接原共享 runtime/Observation 配额和冷却；保留原顺序、missing/error 优先与
  原 observation parser，间隔等待后再给备用节点完整请求预算，仍受原三秒总限。
  `259b2fe67a62045a9cb6c5c06c039ab93f80d33b` /
  [Actions 44217059338](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44217059338)
  首次终态 success；artifact `14390716531` 下载独立核对：完整并发 1850
  （267.93 秒）/串行 1850（512.67 秒），零失败/忽略/过滤且原始名称相同；父批
  1831 全保留、恰增十九。shared/oracle 14/真实 HTTP 3/实际 adapter 2、原全部
  专项、49 类型/150 原始字段和全部 89 步通过。原 reader/parser/测试及 SDK 其他
  观测、Application/账本、原共享 admission/transport/cooldown、旧 HTTP 夹具/
  wire 测试及 child 期限逐字核对；精确请求/观测及完整备用预算见证通过。全树
  1128 文件/13,703,583 字节及模式和 identity/依赖哈希一致。main/索引/冻结历史
  保持，不重跑终态或另发文档 CI、不本地 Cargo 重试或做生产操作。未部署；下一
  原入口为 SDK 原签名观测/其他 Quote 观测，DAMM/SDK 内部资源和其余原门槛保留。
  不新增任务或分母，见 [Quote 状态共享读取](QUOTE_STATUS_SHARED_READS.md)。
- 原 NP-03 WSOL 广播所有权本批已独立源码/离线验收：原首 RPC 固定封装随准备/签名移动，实际广播只接受
  通过原 policy/钱包/快照校验的不可变借用包；原 RpcClient 发送配置、签名回执检查、
  Application 十二秒期限/不确定态和账本顺序保持。`d1d9038fec184cb62519741572d70e81fab4fc57` /
  [Actions 44210742181](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44210742181)
  首次终态 success；artifact `14386593797` 下载独立核对：完整并发 1831
  （233.10 秒）/串行 1831（447.86 秒），零失败/忽略/过滤、原始名称相同，父批
  1817 全保留且恰增十四。lane 12/实际 adapter 2、原全部专项、47 类型/144 原始
  字段拒绝和全部 86 步通过。原 root oracle/旧测试/签名/快照/账本、旧 wrapper
  断言和 HTTP 夹具原分支逐字核对，精确请求/回执见证通过；全树 1123 文件/
  13,648,153 字节及模式、identity/依赖哈希一致。main/索引/冻结历史保持，不重跑
  终态或另发文档 CI，不本地 Cargo 重试或做生产操作，不新增任务/分母。未部署；
  独立 Quote 状态、DAMM/不透明 SDK 读取任务、全局配额及原其他门槛继续，见
  [WSOL 广播所有权](WSOL_BROADCAST_OWNERSHIP.md)。
- 原 NP-03 WSOL 未签名/已签模拟本批已独立源码/离线验收：实际两个 wrapper
  接原 runtime/既有模拟配额，保留原授权/快照、请求、错误和签名/账本字节；
  显式要求响应有 err，缺失不能伪装成功。修正源码
  `9d4093617be00634c2e05a453daf9ff6e0bb1bb9` /
  [Actions 44204140210](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44204140210)
  首次运行 success；artifact `14385056342` 下载独立核对：完整并发 1817
  （270.55 秒）/串行 1817（504.81 秒），零失败/忽略/过滤且原始名称相同；
  父批 1799 全保留、恰增十八项。shared/oracle 14/真实 HTTP 3/实际 adapter 1、
  原全部专项和 84 步骤通过；45 类型/138 原始字段拒绝、原源码/恢复通过。原
  模拟 oracle/测试/授权/准备/指令/签名/发送和账本顺序、精确请求/结果及实际
  两 wrapper 原执行 RPC 零请求已核对；全树 1119 文件/13,609,187 字节及模式、
  identity/依赖哈希一致。main/索引/冻结历史保持，不本地 Cargo 重试、不做生产
  操作、不重跑终态或另发文档 CI。签名模拟仍向原配置 RPC 交出可重放签名包，
  provider 信任假设不变；包装保留原广播 RPC，不声称整链路完全迁移。WSOL
  广播资源、独立 Quote 状态、SDK 内部及其他原门槛继续，不新增任务或分母，见
  [WSOL 模拟读取边界](WSOL_SIMULATION_SHARED_READS.md)。
  首轮 `2a1cc27` / Actions `44202635009` 因审查发现缺失 err 可误判成功而主动取消；
  显式字段要求和新旧负例已在上方修正源码独立验收，不重跑原候选或将部分日志算通过。
  `33cece1` / Actions `44203291583` 在测试类型检查因夹具跨模块构造私有
  HttpReply 字段失败；已改用既有测试构造 API，不放宽可见性，不改生产代码。
- 原 NP-03 WSOL 准备读取本批已独立源码/离线验收：真实 execution wallet 通过
  原 runtime/既有五步 scope 重查准备证据，保留原金额/授权/临时 signer 和签名
  发送。复用原十二类/配额，不增 client/cache/worker。源码
  `cf5e63bca191362bf7f234af942b63f73ffc8e16` /
  [Actions 44195580476](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44195580476)
  首次终态 success，artifact `14381545057` 下载独立核对：完整并发 1799
  （263.49 秒）/串行 1799（485.64 秒），零失败/忽略/过滤且名称相同；父批
  1783 全保留、恰增十六项。shared/oracle 12/真实 HTTP 3/ownership 1、原全部
  专项及 81 步骤通过；42 类型/129 原始字段拒绝和原源码/恢复通过。原 root
  oracle/测试、授权/指令/签名/发送、消息和五次请求见证已核对；全树 1114 文件/
  13,553,143 字节及模式、identity/依赖哈希一致。main/索引/冻结历史保持，不
  本地 Cargo 重试、不做生产操作、不重跑终态或另发文档 CI。准备包装仍保留
  RPC 供后续模拟/发送，下一原入口为 WSOL 未签名/已签模拟；独立 Quote 状态、
  SDK 内部资源及原其他门槛继续，不新增任务/分母，见
  [WSOL 准备读取边界](WSOL_PREPARATION_SHARED_READS.md)。
- 原 NP-03 WSOL 签名状态读取本批已独立源码/离线验收：生产 reader 不再持执行
  RpcClient，共用原 runtime/Observation 配额，保留首 Default lane/错误顺序、
  单签名 history 和严格 finality/error/slot parser。`ec608c08db09981eee27c5b15727e6d660aee6a8` /
  [Actions 44188954352](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44188954352)
  首次终态 success；artifact `14377382655` 下载独立核对：完整并发 1783
  （244.35 秒）/串行 1783（447.74 秒），零失败/忽略/过滤且原始名称相同；
  父批 1769 全保留且恰增十四项。shared/oracle 11/真实 HTTP 3/status 合同 59、
  原全部专项和 78 步骤通过；40 类型/123 原始字段拒绝、原源码/恢复成功。
  旧 RpcClient oracle/六项旧测试/严格 parser、零归一化请求与结果见证和 HTTP
  终态矩阵已核对；全树 1110 文件/13,509,839 字节及模式、identity/依赖哈希一致。
  原 main/索引/冻结历史/SDK/迁移保持；未改状态账本/CAS 或签名广播，不本地
  Cargo 重试、不做生产操作、不重跑终态或另发文档 CI。后续续原 WSOL 准备/
  模拟读取、独立 Quote 状态、SDK 内部资源及其余原门槛，不新增任务或分母，见
  [WSOL 状态读取边界](WSOL_STATUS_SHARED_READS.md)。
- 原 NP-03/TS-07/09 WSOL 只读报价已完成本批源码/离线验收：原首 RPC 的 URL/commitment 投影给 SDK
  estimator 和借用五步 scope，共用原 runtime；余额/ATA/租金/hash/fee 顺序及
  Token/native/金额/租金/退款/错误政策保留，原 estimator 仅测试保留、原构建器
  字节不动。公开 Message helper 不交出临时密钥，不改 conversion 账本/签名提交/
  状态。`c196d1b6f1ec3124e141114441e855c18dc70f4b` /
  [Actions 44182843252](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44182843252)
  首次运行终态 success；artifact `14374676338` 下载独立核对：完整并发 1769
  （200.75 秒）/串行 1769（379.92 秒），零失败/忽略/过滤，原始名称集合相同；
  父批 1749 名称全保留且恰增源码二十项。scope 10/SDK oracle policy 7/真实 HTTP 3、
  原全部专项及 75 步骤通过；39 类型/120 原始字段拒绝（含 Message/借用会话）、
  原源码/恢复与 Python policy 十二项通过。旧 estimator/构建器 blob 保持，新旧
  报告/归一化请求逐项见证通过。全树 1105 文件/13,473,071 字节和模式与快照一致，
  identity/manifest/lock/vendor manifest 匹配。旧串行 stderr 精确关联/两正七负
  格式回归通过，未改日志或省略名称。原 main/索引/冻结历史/SDK/迁移保持，不本地
  重试 Cargo、不重跑终态或另发文档 CI；记录随下一源码批归档。下一原 NP-03
  入口为 WSOL 状态读取：原单签名/history 和更严格 finality/context/error 检查
  应保留，不能换成宽松普通 parser；状态账本/CAS、准备/模拟/签名提交仍未迁移。
  其他独立读取/SDK 内部资源和原门槛继续，不新增任务或分母，见
  [WSOL 报价读取边界](WSOL_QUOTE_SHARED_READS.md)。
- 原 NP-03/TS-07/09 聚合模拟和余额基线已完成本批源码/离线验收：根 SDK 仅投影原 runtime/Default URL/
  钱包公钥，SDK adapter 保留先验基线/Token 校验后模拟的顺序及原报告/CPI/同 slot
  差额政策。固定零签名 packet、pair/loaded keys 和基线 slot，一次性借用会话复用
  原 HTTP/冷却与第十一类预算，不新增 client/cache/signer/worker。
  `2f12c97fad9b35d81c38e4515b03362c410234b9` /
  [Actions 44173368003](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44173368003)
  首次运行终态 success；artifact `14371180749` 下载独立核对：完整并发 1749
  （220.73 秒）/串行 1749（402.90 秒），零失败/忽略/过滤，原始名称集合相同；
  父批 1729 名称全保留且恰增源码二十项。scope 10/SDK pipeline 7/真实 HTTP 3、
  原全部专项和 72 步骤通过；37 类型/114 原始字段拒绝含固定 static lifetime、
  原源码/恢复成功，Python policy 十二项通过。全树 1098 文件/13,405,912 字节及
  模式与快照一致，identity/manifest/lock/vendor manifest 匹配。原纯函数规范 SHA
  与旧测试模块逐字核对保持；旧串行 stderr 精确关联/两正七负格式回归通过，未改
  日志或省略名称。原 main/索引/冻结历史/SDK/迁移保持，不本地重试 Cargo、不重跑
  终态或另发文档 CI；记录随下一源码批归档。下一原 NP-03 入口为 WSOL 只读报价：
  原余额/ATA/租金/hash/fee 五类读取仍借执行 RPC，应保留原估值和金额校验、独立
  conversion 账本/签名提交，逐项接原 runtime。其他独立读取/SDK 内部资源和原
  门槛继续，不新增任务或分母，见 [聚合模拟读取边界](AGGREGATOR_SIMULATION_SHARED_READS.md)。
- 原 NP-03/TS-07/09 聚合预检账户读取已完成本批源码/离线验收：Mint/ATA 与 ALT 的原校验提到 SDK
  adapter，三条实际入口（独立预检/Quote 提交/普通聚合买入含 observed）共用原
  bootstrap runtime。原第一 Default lane/commitment、Token 程序与 ALT 解码/停用
  政策保留；第十类单在途/100 ms、固定只读请求/64 KiB/两秒 HTTP 和共享冷却，
  不新增 client/cache/signer。`10c5b4b0f8cd8ba04c03d088850e0806e2c35f18` /
  [Actions 44164727015](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44164727015)
  首次运行终态 success；artifact `14366377330` 下载独立核对：完整并发 1729
  （212.62 秒）/串行 1729（387.94 秒），零失败/忽略/过滤、原始名称集合相同；
  父批 1713 名称全保留且恰增源码十六项。runtime 8/SDK policy 5/真实 HTTP 3、
  原参数 28/寻池 8/8/2/hop 8/10/2 与执行回归通过；35 类型/108 原始字段拒绝、
  原源码/恢复及全部 69 步骤通过。全树 1090 文件/13,341,064 字节和模式与快照一致，
  identity/manifest/lock/vendor manifest 匹配。旧串行 stderr 精确关联/两正七负
  格式回归通过，未改日志或省略名称。原 main/索引/冻结历史/SDK/迁移保持，不本地
  重试 Cargo、不重跑终态或另发文档 CI；记录随下一源码批归档。模拟及其余额基线、
  独立 quote/WSOL、SDK 内部资源和其余原门槛继续，见
  [聚合账户读取边界](AGGREGATOR_ACCOUNT_SHARED_READS.md)。不新增任务或分母，未部署。
  首轮 43a17e0 / Actions 44163177413 / artifact 14365660021 已终态失败于锁定版本
  UiAccount 无 decode 方法；仅十项前置通过，测试/门禁未运行。已按既有 Base64/
  owner/size 政策组装账户并补负例，修正源码已在上方 10c5b4b 独立验收，不重跑旧失败或改依赖。
- 原 NP-03/TS-07/09 两跳寻池已完成本批源码/离线验收：原 Default lane、原缓存实例/锁、30 秒正缓存、
  两个完整空扫描规则保持；扫描/候选/CPMM 参数/金库余额已接相同 runtime 的新有界
  只读 scope。五秒总期限、32 次 SDK 准入、单在途/100 ms 与共享冷却，不新增 signer/
  cache/HTTP。`17bc3be30b386cf4f16b80c9f8fdc864dda43167` /
  [Actions 44156380200](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44156380200)
  首次运行终态 success，artifact `14362832442` 下载独立核对：完整并发 1713
  （194.93 秒）/串行 1713（355.03 秒），零失败/忽略/过滤、原始名称集合相同；
  父批 1693 名称全保留且恰增源码二十项。scope 8/SDK cache 10/真实 HTTP 2、
  参数 scope 28 和原寻池 8/8/2 通过；35 类型/108 原始字段拒绝、原源码/恢复及
  全部 66 步骤通过。全树 1084 文件/13,296,707 字节和模式与测试快照一致，identity/
  manifest/lock/vendor manifest 匹配。串行旧子进程 stderr 严格按固定诊断和原名称
  关联，两正/七负格式回归通过，未改日志或省略名称。原 main/索引/冻结历史保持，
  不本地重试 Cargo、不重跑终态或另发文档 CI；记录随下一源码批归档。模拟/独立
  quote/WSOL 读取、SDK 内部资源与其余原门槛继续，不新增任务或分母，见
  [两跳寻池边界](SOL_HOP_SHARED_READS.md)。
- 原 NP-03/TS-07/09 普通寻池子项已完成本批源码/离线验收：原 Default lane 投影后交只读 SDK adapter，
  Pump/DBC/PumpSwap 的顺序、Helius 随机选择及每节点三秒总期限保留。分支分别持
  sticky failure，接原 runtime 的两分支/100 ms 独立预算和同一冷却，不降低参数
  250 ms 门槛、不复制 decoder 或创建新 HTTP。
  `cde04bdc9236a7964ac735e01615c05631d3ccda` /
  [Actions 44146511097](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44146511097)
  首次运行终态 success，artifact `14359025763` 下载独立核对：完整并发 1693
  （182.21 秒）/串行 1693（340.51 秒），零失败/忽略/过滤；父批 1675 名称全保留，
  恰增 18（含一个隔离 helper）。寻池 scope 8/loader 8/真实 HTTP 2、原参数 scope
  20/loader 6/HTTP 4 全过，34 类型/105 个原始字段拒绝、原源码/恢复及全部 63 步骤
  通过。全树 1079 文件/13,241,022 字节与测试快照一致，identity/manifest/lock/
  vendor manifest 匹配。串行一处子进程 stderr 混入前缀，严格按原名称、固定夹具
  诊断及紧随 `ok` 关联，并通过两正/七负日志格式检查，未改原始日志或省略名称。
  原 main/索引、冻结历史、SDK 和迁移保持；未本地编译、部署或读生产数据。
  不重跑终态或另发文档 CI；记录随下一源码批归档。两跳/模拟/其他独立读取及
  原完整门槛仍保留，见 [普通寻池边界](POOL_DISCOVERY_SHARED_READS.md)。
- 原 NP-03/TS-07/09 参数读取子项已完成本批源码/离线验收：普通准备与预热接同一
  runtime 的只读 scope，SDK decoder 不复制，执行 RPC/sign/sender 不替换。PumpSwap
  两项固定配置合入同次六账户快照，原四项不变；两项 raw 值仅本 scope 使用，结束
  后释放，不新增跨会话缓存、不放宽 250 ms 间隔，避免冷配置 180 ms 超时冲突。
  `1792c518a3937e308a71674875f658a823761909` /
  [Actions 44133854054](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44133854054)
  首次运行终态 success，artifact `14351589608` 已下载独立核对：完整并发 1675
  （156.47 秒）/串行 1675（294.26 秒），零失败/忽略/过滤，名称集合相同，父批
  1652 全保留且恰增 23（含一项隔离 helper）。参数 scope 12/loader 6/真实 HTTP 4、
  原买入 runtime 6 全过；冷成功和可选故障分别在独立进程验证。33 类型/102 原始
  compiler spans、原源码/恢复检查及全部 60 步骤通过。全树 1074 文件/13,188,757
  字节与当前源码一致，identity/manifest/lock/vendor manifest 匹配，真实 main/
  索引、冻结历史、SDK 与迁移保持。不重跑终态或另触发文档 CI；验收记录随下一批
  源码归档。小 VM 在 2200 MiB/50 秒触限后未重试，最终类型检查在本批云端通过。
  详见 [参数读取边界](PROTOCOL_PARAMETER_SHARED_READS.md)。两跳寻池/模拟/SDK 内部及
  独立读取、全局额度和其余 NP-01/03/04/05/真实环境门槛仍在原队列，不新增任务。
- 本轮续原 NP-03/TS-07/09：普通买入账户存在性提示接同一个 SharedReadRuntime 的
  preparation permit/250 ms 起点预算和跨类别 429 冷却；移除根 SDK 门面该存在性
  helper 的唯一调用及旧实现，原 program-label 布尔语义保留，不冒充 ATA/余额或签名
  证据。新增十四项合同与一个 CI 专项已独立验收。API/hash/JSON、claim/journal/sender/
  迁移及 SDK 不改；其他参数/模拟/
  寻池读取仍须续作。详见 [存在性读取边界](PREPARATION_PRESENCE_SHARED_RPC.md)。
  本批 `27bd9969a5a725547ae22a0c97701030be9b30aa` /
  [Actions 44115951258](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44115951258)
  已终态 success，artifact `14343274443` 下载独立核对：完整并发 1652（168.50 秒）/
  串行 1652（296.55 秒），零失败/忽略/过滤；两套原始名称集合相同、父批 1638 全
  保留，增量恰为上述十四项。presence 专项 13、原实际买入 runtime 5、controlled-buy
  6、claims 3、executor 10、sender 15、扫描 17、买入 probe 79 通过；原源码/恢复
  Clippy 成功，原始 spans 独立核对 99 字段拒绝及全部 57 步骤 exit=0，identity/
  manifest/lock/vendor manifest 匹配。发布审计 1066 文件/13,089,071 字节、20 白名单
  路径；最终受限 test check 11 秒/RSS 1,342,232 KiB，production check 10 秒/RSS
  772,052 KiB、原七组警告，Python policy 十项/workflow 七项与 SDK 来源通过。
  原 main/索引/冻结历史保持；不重跑终态 CI，验收记录不单独触发文档 CI，未部署。
- 以下原 NP-01 买入执行批次已验收，不重做或将其结果套用于上述新读取变更。
- 本次续同一个 NP-01：原买入候选增加 live 只读整笔预览、线性 permit 的受控签名及
  私有签后 artifact；原 `OriginalDammBackend` 按方向分派，同一 journal/CAS/独立
  回读后接固定 Buy sender，不走高层 SDK worker。原 Durable Live 直接 DAMM classifier
  同步接正整数 Native SOL/WSOL 和既有 USDC；同一个 SDK 索引页纳入 buy/sell，不改
  DDL、旧 SellV1 或 hash/API。源码已接，不代表已部署或实际节点验收。
- 新增十七项合同覆盖实际准备 RPC→零签名报价/整笔预览→原 signer→临时 SQLite
  落盘/读回→固定发送器、两资产、错回执/丢写/取消；另有 owner 双连接竞争/重开、
  资产/模式/方向范围、原 executor 分流/禁播/过期、混合扫描，以及精确 key/send 次数。
  本批 `882e514a499df01c68c704671ecdf335755e8ea9` /
  [Actions 44109233896](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44109233896)
  已终态 success，artifact `14340653473` 下载独立核对：完整并发 1638（149.17 秒）/
  串行 1638（275.68 秒），零失败/忽略/过滤；原始名称集合相同、父批 1621 全保留，
  增量恰为源码十七项。controlled-buy 6、buy-claims 3、原实际买入 runtime 4、executor
  10、sender 15、扫描 17、买入 probe 79 全部通过；原源码/恢复 Clippy 成功，原始
  spans 独立核对 99 个不同字段拒绝及全部 56 步骤 exit=0。identity/manifest/lock/
  vendor manifest 匹配，发布审计绑定 1062 文件/13,058,154 字节；受限 test check
  13 秒/RSS 1,427,180 KiB。夹具使用原 rate-limiter profile，并逐字核对实际 discovery
  消息。真实 main/索引、冻结历史、旧 SellV1、SDK/依赖/迁移保持；不重跑终态任务，
  验收记录不单独触发文档 CI。不代表真实节点/供应商、生产恢复或部署验收。
  详见 [原执行接线](SDK_ORIGINAL_DAMM_RUNTIME.md#2026-11-17-原买入受控执行接线)。
- 以下为已完成批次的历史证据，关于“买入 claim 仍闭”的描述仅适用于对应快照；
  不据此重做已完成工作，也不将任一批结果套用于后继源码。不新增任务或完成度分母。
- 本轮续原 NP-01 的买入历史证据和原意图绑定：共同 adapter 已按记录方向重构原完整
  消息并校验有界 canonical 签名字节/钱包；卖出入口仍有专用方向闸门。原 journal
  行及签名回读接此校验器，历史请求按方向重放原 V1 整数/费用默认值及 hash，原
  ownerless capture/finality 已可核对买入历史。`validate_candidate_intent` 支持买入
  原 admission 的资产/数量/钱包/执行意图绑定，但 validate_origin 前置原 operation
  classifier 未改，买入 claim/实际 signer/sender/自动扫描仍未开放。详见
  [买入历史证据](SDK_BUY_HISTORY_EVIDENCE.md)。
- 新增十六项源码合同覆盖真实买入消息/临时 key 的历史签名、坏包/匹配摘要的指令
  篡改、买卖隔离、原意图突变及 claim 继续拒绝、V1 显式金额/原整数和费用默认值/
  精度、临时 SQLite 两资产四 signed stages、身份/签名破坏、finality CAS/原 nonce/
  上下文替换和原扫描继续排除买入。本批 `f029d55c1778f0688015d85d29dab7562146ee39` /
  [Actions 43785820055](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43785820055)
  已终态 success，artifact `14190956041` 下载独立核对：完整并发 1621（120.77 秒）/
  串行 1621（227.60 秒）零失败/忽略/过滤，两套原始名称相同、父批 1605 全保留，
  增量恰为源码十六项。买入历史/意图专项 18（含原两项）、共同记录 9、买入 79、
  原源码/恢复 Clippy 成功、原始 spans 的 93 个不同字段拒绝及全部 53 步骤通过。
  identity/manifest/lock/vendor manifest 匹配，发布审计绑定 1059 文件/12,989,654
  字节、30 白名单路径；原 claim/sender/扫描 SQL/迁移、旧 SellV1、冻结历史、真实
  main/索引保持。最终受限 test check 11 秒/RSS 1,365,552 KiB，production check
  5 秒/RSS 672,372 KiB，原七组警告；fmt、Python 十项/workflow 七项通过。不把
  合成历史 fixture 的签名当作生产受控 signer 验收；不部署或扩大买入 claim。
  终态 CI 不重跑，验收记录不单独触发文档 CI。
- 前批原 NP-01 的原 journal 类型边界：`SdkPreparedRecord` 明确持有旧 SellV1 或独立
  BuyV1，原 Application/backend/journal/permit、候选及回读接口已改为此项目类型。
  按 version/profile 严格分派，不试探式回退，不增加 JSON 外壳；旧 SellV1 源码原字节
  保持。当批买入 codec 被共同 decoder 使用，但原 claim/origin、重构/签名及恢复仍
  显式限制卖出；买入不能由名义记录获得 owner/permit。prepare/signing 在 SQL 之前
  拒绝错误方向，见 [原账本记录边界](SDK_PREPARED_RECORD_BOUNDARY.md)。
- 本批新增十一项合同覆盖旧 JSON/实际落库原字节、真实买入往返、严格标签/重复字段/
  大小和摘要、关闭临时 pool 的 SQL 前拒绝、损坏行/原 owner 与只读恢复、原控制器
  零 signer/send 和线性 permit 方向绑定；既有买入矩阵补共同 codec 检查。本批
  `2fe436dff1326b5ce0d069804df2e4ba3ad6b07b` /
  [Actions 43781625542](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43781625542)
  已终态 success，artifact `14189296618` 下载独立核对。完整并发 1605（142.55 秒）/
  串行 1605（261.47 秒），零失败/忽略/过滤；两套原始名称集合相同，父批 1594 全保留，
  增量恰为源码十一项。共同记录专项 9、买入专项 79、原源码/恢复 Clippy 成功，原始
  spans 的 93 个不同字段拒绝及全部 52 步骤通过。identity/manifest/lock/vendor manifest
  匹配，发布审计绑定 1052 文件/12,938,550 字节、35 白名单路径；真实 main/索引、
  旧 SellV1、claim/sender/扫描 SQL、迁移与冻结历史保持。本地最终受限 test check
  10 秒/RSS 1,319,604 KiB、production check 13 秒/RSS 1,004,024 KiB，原七组警告；
  fmt、Python 门禁十项/workflow 七项通过。不升级 SDK、不修改 schema 或放开买入
  claim，不部署/重启或触碰生产数据。不重跑终态 CI，验收记录不另触发文档 CI。
- 本批记录快照 `19f80cb00f2816341791a359d4504bc5fe83ea1d` 已独立验收；[Actions 43777721549](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43777721549) 终态 success，artifact `14187333877` 下载核对。完整并发 1594（140.68 秒）/串行 1594（257.60 秒），零失败/忽略/过滤；两套原始名称集合相同、父批 1583 全保留，增量恰为十一项新合同。买入专项 79、原源码/恢复成功、原始 compiler spans 的 93 字段拒绝及全部 51 步骤通过。发布审计绑定 1047 文件/12,913,244 字节、16 白名单路径，全树哈希一致，旧 SellV1/Application/claim/journal/sender/迁移及 manifest/lock/vendor/冻结历史/main/真实索引保持。终态任务不重跑，验收记录不另发文档 CI。
- 当前续原 NP-01 的买入记录基础：独立 `DammBuyCandidateRecord`/`damm-v2-buy-v1` 名义数据已接原候选生成和整笔复核；旧 SellV1 类/JSON/校验未改，不伪造卖出比例。保存原始整数、账户/精度/slot、同 bank 资金、费用和消息摘要，纯重构后再与原运行时证据比较；名义记录不能恢复 fresh Quote/owner/签名许可，往返编码方法仍仅测试启用，未接库。
- 十一项新增合同含真实矩阵往返、USDC→WSOL、消息/资金篡改、名义与原证据分离、过期/买卖隔离/未知字段、体积/大整数、模式与 provider 原名称，已在本批专项和完整 CI 实际执行；原始测试名增量独立核对，不借用父批 1583。
- 本地最终受限 test check 11 秒/RSS 1,377,028 KiB、production check 18 秒/RSS 1,056,900 KiB 通过，原七组生产警告；fmt、Python 门禁十项/workflow 七项通过。未升级 SDK、链接本地大型测试、部署/重启、触碰生产配置/钱包/数据库或发送交易。
- 本批快照 `ec1d2c9b1569802e245239d0c0437261be578a0e` 已独立验收；[Actions 43773807696](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43773807696) 终态 success，artifact `14185728930` 下载核对。完整并发 1583（155.62 秒）/串行 1583（274.91 秒），零失败/忽略/过滤；两套原始名称集合相同，父批 1564 全保留，增量恰为十九项新合同。买入专项 68、93 字段拒绝及全部 51 步骤通过。发布审计绑定 1044 文件/12,875,578 字节、21 个白名单路径，全树源码哈希一致，manifest/lock/vendor/冻结历史/真实 main 和索引不变。不重跑终态任务，也不另发文档 CI。
- 用户已恢复 active，续原 NP-01 买入最终消息子项，不重跑 `61af682` 的终态 CI。本批源码已有 `DammBuyMessage`、私有整笔候选、原 lane 入口实际请求复查、protocol/full 单次回调和返回签名绑定；模拟走原只读 RPC/quote slot 的零签名包，不进入 SDK signer。费用封装/编译/provider 小费表复用从原卖出提取的纯函数，旧回归入口保留，无第二套 Client/Registry/账本。
- 同次报价已验证的剩余 native 余额还须覆盖 300 lamports 优先费和原取整规则的小费；不足/溢出为 typed 终态准备拒绝，不切 lane。新增十九项 Rust 用例（含 64 组真实 SDK 原始指令/独立整笔编译及原普通编排串联）已由本批专项和完整 CI 实际执行。模拟不再生成旧 SDK 的非广播签名，现有可空 signature 为 null，不伪造旧值。暂停前夹具 wallet 遮蔽已修正并验收；原始 compiler spans 独立确认 93 个不同字段拒绝，原源码/恢复检查成功，不借用父批的 87。
- 最终本地受限 test check 11 秒/RSS 1,387,596 KiB、production check 5 秒/RSS 601,988 KiB 通过；原七组生产警告保持，测试构建无新增警告。格式、Python 边界工具十项和 workflow 七项通过；不链接本地服务测试，不将元数据检查当作 Rust 运行验收。
- 此批未增加 Live/operation profile、owner、表结构或发送路径。买入仍缺原 owner 的 prepared/signed 原字节账本/固定 sender 与恢复，最后回调之后的 SDK worker 时效和副作用也未完成；不把局部回调审计或无签名模拟当作这些门槛的完成。
- 前序准备快照 `61af682d81a52b69db891ebf12226095c7004cd6` 已在原验证分支独立验收；[Actions 43734656743](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43734656743) 终态 success，artifact `14170038159` 下载核对。完整并发 1564（152.42 秒）/串行 1564（268.72 秒），零失败/忽略/过滤；原始名称集合相同、父批 1548 全保留，增量恰为源码中的十六项。买入专项 49、87 字段拒绝及全部 51 步骤通过。发布前核对 1039 文件/12,820,838 字节、19 个白名单路径，全树与工作区源码哈希一致，manifest/lock/vendor/冻结历史/真实 main 与索引保持。不重跑父批，不另触发文档 CI。
- 当前原 NP-01 买入准备/min_out 子项已接生产源码：同一 lane 的九账户/零签名净报价进入 ordinary buy，原整数滑点算出最小输出，不再由正式参数工厂固定 Some(1)。缺失/零值/舍入或报价失败拒绝，无常量回退；非 DAMM 保持 None。冻结钱包/provider/模式/完整 ExecuteTrade、池与 Token 程序、输入量、blockhash、原费用策略与请求最小输出；SDK 入口前再核对三秒期限，输出捕获仍同一个 per-invocation mutex/atomic，以名义枚举区分买卖。
- 新增十六项合同（绑定十项、当前 bank 两项、准备四项）已由本批专项与完整 CI 实际执行，原始测试名增量独立核对；本地受限 check 13 秒/RSS 1,286,520 KiB、production check 12 秒/RSS 1,004,892 KiB 通过，原七组警告保持。原源码/恢复 Clippy 成功，原始 JSON spans 独立确认三层共 87 个不同字段拒绝。旧 V1 参数指纹用显式传入原 1 验证未变，不修改冻结 oracle 或将其当作执行许可。
- 这是原既有 SDK 买入路径的准备保护，不新增 Live/SDK operation profile、owner 或表结构。最终不可变消息授权、入场后 SDK worker/时效、原签名字节账本/固定 sender、共享 RPC 全局预算与真实供应商字段能力仍未完成；SDK simulate 也不冒充全程无 signer。未升级 SDK、构建 release、部署/重启、读取生产配置/钱包/数据库或发交易。
- 上个已验收兼容基线 `fca54ff9a87581822502707bd9226a5a917c2786`；[Actions 43730145561](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43730145561) 终态 success，artifact `14168537736` 下载独立核对。买入专项 33、完整并发 1548（138.47 秒）/串行 1548（251.95 秒），父批 1538 保留、原始名称增量恰为十项；87 字段拒绝、原源码/恢复成功及全部 51 步骤通过。审计绑定 1037 文件/12,779,254 字节、12 路径，不重跑终态任务。
- 上个已验收资金取证基线 `5e0ed8f64bcdfc8b0b5b4cdd61a4051df41114fe`；[Actions 43727995203](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43727995203) 终态 success，artifact `14167687550` 下载独立核对。买入专项 23（含新增 16）、完整并发 1538（138.18 秒）/串行 1538（252.38 秒），原始测试名集合相同且较父批恰增 16；87 字段拒绝、原源码/恢复及全部 51 步骤通过。发布审计绑定 1036 文件/12,760,389 字节、11 路径；不重跑终态 CI，也不将当批保守拒绝范围冒称本批兼容已验收。
- 上个已验收买入 ABI 基线 `48f15b3300e0206132d771bb7807dd1cfb63eef6`；[Actions 43725621186](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43725621186) 终态 success，artifact `14166866773` 下载独立核对，不再等待或重跑。专项七项、完整并发 1522（147.65 秒）/串行 1522（258.80 秒），原始名称增量恰为这七项；87 字段拒绝、原源码/恢复成功及全部 51 步骤通过。审计绑定 1034 文件/12,713,954 字节、12 路径，manifest/lock/vendor 和真实 main/索引/冻结历史保持。
- 上个已验收基线 `796ba5037149db4c3511aff7c3c7ae80f83cbc67`；[Actions 43722582055](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43722582055) 终态 success，artifact `14165837512` 已下载独立核对，不再等待或重跑。发布审计绑定 1032 文件/12,691,338 字节、17 变更路径；SQL helper/recovery 字节、WAL/Full/Normal/连接上限、manifest/lock/vendor 和其他原源码与父快照一致。
- 原 NP-03 Shadow writer 关闭/排空本批源码及离线验收完成：同一个原 1024 项 FIFO、pool 和 SQL 调用，保留原 handle、取消安全等待及历史错误退出诊断。新增专项 13/4 共 17；完整并发 1515（147.21 秒）/串行 1515（258.54 秒），原始测试名集合相同，较父批 1498 恰增加这 17 项，全部 50 步骤 exit=0。三层原始诊断独立确认 87 字段拒绝，原源码/恢复检查成功。详见 [Shadow 生命周期](SHADOW_WRITER_LIFECYCLE.md)。
- 新 `TRADE_SHADOW_DRAIN_TIMEOUT_MS` 默认 5000、范围 1..300000，独立于普通任务预算，Memory/Durable 无队列退出动作；原 health/非阻塞 enqueue/幂等及签名权威不改。未修改线上配置、SDK、服务或生产数据。
- 前序普通任务生命周期 `c6f2745e2fee315d454b0c7327d1469f6c1fb441` 已验收；[Actions 43718342603](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43718342603) 终态 success，artifact `14163942665` 下载核对，不再等待或重跑。该批审计绑定 1029 文件/12,654,972 字节、25 个变更路径，不套用给当前 Shadow 子项。
- 普通后台任务登记/原 executor 派发、停机闸门、有界 drain 与原刷新/观测后置停止已完成本批源码/离线验收。专项 12/4/1 共 17 项全部通过；完整并发 1498（134.03 秒）/串行 1498（244.95 秒），原始测试名集合相同，较父快照恰好增加这 17 项。全部 48 步骤 exit=0。详见 [生命周期边界](ORDINARY_TRADE_TASK_LIFECYCLE.md)。
- 新 `TRADE_TASK_DRAIN_TIMEOUT_MS` 默认 30000、范围 1..300000，只用于未来源码启动时的普通任务退出等待；未改线上配置或服务。三层门禁的原源码/恢复检查成功，独立读原始 JSON spans 确认 87 个不同字段拒绝：28 个选定类型及 outside_alias 在三层各一组，含两个新运行时类型，不借用父批 81。
- 最终本地 test check 9 秒/RSS 1,241,036 KiB；production check 12 秒/RSS 994,552 KiB，原七组警告。完整运行只在本批源码/公开夹具 CI，没有本地测试大链接、SDK 升级、部署或生产操作。发布后仅更新验收与停机 503 的原 key 说明，不单独触发文档 CI。
- 前序 Record/Tape 独立构建合同 91b3d94 已验收，不再等待或重跑；明细见下表与 [后端合同证据](BACKEND_CONSTRUCTION_CONTRACT.md)。工具 Rust 3、Python 门禁 10/workflow 7 及旧专项在本批继续通过。整体 NP-01/03/04/05、真实环境和发布仍未完成。
- 设计补充已逐项核对实际源码并归回主计划 3.3：Parser、SDK 门面、资源所有权与能力证据均有基础和明确剩余门槛，RouterLeg 仅设计方向。`src/infrastructure/` 已有任务与退出编排，附件“缺少目录”的旧判断不再适用。修正普通接受/执行已验收却仍列“待验”的旧摘要；不新增任务编号、分母、依赖或运行能力。现有共用 Engine 的两候选测试不计作 NP-04 独立双构建后端完成。

## 已验收基线

| 范围 | 提交 / CI / artifact | 已核对结果 |
| --- | --- | --- |
| 普通 Pump/DBC/PumpSwap 寻池共享只读分支 | `cde04bd` / `44146511097` / `14359025763` | scope 8/loader 8/HTTP 2；完整并发 1693（182.21 秒）/串行 1693（340.51 秒），新增 18；105 字段拒绝，63 步骤通过 |
| 协议参数/预热共享 scope 与冷配置快照 | `1792c51` / `44133854054` / `14351589608` | scope 12/loader 6/HTTP 4、原买入 runtime 6；完整并发 1675（156.47 秒）/串行 1675（294.26 秒），新增 23；102 字段拒绝，60 步骤通过 |
| 普通买入 presence 共用 preparation 预算 | `27bd996` / `44115951258` / `14343274443` | presence 13、原买入 runtime 5；完整并发 1652（168.50 秒）/串行 1652（296.55 秒），新增 14；99 字段拒绝，57 步骤通过 |
| 原 owner 受控买入、固定原字节发送与 claim/扫描 | `882e514` / `44109233896` / `14340653473` | 新增 17；完整并发 1638（149.17 秒）/串行 1638（275.68 秒）；99 字段拒绝，56 步骤通过 |
| 买入原意图/消息签名证据与 V1 历史回放 | `f029d55` / `43785820055` / `14190956041` | 新专项 18（新增 16）；完整并发 1621（120.77 秒）/串行 1621（227.60 秒）；93 字段拒绝，53 步骤通过 |
| 原 SDK journal 的买卖类型化记录边界 | `2fe436d` / `43781625542` / `14189296618` | 共同记录专项 9、新增 11、买入 79；完整并发 1605（142.55 秒）/串行 1605（261.47 秒）；93 字段拒绝，52 步骤通过 |
| 买入独立名义记录与原候选数据绑定 | `19f80cb` / `43777721549` / `14187333877` | 买入专项 79（含新增 11）；完整并发 1594（140.68 秒）/串行 1594（257.60 秒）；93 字段拒绝，51 步骤通过 |
| 买入整笔候选/回调与无签名模拟 | `ec1d2c9` / `43773807696` / `14185728930` | 买入专项 68（含新增 19）；完整并发 1583（155.62 秒）/串行 1583（274.91 秒）；93 字段拒绝，51 步骤通过 |
| 买入净报价/min_out 准备绑定 | `61af682` / `43734656743` / `14170038159` | 买入专项 49（含新增 16）；完整并发 1564（152.42 秒）/串行 1564（268.72 秒）；87 字段拒绝，51 步骤通过 |
| 买入同步/预充值兼容原型 | `fca54ff` / `43730145561` / `14168537736` | 买入专项 33（含新增 10）；完整并发 1548（138.47 秒）/串行 1548（251.95 秒）；87 字段拒绝，51 步骤通过 |
| 买入只读资金/净输出原型 | `5e0ed8f` / `43727995203` / `14167687550` | 买入专项 23（含新增 16）；完整并发 1538（138.18 秒）/串行 1538（252.38 秒）；87 字段拒绝，51 步骤通过 |
| 买入 ABI/funding 原型 | `48f15b3` / `43725621186` / `14166866773` | 专项 7（含 72 SDK builder 对照）；完整并发 1522（147.65 秒）/串行 1522（258.80 秒）；87 字段拒绝，51 步骤通过 |
| 原 Shadow writer 生命周期 | `796ba50` / `43722582055` / `14165837512` | 专项 13/4；完整并发 1515（147.21 秒）/串行 1515（258.54 秒）；87 字段拒绝，50 步骤通过 |
| 普通后台交易生命周期 | `c6f2745` / `43718342603` / `14163942665` | 专项 12/4/1；完整并发 1498（134.03 秒）/串行 1498（244.95 秒）；87 字段拒绝，48 步骤通过 |
| 原普通 SOL Quote 双假构建后端合同 | `91b3d94` / `43714527309` / `14162686991` | 专项 8；完整并发 1481（129.52 秒）/串行 1481（236.54 秒）；81 字段拒绝，45 步骤通过 |
| 三层类型/别名门禁 | `e3be442` / `43711006653` / `14161032189` | 原源码/恢复检查成功，81 个独立字段拒绝；完整并发/串行各 1473，44 步骤通过 |
| 普通执行 Application 与项目资源 ports | `26445c5` / `43707853050` / `14159283766` | 专项 7，完整并发 1473（112.34 秒）/串行 1473（209.87 秒），全部步骤通过 |
| 普通接受 Application 与原 V1 codec | `26be717` / `43706312318` / `14159195795` | 专项 8，HTTP 202，完整并发 1466（129.94 秒）/串行 1466（238.29 秒），全部步骤通过 |
| Readiness 窄设置/项目 observer | `c220ba8` / `43696813225` / `14154584905` | 专项 6，完整并发/串行各 1458，全部步骤通过 |
| 原 Bootstrap 资源组装 | `970d835` / `43695116074` / `14154666415` | 专项 4，完整并发/串行各 1452，全部步骤通过 |

以上完整套件均零失败/忽略/过滤；源身份及 manifest/lock/vendor 已核对。
只证明对应快照的源码/离线验收，不覆盖后继改动，不等于 main 合并、真实节点或生产发布。

## 已结束的失败

- `7495079` / Actions `44142801934` / artifact `14356861217`：类型检查、105 字段
  门禁及全部专项通过（新增十八项均运行）。完整并发 1690 过/3 失败，串行未执行。
  原 readiness/native/quote 取消夹具把所有类别容量写死为 1；新寻池类明确为 2。
  已改为逐类精确向量 `[1,1,1,1,1,1,1,2]`，旧释放/起点/drop 断言保持，不放宽
  为非零判断或修改生产额度。后继 cde04bd 已独立完成全套验证，不重跑终态失败。
- `7a63477` / Actions `44132188285` / artifact `14351441124`：源码复核发现 SDK
  冷配置 180 ms 与原逐请求 pacing 冲突，已主动取消被取代的运行。不计整体验收，
  不重跑；新源码以同 bank 六账户快照和隔离冷成功回归解决，已由 1792c51 独立验收，
  不降低超时/限流门槛。
- `255ef09` / Actions `44130661475` / artifact `14348964516`：十个前置步骤通过，
  测试类型检查 221 秒失败。新共享 loader 夹具缺 accounts/RpcRequest 导入且访问
  私有 body 方法；已补显式导入及仅 cfg(test) 的 request/reply helper，不放宽生产
  可见性、loader 或断言。该快照未运行 Rust 测试/102 字段门禁，后继 1792c51 已独立
  验收，不重跑终态失败。
- `ebc51d4` / Actions `44104753013` / artifact `14339127559`：前 29 步通过，
  executor 十项及原卖出 runtime 三项通过。新增买入 runtime 四项未通过：夹具的
  同 bank 余额/token/CPI 索引绑定未开启 rate-limiter sysvar 的旧 discovery 消息。
  源码复核确认原参数工厂开启该 sysvar，不是最终消息的 Compute Budget 差异；
  测试夹具现使用相同配置并逐字核对完整 discovery 消息，不以重排索引掩盖差异。
  受限 metadata check 13 秒通过，RSS 1,427,180 KiB；后继 882e514 的完整买入与两套
  回归已独立验收，不重跑该终态失败。
- `76f898d` / Actions `44107876296`：源码复核发现其索引映射修正未匹配实际
  rate-limiter profile，主动取消已被取代的运行；不计为验收、不重跑。后继只更正
  夹具 profile 和完整消息断言，生产校验/准备行为不改。
- `b572b7a` / Actions `44102178151` / artifact `14336409343`：前 27 步通过，
  controlled-buy 六项、buy-claims 三项、sender 十五项及 99 字段门禁通过；executor
  专项 9/10。新增测试误把原“解析钱包后检查 blockhash”断言为先检查 blockhash，
  已按原顺序修正并补重复调用不重入断言，不改生产行为。实际完整买入 runtime 和
  两套完整回归在该失败快照未执行，后继 882e514 已独立验收；不重跑该终态失败。
- `8bd7796` / Actions `43710108477` / artifact `14159823162`：服务类型检查、工具三项与全部前置通过；Clippy 全局配置未转发，原锁定 zap 来源在离线模式下无法解析。未进入生产门禁、81 负例或完整回归。修正已在 e3be442 完整验收，不重跑旧失败。
- `c58eb8c` / Actions `43705256866` / artifact `14158646075`：新接受夹具 5/3；Shadow 异步镜像等待与 oracle 钱包输入已在 26be717 修复验收，不再排队。
- 其他旧失败及修复保留在历史和各子项文档；不得仅凭历史的 `in_progress` 或“待验”重启任务。

## 续作顺序

1. 普通任务生命周期、Shadow writer、Record/Tape、买入只读取证/兼容/准备/消息/独立记录、原 journal 类型边界、原意图/历史回放，以及原 owner 的受控买入候选/signer、CAS/签名字节回读、固定 Buy sender/runtime 和 claim/扫描均已按各自源码独立验收，不重复实施或重跑终态 CI。882e514 只完成 Durable Live SDK 直接 DAMM 的两类输入子路径；续下方原 NP-01/03/04/05 剩余范围，不以历史恢复 owner/重签/重发，不另立任务，也不核对/修改线上版本。
2. 原 NP-03 不重做或重跑上方已验收的读取、生命周期、转账准备、确认和广播。
   普通存储与登记权限现已独立验收；继续 SDK 内部任务、其他独立操作、全局
   RPC/cache 配额与退出/刷盘门槛。三层编译门禁不代表全部依赖/能力流完成。不恢复
   owner 或重签/重发。
3. 原 NP-01 的其他资产/协议/backend/mode 的不可变构建、完整授权和原签名字节账本继续保留；已验收的 DAMM Durable Live 直接买卖子路径不重做，也不冒充全量。其他 Live worker/模拟和 Memory/Shadow 仍须按实际路径验证，不能继承本批的原 owner 保证。
4. NP-04 本批只证明原 Durable SOL Quote 买入控制器的共同合同，其他业务路径适用性、真实 Router/SDK 不由 fake 证明；NP-05 来源/升级准备和主计划的真实环境/迁移/回滚门槛继续。当前不授权实际 SDK 升级或生产动作。

详细剩余范围见 [进度盘点](PROGRESS_OVERVIEW.md)、[执行资源边界](ORDINARY_EXECUTION_PORT.md)、
[开发验收](DEVELOPMENT_CHECKS.md)、[CI 证据](GITHUB_ACTIONS_VALIDATION.md) 和 [资源所有权](APP_STATE_RESOURCE_OWNERSHIP.md)。
文档整理不新增任务、修改完成度分母、移除未完成条件或调整资金安全优先级。

## 不变边界

- 仅 Trade Service；不修改 Copy Trading、Bridge、sdk9 或其他项目。
- PumpAPI 新交易对接继续暂停，既有兼容合同保留；不停止 ZMQ、不改线上 backend/配置。
- 不升级 SDK，不构建 release，不部署/重启，不读取或操作生产钱包/数据库，不发送交易。
- 小 VM 不链接/执行服务测试二进制；完整验证只在获准的源码/公开数据、仅回环 CI 运行。
- CI 仅 `ci/trade-service-validation-*` 分支，不合并/重写/强推 main，不上传密钥、运行数据或编译产物。
- 未知结果只追踪原签名，不换 key/路线/backend 重签重发；不复制 reservation、缓存或持久化权威。
