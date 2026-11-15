# NP-02 / TS-08-B 普通交易 API 路由与 Handler

日期：2026-11-13。状态：源码及本阶段离线验证通过；未部署。不升级 SDK、修改交易逻辑、重启服务或接触生产数据。

## 实际改动

- `src/api/handlers/trades.rs`：四个 Handler 及原私有接受/等待 helper 从 `http.rs` 原样迁入；鉴权直接调用同一 `auth::authorize_trade`，没有旧位置转发层。
- `src/api/routes.rs::trade_routes`：唯一注册普通/批量/查询路径，由 `http::router` merge 后统一注入原 AppState。
- `src/api/dto/trades.rs::ListTradesQuery`：保留原 `Option<usize>` 提取语义；不增加 trim、别名或默认字段。既有 models DTO 重导出继续兼容使用。
- `src/http_contract_tests/trade_execution_tests.rs`：两项新合同先在旧 Handler 位置通过，随后扩展到独立路由组和总路由两种组装方式。

| 方法 | 路径 |
| --- | --- |
| POST | `/v1/trades` |
| GET | `/v1/trades` |
| POST | `/v1/batch/trades` |
| GET | `/v1/trades/:request_id` |

## 保持的行为

- 来源 IP/Bearer 鉴权、JSON/Query 在 Handler 前提取、错误 JSON 与 HTTP 状态保持。
- 原钱包身份 lease 生命周期、默认/强制钱包选择、校验/拒绝顺序、V1 payload、hash、唯一 reservation 和 duplicate/conflict 保持。
- 批量仍逐钱包调用同一接受函数，空列表仍拒绝；混合错误及重复钱包不新增执行。
- 原日志、task spawn、响应等待时点、20ms 轮询、200/202 判定与 durable 查询权威保持，不增加自动重试。
- 查询仍默认 20、限制在 1..100；durable 读取失败不自动回退内存。
- 普通 SDK、legacy 聚合及既有 PumpAPI 路由分支没有更改。保留旧 PumpAPI 合同作为回归不代表恢复对接开发。

## 验证与未完成项

移动前全套 776 项通过，新增两项旧位置合同通过。移动后两项合同验证独立/总路由的鉴权、错误方法、提取器错误、缺钱包/空批量，以及 Memory/Durable 两模式的 SDK fake 成功、重复/冲突、混合批量、GET 列表/详情和执行次数。仅内存 SQLite/public fixture，无 SDK 钱包、网络、签名或广播。

最终验证：

```bash
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

最终源码的全套默认并发与单线程各 778 项通过，HTTP 155 项通过，生产 check/格式/diff 检查通过；生产仅两条原有兼容导出警告（models 的领域类型、persistence 的 request_hash），无新增抑制或 ignored。无 SDK/vendor、原迁移修改；manifest/lock/release 与拆分前指纹相同：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

本批没有增加新执行能力或完成普通发送前签名账本；`accept_trade_for_wallet`、结果等待和订单查询仍需进一步经 Application/query ports 薄化，不能把文件移动当作整个 TS-08 或基础设施隔离完成。SDK/legacy 安全工作按 `ORDINARY_SDK_SUBMISSION_SAFETY_WORKPACK.md` 继续。

后续检查点：TS-08-C 已将这里的订单查询/结果等待接入项目只读 source 和 Application，进度与验证见 `ORDINARY_TRADE_QUERY_BOUNDARIES.md`。本文件 778/155 是 B 阶段的冻结记录，不替代后续全量结果。

## 2026-11-15 原接受流程的 Application 收口

归入现有 TS-02/08、NP-02/03，不新增任务编号。当前源码与八项新合同已在 26be717 /
Actions 43706312318 完整验收，artifact 14159195795 下载核对：八项专项与完整并发
1466（129.94 秒）/串行 1466（238.29 秒）通过，HTTP 202、全部步骤 exit=0。
前序 c220ba8 的 1458 不包含这些修改，后继执行编排的七项新合同也不在此快照。

- `application/trade_intake.rs` 接窄 Settings、项目钱包身份/地址解析 port 和原
  journal/index/reservations，按原顺序解析默认或强制钱包、绑定 DTO 钱包元数据、
  校验、拒绝记录/尽力镜像、执行政策、项目 command、原 admission 保留。
  不访问 API DTO、Config、SDK、Keypair，不生成第二个幂等键或账本。
- `ports/trade_intake.rs` 只定义借用输入和惰性 payload 工厂；V1 DTO 的实现位于
  `adapters/persistence/trade_admission.rs`，原 hash/JSON/记录转换不移动到业务层。
  Memory/重复请求仍不提前序列化，Durable/Shadow 保持原保留及镜像规则。
- `AppState::trade_intake()` 借用原资源，无新 Arc/store/cache 或后台任务。
  Config 只投影原 validation、command、broadcast 字段，不新增配置或值检查。
- HTTP 保留鉴权、错误/HTTP 映射、原 task spawn/结果等待与批量逐项顺序；
  Application 返回钱包身份 lease，由 Handler 保留到原响应等待结束。
  rejected/duplicate 不生成执行输入，Created 只消费原 `AdmittedTrade` 一次。
- 五处原日志保留 `api::handlers::trades` target、原字段和相对时序；成功接单日志
  target 遗漏已补并新增 scoped subscriber 回归，不修改全局 subscriber 或过滤配置。
  原 DTO `to_command`/validation 兼容 helper 仅测试保留作旧行为 oracle。

新增七项 Application 合同覆盖：三种存储模式的强制钱包/hash/JSON/lease、缺钱包先于解析、
校验先于政策且拒绝不占正常 key、重复/冲突与来源元数据不重写原记录、存储失败模式差异、
所有旧 accepted fixture 到完整项目 command 的对照，以及原成功/拒绝日志 target。
第八项真实 Router + fake backend 门闩用例覆盖 dispatch/响应等待期间的身份 lease。
测试仅公开/合成输入、内存 SQLite 和 fake backend，不加载真实 SDK 钱包或发送交易。

本批本机验证：`cargo fmt --check`、workflow policy 七项通过；受限 test check
`20261115T103713Z-198978` 9 秒/RSS 1,331,668 KiB，bin check
`20261115T103808Z-199318` 4 秒/RSS 575,448 KiB，均 exit=0，原七组生产警告保留。
CI 增加 `ordinary-intake` 专项，保留全部既有专项及完整并发/串行；不在小 VM 链接测试。

首轮 c58eb8c / Actions 43705256866 / artifact 14158646075 已终态失败并下载核对：
所有前置步骤、HTTP 202 及原专项通过，ordinary-intake 五项通过/三项失败，完整两套未执行。
两项新夹具误把 Shadow 队列提交视为落库完成，已改为有界读取实际原行（不 sleep、不忽略
SQL 错误、不修改原字节断言）；另一项旧 command oracle 误固定默认钱包，已传入 fixture
目录实际解析的显式/默认 ID，保留全部 command 字段比较。不改生产存储/钱包规则或旧 fixture。
修正后受限 test check `20261115T105239Z-204758` 9 秒/RSS 1,272,116 KiB、格式通过；
修正已由上述后继快照验收，不重跑旧失败快照或冒用前序 1458。

这批不是全部 Application/资源隔离、全协议授权、SDK 升级或生产恢复完成；
普通执行/状态存储的剩余具体依赖、全局 RPC/生命周期、双假后端及发布门槛继续保留。
