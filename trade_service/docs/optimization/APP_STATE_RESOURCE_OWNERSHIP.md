# NP-03 / TS-09-A 生产 AppState 资源所有权

原 NP-03 的生产请求配置已从完整启动配置拆出，验证进度见
[AppState 请求配置](APP_STATE_REQUEST_SETTINGS.md)。本页既有资源与任务交接不变。

日期：2026-11-14。状态：源码与离线验证通过，未部署；不是完整 AppState 或基础设施隔离完成。

后续 NP-01-B3a 已完成下述 Quote 签名边界过期复现与最小修复，见 `QUOTE_SIGNING_FRESHNESS.md`；其并发/串行各 989、HTTP 196 不改本页 986 的历史检查点含义。

## 实际变更

- 逐项核对生产读取点后，将 `AppState.wallets`、`wallet_repository`、`aggregator_quotes`、`conversions` 四个旧句柄限定为 `cfg(test)`。普通生产构建不包含这些字段；测试保留原名称以兼容已有夹具。
- bootstrap 仍创建并注入原 registry/repository/cache/store Arcs，未重新建立运行时、缓存、数据库或幂等权威，未调整服务初始化次序。
- 实际请求通过已接入的项目服务使用原资源；四个冗余字段不再作为生产 AppState 的绕过入口。
- 在此历史检查点，full Config、普通 admission/index/persistence、参数/blockhash cache 和项目服务 ports 仍在 AppState。后继 `APP_STATE_REQUEST_SETTINGS.md` 只拆了 full Config；此项不代表其他具体资源均已隔离。

## 两项新增合同

`src/http_contract_tests/state_resource_tests.rs`：

1. 替换测试专用字段后，实际 HTTP Router 仍从原钱包服务返回原默认钱包；管理入口仍保留原 File 模式拒绝，而非误用替换的数据库仓储。原四资源在服务持有期间存活，释放整个 Router 后 Weak 均不可升级。只用空 registry、临时库与内存 store，不初始化真实 SDK/RPC。
2. 仅给夹具字段填入 ConversionStore 不会启用未配置 journal 的兑换服务；原 503、阶段/错误码/no-store 保持。最后 Router 释放后该测试 store 也释放。

这些测试证明当前组装的原引用和请求权威没有被替换，不证明实际 SDK 加载、生成、生产关停、后台任务或操作系统资源均已验收。普通生产编译另行核对，不能只靠包含测试字段的 test build。

## 验证

```bash
cargo test --locked --offline --quiet state_resource          # 2 passed
cargo test --locked --offline --quiet                         # 986 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 986 passed
cargo test --locked --offline --quiet http_contract_tests     # 196 passed
cargo check --locked --offline --bin pump-trade-service       # passed
cargo fmt --check                                            # passed
git diff --check                                             # passed
```

本轮 production check 实际观察到五组警告：models/persistence 的兼容重导出，以及 QuoteStore 的 insert/get、WsolQuote 的两个兼容校验方法、旧 SDK wallet_balance 未使用。后三组所涉实现没有在本阶段修改，未用 allow 或删方法掩盖；历史检查点记录的“两组”不能替代本轮实际输出。全套 test build 没有新增警告。

对照前阶段，wallets、wallet_repository、executor、ordinary_execution adapter、vanity worker、wallet_ops、PumpAPI 实现、Cargo.toml/lock、现有 release 二进制 SHA256 均未变化。只修改 main 字段暴露、测试接线/新增测试及本轮记录；未升级 SDK/vendor、构建 release、接触生产配置/钱包/数据库、联网或重启。

## 续作

- NP-01 继续最终可签消息授权、普通原签名只读对账、其他 backend/存储模式的唯一 signed journal；不在现有普通 GET 中悄悄加 RPC，也不自动重签或重发。
- 当前共享聚合签名适配器没有在取出待签消息时重新检查 Quote 有效期；授权期间的异步等待可能使旧检查过期。先补无 signer 的失败复现，再实施最小局部安全修复，不扩大到暂停的 PumpAPI。
- NP-03 后续继续资源/接受入口、依赖方向与兼容桥核查；真实运行验收独立授权。

持续目标保持 active，文档更新不暂停。PumpAPI 交易对接仍暂停，不停止 ZMQ，不改线上配置。

## 2026-11-15 原资源所有权核对

本节落实主计划 3.3 的资源所有权补充，归入原 NP-03/TS-04/09，不创建新任务或资源。
依据当前 `bootstrap::initialize/complete`、`main`、`AppState::trade_intake/trade_queries`
和普通 Handler；只作源码核对，不能替代真实初始化或生产退出测试。

| 资源 | 创建/共享位置 | 当前退出责任与已有证据 | 尚缺的范围 |
| --- | --- | --- | --- |
| 钱包 Registry/Repository | initialize 创建原实例，SDK/钱包服务共享原 Arcs；main 留日志句柄 | 原资源替换/Weak 释放与 bootstrap 合成组装合同 | 真实 SDK 客户端/密钥内存清理、钱包原子激活和后台任务全部释放 |
| SharedReadRuntime | initialize 创建一次，现有十二类读取及转账准备/确认使用同一 Arc | 转账准备复用原 WsolQuote 配额，`a5420ba` 独立验收；确认复用原 Observation，待验收 | 转账广播所有权、SDK opaque 内部/全服务配额与生产门槛，不重做已验收读取 |
| BlockhashCache/Refresher | 原缓存由 refresher、readiness、AppState 持有，main 接原 refresher owner | 后继普通任务生命周期将原资源保留到 drain 完成/超时后才停止和 await；c6f2745 独立专项与完整各 1498 验收，不借用旧生命周期证据 | 链上有效期/真实节点、异常进程退出与 SDK 内部等其他任务 |
| ParamCache | complete 创建一次，PoolQueryService 与 executor writer 共享 | 同实例写入/保留键、最后服务释放后回收，见 PARAM_CACHE_RESOURCE_BOUNDARY.md | 缓存版本/全局一致性与 SDK 内部参数缓存 |
| TradeStore/IdempotencyStore/TradePersistence | initialize 打开原持久化，complete 创建原内存资源；接受/查询/claims/执行共享 | 接受与普通执行服务借用原实例/项目 ports；26be717、26445c5 已验收，原 reservation/hash/JSON 和文件恢复合同继续适用；普通任务与 Shadow 生命周期见下两行 | 根具体资源隔离、全部 backend/mode 的 signed journal 及生产刷盘 |
| QuoteStore/ConversionStore | complete 创建原报价缓存，initialize 按配置打开可选 conversion store | 原服务持有和 bootstrap Some/None/Arc 交接合同 | 不合并独立操作账本；真实节点/恢复与发布仍独立验收 |
| ObservationPlan/Worker | complete 只 prepare，main 成功 bind 后按原开关 start | 普通 drain 之后 request/await 原 worker；另一资源报错也完成 join；c6f2745 四项 helper 合同和全套已通过 | 真实部署调度、断电与全系统 RPC 预算 |
| 普通交易后台任务 | bootstrap 创建一个共享 TradeTasks；原 Handler 接受 await 前登记，Created 移交原 executor | 线性 ticket 只记录生命周期；17 项新合同与完整各 1498 在 c6f2745 验收，原 ledger/owner 不变 | SDK opaque worker、其他独立操作/靓号任务与真实进程退出尚未覆盖 |
| 原 Shadow writer | 原 TradePersistence 创建单个 1024 项 FIFO，私有组件持有原 pool/handle；main 留原 persistence Arc | 普通 drain 和原 refresher/observer join 后关闭/等待原队列；796ba50 的专项 13/4、完整各 1515 验收，超时/取消保留同一 handle，无重放 | 仅原 SQL 返回及 FIFO 完成证据，不是 fsync/断电、全部生产者停机或新增 durable authority；不重复实施 |
| 原靓号任务 | 原 WalletOps 保留唯一 semaphore/map，私有句柄 owner；Bootstrap 把同一 Arc 交原服务和 main | 仓储 await 前登记，信号关闸/原取消标记并行 join；超时/取消不丢句柄。3d72a14 / Actions44245588737独立验收十三新合同/完整各1939/101步 | 原生成/import 主函数保持；不是原子取消/加载回滚、完整 SDK supervisor 或生产恢复；见 VANITY_TASK_LIFECYCLE.md |

最初核对时仅有 `axum::serve(...).with_graceful_shutdown`，不能证明脱离 HTTP 的任务完成。
后继本批源码接入原 NP-03 的普通任务登记/有界等待，但不把它当全服务/SDK worker supervisor，
也不释放 owner 或换 key/路线/backend 重签。新关闭窗口和参数的实际边界见
`ORDINARY_TRADE_TASK_LIFECYCLE.md`；17 项新测试已由 c6f2745 独立 CI 验收，不改线上配置或服务状态。

## SDK 生命周期边界

这是原 NP-03 余项的源码核对，与 NP-01 的签名前授权/原字节账本共用既有验收，
不新增一套任务跟踪器，不调整 SDK 6.0.2。

| 边界 | 现有所有权 | 下一项证明 |
| --- | --- | --- |
| 应用任务 | `TradeTasks` 记录 admission 和 executor，`service_shutdown::finish` 等待它们，然后关闭原读取和 Shadow 资源 | 已验收的 drain 不重做，不把执行 future 已返回当成 provider 已完成 |
| Opaque SDK 买卖 | `lane_execution` 只将一个已选 sender 交给 SDK；`async_executor.rs` 创建进程级队列/runtime。Job 自持 payer、指令和 sender。五秒结果等待到期可早于 worker 完成；`wait_for_all_submits` 也有界 | 使用阻塞的离线 sender 见证应用调用结束后工作仍可能存在，且外层不换 lane、重签或重发。不添加虚假的 cancel/join；实现上优先继续代替 opaque 签发入口 |
| 受控 DAMM Durable Live | 原 journal/owner 读回绑定签名字节后走受控 sender；不是 SDK 全量买卖入口 | 保留既有原字节和未知结果合同；不把它声称为所有协议、存储模式或 signer 都已覆盖 |
| Provider 连接 | Temporal ping/QUIC driver 与 Astralane ping 拥有各自 SDK 句柄、停机或 Drop 行为。应用没有通用的 await/join 权限 | 独立核对连接保活的生命周期和耗时，不能从 Registry/Router 的 Weak 释放推导后台任务已停止。供应商送达及真实关停仍需单独授权 |

就地复用原 owner、测试 transport 与 no-fallback 合同。若固定 SDK 没有任务完成或取消接口，
保留未验证状态，不能通过扩大超时或改文案冒充退出完成。未运行真钱包、节点或交易。

后继源码补两项离线见证，由下述资源批独立验收。测试使用固定 SDK 的
`execute_parallel` 与生产的 lane 编排、结果分类和 `TradeTasks::drain`；跳过
高层协议构建，合成包只进入内存。分别在原发送器进入后取消 caller，以及让 SDK
自然等待超时并获得应用层 clean drain。两条路径都要求原发送器仍未完成、释放后
仅完成一次，且无备用 lane/观测/重发；第二条仍为 `Unknown`，没有凭空补入签名。
此证据是行为基线，不是生命周期修复，也不代表 SDK/provider 提供 cancel 或 join。

## AppState 的只读 Blockhash

根请求状态只接原 Application `CachedBlockhashRead`，不再持有
具体 `BlockhashCache` 类型。Bootstrap 将同一个 Arc 转成只读端口给 ordinary
executor，原 readiness adapter 和 refresher 继续用原对象；不分配第二个 cache，
不改 TTL、刷新过程、时钟、槽位或读取/拒绝顺序。保留原组装指针和释放断言。
新增离线见证使用一个门控 hash：更新前两处均见空，原刷新后执行端口能取得该 hash、
实际 readiness 聚合读到 freshness；只读端口仍持有原分配，最后释放后可回收。
此处未改变 TradeStore、journal、idempotency 或 RPC 冷却；见下述验收。

## 后台执行不持有 HTTP 状态

普通订单在原 ticket 上派发时只移动原 journal/index/幂等、只读 Blockhash、两种执行端口、
参数 writer 和复制的执行策略，不克隆完整 `AppState`。HTTP 认证、钱包准入目录、
报价和管理服务不随后台任务延长生命周期；实际执行立即使用原对象配置和原任务登记。
原拒绝/重复处理、wallet identity lease、claim、HTTP 等待、存储与关停顺序保持。

新增真实 HTTP + Durable 夹具：执行端口阻塞后释放 Router，确认认证投影和准入目录已回收，
同时原 journal 仍记录 Submitting、等待中的 ticket 和原幂等/index 仍存活；释放后任务
只完成一次，原 journal 转为 Submitted，随后原资源可释放。使用纯内存 fixture，
不调用 SDK、供应商或真实交易，不声称覆盖 SDK 进程级 worker 的退出。

## 关停句柄独立交接

Bootstrap 将同一任务组和同一持久化实例交给 `OrdinaryShutdown`；main 不再通过
`AppState` 获取这两个句柄。信号只携带关闸动作，原 helper 继续先等待已登记
的工作，后停止刷新/观察、排空 Shadow；预算起点、超时和错误优先级保持。
新 Bootstrap 测试释放请求状态后，验证关闸作用于原任务组，原未完成票据仍计入
退出报告，并且原 refresher 和 Shadow writer 均由同一个关停句柄收尾。
只使用离线夹具和内存数据库；不声称已完成生产落盘或 SDK 全局任务退出。

本批 `65ef56e` / [Actions 44280875076](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44280875076)
及 artifact `14419816862` 独立核对通过：并发1989（385.91秒）、串行1989（735.10秒），
原1984名称全保留、恰增五项；208项 HTTP、113步、59类型/180原始字段通过。
首批缺少的测试入口已仅用 `cfg(test)` 转接恢复；原生产执行和关停顺序不变。
SDK 的两项见证只证明既有 worker 行为，不把应用 drain 称为 SDK 发送取消。未部署。

## 普通交易存储权限

生产 `AppState` 不再暴露具体 journal、索引或幂等字段。组装入口把同一组三个
Arc 交给存储适配器，准入、查询和执行各用已有应用接口；后台任务只保留这组三个
原句柄与其执行端口。关停仍接收 Bootstrap 的原句柄，不经过 Router。
不新增连接池、缓存或幂等权威。测试的替换夹具构造相同的借用视图，不保留两份
可静默分叉的引用。Memory/Shadow 仍读原索引，Durable 仍读原持久化记录；
claim 的身份与 CAS、SQL、JSON 和状态更新逻辑完全保留。

新增两项离线见证：三种模式下用新接口取得 claim、从旧入口确认不能再占用，
比较读取来源和写回原记录；并验证替换测试持久化实例不改变既有快照。
两种存储适配类型不允许进入 Application、Domain、Protocol，编译门禁已验收。

首批 `7f6f3c7` / [Actions 44287408599](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44287408599)
停在 Rust 类型检查：`OrdinaryExecutionStorage::SdkJournal` 隐藏后未声明 `Send`，
后台持有该句柄跨 await，因此生产和测试编译均报 `E0277`。修正只声明同一
句柄需要 `Send`，不改接口实现、SQL、任务或断言；首批终态不重跑。

## 请求只登记任务

HTTP 只持有 `TradeTaskAdmission`，不能直接关闭或排空任务组；Bootstrap 给
`OrdinaryShutdown` 的仍是原 `TradeTasks`。登记复用原流程，已获得的票据仍在线性
交接路径内。所有拒绝顺序、关闭后的 503、读取路径、排空状态和超时保持不变。
新测试验证两份登记句柄与原关闸共享状态、最后的原票据仍持有该实例；
原关停交接和 HTTP 生命周期测试也经新入口。未部署。

`1d45d25` / [Actions 44289227980](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44289227980)
首次运行 success，artifact `14421994464` 下载独立核对：并发1992（384.69秒）/
串行1992（733.56秒），原1989名称全保留、恰增三项；HTTP208、115有序步骤、
62类型/189字段拒绝、原源码与依赖身份通过。修正相对首批只有原关联类型的
`Send` 声明和记录；原任务体、十三个 claim 夹具调用和同一存储权威保持。

## SDK Configuration Boundary Follow-Up

Independently accepted NP-03 continuation0530bed / Actions44363743712 /
artifact14457878477 after accepted f519207. The two identical config
builders in `SdkTradeClient::new` now delegate to one small SDK-adapter helper.
RPC/provider order, backup policy, payer parsing, both original
`TradingClient::new` calls and Registry startup/add/remove behavior remain
unchanged. WSOL startup and seed optimization remain disabled, with the same
SDK defaults/log/MEV/commitment values. This is configuration projection only,
not shared infrastructure, a new cache, a client initializer or a lifecycle fix.

Three Rust contracts compare the exact old builder for24 settings,
check complete infrastructure identity including auth/region/custom URL/min-tip/
transport/order/commitment/core/MEV flags, and keep per-client log/min-tip/version
settings separate from infrastructure identity. Tests do not initialize SDK
clients, RPCs, wallets or tasks. SDK TradeConfig/InfrastructureConfig are added
to the existing three-layer forbidden-type gate: all68 types/207 raw fields
rejected in the actual compiler CI. Two related public-factory contracts are recorded
under existing NP-05 in SDK_PROVENANCE.md, not a new protocol/task.

Matching raw artifact/source audits passed full concurrent2070 (394.39s) and
serial2070 (749.17s), original2065 names retained plus exactly these five,
config3/surface2/Pump56/HTTP208 and127 ordered zero-exit steps. Original complete
initializer/Provider/wallet order reconstructs; four changed-config policy
negatives reject. This does not prove actual infrastructure reuse or cleanup.

The pinned SDK constructor still creates infrastructure per wallet/RPC/provider
combination and starts a rent updater without exposing its handle. Public
`from_infrastructure` resets some client fields and does not reproduce
`new`'s global logging/clock/INSTANCE effects. The SDK INSTANCE can retain one
payer; only its definition, not any service/SDK caller, was found by a full
source search. This is not proof of key-memory cleanup. Reuse must also handle
partial provider initialization: caching an incomplete SDK result could prevent
later wallet additions from recovering a provider, unlike today's fresh init.

Do not copy the SDK initializer, use a hash string as full configuration
identity, merge different credential/transport/MEV contexts, or claim that an
Arc drop cancels its background tasks. Any later actual reuse needs Registry
ownership, complete config equality, wallet/payer isolation, unchanged client
flags/provider order and failure/concurrency/lifecycle evidence. No such reuse,
SDK update, production operation or wallet access is performed in this step.

## Provider Configuration Adapter Follow-Up

NP-03 / TS-04/07 continuation after independently accepting the exact0530bed
run/artifact, not a new protocol or task. Provider mapping and service-setting to
SDK-setting conversion have moved from the root SDK facade to
`adapters/sdk/provider_config.rs`. The original lane authorization helper now
uses its sibling adapter rather than calling back into the root facade.
The two original SDK initializers, configuration projection, payer parsing,
RPC/provider order, backup policy, selected-sender checks and execution flow
remain unchanged. No SDK infrastructure is shared or initialized here.

All original converter bodies, provider names, regional aliases, transport
defaults, optional Jito auth and validation/error ordering reconstruct exactly.
Explicit empty auth and custom URLs remain forwarded verbatim; this extraction
does not silently tighten configuration validation or normalize credentials.
Default RPC append and REQUIRE_MEV_SUBMIT checks remain in the original
initializer. The SDK supports more providers than this service exposes; that
does not open new public choices or alter API/fees/MEV/fallback policy.

Five new unexecuted Rust contracts cover512 subset/global-flag combinations,
exact six-provider fields and order, disabled invalid values, earliest enabled
error, token-before-region-before-transport, all regional aliases and distinct
BlockRazor/Astralane modes, owned verbatim credentials and the eight existing
public choices against the actual DAMM provider guard. They never construct
SDK clients, payers, RPCs or workers. The existing three-layer compiler policy
adds five SDK SWQoS types. The same original NP-03 successor also bounds the
advisory candidate cache, whose concrete type is forbidden in the same gate;
combined expected74 types/225 raw fields and eight new Rust tests await CI.
Cache policy and three contracts are recorded in PARAM_CACHE_RESOURCE_BOUNDARY.md.

Source reconstruction and six rejected policy mutations passed locally, as did
format and policy checks. These do not replace Rust execution. The accepted
parent contains neither provider conversion nor cache-bound tests; the coherent
successor needs independent verification in the same validation branch.
The original Pump permission/journal, provider/ABI/funding, all-mode safety,
actual SDK resource ownership and real release gates remain incomplete.

Read-only follow-up checked the current SharedReadRuntime implementation:
the twelve class slots are independent (Discovery has two, the others one),
and pacing is per class while429 cooldown is shared per canonical credentialed
endpoint. This is not a provider-account or cross-process global budget.
Do not infer that two different RPC keys have independent account quotas, or
replace the reserved class capacity with one semaphore that can starve the
500ms blockhash/readiness paths. Actual global admission remains in the
original NP-03 queue; the candidate metadata bound does not close that gap.

The same read-only review followed WalletRegistry::add and the existing
WalletManagementService ports. Add checks the map before awaiting SDK client
construction, then inserts without a second ownership/activation check; two
same-ID callers can both initialize clients. Create/import and delete also
await repository/runtime steps independently, so a registry-only loading lock
would not bind database enabled state to runtime installation. Cancellation
after SDK initialization begins still has the unowned-worker limitation above.
This is an identified original wallet activation/lifecycle gap, not a completed
fix or authorization to change wallet storage. A later fix must use the existing
repository/runtime ports, preserve existing partial-failure/error contracts,
test competing activation/disable/cancellation, and distinguish registry
ownership from actual SDK task cleanup. No extra reservation/journal or SDK
initializer is introduced by this review; current wallet source is untouched.
