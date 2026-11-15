# Trade Service 独立优化与可升级架构实施计划

- 版本：V1.0
- 日期：2026-11-12
- 状态：2026-11-15 目标 active，PumpAPI 开发暂停。C3d1 已在源码接 SDK 原签名显式只读 context/历史 replay/finality 与迟到 receipt 保护，新增 18 项仅受限 check，完整运行待新 CI；自动扫描/索引/worker 尚未扩至 SDK，按 C3d2 继续。最近已验收 C3c2 `fd1c7cb` / Actions `43685088919` / artifact `14150028402` 下载核对，完整并发 1398（132.54 秒）/串行 1398（235.13 秒）全部通过，不覆盖本批恢复用例。未部署/实网验收，其他方向/协议、资源/升级/发布门槛继续。SDK 固定 6.0.2，不操作生产数据，详见 [执行状态](optimization/EXECUTION_STATUS.md)、[SDK 恢复](optimization/SDK_ORIGINAL_SIGNATURE_RECOVERY.md) 与 [CI 证据](optimization/GITHUB_ACTIONS_VALIDATION.md)。
- 依据：本地 `trade_service/` 源码，以及《Sol Bot Project Optimization Implementation Plan V1.1》第 5、11、14、15、20 节。

当前 C3d1 在 C3c1/C3c2 已验收执行链路上补 SDK 显式只读恢复与最终性核心，18 项新用例待 CI，不把父提交 1398 当本批验收。旧聚合的 Quote/scope/阶段过滤保持，SDK 恢复不创建新 owner 或交易，也不因缺证据而重签/重发。下一 C3d2 是有界索引扫描和 worker 接线，不能宣称当前已有 SDK 自动恢复。其他 SDK/模式仍 legacy，真实准备 RPC/供应商/发布未验收。保持原参数/hash/JSON，不升级/部署或接触生产数据。

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
| NP-01 | 普通 SDK/legacy 聚合构建、签名、广播与账本缺口复核 | C3c1/C3c2 新 Durable Live DAMM SDK 卖出原 owner/受控 signer/原字节账本/固定 sender 与四项串联已验收，完整并发/串行各 1398、sender 14。C3d1 显式只读恢复/历史 replay/finality 和迟到 receipt 保护已有源码，18 项仅受限 check，运行待 CI；索引/归并扫描/worker 接线按 C3d2 继续。其余 SDK/模式仍 legacy，全方向/协议/backend/mode/资源/发布门槛未完成，不恢复 owner、不伪造 Quote，不把离线通过当实网验收。原参数/hash/JSON/vendor 保持，无部署/生产操作 |
| NP-02 | 剩余非 PumpAPI Handler/Application 分拆 | B/C 普通交易/查询、D Pool/系统、E 全部 Handler/DTO/路由组、F 余额、G 管理以及 H 靓号任务控制已接项目边界；H 新增十三项，最终并发/串行各 984、HTTP 194、check/fmt/diff 通过，旧 C2b 时序测试加门闩/零写审计不改生产实现，见 `optimization/VANITY_APPLICATION_AND_WORKER_BOUNDARIES.md`。直接 worker 已移出 AppState，生成函数原字节保持；其他资源/接受入口仍待收口，不改公开路径、参数、鉴权、原部分失败顺序、存储/密文格式和默认 backend |
| NP-03 | 基础设施与依赖方向收口 | TS-09-A 四个冗余生产字段仅测试保留；RPC-01A 配置去重，B1/B2/C1/C2/A3c 的刷新/观测/readiness/两个查询余额/准备余额共六类接 main 同一 runtime、独立容量/间隔与共享冷却。准备 reader 只持公开地址/当前 URL/runtime，raw 固定三地址，真实 HTTP 无 SDK 内部重试，流式 cap/最小 slot/时效/取消有测试。原 SDK reader/17 项用例仅 oracle，必要 Arc 注入不改钱包初始化或发送 client。寻池/参数/ATA/独立 quote/WSOL/模拟/SDK 内部读取仍未全覆盖，资源/接受/index/journal/cache 与依赖检查继续，不新增平行框架 |
| NP-04 | 通用 Backend 契约验收 | 用两个无 RPC/无 signer 的假后端验证金额、能力拒绝、取消和不确定结果；不新增 Router SDK 依赖或 PumpAPI 接入 |
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
