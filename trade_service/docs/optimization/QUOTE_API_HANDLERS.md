# TS-08-A 独立报价 API 路由与 Handler

状态：源码与离线通过，未部署。不改变 SDK、资金逻辑、配置、钱包或数据库。

## 实际目录

```text
src/api/
  routes.rs             # 独立报价五条路由组
  response.rs           # 原 error JSON helper
  handlers/quotes.rs    # 五个薄 Handler 与原错误映射
  dto/sol_buy.rs        # 原请求/响应及 QuoteWalletQuery
  dto/quote_submissions.rs
```

`http::router` 合并同一报价路由组；没有重复注册或旧 Handler 转发层。报价 Handler 仅使用鉴权、DTO、应用 facade 与错误映射，不访问 WalletRegistry、SDK client、RPC、SQL、QuoteStore 或具体 journal。鉴权直接调用原 middleware，与旧 authorize_protected 包装同义。

原通用 error helper 原样移至 `api/response.rs`，其余旧 Handler 通过 import 复用，保持状态码、JSON/Content-Type。未改鉴权模块内部 helper 或其他资金入口。

| 方法 | 路径 |
| --- | --- |
| POST | `/v1/quotes/sol-buy` |
| GET | `/v1/quotes/sol-buy/:quote_id` |
| POST | `/v1/quotes/sol-buy/:quote_id/preflight` |
| POST | `/v1/quotes/sol-buy/:quote_id/submit` |
| GET | `/v1/quotes/sol-buy/:quote_id/submission` |

Query DTO 仅移动，不新增默认值、trim、UUID 或钱包别名规则；JSON/Query 提取先于 Handler 鉴权的旧行为保持。提交与预检仍为 POST，对账仍为 GET，成功对账仍 no-store。

## 验证

先在旧 Handler 位置完成 F3 全套并发/串行各 435 项；移动后全套 435 项仍通过。新增 1 项独立 route group 测试，对比组装前后五条路径的鉴权、来源拒绝、缺钱包、错误方法和未知路径。

最终 `cargo fmt --check`、默认并发/串行各 436 项、HTTP 85 项及 `git diff --check` 通过。既有 quote 创建/查询、preflight、submit、reconcile 成功/失败、并发与恢复合同均直接命中新位置，不仅测试函数映射。

本批不宣称整个 API/AppState/infrastructure 完成分层。普通交易、钱包管理、余额与转账、WSOL 兑换等 Handler 仍有后续边界；当前队列以 `EXECUTION_STATUS.md` 为准。
