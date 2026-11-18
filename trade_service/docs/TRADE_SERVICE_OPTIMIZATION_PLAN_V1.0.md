# Trade Service 独立优化与可升级架构实施计划

- 版本：V1.0
- 日期：2026-11-12
- 2026-11-18 当前原 NP-03：原生 SOL 转账准备 `a5420ba` 已独立验收各1960、104步/
  171字段；转账确认读取已接入现有观察配额，待独立 Rust 验收。保留原签名/选中
  节点/commitment/十五秒预算；签名、广播和账本不改。靓号 `3d72a14` 各1939的
  原证据保留，不重做。见 [执行状态](optimization/EXECUTION_STATUS.md)。
- 2026-11-18 最新续原 NP-03：DAMM 报价账户/probe 已由 `903d646` / Actions
  `44240714400` / artifact `14400459607` 独立验收，完整并发 1926（329.86 秒）/
  串行 1926（636.14 秒），原 1897 名称全保留、恰增二十九；99 步及 55 类型/
  168 原始字段通过，原方法/旧断言/全树身份已核对。未部署。下一原靓号独立
  任务生命周期现由上方 3d72a14 独立验收，不继承父批结果。SDK 固定 6.0.2，无新配置/生成算法，见
  [报价读取验收](optimization/DAMM_QUOTE_SHARED_READS.md) 和
  [靓号退出边界](optimization/VANITY_TASK_LIFECYCLE.md)。不新增任务或分母。
- 状态：2026-11-17 按用户续原非 PumpAPI 队列。原 NP-01 受控买入 signer/签后产物、原 owner/CAS/签名字节落盘回读、固定 Buy sender、runtime 与 claim/扫描已在 `882e514` / Actions `44109233896` 独立验收：完整并发/串行各 1638 项、99 字段门禁及全部 56 步骤通过，父批 1621 全保留且新增恰为十七项。不重复原任务或增加分母；仅覆盖原 Durable Live SDK 直接 DAMM 的 Native SOL/WSOL 与既有 USDC。其他 NP-01/03/04/05 和真实节点/发布门槛保留；SDK 固定 6.0.2，不构建 release/部署/操作生产数据，详见 [当前状态](optimization/EXECUTION_STATUS.md)、[开发门禁](optimization/DEVELOPMENT_CHECKS.md) 与 [CI 证据](optimization/GITHUB_ACTIONS_VALIDATION.md)。
- 同日续原 NP-03 的普通买入账户存在性读取：源码接原 preparation 容量/起点与共享
  冷却，保留原布尔提示语义；`27bd996` / Actions `44115951258` / artifact `14343274443`
  独立验收完整并发/串行各 1652、57 步骤及 99 字段拒绝通过，父批 1638 保留且新增
  恰为十四项，不套用上批结果。参数/寻池/
  模拟和全局额度仍在原队列，不重复立项，见 [读取接线](optimization/PREPARATION_PRESENCE_SHARED_RPC.md)。
- 2026-11-18 续原 NP-03 / TS-07/09 参数读取：普通交易 loader 与预热已接同一
  SharedReadRuntime 的有界只读 scope，原 SDK decoder、lane 顺序及提交权威不变。
  PumpSwap 冷配置合入同次快照，不放宽 pacing。`1792c51` / Actions `44133854054` /
  artifact `14351589608` 独立验收完整并发/串行各 1675，父批 1652 全保留且恰增
  23（含一项隔离 helper）；102 字段拒绝与全部 60 步骤通过。本机检查触及
  2200 MiB 限制后未重试，最终类型检查由本批 CI 通过，不重跑终态或另发文档 CI。
  寻池/模拟/独立及 SDK 内部读取、全局预算与其余原门槛继续，不据此宣称整体完成。
  见 [参数读取边界](optimization/PROTOCOL_PARAMETER_SHARED_READS.md)。
- 依据：本地 `trade_service/` 源码，以及《Sol Bot Project Optimization Implementation Plan V1.1》第 5、11、14、15、20 节。
- 2026-11-18 续原 NP-03 DAMM 候选模拟/签名前 preview：买卖及 legacy/original
  路径接原 runtime，保留原消息/slot/四秒期限及前后 freshness，签名账本不改。
  `26e4b8b` / Actions `44234932246` / artifact `14397918749` 独立验收完整并发
  1897（292.52 秒）/串行 1897（561.63 秒），原 1874 名称全保留、恰增二十三；
  shared/oracle 12/实际候选方法 8/真实 HTTP 3、旧运行合同、52 类型/159 原始
  字段及全部 96 步通过，原字节不变量/精确新增请求/全树身份已核对。报价 probe
  与账户证据链当时未接线，现由上方 903d646 独立验收；不新增任务/分母，不重跑终态或另发文档 CI，未部署，
  见 [候选读取边界](optimization/DAMM_CANDIDATE_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 SDK 原签名和 Quote 提交观测：实际路径接原 runtime，
  保留原 skip/总预算/选定节点/Unknown 和签名，受共享间隔而不扩预算或补单；
  `e81dda4` / Actions `44223334225` / artifact `14392698134` 独立验收完整并发
  1874（292.68 秒）/串行 1874（549.27 秒），原 1850 名称全保留、恰增二十四；
  SDK shared/oracle 12/Quote adapter 6/控制器假 invoke 3/真实 HTTP 3、50 类型/
  153 原始字段及全部 93 步通过，原字节不变量和全树身份已核对。不新增任务/
  分母、不重跑终态或另发文档 CI，未部署，见 [SDK 签名共享读取](optimization/SDK_SIGNATURE_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 独立 Quote 对账状态读取：原前两 RPC 元数据快照共用
  Observation 配额/冷却，保留原 parser/missing 策略及节点完整请求预算。`259b2fe` /
  Actions `44217059338` / artifact `14390716531` 独立验收完整并发 1850（267.93 秒）/
  串行 1850（512.67 秒），原 1831 名称全保留、恰增十九，shared/oracle 14/真实
  HTTP 3/实际 adapter 2、49 类型/150 原始字段及全部 89 步通过。原字节不变量与
  全树身份已核对，不新增任务/分母、不重跑终态或另发文档 CI，未部署，见
  [Quote 状态共享读取](optimization/QUOTE_STATUS_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 WSOL 广播所有权：固定原 RPC 和原默认发送政策，实际
  广播仅接原校验后的借用包。`d1d9038` / Actions `44210742181` / artifact
  `14386593797` 独立验收：完整并发 1831（233.10 秒）/串行 1831（447.86 秒），
  原父批 1817 名称全保留、恰增十四，lane 12/实际 adapter 2、47 类型/144 原始
  字段及全部 86 步通过。原字节不变量、精确请求/回执及全树身份已核对。不新增
  任务/分母、不重跑终态或另发文档 CI，未部署，见 [WSOL 广播所有权](optimization/WSOL_BROADCAST_OWNERSHIP.md)。
- 2026-11-18 续原 NP-03 WSOL 两种模拟：真实 wrapper 使用原共享模拟配额/HTTP/
  冷却，保留授权、快照、原请求参数和错误映射；已签包发送给同一 RPC 的信任
  假设不变。修正源码 `9d40936` / Actions `44204140210` / artifact `14385056342`
  独立验收完整并发/串行各 1817，父批 1799 名称全保留、恰增十八项；138 字段和
  全部 84 步骤通过，原始名称/spans/全树和新旧请求/结果已核对。缺失 err 拦截
  已验证，不改签名广播/账本顺序、不本地重试 Cargo 或做生产操作，见
  [WSOL 模拟读取边界](optimization/WSOL_SIMULATION_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 WSOL 准备读取：真实 execution wallet 通过原 runtime/既有
  五步会话重查准备证据，原授权/金额/临时 signer/签名发送保持；不增加读取类别
  或资源。`cf5e63b` / Actions `44195580476` / artifact `14381545057` 独立验收完整
  并发/串行各 1799，父批 1783 名称全保留、恰增十六项；129 字段与全部 81 步骤
  通过，原始名称/spans/全树和旧 oracle/消息/请求见证已核对。不套用父批或重复
  立项、不本地重试 Cargo 或做生产操作；后续模拟/发送仍保留原 RPC，见
  [WSOL 准备读取边界](optimization/WSOL_PREPARATION_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 WSOL 签名状态读取：源码移除 reader 的执行 RPC 所有权，
  共用原 runtime/Observation 配额，保留单签名/history 和严格 finality/error/slot
  校验。`ec608c0` / Actions `44188954352` / artifact `14377382655` 独立验收完整
  并发/串行各 1783，父批 1769 名称全保留、恰增十四项；123 字段与全部 78 步骤
  通过，原始 spans/名称/全树、原 parser/oracle/请求见证已核对。不重复立项或使用
  父批证据、不本地重试 Cargo 或做生产操作；WSOL 准备/模拟及其他原门槛继续，见
  [WSOL 状态读取边界](optimization/WSOL_STATUS_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 WSOL 只读报价：原首 RPC metadata 接原 runtime 的五步
  有界会话，保留原 native/金额/费用/租金/退款与错误顺序；旧 estimator/构建器
  字节保持。`c196d1b` / Actions `44182843252` / artifact `14374676338` 独立验收
  完整并发/串行各 1769，父批 1749 名称全保留且恰增二十项；120 原始字段拒绝及
  全部 75 步骤通过，报告/请求见证、原始 spans/源码身份/全树已核对。不重跑终态
  或另发文档 CI，不套用父批、不本地重试 Cargo、不动签名提交或生产状态，见
  [WSOL 报价读取边界](optimization/WSOL_QUOTE_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 聚合模拟和余额基线：原 runtime 的一次性借用会话执行
  固定零签名/两步/pair/slot，先验基线后模拟，保留原 Token/CPI/报告和签名提交权威。
  `2f12c97` / Actions `44173368003` / artifact `14371180749` 独立验收完整并发/
  串行各 1749，父批 1729 名称全保留且恰增二十项，114 原始字段拒绝及全部
  72 步骤通过，原始名称/spans/源码身份/全树已核对。不重跑终态或另发文档 CI，
  不套用父批、不本地重试 Cargo 或做生产操作；独立 WSOL/quote 和其余门槛继续，见
  [聚合模拟读取边界](optimization/AGGREGATOR_SIMULATION_SHARED_READS.md)。
- 2026-11-18 续原 NP-03 聚合预检 Mint/ALT 读取：原三条入口共用原 runtime，
  保留第一 Default lane/commitment、owner/ATA/ALT 校验和后续交易权威。新增第十类
  有界只读预算，`10c5b4b` / Actions `44164727015` / artifact `14366377330` 独立
  验收完整并发/串行各 1729，父批 1713 名称全保留且恰增十六项，108 字段及全部
  69 步骤通过，原始 spans/源码身份/全树一致已核对。不重跑终态或另发文档 CI，
  不套用父批、不本地重试 Cargo 或做生产操作；模拟/独立读取和其余原门槛继续，见
  [聚合账户读取边界](optimization/AGGREGATOR_ACCOUNT_SHARED_READS.md)。
- 2026-11-18 接续原 NP-03 两跳寻池：保持原缓存和政策，将既有 CPMM hop 的扫描、
  参数/余额读取接同一 runtime 的专用 scope。`17bc3be` / Actions `44156380200` /
  artifact `14362832442` 独立验收完整并发/串行各 1713，父批 1693 名称全保留且
  恰增二十项；108 字段及全部 66 步骤通过，源码身份/原始 spans/全树一致已核对。
  不套用父批、不本地重试大编译、不重跑终态或另发文档 CI、不做生产操作；
  模拟/独立读取与其余原门槛继续，见
  [两跳读取边界](optimization/SOL_HOP_SHARED_READS.md)。
- 2026-11-18 再续原 NP-03 普通寻池：原算法接只读 SDK adapter 和共享 runtime 的
  独立分支预算，不改 SDK/路由优先级/API/每节点总期限。`cde04bd` / Actions
  `44146511097` / artifact `14359025763` 独立验收：完整并发/串行各 1693，父批
  1675 名称全保留且恰增 18，105 字段及 63 步骤通过。不重跑终态或另发文档 CI，
  不本地重试大编译或做生产操作；两跳/模拟和其余门槛继续，见
  [寻池读取边界](optimization/POOL_DISCOVERY_SHARED_READS.md)。

当前 C3d2 的有界后台扫描/迁移夹具修复及 C3e Default 完整准备离线链路已通过完整 CI，
不将其扩大为其他方向/协议/provider 或生产验收。
原聚合查询/索引/Quote scope 保持，SDK 与聚合共享一个游标、批量上限、runtime 与默认关闭开关；
不恢复 owner、重签/重发或静默修复索引。未部署或启用线上任务，其他 SDK/模式仍 legacy，
真实供应商/生产恢复/发布未验收。NP-03 只复用原资源与窄配置投影，不改变 API/校验/hash/JSON。

NP-03 API 鉴权/普通校验配置边界及五项新增合同已在 cb94bdd/43693519994 完整验收；
前序 449731c/Actions 43691952757 只含默认值/缓存六项，不冒用其结果。
31 个 Handler 鉴权位置与普通校验/执行政策的顺序保持，详见 `optimization/API_CONFIG_BOUNDARIES.md`。

再接续 bootstrap 组装入口：main 仅保留监听/日志/任务启动与停机协调，原两段资源加载/
服务组装移到 bootstrap.rs，原语句字节一致、原句柄/顺序不变。四项新合同与完整两套
回归已在 970d835 验收，详见 `optimization/BOOTSTRAP_ASSEMBLY_BOUNDARY.md`；不把
提取组装当作所有资源隔离或真实 SDK/生产初始化验收。

NP-03 接续 readiness：汇总层只接三值设置/项目钱包诊断 observer，原 SDK probe/Registry/
共享 runtime 留在 SDK adapter，原 observe_health 与 V1 ready 规则不变。六项新增已在
c220ba8 的专项及完整回归通过，不在前序 970d835 的 bootstrap CI，详见
`optimization/READINESS_RESOURCE_BOUNDARY.md`；不因此宣称真实节点或已部署验收。

## 1. 范围与目标

本计划只针对 Trade Service，保留一个服务、一个交易 API 和现有签名权威。

核心目标：

1. 后续升级 `sol-trade-sdk`，改动尽量集中在 SDK adapter、版本清单和回归测试，不要求 Bot 修改协议。
2. 新增协议有固定接入入口、能力矩阵、测试样本和发布流程，不再到处修改大型 `match`。
3. 交易 API、金额语义、钱包身份、持久化幂等和签名安全不因目录迁移或 SDK 升级而变化。
4. 明确区分“代码存在”“构建通过”“模拟通过”“允许实盘”，不把 SDK 支持声明直接当成服务交易能力。

不包含：

- 不修改 `copy_trading`、Telegram UI、跟单策略、限价策略或 Bridge 消息合同。
- 不操作 Copy Trading 的大数据库，不把其备份或重构作为本服务离线开发的前提。
- 不顺带升级 SDK、替换 Router SDK、改变公开 API 或开放新协议实盘。
- 不移动整个仓库、不改服务名、部署路径、钱包加密格式或数据库地址。
- 不修改旁边的 `trade_service-sdk9/` 工作目录。

“便于升级”不等于“升级没有影响”。Solana 类型、交易编码、Token Program、账户布局、手续费和上游 SDK 行为变化，仍可能需要兼容调整及逐协议回归。

### 1.1 2026-11-13 范围调整与恢复顺序

用户先要求暂停 PumpAPI 相关交易对接、继续其他优化，随后要求“记录修改计划，先不动”，当时仅更新文档并暂停任务；随后用户重新激活持续任务。现仅恢复下列非 PumpAPI 队列的源码/离线验证，不恢复 PumpAPI 功能开发，不进行 release 构建或部署。

- 暂停 PumpAPI 交易对接的协议账户证据扩展、完整消息授权、签名/提交接入及新增能力验收。保留 H1 至 H2-B2-C 的代码、测试与历史检查点，不删除、回滚或把未完成阶段记为完成。
- 这里的暂停是开发范围调整，不是停用线上 PumpAPI backend，也不是停止 PumpAPI/ZMQ 数据流；不改执行方式、路由、能力响应、配置或服务状态。
- Pump.fun、PumpSwap 等协议的普通 SDK 交易路径不因名称包含 Pump 而自动归入暂停范围；必须按实际 backend 区分。现有 PumpAPI 兼容合同不因暂停而移除，未来全量离线回归仍可包含其既有测试。
- 通用后端边界与资金安全要求不取消；普通 SDK/legacy 聚合的独立安全工作保留，但不能借此继续实现 PumpAPI，也不能把目录拆分算作资金安全修复。

恢复后的非 PumpAPI 队列如下；局部交付不等于对应整项已完成，最新验证以执行记录为准：

| 顺序 | 任务 | 交付与边界 |
| --- | --- | --- |
| NP-01 | 普通 SDK/legacy 聚合构建、签名、广播与账本缺口复核 | C3c1/C3c2 原受控 SDK 执行、C3d1 原签名恢复、C3d2 扫描/010 及 C3e Default 完整准备串联已离线验收。后继 6c50aca 完整并发/串行各 1437，runtime 5、扫描 16、历史迁移 1、HTTP 200、参数 3 全过；原 005 夹具清理问题已修复，不改生产校验。其余方向/协议/backend/mode/资源/发布门槛继续，不恢复 owner、不伪造 Quote，不把离线通过当实网验收。原参数/hash/JSON/vendor 保持，无部署/生产操作 |
| NP-02 | 剩余非 PumpAPI Handler/Application 分拆 | B/C 普通交易/查询、D Pool/系统、E 全部 Handler/DTO/路由组、F 余额、G 管理以及 H 靓号任务控制已接项目边界；H 新增十三项，最终并发/串行各 984、HTTP 194、check/fmt/diff 通过，旧 C2b 时序测试加门闩/零写审计不改生产实现，见 `optimization/VANITY_APPLICATION_AND_WORKER_BOUNDARIES.md`。直接 worker 已移出 AppState，生成函数原字节保持；后继普通接受 26be717 和执行 26445c5 已验收，不重新排队。剩余资源/生命周期归原 NP-03，不改公开路径、参数、鉴权、原部分失败顺序、存储/密文格式和默认 backend |
| NP-03 | 基础设施与依赖方向收口 | 四个旧资源/具体 params 字段仅测试保留；默认值/缓存/鉴权/校验/bootstrap/readiness、接受/执行及三层门禁、普通任务/Shadow/靓号生命周期已按各自源码验收。十二类读取共用原 runtime/冷却，既有读取逐项保留证据，转账准备 `a5420ba` 验收各1960/171字段/104步。转账确认已接线待 Rust 验收；广播所有权、根具体资源、SDK内部任务、全局预算和真实退出/刷盘门槛继续，不重做已验收子项 |
| NP-04 | 通用 Backend 契约验收 | Record/Tape 独立构建/候选八项合同接现有普通 SOL Quote 控制器与原 SQLite authority，验证金额、能力拒绝、取消、未知和签后不换后端；91b3d94 / Actions 43714527309 已验收，专项 8、完整各 1481，见 optimization/BACKEND_CONSTRUCTION_CONTRACT.md。其他业务路径适用性仍待验，不新增 Router SDK 依赖、生产候选 profile 或 PumpAPI 接入 |
| NP-05 | SDK 来源与后续升级演练准备 | A 已核对固定上游 6.0.2 commit/tree、372/375 文件及既有 DBC 差异 8 修改/3 新增，486 行补丁隔离重放完全一致；只读来源 checker 与八项负例通过并加入后续 CI，见 `optimization/SDK_PROVENANCE.md`。上游 tag/commit 无签名，实际适配/升级与协议演练、release/部署仍有独立门槛，不覆盖 vendor |

PumpAPI 缺口仍计入完整优化与 Live 准入门槛；完成上述非 PumpAPI 队列不等于整体完成。后续实施仍限定 Trade Service，不触碰生产钱包/数据库，不签名广播、不重启服务。默认不提交推送；2026-11-15 用户另外授权仅源码/公开测试数据的 GitHub Actions 与设备登录，允许独立 `ci/trade-service-validation-*` 验证分支，不合并、改写或强推远端 main，不发布凭据/运行数据或部署，见 `optimization/GITHUB_ACTIONS_VALIDATION.md`。

## 2. 本地现状与已知风险

以下保留制定计划时的源码基线，不代表当前文件行数、完成链上验收或核对了上游最新版本。2026-11-14 当前 `main.rs` 394 行、`http.rs` 17 行、`sdk_client.rs` 1863 行；其余进展以本轮审查和执行记录为准。

| 位置 | 当前职责与耦合 | 优化方向 |
| --- | --- | --- |
| `src/http.rs`，约 1,359 行 | 路由、鉴权、请求解析、钱包管理、报价、交易保留及响应处理 | Handler 只负责边界转换，业务编排进入 application |
| `src/sdk_client.rs`，约 3,004 行 | SDK 初始化、RPC 寻池、Quote、买卖、模拟、发送、MEV、转账等 | 逐个职责提取，不一次重写 |
| `src/models.rs` / `validation.rs` | 公开 DTO、协议枚举、执行参数与校验混合 | 保留 API DTO，提取内部领域类型与边界转换 |
| `src/protocol_capabilities.rs` | 静态能力列表，含 live 与 observe 声明 | 校准为有证据的协议注册表，保留现有输出合同 |
| `src/wallets.rs` / `blockhash_cache.rs` | 钱包运行实例和 blockhash 直接依赖 SDK 客户端 | 钱包身份、签名接口与 SDK/RPC 生命周期分离 |
| `src/pumpapi_client.rs` | PumpAPI Local Transaction 路径直接使用 SDK `TradingClient` | 独立构建来源，复用受控签名与发送边界 |
| `src/aggregator_quote.rs` / `conversion.rs` | 聚合报价、交易审计、包装解包各有状态和流程 | 保留独立操作语义，接入清晰的持久化与签名边界 |
| `src/persistence.rs` | 普通交易 durable 保留与重启恢复 | 保持唯一权威，不被内存缓存或新仓储层绕过 |

本地依赖声明为 `sol-trade-sdk = "6.0.2"`，通过 `[patch.crates-io]` 指向 `vendor/sol-trade-sdk`；同时直接使用多项 Solana crates。实际可复现版本必须由 `Cargo.lock`、vendor 来源 revision、内容校验和及本地补丁共同确定，不能只记录版本号。

必须单独处理的 P0 问题：

- 普通 `/v1/trades` 和批量交易共用 durable reservation；独立 `/v1/quotes/sol-buy/:quote_id/submit` 当前不能据此认定具有同样的跨重启保障。
- 转账和 WSOL 兑换具有各自的持久化流程，需要分别核对，不能强行塞进普通 TradeRequest。
- 默认 legacy 下钱包管理/导出仍共用交易凭据；已增加需显式启用的 separate 管理模式，调用方迁移和运行时启用未完成，不能在文件移动时悄悄改授权行为。
- DBC 的 Token-2022 文档与静态能力声明存在待核实差异。
- `ENABLE_SIMULATE` 不是不可绕过的全局禁止广播开关；当前源码没有通用 `TRADE_SUBMISSION_ENABLED`。离线测试不能依赖一个不存在的开关保障安全。

执行更新：已新增 `TRADE_BROADCAST_DISABLED`，约束当前已审查资金入口，并保守拒绝 SDK 完整买卖（含 SDK simulate）；它默认关闭，尚未在线启用，也不等于无密钥/OS 网络沙箱。能力型 RPC/signer ports、真实 SDK 初始化与操作者迁移仍有独立门槛，见 [安全策略验收](optimization/EXECUTION_SAFETY_AND_ADMIN.md)。

## 3. 目标目录与责任

沿用 V1.1 的分层方向；以下是目标结构，不要求一次创建所有文件，也不把空目录算成实现。

```text
trade_service/
  src/
    main.rs
    bootstrap.rs
    config.rs
    api/
      mod.rs
      routes.rs
      dto/
      handlers/
        trades.rs
        wallets.rs
        pools.rs
        quotes.rs
        conversions.rs
        admin.rs
      middleware/
        auth.rs
        request_id.rs
        rate_limit.rs
    application/
      mod.rs
      ports/
      trade_service.rs
      quote_service.rs
      pool_resolver.rs
      wallet_service.rs
      conversion_service.rs
      trade_reconciler.rs
    domain/
      mod.rs
      trade.rs
      wallet.rs
      route.rs
      quote.rs
      protocol.rs
      errors.rs
    adapters/
      mod.rs
      sdk/
        mod.rs
        sol_trade_sdk.rs
        protocol_params/
        router_backend.rs
      pumpapi/
      aggregator/
      rpc/
      signing/
      persistence/
    protocol/
      mod.rs
      registry.rs
      pump/
      pump_amm/
      bonk/
      stonkfun/
      meteora_dbc/
      meteora_damm_v2/
      raydium/
      orca/
      meteora_dlmm/
    infrastructure/
      mod.rs
      blockhash_cache.rs
      param_cache.rs
      idempotency.rs
      persistence.rs
      metrics.rs
  tests/
    contracts/
    fixtures/
    recovery/
    sdk_compat/
  docs/
  vendor/sol-trade-sdk/
```

实际编写集成测试时，明确 Rust 测试入口：必要时增加 `src/lib.rs` 或显式测试 target；目录本身不会自动成为 Cargo 测试 target。

### 3.1 依赖方向

```text
API -> Application -> Domain
                 -> Ports
Adapters implement Ports
Protocol policies -> Domain
Bootstrap wires adapters, infrastructure and services
```

- Domain 不依赖 Axum、SQLx、具体 SDK、RPC 客户端或私钥对象。
- API 不访问 `TradingClient`、`DexParamEnum`、SDK pool params 或钱包 Keypair。
- Application 决定校验、保留订单、签名授权、执行与状态更新的顺序。
- Adapter 负责外部实现与内部类型的转换，不决定绕过幂等或授权。
- `bootstrap.rs` 作为组装入口，集中创建实例，避免 adapter 反向引用整个 `AppState`。

### 3.2 避免重复抽象

- `protocol/` 放协议身份、别名、能力与校验策略；SDK 专有参数构造留在 `adapters/sdk/protocol_params/`。
- `adapters/persistence/` 实现仓储接口；`infrastructure/persistence.rs` 只负责连接、迁移和健康检查。durable reservation 不能两处各做一套。
- blockhash、路由和参数缓存由明确的接口访问；私钥和 SDK 客户端不得作为通用缓存值对外暴露。
- Router backend 仅在探针进入适配阶段时创建，默认关闭，不以这次重构替换现有后端。
- 现有 Pool 枚举可先保留。中心化的一处穷尽映射是可接受的，不为追求“零 match”引入动态插件或弱类型配置系统。

### 3.3 2026-11-15 设计补充的核对与归并

本节是现有任务的边界和验收补充，不新增任务编号、第二套计划或完成度分母。
仅吸收 Parser 职责、SDK 兼容门面、资源所有权、能力证据及 RouterLeg 设计方向；
不引入参考项目的语言/运行依赖，不因文档补充恢复持续源码目标或 PumpAPI 工作。

| 设计点 | 本地已存在的基础 | 归入原任务 | 仅补充的边界与验收 |
| --- | --- | --- | --- |
| DEX Parser 职责划分 | `adapters/sdk/protocol_params/` 集中参数加载与成员检查；已有合成 loader、raw account 与 DAMM 账户证据 | TS-03/05、NP-01/03 | 区分 RPC 取得账户、纯解码、协议政策、SDK 参数映射；按实际调用渐进整理，不新增全协议 Parser 框架。既有 `TradeAddressParser` 仅作地址语法解析，不能记为 DEX 账户解析完成 |
| SDK 兼容门面 | `sdk_client.rs` 已委托多个 `adapters/sdk/` 实现，原 sender/journal 边界已局部验收 | TS-04/07/08、NP-01/03 | 按真实调用方逐项迁移，再保留必要兼容委托；SDK 参数与指令类型不进入 Domain/Application。不按文件大小验收，不为对称增加 Client/signing/submission 包装层 |
| 资源所有权 | Bootstrap 已交接原 Arcs/refresher；参数缓存与八类只读 runtime 已复用原实例 | TS-04/09、NP-03 | 在原资源记录中标明创建者、共享者、任务启动/取消及退出 owner；复用既有释放/失败测试，缺口再补。Infrastructure 管生命周期，adapter 管转换；不新建缓存、连接池、worker 或持久化权威 |
| 能力证据 | `protocol/registry.rs` 为唯一身份/旧政策来源，V1 capability 只是兼容投影 | TS-03/07/09/11、NP-01/04/05 | 证据绑定 backend、协议/程序与布局版本、方向、Quote/Token 扩展、直接/多跳、模式和源码/测试身份；标明未支持/未验证范围。不加另一套 registry，不靠旧 live/builder 布尔值授权 |
| RouterLeg 设计方向 | 4.3 已保留项目意图、候选/验证路线、受控未签名计划与后端合同 | TS-02/04/05/07、NP-04 | 仅保留逐跳输入输出资产、原始整数金额、池/程序身份和 min_out/费用约束的表达需求。raw AccountMeta/Instruction/SDK 类型留在 adapter；不创建 RouterLeg 实现、Router SDK 依赖或多跳发送通道 |

归并后的验收规则，不单独扩大功能范围：

1. **账户解析不等于路线授权。** Parser 接收已有账户快照与必要上下文，不持 RPC、钱包、
   signer 或发送能力；纯解码与项目协议校验可分层实现，不重复 vendor decoder。
   错 owner/discriminator/布局、截断数据、错误 Mint/方向/vault/权限及 Token 扩展按适用范围
   有拒绝证据；active/migration、slot/时效由验证流程核对。`PoolRouteHint` 不升级为可信路线。
2. **迁移按行为而非目录验收。** 旧参数、错误/顺序、费用、hash/JSON、幂等与恢复合同保持；
   新旧实现对照使用既有 fixture/oracle，不能重写期待值接纳差异。未迁移入口明确保留，
   不以 facade 存在宣称所有 backend/方向已隔离。
3. **一项操作只有原来的持久化权威。** 生命周期收口不合并钱包、兑换、普通交易数据库，
   不复制 reservation/journal。SDK 专属参数缓存仍在适配边界，通用层不泄漏 SDK 类型。
4. **证据按范围准入。** 解析/寻池/报价/构建/模拟证据与服务级受控签名/发送准入分开；
   版本或绑定上下文改变须重验，来源不可用/未知/过期时不给新增许可。旧 V1 字段及现有
   准入规则不随文档改变，不能以若干全局布尔值替代证据，也不要求本轮补完全部协议实盘。
5. **逐跳描述不能绕过整笔审计。** 后续实际接 Router 时仍须核对最终不可变交易的资产、
   输入预算/min_out、signer/权限、费用/小费、临时账户创建/关闭与 ALT/交易版本；统一经过
   原授权、保留、签名字节账本和固定 sender。签名后不换路线、blockhash 或后端重签，
   不开放任意交易签名。该条只补设计约束，不授权现在实现 Router。

已有详细依据继续复用 [参数映射](optimization/SDK_PROTOCOL_PARAMS.md)、
[Registry](optimization/PROTOCOL_REGISTRY.md)、[Bootstrap](optimization/BOOTSTRAP_ASSEMBLY_BOUNDARY.md)、
[缓存所有权](optimization/PARAM_CACHE_RESOURCE_BOUNDARY.md) 和 [共享 RPC](optimization/SHARED_READ_RPC_RUNTIME.md)。
Copy/Bridge 的边界仍按 4.4，SDK 来源/补丁/隔离演练仍按第 5 节；本补充不重复立项，
也不修改 API、金额/钱包语义、数据库、依赖/vendor、配置、默认 backend 或生产状态。

#### 本地核对结果与剩余验收

下表补充本节的源码证据，不创建另一份任务清单。附件对参考项目的描述仅作设计输入，
不是本轮已重新审计其远端实现或最新版本的结论。不得把目录存在、局部测试或文档归并
当作全部设计已实现。

| 设计点 | 当前源码证据 | 尚未完成的原任务范围 |
| --- | --- | --- |
| Parser | `src/adapters/sdk/protocol_params/mod.rs` 仍直接接 `RpcClient`，集中加载、协议检查及 SDK 参数转换；`checks.rs`、`quote_tokens.rs` 已拆出纯检查 | NP-01/03 内按实际读取路径分离已有账户快照的解码/检查与 RPC；仍不可将参数 loader 称为独立纯 Parser，不复制 vendor 解码或新增第二套发现缓存 |
| SDK 门面 | `src/sdk_client.rs` 已委托 `adapters/sdk/` 多个实现；`application/ordinary_runtime.rs` 经项目 ports 执行，26445c5 已验收 | 兼容门面尚未收完：根 Client 仍有 SDK 初始化、寻池及聚合模拟实现。按调用迁移并保留旧 oracle，不另造 Client；三层选定类型门禁不证明所有 SDK 能力均已隔离 |
| 资源所有权 | `src/bootstrap.rs` 交接原资源；`optimization/APP_STATE_RESOURCE_OWNERSHIP.md` 已列创建/共享/退出责任；接受与执行使用原权威；普通后台任务与 Shadow writer 生命周期分别在 c6f2745、796ba50 验收 | 原任务追踪/关闸/drain 和 Shadow FIFO 排空不再排队；`src/infrastructure/` 已有任务与退出编排，不能沿用附件中“缺少该目录”的旧判断。NP-03 继续根具体资源、SDK 内部/独立操作任务与剩余 RPC/cache 治理，不重复建立 runtime、连接池、cache、reservation 或 journal |
| 能力证据 | `src/protocol/registry.rs` 是唯一身份/旧政策表；`protocol_capabilities.rs` 只投影旧 V1；`domain/sdk_candidate.rs` 与 `sdk_buy_candidate.rs` 分别定义 SellV1/BuyV1，原 journal 通过 `SdkPreparedRecord` 严格分派；DAMM 受控买入链路在 882e514 独立验收 | NP-01/04/05 按 backend/版本/方向/Quote/Token/模式/路线绑定证据。DAMM Durable Live 直接买卖的局部验收不覆盖其他协议/模式或真实节点；Parser/Builder/V1 live 声明不新增实盘许可 |
| 通用后端 / RouterLeg | `application/ports/ordinary_sdk_submission.rs` 已有后端私有 Candidate/Signed 类型与原 journal；其原 First/Second 测试仍共用 Engine。后继普通 SOL Quote 控制器的 Record/Tape 独立构建/候选八项合同已在 91b3d94 验收 | 本批共同合同不重做；其他业务路径的适用性和真实 Router/SDK 仍须独立证据，不用原两包装测试冒充独立构建。不为测试泛化扩大生产 profile；RouterLeg 仅保留本节与 4.3 的设计方向 |

推进时沿用 `EXECUTION_STATUS.md` 的 NP-01 当前续作点及 NP-03/04/05 原队列；设计归并本身仅核对和校准记录，
不把相同缺口分别计入“新设计项目”和“旧优化任务”。完整当前状态只看
`optimization/EXECUTION_STATUS.md`，历史终态通过/失败不重新等待、执行或计数。

#### 原 NP-01 资金安全续作：DAMM 买入输出

本项是原“其他方向”缺口的源码核实，不新增任务、协议或执行许可。前序核实到
`adapters/sdk/ordinary_params.rs::ordinary_buy_params` 对 DAMM 固定给出
`fixed_output_token_amount=Some(1)`；固定 SDK 的 exact-in/partial-fill 分支直接将其
编码为 `minimum_amount_out`，不能据此宣称买入已具有完整滑点保护。现有可信输出证据、
候选 profile 和原 owner 受控签名链主要覆盖直接卖出，不自动覆盖买入；未核对线上二进制。

后继按下列顺序复用原 NP-01 的准备、报价和签名边界，不先改常量或扩大 profile：

1. 用公开夹具固定当前 SDK 买入 ABI、两侧 Mint/方向与整数单位；将只读报价探针的
   `min_out=1` 与正式交易最小输出明确区分，不能因 probe 成功就授权正式消息。
2. 可信买入报价须绑定原钱包、池/程序/布局、输入输出资产、精确输入量、Token Program、
   费用、blockhash、slot 与期限；SOL 输入须覆盖包装/账户准备和费用预留，不能仅反转
   卖出 Mint 或假定钱包已有 WSOL。沿用已有 reader/证据流程，失败、过期或不支持则拒绝。
3. 用已验证的本笔净输出和整数滑点计算正式 min_out，绑定最终不可变候选及原 owner 的
   签名字节账本/固定 sender；仅有报价数值不构成签名许可，不降低其他方向既有门槛。
4. 不广播测试覆盖 WSOL/USDC、两侧方向、精度/舍入/零输出/溢出、Token 扩展/费用、
   账户/余额不足、超时/取消及上下文变化；完整回归仅用获准源码 CI。真实节点、上线与
   资金验收独立授权，公开 API/hash/JSON 和 SDK/vendor 不变。

此优先级先于继续纯目录整理；Parser 职责、SDK 门面与能力证据在实际买入链路中逐步吸收，
不为设计对称另造 Parser 框架、Client、registry 或资金权威。

上述第 1 步的 ABI 原型已在 `48f15b3` / Actions `43725621186` / artifact `14166866773`
独立验收：七项专项、完整并发 1522（147.65 秒）/串行 1522（258.80 秒），51 步骤
exit=0；原始测试名增量恰为七项、87 字段拒绝及源身份已核对，不重跑旧任务。
当批 `adapters/sdk/damm_probe/buy_plan.rs` 为 `cfg(test)` 独立名义类型，不接生产准备。
72 组真实 SDK 指令对照覆盖两侧方向、WSOL/USDC 及 SOL 别名、目标 Token/Token-2022、
sysvar 和三个原始整数输入（含 u64::MAX）；未充值测试身份只用于 builder 的公钥读取。
其余合同检查精确充值/sync_native/不关闭原 WSOL、USDC 单位、拒绝边界、上下文绑定、
与原预持币 probe 的区别，以及 SDK 的滑点字段不会代替 fixed_output 编码。

这是 ABI 与能力边界证据，不是余额、租金、完整费用、池状态/扩展验证或可信报价完成。
原型本身的 floor=1 仅用于无签名发现；当批正式普通买入仍为旧实现、未改 Some(1)，
不能宣称已修复买入滑点。后继在同一 NP-01 内补 SOL/WSOL funding 的同 bank
账户取证、动态顶层指令分组/CPI 净输出与可选同次 pre/post balances，再绑定正式 min_out
和最终消息/原 owner；不以现有卖出八账户、两顶层指令假设直接接入买入，不因此推广
新的 Live operation profile、签名许可、API 或 SDK/vendor 改动。

后继当批在同一测试原型增加买入只读取证核心，未接普通准备/签名：同一九账户快照
（含 System 钱包）、有界零签名模拟，复用原纯解码、Token/扩展、CPI 转账与同次 token
balance 校验，不构造假的账户余额或再建 RPC/cache/账本。原卖出继续用 Existing 输入及
原 output-setup=0/swap=1；买入原型单独识别 input-setup、output-setup 与 swap 索引。
本笔 Native SOL 充值与 ATA 初始化由固定消息绑定，租金/fee/钱包支出只能对比同一次
模拟的 pre/post lamports，净输出按该模拟 epoch 的 Token-2022 费用计算，不跨 RPC bank 相减。

上述只读取证原型已在 5e0ed8f / Actions 43727995203 / artifact 14167687550 独立验收：
买入专项 23（含新增 16）、完整并发 1538（138.18 秒）/串行 1538（252.38 秒），51 步骤
exit=0；原始名称增量、87 字段拒绝与源身份核对，不重跑旧任务。
同 bank fee/pre/post native 与 token balances 必须完整；节点不支持时拒绝，不回退常量 1。
该验收快照保守拒绝未同步 WSOL、预充值未初始化 ATA，不能据此宣称覆盖这些状态；
也不提供最终计算费/小费政策、全消息授权、原 owner 账本或真实节点能力验收。
完整执行准入仍需最终输入/费用/消息绑定及原 owner，不能直接把只读净输出数值塞入
opaque SDK 并称完整授权。当批 API、普通买入 Some(1) 与 SDK/vendor 未改。

后继在同一 NP-01 内补上述兼容：按固定 SDK 的 sync_native/ATA 初始化语义，允许规范
ATA 在读取时为非执行、空数据的 System 账户，余额不伪装成 Token 数量；USDC 输入仍须
已有真实 Token 余额。同步前的多余 WSOL lamports 和初始化后的 native 初始量，只从同次
模拟的 pre lamports/pre token balances 与已核验 post reserve 计算，不使用早期 RPC 差额。
租金只记实际补差，超出租金的预充值保留在原 ATA；USDC→WSOL 输出不额外包装输入，也
不把未同步的输出 lamports 冒充交换所得 Token。余额倒挂、未完整同步、信用溢出、额外支出
与不完整证据继续拒绝。共用转账 Parser 仅提取转账/初始化，资金校验提供命名信用量，
旧卖出显式使用零信用量/原布局，未放宽其准入或导出新的业务 API。

此兼容已在 fca54ff / Actions 43730145561 / artifact 14168537736 独立验收：新增十项、
买入专项 33、完整并发 1548（138.47 秒）/串行 1548（251.95 秒），全部 51 步骤 exit=0，
原始名称增量、87 字段拒绝及源身份独立核对。其当时仍为测试原型，不冒充正式买入完成。

2026-11-16 当前续原准备子项：只读取证接入原 `LanePreparation` 和 ordinary buy，正式
参数工厂必须显式接收正值 min_out，非 DAMM 保持 None；实际值由本笔净输出/整数滑点
计算，不以旧常量回退。`DammBuyQuote` 冻结原 ExecuteTrade、钱包/provider/V0 模式、
池/程序、原始输入量和 blockhash，复查请求 flags/费用与 3 秒证据期限；SDK 入口前再次
核对，缺失名义证据也拒绝。`CapturedDammOutput` 沿用同一个每次调用的 mutex/atomic，
买卖证据不能互换，不创建新 RPC client、缓存、队列、reservation 或 journal。原 WSOL
买入类型/包装和 USDC 原始单位、V1 hash/JSON/Oracle 保持；Quote 当前 bank 的非零 slot
直接来自同一次九账户读取，不额外查询 getSlot，也不冒充已有余额快照的 slot。

新增十六项合同：绑定十项（含真实只读取证→实际准备/投影→入口复查串联）、当前 bank
两项、准备/拒绝/取消/显式 min 四项；受限 check 13 秒/RSS 1,286,520 KiB、production
check 12 秒/RSS 1,004,892 KiB 通过，原七组警告保持；完整运行已由本批独立 CI 验收。原冻结
参数指纹显式传入历史的 1 验证投影一致，不重写 oracle 迎合差异，也不把此控制样本当
执行许可。节点缺少同次模拟 fee/pre/post lamports/token balances、未审计模式或扩展均拒绝。

本批 `61af682` / Actions `43734656743` / artifact `14170038159` 已下载独立核对：
买入专项 49，完整并发 1564（152.42 秒）/串行 1564（268.72 秒）零失败/忽略/过滤，
全部 51 步骤通过。两套名称相同、父批 1548 保留，增量恰为上述十六项；原始 compiler
spans 确认 87 字段拒绝、原源码/恢复成功。identity 与 manifest/lock/vendor manifest
匹配，审计绑定 1039 文件/12,820,838 字节、19 路径，不重跑终态 CI 或另发文档 CI。

本子项只加固旧已许可 SDK 买入的准备参数，不完成上方第 3 步或扩大 Live/operation profile。
SDK 入场后的 worker/时效、最终不可变消息/完整费用和权限授权、原 owner 的买入签名字节
账本/固定 sender、共享 RPC 全局治理、真实供应商字段能力/生产恢复/发布仍须继续。
不因本批提前拆出准备保护而删除、降低这些门槛，亦不把 SDK simulate 称全程无 signer。
源码未部署或触碰生产数据，不表示已核对/修复线上版本。

2026-11-16 恢复后续同一 NP-01：买入 `DammBuyMessage` 已接原 lane 准备、实际请求入口
复查及 SDK protocol/full 回调，私有候选保存完整零签名交易，核对充值/ATA、池/程序、
原始金额/min_out、权限、计算费、小费目的地与顺序，再独立对照编译字节。回调 clone
共用单次状态，失败、重复、取消或原报价过期均拒绝；返回签名须绑定冻结消息，异常
保留所有已知签名，不换 lane。模拟分支复用原只读 RPC/quote slot，不进入 SDK signer。

原卖出的费用封装、规范编译和 provider 小费表按原行为移入 `damm_instructions/message.rs`
供两侧复用，保留旧测试入口/指纹，不另建 Client/Registry/执行通道。新费用预留只读取
同一次已核验报价模拟的 wallet post lamports：原报价已扣充值、实际租金和基础费，正式
固定 envelope 还需 300 lamports 优先费及按原 SDK 规则取整的小费；不足/溢出为终态
准备拒绝，不据此换 provider。不会用更早账户 RPC 的钱包余额冒充同 bank 预留。

新增十九项已由本批源码 CI 实际运行，含 64 组真实 SDK 原始指令及独立整笔编译对照、
资金边界、ATA 创建冻结、无签名模拟、错误/取消/过期、回调顺序/并发/权限/费用和签名
保留。两种买入专属 adapter 类型加入现有三层类型门禁，93 字段拒绝（含外部别名）
已独立读原始 compiler spans 核对；原源码/恢复检查成功。
其中一项串到原普通执行编排，核对模拟结果不带交易签名、不查询/广播；旧 SDK 模拟
曾生成非广播签名，本子路径移除 signer 后现有可空 signature 返回 null，不伪造签名维持旧值。
暂停前的夹具 wallet 变量遮蔽已修正，不删除用例或放宽 oracle。
最终受限 test check 11 秒/RSS 1,387,596 KiB、production check 5 秒/RSS 601,988 KiB
通过，原七组生产警告保持；Python 门禁十项/workflow 七项与格式通过。本地不链接
大型测试，以上元数据检查不是本批 Rust 运行或真实供应商验收。

独立验收快照 `ec1d2c9` / Actions `43773807696` / artifact `14185728930` 已终态 success，
日志下载核对：买入专项 68，完整并发 1583（155.62 秒）/串行 1583（274.91 秒）零失败/
忽略/过滤，全部 51 步骤通过。原始名称集合相同、父批 1564 全保留，增量恰为上述十九项；
identity/manifest/lock/vendor manifest 匹配，发布审计绑定 1044 文件/12,875,578 字节、
21 路径。真实 main/索引/冻结历史不变，不重跑终态 CI 或为验收记录另发文档 CI。

此子项不是完整买入授权/账本完成：最后 SDK 回调与实际签名/发送之间仍由旧 worker
处理；原 owner、prepared/signed 原字节持久化/固定 sender 及恢复、全局 RPC 与真实节点
门槛继续按原队列推进，不新增 nominal operation profile 或生产许可，也不扩展暂停的 PumpAPI。

下一步仍在本项内，按本地实际依赖接线，不另立计划：

- `domain/sdk_candidate.rs` 当前只有 SellV1 及卖出比例/余额乘法校验；买入需明确的名义
  类型和原始 Quote 数量/native 预算，不能伪造卖出比例或放宽旧校验。旧 SellV1 的
  JSON、消息摘要及拒绝夹具保持，新类型也不能仅凭反序列化恢复签名许可。本轮已接
  `SdkPreparedRecord` 共同类型/严格 codec，具体买入执行授权仍按下述剩余边界推进。
- `application/ordinary_claims.rs`、`sdk_preparation.rs` 当前 scope/origin 只接受直接卖出；
  必须由完整买入意图及原 admission/owner 绑定接入，不能先扩大 claim 再补构建授权。
- 原 persistence 的候选校验/签名字节读回及 `replay_sdk_observation` 也绑定卖出；后续买入
  元数据与历史 V1 request/hash 需按方向验证，未知 prepared/signing 状态不重新取得 signer。
  010 索引本身未限制方向，但 `ordinary_observation_scan.rs::SDK_PAGE_SQL` 明确筛选
  `t.side='sell'`；买入只读扫描必须与验证器共同接线并保持旧分页/索引与预算合同，
  不能因复用同一表就宣称已自动恢复买入。
- `OriginalSdkSender::send_packet` 的 provider 调用当前固定 `TradeType::Sell`；买入接入时
  必须同时冻结并核验真实方向，不能只复用类型名或在发送前临时推断。仍从原 client
  取得同一个 keyless sender，保留原签名字节校验、唯一发送占用与只读恢复，不新增通道。

上述是对原 owner 接线剩余范围的源码核对，不代表本批已开放买入 profile/签名/发送，
也不授权修改生产库或重提任何订单。完整故障/方向拒绝和旧兼容回归须独立验收。

2026-11-16 后继先落实同一 NP-01 的记录基础：新增项目自有 `DammBuyCandidateRecord`
及 `damm-v2-buy-v1` 数据标签，和旧 SellV1 名义记录严格区分；未修改旧
`domain/sdk_candidate.rs` 的字段、序列化或校验。买入记录没有 sell_percent/伪造持仓比例，
保存真实输入/净报价/min_out、账户/程序/精度、Quote slots、原模式/MEV/确认设置、
同次模拟 native before/deposit/rent/base fee/remaining、正式优先费/小费与消息 SHA。

记录已进入原 `DammBuyMessage::prepare/validate_full`：先沿原只读证据生成，纯重构核对
规范 ATA/程序、WSOL/USDC 单位、完整指令与消息；再与原运行时 capture 逐字段比较。
名义数据能自洽、能读回并不证明原 RPC 事实/新鲜性，不能由记录制造 Quote、owner 或
签名许可。过期记录仍只能作历史数据；当批序列化往返方法仅在测试中启用，未接库。
provider 名称映射按原字面值提取为买卖共用纯函数，不新增 provider/发送实例。

新增十一项合同：真实候选矩阵往返、同 bank 资金、USDC→WSOL、23 组消息字段篡改、
资金/slot/min_out/溢出拒绝、名义记录与原证据分离、过期不能续期、买卖解码隔离与
未知版本/字段、消息/JSON 体积、2^53+1/u64::MAX 整数与模式/MEV/确认记录及 provider
原名称合同。受限 test check 11 秒/RSS 1,377,028 KiB，production check 18 秒/
RSS 1,056,900 KiB 通过，原七组警告保持；格式及 Python 门禁十项/workflow 七项通过。
独立源码验收 `19f80cb` / Actions `43777721549` / artifact `14187333877` 已终态 success，
日志下载核对：完整并发 1594（140.68 秒）/串行 1594（257.60 秒）零失败/忽略/过滤，
专项 79、原源码/恢复成功、原始 spans 的 93 字段拒绝及全部 51 步骤通过。原始名称集合
相同，父批 1583 全保留，增量恰为上述十一项；identity/manifest/lock/vendor manifest
匹配，审计绑定 1047 文件/12,913,244 字节、16 路径，旧 SellV1/Application/claim/journal/
sender/迁移与真实 main/索引/冻结历史保持。终态 CI 不重跑，记录更新不另发文档 CI。

本步不是原订单 admission/claim、prepared/signed 落盘或发送/恢复接线完成；原
`SdkCandidateMetadata`/claim/sender/扫描仍按旧卖出范围，下一步需把独立买入记录接回
原单一 journal 与 origin/permit、固定方向 sender 和只读恢复，不改旧 SellV1 或伪造
卖出数据复用。暂停的 PumpAPI、SDK 版本、API/hash/JSON、库结构及生产状态均不动。

2026-11-16 继续同项：共同 `SdkPreparedRecord` 已接原 backend/journal、StoredSdkCandidate
和线性 permit 的类型接口，严格按 version/profile 分派到旧 SellV1 或独立 BuyV1 codec，
没有 enum JSON 外壳、宽松解码回退或假卖出比例。旧 `domain/sdk_candidate.rs` 原字节
保持；买入 codec 被共同 decoder 使用，但原 claim/origin、具体重构/签名及恢复明确
限制 SellV1。prepare/signing 的错误方向在 SQL 之前拒绝，不扩大原 owner/Live 范围。
新增十一项合同和真实买入矩阵的共同 codec 检查，最终受限 test check 10 秒/
RSS 1,319,604 KiB、production check 13 秒/RSS 1,004,024 KiB 通过，原七组警告。
独立 `2fe436d` / Actions `43781625542` / artifact `14189296618` 已终态 success，
日志下载核对：完整并发 1605（142.55 秒）/串行 1605（261.47 秒）零失败/忽略/过滤，
名称相同、父批 1594 全保留且增量恰为十一项。共同记录专项 9、买入 79、原源码/
恢复成功、原始 spans 的 93 字段拒绝及全部 52 步骤通过。identity/manifest/lock/
vendor manifest 匹配，审计 1052 文件/12,938,550 字节、35 路径，真实 main/索引/
冻结历史保持。不重跑终态 CI，也不为验收记录另发文档 CI。详见
[原账本记录边界](optimization/SDK_PREPARED_RECORD_BOUNDARY.md)。下步继续原买入
owner/origin、受控签名/落盘、固定方向 sender 与恢复/扫描接线，不另建 authority。

2026-11-16 后继同项落实原 owner 接线的校验前置：共同 adapter 按原 record 方向重构
整笔消息，再验有界 canonical 签名字节、单 signer/payer/钱包签名与原回执。原卖出
signed 验证实现提取复用，卖出专用入口仍拒绝买入；原 journal 行/签名回读使用共同
校验器，不仅验证 JSON/hash。原 admission 的买入意图绑定已支持 Native SOL/WSOL 与
既有 USDC 的整数输入/资产/钱包、路线、模式、费用/MEV，但原 claim classifier 保持
卖出范围，纯绑定和历史签名都不授予 owner/permit。

原 V1 历史回放按记录方向分派，显式金额复用同一个旧 parser；仅把默认金额提供者
变为惰性闭包，正常验证仍在原位置转换当前默认，历史省略的 WSOL 数量使用原 raw
整数，不经 f64 或 Config，USDC 仍须显式金额。请求/hash 编码不改。原 ownerless
capture/finality 已能核对买入历史的原 parent/key/hash/签名/上下文；没有 signer、发送
或新 authority，原自动扫描仍排除买入。旧“损坏买入行关联卖出”合同只更新 nominal
重构能力断言，保留全部原 parent/owner/读回拒绝和零变更断言。

新增十六项真实构建/历史临时 key/原意图/V1 精度/临时库与 finality 合同；最终受限
test check 11 秒/RSS 1,365,552 KiB、production check 5 秒/RSS 672,372 KiB 通过，
原七组警告。独立 `f029d55` / Actions `43785820055` / artifact `14190956041` 已终态
success，日志下载核对：完整并发 1621（120.77 秒）/串行 1621（227.60 秒）零失败/
忽略/过滤，两套名称相同、父批 1605 全保留且增量恰为十六项。新专项 18、共同记录
9、买入 79、原源码/恢复 Clippy 成功、原始 spans 的 93 字段拒绝及全部 53 步骤通过。
identity/manifest/lock/vendor manifest 匹配，发布审计 1059 文件/12,989,654 字节、
30 路径，真实 main/索引/冻结历史保持。终态 CI 不重跑，后续验收记录不另发文档 CI。
不把合成历史行/签名当作受控买入 signer 或生产恢复验收。详见
[买入历史证据](optimization/SDK_BUY_HISTORY_EVIDENCE.md)。
后继继续受控买入候选/signer、原 owner 的 CAS/签名字节落盘、固定 Buy sender/runtime
与 claim/扫描共同接线；不先扩大 claim，不由历史记录恢复执行许可或重签/重发。

2026-11-17 同项后继：原不可变买入候选、live 整笔零签名预览、线性 permit 受控签名
及私有签后 artifact 已接同一个 `OriginalDammBackend`、journal 与固定 Buy sender。
只有原 Durable Live SDK 直接 DAMM 的正整数 Native SOL/WSOL 或既有 USDC claim
同步准入，同一个 SDK 索引页包含 buy/sell；未改 DDL、旧 SellV1、hash/API 或生产配置。
十七项新增合同已由 `882e514` / Actions `44109233896` / artifact `14340653473`
独立验收：完整并发 1638（149.17 秒）/串行 1638（275.68 秒），父批 1621 全保留，
新增名称恰为十七项；99 字段拒绝及全部 56 步骤通过，identity/manifest/lock/vendor
manifest 匹配。不用历史签名恢复 owner，不扩大到其他协议/模式；
原高层买入 SDK worker 不参与该新子路径。详见 [原执行接线](optimization/SDK_ORIGINAL_DAMM_RUNTIME.md)。
后继未完成范围仍为其他协议/模式的完整授权/账本、共享 RPC、资源/依赖边界及真实
环境/迁移/回滚，不由本批局部接线宣称整体优化完成。

## 4. 稳定类型和后端边界

### 4.1 外部 DTO 与内部模型分离

现有 API DTO 迁入 `api/dto/` 时保留字段、别名、默认值、可空行为、枚举字符串和 HTTP 语义。内部逐步使用：

| 类型 | 必须表达的含义 |
| --- | --- |
| `WalletRef` | 稳定 wallet_id 和已校验公钥，不携带私钥 |
| `RouteHint` | 候选协议、池、Mint；尚未验证，不能当成已批准路由 |
| `VerifiedRoute` | 已核实池 owner、两侧 Mint、Token Program、方向、上下文 slot 和验证时效 |
| `TokenAmount` | Mint、原始整数单位、decimals；SOL 与自定义 Quote 不能混用 |
| `TradeIntent` | 钱包、方向、输入资产、输出资产、数量、滑点、MEV、模拟模式及幂等身份 |
| `Quote` | 输入量、预计输出、最小输出、费用估计、路由指纹、来源和有效期 |
| `PreparedTransaction` | 受控的未签名交易表示、消息哈希、上下文和来源 backend，不包含 Keypair |
| `SimulationResult` | 模拟错误、费用/计算量、余额变化、slot 及结果可信程度 |
| `SubmitResult` | 请求身份、签名和提交结果，不把 submitted 等同 confirmed |

公开接口现有浮点金额不直接删除。边界处按已冻结的舍入规则转换为整数，并对 NaN、无穷、负数、溢出及互斥金额字段做回归验证。

内部公钥、哈希和交易类型的选择须显式审查：可使用项目拥有的验证型包装和规范序列化字节；链类型只在边界转换一次。不能把 SDK 类型改名后继续泄漏，也不能为“解耦”在每层重复序列化。

### 4.2 后端接口分阶段建立

先围绕实际调用提取 resolve、quote、prepare/build、simulate、submit 能力接口，再组合现有 SDK Backend。接口采用何种 trait/async 实现由当前 Rust 工具链与真实 SDK 生命周期决定，不照抄示意签名。

关键约束：

1. 不要求所有后端都有相同 Quote 算法；能力明确区分原生报价、模拟推导报价、聚合器报价和不支持。
2. 若 SDK 只有内部构建并发送的组合入口，先封装成受控执行入口，标记无法独立 build；没有证据前不声称已分离模拟和发送。
3. 所有可能广播的入口必须经过 Application 的授权及持久化保留。SDK 内部发送同样算广播，不能只检查直接 RPC 调用。
4. 原 `execution_method=sdk/pumpapi` 的语义保持不变。增加内部 backend 不能自动改变公开枚举或默认执行路径。
5. 不通过普通 Handler 暴露 signer；签名适配器核对 payer、允许的指令、金额、费用、目标账户和绑定的请求。
6. 新旧 backend 对比只做离线或不广播模拟，禁止为了比较结果同时发送两笔交易。

### 4.3 通用扩展边界与验收补充

2026-11-13 补充：先完成本优化，Router SDK 后接。不新增 SDK/Router 依赖、不部署链上程序、不替换已可用的直连路径，也不为预留接口建立空实现。以下并入既有任务，不改变当前资金安全工作包的优先级。

| 所属任务 | 必须保留的通用边界 | 验收证据 |
| --- | --- | --- |
| TS-02/04 | 业务意图、整数金额、钱包身份和执行结果归项目所有；第三方 SDK 类型仅在适配边界转换 | 项目类型/fake port 测试；业务/API 不引入 DexParamEnum、Keypair 或 SDK transaction |
| TS-03/05 | 寻路、报价、链上验证和执行分开；路线能表达各跳池/Mint/Token Program、时效、来源及验证证据 | 单跳/多跳项目 fixture，候选 RouteHint 不得冒充 VerifiedRoute；不以此新增交易能力 |
| TS-03/07 | 按后端/协议/Quote/方向/模拟/构建能力开放操作，不因存在 SDK builder 就放开 Live | 支持与不支持矩阵及拒绝测试；现有公开 capability 合同与新内部证据分开处理 |
| TS-S1/04/07 | 新后端提供受控未签名计划，统一经过意图审计、授权、幂等、签名证据落盘和广播；不开放任意交易签名 | fake 构建后端接入同一编排，校验/写失败不得发送；不能绕过 journal。旧 SDK 组合入口先按 4.2 显式保留能力限制，后续安全任务处理，不谎称已拆分 |
| TS-07/09 | 后端选择/回退仅限签名前，签名后不自动更换路线、blockhash 或后端；结果不明仅查原签名 | 不确定结果/取消/恢复测试证明无换后端重签或重复广播；各操作仍只有一个幂等权威 |

完整验收时，以两种无 RPC、无真实 signer 的假构建后端运行相同的业务合同，验证扩展不依赖具体 SDK。TS-09 增加依赖方向检查，明确兼容 re-export 和尚未迁移入口的清单，不能通过白名单掩盖新增类型泄漏。当前已有 fake execution/runtime 测试仅是局部证据，不表示这两项整体门槛已经完成。

路由和构建模型按实际调用逐步补齐，不能仅为未来 Router 复制一套缓存、订单或 reservation；不得借预留多跳字段改变公共 JSON、旧 fingerprint、数据库格式或默认后端。Router 的版本冲突、链上部署与具体协议能力留作后续独立工作包，本补充不授权升级、网络调用、实盘或发布。

### 4.4 外部消费者兼容边界

Copy Trading 的 HTTP 对接、Bridge -> Copy 的事件边界，以及 Trade Service 内 PumpAPI 构建 backend 是不同责任。已有三服务规则集中见 [外部服务兼容清单](optimization/SERVICE_INTEGRATION_BOUNDARIES.md)：保持钱包身份、金额单位、旧 DTO/状态、原 key/hash/JSON 和调用方恢复约束；事件/候选池不等于已授权路线。关联身份不作为新增必填字段，管理凭据迁移与 Bridge schema/回放仍单独验收。本轮只明确约束，不修改 Copy/Bridge 或恢复 PumpAPI 功能开发。

## 5. SDK 升级机制

### 5.1 版本与本地补丁账本

规划新增 `docs/sdk/`，记录当前可复现版本、升级检查单、兼容矩阵及本地补丁清单。账本至少包含：

- SDK 版本、上游 URL/revision、vendor 内容校验和、Cargo.lock 哈希、Rust 工具链、目标平台。
- 每项本地补丁的文件、原因、关联故障、测试、上游是否已合并、保留或移除结论。
- Solana 相关依赖版本及重复主版本情况，包括 pubkey、transaction、RPC、Token 接口和序列化边界。
- 支持协议、Quote/Token Program、发送通道和模拟能力的测试证据。

当前 manifest 的版本字符串不能代替这些记录。不得直接覆盖 vendor 后凭“编译通过”认定补丁仍然有效。

### 5.2 每次升级的固定流程

1. 固定当前工作树和可运行二进制身份，识别未提交改动；不要把整个脏工作树当成纯 SDK 升级。
2. 审阅目标版本 changelog、源码差异、依赖变化、账户布局及默认手续费/发送策略变化。
3. 在隔离开发目录或明确分支试升；保留生产 SDK、运行二进制和钱包数据。
4. 逐条处理补丁账本：上游已修复则验证后移除，未修复则最小重放，不盲目复制旧实现。
5. 只更新适配层、必要的依赖与对应测试。若需要改 Domain、API 或数据库，停止当作“普通 SDK 升级”，另做兼容设计。
6. 跑编译、API 合同、协议 fixture、金额/报价、MEV/费用、幂等恢复和不广播模拟。
7. 对比新旧行为：池选择、账户列表、方向、单位、min_out、费用、Token Program、模拟余额变化和失败分类。
8. 差异需解释并分类；动态 blockhash、签名、slot 等不能要求逐字节相同，也不能因此忽略实质差异。
9. 生成升级报告和可回滚制品；通过准入后先受限部署，再进行单独授权的小额实盘验收。

准入失败则保持原版运行，不为了赶升级放宽 Quote、owner、min_out 或模拟校验。

### 5.3 编译与 CI 计划

待任务实施时使用以下基本检查，本规划阶段不执行构建：

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked
cargo build --release --locked -j 2
```

离线环境仅在依赖完整时加 `--offline`。规划 CI 在无生产密钥的环境运行合同与 fixture 测试、SDK 类型边界检查及构建，输出带 commit、lockfile、SDK revision 和哈希的制品；当前不能假设仓库已有合用的 workflow。

不得把 `.env`、钱包库、私钥、口令、生产日志或敏感备份上传到 Git/CI。CI 制品能否运行还取决于目标架构、系统动态库和构建环境，下载后须核验哈希与兼容性。

## 6. 协议接入机制

### 6.1 一个注册入口，多维度能力

注册表是协议身份与能力的单一来源，API 输出、参数校验和 backend 选择使用同一份定义。协议的可用性至少包含：

- protocol_id、历史别名、Program ID、池布局版本。
- Token Program、Transfer Fee/Hook 等扩展支持情况。
- Quote Mint、买卖方向、ExactIn/ExactOut、单跳/多跳、独立 quote/build/simulate 能力。
- SDK builder 存在、服务适配完成、fixture 通过、模拟通过、允许 Live 的独立证据。
- backend、功能开关、已验收版本、已知限制和失败原因。

旧 `/v1/capabilities` 字段保持兼容；更细的内部状态映射到现有合同。新字段或 schema 变化单独做消费者兼容检查，不偷偷改既有 `live/observe` 的含义。

### 6.2 新增协议的标准工作包

| 步骤 | 交付物 | 不通过时 |
| --- | --- | --- |
| P01 身份与账户 | 官方 Program ID、owner、discriminator、账户布局、迁移状态样本 | 仅观察，不构建 |
| P02 资产与方向 | 两侧 Mint、decimals、Token Program、输入输出方向与余额校验 | 拒绝不明 Quote，不默认替换为 WSOL |
| P03 Resolve | 从 hint 得到链上 VerifiedRoute，包含超时、缓存和失效行为 | 查询失败与确无池分开表达 |
| P04 Quote | 可解释的输入量、费用、输出、min_out 和过期条件 | 无可信输出则不允许伪造 min_out=1 开放 Live |
| P05 Build | SDK 参数映射、必需账户、ATA、wSOL、ALT、计算预算 | 不靠修改全局白名单绕过协议限制 |
| P06 Simulate | 独立买/卖样本，余额差额与失败分支 | 标为未验收，不扩大能力 |
| P07 发布 | 协议开关、批准范围、授权 canary、回滚记录 | 保持旧协议和旧默认路径 |

新增协议的正常改动范围应是：一个协议政策模块、一个 SDK 参数映射、注册表条目、fixtures、验收文档。确有新业务语义再扩 Domain/接口；不得承诺所有协议只需加一行注册。

SOL 买入与池的 Quote 支持分开验收。支持 USDC/custom Quote 直买，不代表具备 SOL 路由；多跳需另验输入预算、路由时效、原子性、滑点分配和失败结果。未知协议默认关闭。

## 7. P0 安全与持久化边界

目录迁移与行为修复分别审查，不在一次大提交里混做。

| 项目 | Trade Service 内的工作 | 验收 |
| --- | --- | --- |
| 普通/批量交易 | 保留现有 reservation，覆盖 SDK、PumpAPI、普通交易内聚合路径 | 同键同请求返回原结果，同键冲突拒绝，保留先于副作用 |
| 独立聚合器提交 | 为 quote-bound 操作建立等效 durable 保留，绑定钱包、报价、模式和请求指纹 | 并发、超时、重启、广播后写库失败不产生无证据的第二笔交易 |
| 转账/兑换 | 保留各自操作模型与账本，复用签名安全与恢复规则 | 不互相复用不兼容的幂等键或回滚语义 |
| 链上对账 | 只读查询签名和确认程度，区别提交、确认、最终确认与 unknown | 查询不能触发补单；不为超时订单生成新 key |
| 管理权限 | trade/admin 权限设计、导出隔离、调用方迁移清单 | 不变更现有 token 后直接切断调用方；交易凭据不能授权私钥导出 |
| 模拟隔离 | fake signer、mock RPC/transport、网络隔离 | 测试无法触达主网广播，即使 simulate 标志误传也不会产生实盘 |

SDK 升级或后端切换不得改变请求指纹的历史语义。若原持久化 hash 格式必须升级，需兼容旧记录并做冲突回归，不通过更换 hash 算法绕过已有幂等记录。

## 8. 性能与运行保障

- 升级前记录初始化时间、RPC 调用数、429 比例、寻池/报价/构建/提交 P50/P95 和缓存命中率；使用固定负载与相同测试条件比较。
- 后端接口不为每次交易重新创建钱包客户端或完整 RPC/MEV 通道。异步安全的共享资源与按钱包隔离的 signer 生命周期分别管理。
- 全局限流、失败切换和并发预算由 RPC adapter 统一执行，不让每个协议各自无限重试。
- 缓存键包含必要的协议、池、Mint 对、Token Program、数据布局/后端版本；配置或 SDK 升级后防止误用过期参数。
- 不以性能优化删除 durable 写入、owner 校验或可信报价；同步写入开销先测量，再优化事务与索引。
- 进程启动、就绪和可交易是不同状态。健康接口不应依赖单次无界 RPC；实际调整必须保留 readiness 的真实性及兼容行为。

## 9. 实施任务与依赖

以下为任务基线；实际实施与验收状态以 `optimization/EXECUTION_STATUS.md` 为准。已有测试可作为起点，但不自动视作本计划任务完成。

| ID | 依赖 | 任务 | 交付与验收 |
| --- | --- | --- | --- |
| TS-00 | 无 | 冻结本地基线、SDK/补丁/路由清单 | 版本与运行身份、旧 API fixtures、问题清单 |
| TS-01 | TS-00 | 补行为、API、故障恢复基线测试 | 普通/批量/独立报价/钱包/兑换各有合同；无真实广播 |
| TS-S1 | TS-01 | 独立聚合器持久化、状态对账与签名入口审计 | 先保留后副作用，unknown 不盲目重发；作为新 Live 准入门槛 |
| TS-S2 | TS-00 | 管理权限及模拟隔离设计和分阶段实施 | 单独变更窗口，不随文件移动改凭据；未授权导出被拒绝 |
| TS-02 | TS-01 | 提取 domain、API DTO 转换与 ports | 金额、别名、默认值及序列化合同不变 |
| TS-03 | TS-02 | 校准协议能力并提取 Registry | 包含 DBC no-hook 差异证据；未验收能力关闭 |
| TS-04 | TS-02 | 隔离钱包运行实例、RPC、blockhash 与 signer | API/application 不再依赖具体 SDK 客户端；不改钱包存储 |
| TS-05 | TS-03、TS-04 | 提取 Pool Resolver 与参数映射 | owner、资产、超时、fallback、缓存回归 |
| TS-06 | TS-05 | 提取 Quote 与金额单位计算 | 原始单位、费用、min_out、过期与多跳边界回归 |
| TS-07 | TS-06、TS-S1 的提交边界 | 提取现有 SDK/PumpAPI/聚合 adapter | 能力差异明确，构建/模拟/发送边界可测 |
| TS-08 | TS-07 | Application 编排与 API Handler 分拆 | 新路径通过合同后再移除旧入口，不双广播 |
| TS-09 | TS-08 | 整理 infrastructure、可观测性与依赖规则 | 无重复 reservation，实现与接口依赖方向可检查 |
| TS-10 | TS-09 | 选一个 SDK 版本做隔离升级演练 | 补丁账本、新旧差异报告、完整回归，不默认上线 |
| TS-11 | TS-03、TS-07 | 选一个未开放协议完成接入演练 | 标准工作包可复用；只到模拟，Live 另需批准 |
| TS-12 | 所有相关验收 | 构建、受控部署和回滚演练 | 制品身份、配置、数据库兼容和运行证据齐全 |

执行规则：一次完成一个责任边界；每项记录“源码完成/离线通过/模拟通过/已部署”，不只写“完成”。外部备份或 RPC 条件不足时继续可离线的合同与 fixture 工作，不绕过上线门槛。

## 10. 测试、发布与回滚

最低测试矩阵：

- API 合同：字段/别名/默认值、状态码、鉴权、错误脱敏、钱包 ID 与公钥映射。
- 协议：买卖方向、支持的 Quote/Token Program、迁移池、无池、错误 owner、过期缓存、无余额和 dust。
- 金额：整数精度、滑点、最小输出、小费、交易费、账户租金和多跳总支出边界。
- Provider：已配置的 MEV/SWQoS 通道、默认策略、模拟手续费策略与实际发送策略的差异。
- 恢复：重复/冲突、DB 写失败、响应丢失、进程崩溃、重启、签名已保存但确认未知。
- 升级差异：相同 fixtures 在旧/新 SDK 上的协议与费用语义比较，不连接生产钱包。

发布前备份范围是本服务实际启用的钱包库、交易库、兑换库及其恢复材料，分别记录一致快照；密钥与加密归档分开安全保管。Copy Trading 数据不纳入本计划操作范围。

回滚原则：

1. 优先回退兼容的二进制和配置，保留已经写入的订单、幂等键和签名。
2. 不把旧交易数据库快照覆盖回生产以“撤销部署”，这可能丢失已广播记录并造成重复交易。
3. 数据库变更采用可兼容的增量迁移；破坏性迁移须独立方案、停写窗口和恢复核对。
4. 每次只部署一个有证据的变化，协议开关与 SDK 版本不同时大范围切换。
5. 所有实际下单、密钥导出/轮换、网络规则改变和生产库替换需要明确授权。

## 11. 工期与第一批任务

在一名熟悉项目的开发者、构建空间与 RPC 条件可用的前提下，初步估算：

| 里程碑 | 范围 | 工作日估算 |
| --- | --- | --- |
| 第一批 | TS-00/01 与 TS-02 起步，形成合同、版本账本和内部 DTO | 3-5 |
| 主体适配与模块化 | TS-03 至 TS-09，穿插提交安全及权限整改 | 10-18 |
| 演练与发布准备 | TS-10、TS-12，升级差异、制品和回滚 | 4-7 |
| 新协议 | TS-11 及后续每个协议 | 通常 3-7 起，复杂 CLMM/DLMM/Hook 另估 |

前三项约 17-30 个工作日，不是工期承诺，也不含外部等待、全部协议扩展或长时间主网观察。本计划包含 V1.1 Phase 4 之外的 SDK 隔离、升级机制和安全验收，因此不能直接套用该阶段单独的 6-10 天估算。

下一轮最先执行 TS-00/TS-01：

1. 列出现有路由、DTO、SDK 公共调用入口、vendor 补丁来源和持久化入口。
2. 建立 API/金额/协议行为样本及不广播测试入口。
3. 完成第一组内部 DTO 与旧 DTO 的映射设计，再开始最小提取。
4. 第一个交付目标是“可验证地隔离一个边界”，不是一次建齐目录，也不是同时追最新 SDK。

本文件定义仅针对 Trade Service 的实施边界与任务。源码修改、测试与部署状态分别记录，不以计划存在或局部试点通过宣称整体完成。
