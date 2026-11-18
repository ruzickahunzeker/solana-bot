# Trade Service 本地 IPC（UDS）双入口实施计划

- 版本：V1.0
- 日期：2026-11-16
- 状态：方案已建立；源码尚未修改；未执行 CI、部署或生产启用。
- 基线：`ci/trade-service-validation-20261115-c3b`
- 独立工作分支：`ci/trade-service-validation-local-ipc-20261116`
- 范围：仅 `trade_service/`；保留现有 HTTP API，新增可选 Unix Domain Socket（UDS）本地入口。
- 原则：一个 Trade Service 进程、一次 Bootstrap、一个 AppState、一套 Application、一套交易持久化/幂等/签名权威。

本计划是独立的后续工作包，不自动改变 `optimization/EXECUTION_STATUS.md` 中当前 NP-01/NP-03 的活动任务顺序。是否开始 TS-IPC 源码实施，仍需按项目当前执行队列安排；不因本文档存在而暂停或替换既有任务。

---

## 1. 目标与决策

### 1.1 目标

1. 让同一台 Linux 主机上的多个本地程序可以通过 UDS 调用 Trade Service。
2. 保留当前 TCP HTTP API、路由、请求/响应 JSON、状态码和错误语义。
3. UDS 与 TCP HTTP 共用同一套 Handler、Application、AppState、钱包、SDK、RPC、缓存、持久化和后台任务。
4. 通过连接复用、并发连接管理和基准测试降低本地 IPC 开销。
5. 通过 UDS 文件权限、Linux peer credentials、独立 UDS Token 和路由白名单控制本地调用权限。
6. 默认关闭 UDS；可以单独启用、逐个迁移调用方，并可通过配置回退到原 HTTP。

### 1.2 首期技术选择

采用 **HTTP/1.1 + JSON over Unix Domain Socket**。

选择理由：

- 可以复用现有 Axum Handler 和 JSON DTO，不改变 Bot/API 合同。
- Python、Rust、Go、Shell 等本地调用方均有成熟客户端实现。
- UDS 不经过 TCP/IP 网络栈，减少本机传输开销。
- HTTP Keep-Alive 可复用连接，避免每个请求重新建立连接。
- 先用性能测试确认收益；不预设固定的延迟提升比例。

首期不采用共享内存、自定义二进制协议、gRPC，也不升级 Axum/Tokio。若 HTTP/1.1 over UDS 的实测开销仍是明确瓶颈，再单独评估协议层优化。

### 1.3 非目标

- 不删除或替换 TCP HTTP API。
- 不新增第二个 Trade Service 进程或第二套 Bootstrap。
- 不复制 Application、钱包注册表、SDK Client、RPC Runtime、TradePersistence 或幂等账本。
- 不修改 Copy Trading、Telegram、Bridge 消息合同或其他服务。
- 不升级 `sol-trade-sdk`、Axum、Tokio 或 Solana 依赖作为本任务的附带操作。
- 不修改钱包加密格式、数据库 schema、数据库地址、交易金额语义、请求 hash 或签名格式。
- 不因 UDS 本地属性而自动授予钱包管理、私钥导出、SOL 转账或 WSOL 提交权限。
- 不部署、不重启生产服务、不读取生产钱包/数据库、不签名或广播真实交易。

---

## 2. 当前分支基线

以下是当前分支源码核对结果，实施时仍需在目标提交上复核：

| 位置 | 当前实现 | UDS 改造要求 |
|---|---|---|
| `src/main.rs` | 创建 `TcpListener`，调用 `axum::serve`，使用 `ConnectInfo<SocketAddr>`；统一处理退出与清理 | 并行启动 TCP 与 UDS listener；统一取消、任务排空和资源清理 |
| `src/bootstrap.rs` | `bootstrap::initialize(config)` 创建资源并返回一个 `AppState` | 只调用一次；两个入口共享同一个 State |
| `src/http.rs` | `http::router(state)` 组合现有 API 路由 | 保留原 Router，新增受限的 `local_router(state)` |
| `src/api/routes.rs` | 路由组包含交易、报价、池、钱包、余额、管理、转账、WSOL 等 | 拆出本地可用路由组，管理和高风险操作默认不进入 UDS Router |
| `src/api/handlers/*` | 多数 Handler 直接提取 `ConnectInfo<SocketAddr>` 并调用鉴权函数 | 统一使用可表达 TCP/UDS 的调用来源类型 |
| `src/api/middleware/auth.rs` | `authorize_trade/internal/wallet_admin` 接收 TCP SocketAddr | 保留 TCP 规则，增加 UDS UID/Token 验证，不伪造 loopback 地址 |
| `src/config.rs` | 读取现有 HTTP、鉴权和安全配置 | 增加独立的可选 Local IPC 配置，默认关闭 |
| `src/infrastructure/service_shutdown.rs` | 统一排空原交易任务、停止刷新器/观察 worker 并关闭原持久化后台资源 | 两个 listener 只协调停止接收；原清理流程仍只执行一次 |
| `trade-service-db.service` | 当前服务以 systemd 运行，已有 `UMask=0077` | 经权限审查后配置 RuntimeDirectory/Socket group/mode；不随意改变服务身份 |

当前 `Cargo.toml` 使用 Axum 0.7。Axum 当前服务入口使用 TCP Listener；UDS 需要单独的 Hyper 连接适配层。实现前必须检查 `Cargo.lock` 中实际解析的 Hyper 版本和可用 feature，再决定最小直接依赖，不应凭传递依赖可见就直接在源码中引用。

---

## 3. 目标运行结构

```text
Local Program A ─┐
Local Program B ─┼── UDS: HTTP/1.1 + JSON ─┐
Local Program C ─┘                         │
                                           ├── Shared AppState
Remote/Existing Client ─ TCP HTTP ─────────┘
                                                   │
                                                   ▼
                                             API Handlers
                                                   │
                                                   ▼
                                              Application
                                                   │
                         ┌─────────────────────────┼────────────────────────┐
                         ▼                         ▼                        ▼
                    Domain/Policy             Durable Journal          Query Services
                         │                         │                        │
                         └─────────────────────────┼────────────────────────┘
                                                   ▼
                                       SDK / RPC / Signer Adapters
                                                   │
                                                   ▼
                                             Solana Network
```

硬性约束：

1. `bootstrap::initialize()` 只执行一次。
2. TCP Router 与 UDS Router 使用同一个 `AppState` 的 clone；不得各自重新创建钱包、SDK、RPC、缓存、数据库或 worker。
3. 两个入口调用同一组 Handler/Application，不复制交易业务逻辑。
4. 两个入口最终使用同一 Durable Reservation、同一签名账本、同一 request/idempotency key 语义。
5. 不能因请求从 UDS 进入就跳过 admission、validation、持久化保留、签名授权、广播策略或对账。

---

## 4. 调用来源与鉴权

### 4.1 统一 RequestPeer

新增建议位置：`src/api/middleware/peer.rs`。

内部类型建议表达：

- `Tcp(SocketAddr)`
- `Unix { uid, gid, pid }`

Linux UDS 连接在 accept 时通过 Unix peer credentials 取得真实 UID/GID/PID，并将可信连接身份注入该连接的每个 HTTP Request。TCP 继续使用真实 `ConnectInfo<SocketAddr>`。

禁止：

- 将 UDS 人为伪装成 `127.0.0.1`。
- 信任 `X-Forwarded-For` 或客户端自行提供的 UID/PID header。
- 仅凭 Socket 路径存在就认定请求已授权。
- 仅凭 PID 白名单识别程序；PID 会复用，第一期以 UID/GID 与文件权限为主。

建议通过自定义 RequestParts extractor 或等效的类型安全边界，让 Handler 不再直接依赖 TCP 专属的 `ConnectInfo`。迁移时必须保留当前 TCP 的 IP 检查顺序、Bearer 行为、错误码、错误正文和日志语义。

### 4.2 UDS 双重准入

UDS 请求必须同时满足：

1. Socket 文件系统权限允许连接。
2. `SO_PEERCRED` 返回的 UID 属于配置的允许列表。
3. `Authorization: Bearer <TRADE_UDS_API_TOKEN>` 正确。
4. 请求路由在 UDS allowlist 中。
5. 交易请求继续通过原 Application 准入和广播策略。

UDS Token 必须独立于 `LOCAL_TRADE_API_TOKEN` 和 `WALLET_ADMIN_API_TOKEN`。开启 UDS 却缺少合法 UID 列表或 UDS Token 时，应拒绝启动 UDS，不能退化为匿名访问。

第一期多个可信本地程序可以共享 UDS Token，并通过 UID/GID 限制主机级调用者。若未来需要不同程序拥有不同交易权限，再独立设计每调用方 scope；不能把共享 Token 描述成多租户授权。

### 4.3 配置草案

```dotenv
TRADE_UDS_ENABLED=false
TRADE_UDS_PATH=/run/pump-trade-service/trade.sock
TRADE_UDS_ALLOWED_UIDS=
TRADE_UDS_API_TOKEN=
TRADE_UDS_MAX_CONNECTIONS=
TRADE_UDS_IDLE_TIMEOUT_MS=
```

说明：

- UDS 默认关闭。
- Socket 路径必须是绝对路径，父目录必须受控。
- UID 列表在 UDS 启用时必须非空，解析失败或重复/非法值应明确报错。
- UDS Token 必须非空、符合 Bearer 格式，且不能与交易/管理 Token 相同。
- 并发连接和空闲超时的默认值应在基准测试后确定，不在未测量时臆造固定阈值。
- Secret 不得通过 Debug、日志、诊断接口或错误响应输出。

---

## 5. 路由范围

### 5.1 首期建议开放

| 路由组 | 路由 | 首期策略 |
|---|---|---|
| System | `/health`、`/ready`、`/v1/capabilities` | 开放 |
| Trade | `/v1/trades`、`/v1/batch/trades`、`/v1/trades/:request_id` | 开放；仍走原交易授权/准入 |
| Pool | `/v1/pools/resolve`、`/v1/pools/sol-hop`、`/v1/cache/warmup` | 开放；保留原 feature gate |
| Quote | `/v1/quotes/sol-buy` 及 get/preflight/submit/submission | 开放；submit 仍走原持久化与签名门槛 |
| Wallet read | `GET /v1/wallets` | 仅公钥/非敏感只读信息 |
| Balance | `/internal/wallet-balances`、`/internal/wallets/:wallet_id/quote-balance/:mint` | 开放给已通过 UDS peer 认证的本地调用方 |

### 5.2 首期不开放

- 钱包创建、导入、修改、删除和私钥导出。
- `/v1/wallets/manage`。
- 靓号钱包任务。
- SOL 转账。
- WSOL 兑换提交。
- 任何尚未完成独立权限审计的管理/资金操作。

UDS 是传输方式，不是管理员身份。后续确有本地管理需求时，应为管理路由建立独立 allowlist、独立凭据和单独验收，不能因“来自本机”而默认开放。

### 5.3 Router 组织

- 保留 `http::router(state)` 的原路由行为。
- 新增 `http::local_router(state)`，只 merge 首期 allowlist。
- 将现有混合的 wallet routes 拆成只读钱包路由和管理路由；TCP Router 继续 merge 两者，UDS Router 只 merge只读组。
- 两个 Router 调用相同 Handler，不复制 Handler 或业务服务。
- 路由未开放时应返回稳定的 404/403 合同，并有对应测试。

---

## 6. UDS Listener 与 HTTP 适配

### 6.1 连接实现

- 使用 Tokio UnixListener 接受连接。
- 使用当前 Axum 0.7 对应的 Hyper 1.x HTTP/1.1 连接服务承载 Router。
- 在隔离编译探针中核实 `hyper`、`hyper-util`、`tower` 的实际版本和 feature；只增加必要的直接依赖。
- 不为 UDS 顺带升级 Axum 0.8 或其他框架。
- 每条连接建立后读取 peer credentials，并在该连接处理的每个请求中注入 RequestPeer。
- 支持 HTTP Keep-Alive；客户端使用长连接和连接池。
- 通过有界 Semaphore/连接预算控制并发连接；限制空闲连接、请求体大小和连接生命周期。
- 多个客户端并发时，不允许单个连接或客户端无限占用服务资源。

### 6.2 Socket 路径和权限

建议运行路径：

`/run/pump-trade-service/trade.sock`

目标权限：

- 目录：仅服务用户及指定调用组可访问。
- Socket：服务 owner、指定调用组；建议目标 mode 为 `0660`。
- 保留当前敏感文件的严格权限，不为 Socket 全局放宽 `UMask=0077`。

实施前先核对现有 systemd 的 User/Group、数据库/钱包文件权限和运行目录所有权。不得直接改变服务 User 或主组而不检查其对现有文件的影响。Socket group/mode 可通过经验证的 systemd RuntimeDirectory/Socket activation 方案，或受控 bind 后的权限设置实现；两种方式先做最小探针再定，不同时实现两套机制。

安全清理规则：

- 不得无条件 unlink 配置路径。
- 路径若是普通文件、符号链接或 owner/group 不符合预期，拒绝启动并告警。
- 只有确认是本服务遗留的 stale socket 后才允许清理。
- bind 后验证最终 owner/group/mode。
- 退出时关闭 listener、排空连接任务，再清理本服务创建的 Socket 文件。
- UDS 禁用时不创建、删除或触碰该路径。

---

## 7. 双入口生命周期与关停

当前 `main.rs` 在 HTTP graceful shutdown 中关闭 `TradeTasks`，随后调用一次 `service_shutdown::finish()`。双入口必须保留这一单一清理权威。

目标流程：

1. Bootstrap 完成一次，生成共享 AppState。
2. 创建 TCP Router 和 UDS Router，均引用同一 AppState。
3. 启动 TCP Listener；若 UDS 开启，再启动 UDS Listener。
4. 一个统一 shutdown signal 通知两个 listener 停止接受新连接。
5. 只关闭一次 TradeTasks admission。
6. 已接受的连接进入 graceful drain；在途交易任务按现有预算排空。
7. 连接任务和 listener task 均被 join/await，不能遗留后台任务。
8. 只调用一次原 `service_shutdown::finish()`，原 blockhash refresher、observation worker、Shadow writer 和 persistence 关闭顺序保持。
9. Socket 清理只由 UDS listener owner 执行一次。
10. 若任一 listener 意外失败，统一通知另一入口退出，并按现有服务失败策略结束进程。

不要让 TCP 和 UDS 各自调用 `service_shutdown::finish()`，也不要让两个入口分别 close 同一资源或创建独立的 shutdown owner。

---

## 8. 性能测试与客户端

### 8.1 基准比较

至少比较：

- 现有 localhost TCP HTTP。
- UDS HTTP/1.1。

两者必须使用同一 release 构建、同一主机、相同 JSON payload、相同 Handler 和连接复用策略。

测试场景：

1. `/health`、`/ready`、`/v1/capabilities`：测量纯入口/序列化开销。
2. 使用 Fake Backend 的交易接受/查询：测量 Handler + Application 入口，不连接生产 RPC。
3. 多客户端并发：1、8、32、64 个调用方；实际负载上限以机器资源为准。
4. 连接复用与新建连接分别测试，不能只测短连接后宣称 UDS 具备稳定低延迟。

记录：

- P50 / P95 / P99 latency。
- requests/sec。
- CPU、RSS、上下文切换（可用时）。
- 活跃连接数、排队时间、错误率和超时。
- 服务端连接拒绝数及各 UID 请求量。

不预设 UDS 必然快多少。若差异处于测量噪声内，保留可选 UDS 即可，不强制迁移所有调用方。

### 8.2 客户端约束

- 所有调用方复用连接。
- 一个业务请求只选择一个传输入口，不允许 TCP 与 UDS 双发。
- 请求超时或响应丢失时，使用原 idempotency key 查询/恢复；不能自动生成新 key。
- 客户端不得因 UDS 连接失败就盲目通过 HTTP 重新提交同一笔交易。
- 记录调用方名称/UID、request_id、idempotency_key 和 trace_id；不记录 Token、私钥或完整敏感请求体。

Python 可使用 HTTPX 的 UDS transport；Go 可使用 Unix Dialer；Rust 可使用兼容 Hyper 的 Unix connector。客户端样例在 TS-IPC-05 提供，不要求所有调用方同时迁移。

---

## 9. 测试与验收

### 9.1 单元与合同测试

- UDS 默认关闭；关闭时不创建 Socket。
- 配置解析：路径、UID 列表、Token、超时和非法值。
- Token 脱敏；配置 Debug 不泄露 UDS Token。
- TCP RequestPeer 保留原 SocketAddr、IP allowlist、鉴权顺序和错误合同。
- UDS RequestPeer 使用真实 peer credentials；不信任伪造 header。
- 未授权 UID、错误 Token、缺少 Token 均拒绝。
- UDS 路由 allowlist 不包含管理/导出/转账/WSOL submit/vanity。
- HTTP Router 原合同测试全部保持。
- UDS Router 对相同 Handler 的请求/响应 JSON 与 HTTP 一致。

### 9.2 集成与并发测试

- 使用临时目录启动真实 UnixListener/Hyper 连接，不启动生产 Bootstrap。
- 使用无真实 signer、无生产钱包、Fake RPC/Backend 的 AppState。
- 多个独立客户端并发连接与 Keep-Alive。
- TCP 与 UDS 对同一幂等键提交同一请求，必须共享同一记录，不产生第二次执行。
- 同 key 不同 request hash 按原冲突规则拒绝。
- UDS 客户端断开、响应丢失和重连，不产生盲目重提。
- 连接数达到上限时有界拒绝/排队，不无限创建 task。
- stale socket、普通文件、符号链接、权限错误和目录不存在均有安全测试。
- shutdown 停止接收、排空连接和交易任务、调用一次 finish、清理 Socket。
- listener 异常时另一入口收到统一 shutdown 通知。
- 测试不得连接生产 RPC/MEV，不得签名或广播真实交易。

### 9.3 生产启用门槛

以下条件全部通过才可申请启用：

1. TCP HTTP 全部合同回归通过。
2. UDS UID + Token + 路由 allowlist 通过。
3. 两个入口确认共享原 AppState/TradePersistence/TradeTasks，不存在第二套 Bootstrap。
4. 跨入口同幂等键测试通过。
5. UDS 连接和 shutdown 生命周期测试通过。
6. Socket owner/group/mode 和 stale cleanup 经过目标 Linux 环境验证。
7. 性能报告明确列出 TCP 与 UDS 的 P50/P95/P99 和吞吐差异。
8. 调用方使用同一幂等键，并具备 UNKNOWN 查询/恢复逻辑。
9. 有关闭 UDS 的回滚步骤。
10. 生产启用另需明确授权；本文档和离线 CI 不构成部署授权。

---

## 10. 任务清单

| ID | 依赖 | 工作内容 | 交付与验收 |
|---|---|---|---|
| TS-IPC-00 | 无 | 基线与依赖探针 | 复核 main/http/routes/auth/Cargo.lock/systemd；确认 Hyper 适配方式；不改生产依赖 |
| TS-IPC-01 | 00 | LocalIpcConfig 与 RequestPeer | 默认关闭；配置校验；TCP 身份合同不变；UDS peer credentials 单测 |
| TS-IPC-02 | 01 | 路由白名单和鉴权边界 | 拆分只读钱包路由/管理路由；local_router 仅开放批准路由 |
| TS-IPC-03 | 01、02 | UDS Listener 与 HTTP/1.1 over UDS | 多客户端、Keep-Alive、UID 注入、Token 校验、连接预算 |
| TS-IPC-04 | 03 | 双入口生命周期与 shutdown | 共享一次 Bootstrap；统一取消；finish 只执行一次；Socket 安全清理 |
| TS-IPC-05 | 03、04 | 集成测试、跨入口幂等和性能基准 | HTTP 合同不变；Fake Backend 并发测试；TCP/UDS 性能报告 |
| TS-IPC-06 | 05 | 调用方样例和灰度准备 | Python/Go/Rust 至少提供所需调用方样例；迁移/回滚说明 |
| TS-IPC-07 | 全部 | 受控启用申请 | 生产权限、systemd、Socket group/mode、监控和回滚证据齐全；需单独授权 |

执行记录必须区分：源码完成、格式检查、单测通过、集成测试通过、完整离线 CI 通过、性能验证、已部署。不得把计划或单项测试通过写成“UDS 已上线”。

---

## 11. 实施顺序与回滚

### 11.1 推荐顺序

1. TS-IPC-00：先核对当前依赖和实际 Handler/鉴权边界。
2. TS-IPC-01：建立 RequestPeer 和配置合同，不启动 UDS。
3. TS-IPC-02：建立本地 Router allowlist。
4. TS-IPC-03：实现 UDS listener，默认关闭。
5. TS-IPC-04：验证两个入口共用资源和统一关停。
6. TS-IPC-05：跑合同、集成、跨入口幂等和性能测试。
7. TS-IPC-06：提供多语言客户端样例。
8. TS-IPC-07：只有用户单独授权后才考虑生产启用。

### 11.2 回滚

- 将 `TRADE_UDS_ENABLED=false` 并按受控流程重启，即可停止使用 UDS。
- 原 TCP HTTP 监听、路由、请求合同和调用方保持可用。
- 不删除订单、幂等记录、签名账本或数据库。
- 不用旧数据库快照覆盖新数据来回滚。
- 不自动把 UNKNOWN UDS 请求改走 TCP 重新提交。
- 回滚不得放宽 HTTP IP allowlist、交易 Token 或钱包管理权限。

---

## 12. 当前未验证事项

本计划只基于当前分支源码和文档进行设计，以下事项尚未验证：

- 当前 Cargo.lock 中 Hyper/Hyper-util 的精确版本和 feature 是否足够。
- 当前目标 Linux 的 Tokio Unix peer credentials 行为。
- systemd RuntimeDirectory、Socket group/mode 与现有 root 服务文件权限的兼容性。
- Hyper 连接层在 Axum 0.7 下的完整编译实现。
- UDS 与 TCP 双 listener 的真实并发和 graceful shutdown。
- UDS 与 HTTP 的实测延迟差异。
- 目标调用方的语言、连接池和重试行为。

这些事项必须由 TS-IPC 对应任务和测试证据解决；不得在验证前标记为完成。

**本计划的核心不是替换 HTTP，而是在现有 Trade Service 外围增加一个受控的本地传输入口。交易业务、钱包、持久化、签名和恢复仍只有一套权威。**
