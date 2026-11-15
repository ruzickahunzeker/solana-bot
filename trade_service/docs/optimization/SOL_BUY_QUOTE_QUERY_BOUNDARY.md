# TS-06-A 独立 SOL 报价读取编排

日期：2026-11-13。状态：源码完成，新增 14 项离线回归，全套 366 → 380，通过，无测试构建警告。未升级 SDK/vendor/依赖、构建 release、部署/重启、读取生产钱包/数据库、访问真实节点或发送交易。

## 迁移前证据

上一轮已在原 Handler 捕获并通过 `sol_buy_http_parameter_baseline_precedes_missing_wallet_and_keeps_get_lookup_first`，本轮保留该期待值，验证参数错误先于缺失钱包、GET 缺失钱包先于缺失 Quote。原缓存 108 组字节、普通聚合 448 组参数及 DTO/journal 指纹继续作为合同。

本轮没有通过初始化真实 SDK 钱包来验证旧成功路径；成功/故障路径采用已加载身份 fake、报价 fake 和共享内存缓存，不声称完成真实聚合器或链上验收。

## 已实施

- `application/ports/sol_buy_quotes`：无 serde 的 4 字段借用输入，保留 literal wallet/Mint 与 u64 lamports/u16 滑点；source 只提供 request 映射、原校验和 QuoteEstimate 获取，不提供钱包、签名或缓存能力。
- `application/sol_buy_quotes`：项目 SolBuyQuoteQueries facade 与 service，组合 source、已有 LoadedWalletDirectory、QuoteCache；不依赖 SDK、wire DTO、Config、AppState、SQL 或 HTTP 状态码。
- `adapters/aggregator/quotes`：仍复用原 `aggregator_quote::validate_request/quote`；原第二次校验、HTTP endpoint、5 秒超时、请求/报价 cap、路由与价格影响政策不变。仅把结果转为已有内部 QuoteEstimate。
- `SolBuyQuoteRequest::query_input`：纯字段借用映射，无默认值、trim、金额换算、地址解析或钱包解析。
- POST/GET quote Handler：只保留原鉴权、项目调用、旧 DTO 转换与错误映射，不再获取 SDK 钱包或直接调用报价网络 helper。
- bootstrap 使用同一身份 directory 与同一 QuoteStore Arc；不增加 Config 引用、钱包加载、SDK/RPC 初始化或缓存副本。

preflight、submit、reconcile 尚未在本批迁移；它们继续通过原兼容桥读同一缓存，签名/journal/资金行为没有改动。

## 保持的顺序

POST：

1. 原 JSON 提取/鉴权。
2. 仅映射请求，执行原参数校验。
3. 按显式 literal ID 查当前已加载身份；不解析默认 ID，不以数据库条目替代运行实例。
4. 持有原只读 lease 跨越报价 await。
5. 报价成功后才读取该 lease 公钥并绑定缓存。报价失败不提前采公钥或写缓存。
6. 返回原 13 字段 BoundSolBuyQuote 响应；整数和字符串价格影响保持原精度，预览不变成 executable=true。

GET：原 query 提取/鉴权 → 显式 ID lookup → 公钥 → cache read。不会调用 POST/source 校验、重新报价、解析默认 ID或刷新 TTL。

| 阶段 | 原 HTTP 行为 |
| --- | --- |
| 参数错误 / 缺少已加载钱包 | 400，原错误文本 |
| 报价网络/源错误 | 502，原 URL 脱敏 |
| 缓存绑定失败 | 503，原错误文本 |
| 缓存缺失/到期/身份不匹配 | 410，原错误文本 |
| 无效 JSON 字段 | 原 Json 提取器 422，先于 Handler 鉴权 |
| 缺失 GET query 字段 | 原 Query 提取器 400，先于 Handler 鉴权 |

没有因为只读服务提取而增加交易总开关校验：禁播时原只读报价仍可使用，不授权任何签名/广播。

## 新增回归

| 数量 | 证据 |
| --- | --- |
| 5 | 纯项目 service：完整整数报价、map/validate/lookup/fetch/pubkey/bind/drop 轨迹、每阶段失败、GET 的 literal ID/错误优先级、fetch 期间删除后继续使用原 lease；无 API/SDK fixture 类型 |
| 2 | 实际 source adapter 的 literal/最大整数映射与原参数 cap/错误顺序，仅测试映射和校验，不调用 fetch |
| 1 | DTO 借用字符串指针及完整整数映射，无提前规范化 |
| 6 | 实际 Router：创建/读取的完整响应与旧缓存共享、无效参数/缺失身份零 fetch、502 脱敏/503、当前身份改变后的真实缓存 410、鉴权/提取器零调用、禁播下只读与显式 ID 行为 |

加上迁移前基线，`http_contract_tests::quote_query_tests` 共 7 项。过期错误的 HTTP 分支使用受控故障注入，原真实 TTL/过期删除规则由 TS-02-D 缓存测试独立覆盖；没有用 sleep 等待链上或本地真实时钟来伪造端到端验收。

测试构建阶段发现新增 query 提取器测试误用了 Json 的 422 预期，已按未改动的 Axum Query 原 400 校准；没有为让测试通过而改业务路径或冻结基线。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet application::sol_buy_quotes
cargo test --locked --offline --quiet adapters::aggregator::quotes
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

结果：service 5 项、source 2 项、HTTP 59 项、全套默认并发/串行各 380 项，0 failed，无测试构建警告。旧普通校验、请求/缓存/持久化字节、协议 loader、幂等/恢复与签名边界合同继续通过，期待值未修改。

Cargo.toml、Cargo.lock、现有 release SHA-256 与 TS-04-E3/TS-02-D 一致；vendor 和原迁移 diff 为空，未改配置或生产制品。本轮测试均未建立真实监听或连接聚合器/RPC。

## 后续 TS-04-F1 工作包（已完成）

后续执行记录：[F1 预检边界](QUOTE_PREFLIGHT_BOUNDARY.md)、[F2 提交边界](QUOTE_SUBMIT_HTTP_BOUNDARY.md)、[F3 对账边界](QUOTE_RECONCILIATION_BOUNDARY.md)。以下保留本批结束时的工作包，当前队列以 `EXECUTION_STATUS.md` 为准。

先处理独立 quote preflight 的 SDK 获取，不连带修改 Live 提交。

1. 审查原 `preflight_sol_buy_quote` 与 `aggregator_quote::preflight/prepare_transaction`：鉴权 → 一次钱包解析 → 第一次缓存身份/时效检查 → 原只读准备/模拟 → 第二次缓存检查 → 原 32 字段报告。
2. 新 port 仅暴露已解析钱包公钥与 bound-quote preflight，返回项目 QuotePreflight，不公开 signer、raw RPC、交易对象或 submit。真实 handle 私有保留原 client Arc。
3. Application 复用 QuoteCache，但不能用本批 `quote_queries.get` 替代两次检查：该 facade 会重新 lookup 当前身份，原 preflight 要继续使用已持有的钱包实例。
4. 保留错误优先级：缺钱包 400、首次/二次 cache 错误 410、准备/模拟失败 502 与原 URL 脱敏。准备失败不做第二次缓存检查；准备成功但已过期必须拦截，不能直接返回旧报告。
5. SDK adapter 复用原 prepare 与不广播模拟，不重复创建 SDK/RPC，不签名、不新建 submission journal；Handler 只负责 DTO/状态映射。Quote ID/钱包仍按原 literal 值处理，不新增 UUID 或默认 ID 校验。
6. 补 fake wallet/cache/预检的时序、过期、注册项删除、错误和零 sign/send 入口测试，以及真实 Router 成功/失败/授权与完整报告合同；继续保留旧 preflight/缓存指纹。

TS-04-F2/F3 再分别处理 submit 的受限签名/journal 装配和 reconcile 的只读 RPC 装配。普通资金签名前账本、VerifiedRoute/可信报价/资产扩展、SDK provenance、升级/发布演练仍按完整主计划推进；本批不把局部 facade 完成视为这些目标已完成。
