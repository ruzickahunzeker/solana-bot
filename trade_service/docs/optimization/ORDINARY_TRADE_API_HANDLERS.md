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
