# TS-02-D 项目类型 QuoteStore 与缓存端口

日期：2026-11-13。状态：新增 6 项离线测试，全套 359 → 365，通过，无测试构建警告。未升级 SDK/依赖、构建 release、部署/重启、读取生产钱包/数据库、调用真实节点或发送交易。

## 迁移前证据

先在原 QuoteStore 提取私有时钟/ID 采样 helper，不改原判断顺序；捕获并通过 108 组 V1 绑定字节指纹及过期/容量/不去重测试，再迁移缓存实现。

指纹：`e543fd29e04c691f13cccd17a8afe094a9881f71b7a6cc6ec6b81348a06d94d3`。

样本包含 4 种 literal wallet、3 种 literal payer、3 种滑点及 3 种整数数量（含 >2^53 和 u64::MAX），固定 clock/ID 后记录完整序列化字节。迁移后只改测试入口映射，不更新该期待值。

## 已实施

- `application/ports/quote_cache` 提供 QuoteCache 的 bind/read 端口，输入是 3 字段绑定元数据与内部 QuoteEstimate，返回内部 BoundQuote。接口没有 SDK、HTTP DTO、RPC 或签名能力。
- `adapters/cache/quotes.rs` 保存 `DashMap<String, BoundQuote>`，只依赖项目模型与标准缓存/UUID/时钟工具，不导入 API/SDK/Config。
- `adapters/cache/quote_v1.rs` 保留原 insert/get wire 入口，只做 DTO/项目值映射；原 `aggregator_quote::QuoteStore` 路径重导出同一个类型。
- 普通 SOL 聚合 adapter 已实际调用项目 bind 端口，不再为入库转换成 wire quote 再转回内部模型。
- main、HTTP、普通聚合仍共享同一个 QuoteStore，不增加存储副本、迁移、配置或清空行为。

QuoteStore 只绑定调用方提供的元数据与已产生的估计值，不重新验证 Mint/余额/路由，也不把 executable=false 改为可执行。QuoteBinding 不是钱包授权、VerifiedRoute 或签名凭据。

## 保持的策略

- TTL 仍为 15,000 ms，容量仍为原 512 条预检查；先删除 expires_at <= now 的项，再检查容量，再生成 UUID v4。
- insert 仍在缓存处理前采时；兼容 insert 在 DTO 转换前采时，避免因边界转换而改变旧读取次序。
- read 仍先查 ID，存在才采时；恰好到期即过期并删除，过期优先于 wallet/payer 不匹配。不存在不采时，不匹配但有效的项不删除。
- 仍按原完整 literal wallet/payer 比较，不 trim、不推导地址、不套用当前默认钱包。
- 不按相同请求去重或覆盖有效项；读取返回独立克隆，修改返回值不影响缓存。
- 原 UUID、wall-clock 截断/加法、字段/JSON 顺序、整数精度、错误文本和共享生命周期保持。

保留的是原容量预检查，不声称新增“并发绝不会超过 512”的原子保证；没有借目录迁移修改现有竞态或时钟回拨政策。

## 测试

新增 4 项迁移前基线：108 组字节、过期/身份错误优先级与 clock 次数、满容量清理/ID 调用顺序、UUID v4 与重复请求保持独立。原 expiry/wallet 测试迁入缓存测试模块，使用本地报价 fixture，数量不变。

新增 2 项内部端口回归：dyn QuoteCache 完整整数/路由/绑定值保留，以及内部端口与旧 wire 入口共用同一份记录和序列化字节。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet adapters::cache::quotes
cargo test --locked --offline --quiet
git diff --check
```

结果：缓存模块 7 项、全套 365 项通过，0 failed；原普通聚合 448 组请求、API/持久化/协议/恢复合同保持。manifest/lock/现有 release 指纹与 TS-04-E3 一致，vendor/原迁移 diff 为空。没有建立测试监听端口或加载真实 SDK 钱包。

## 下一批 TS-06-A

2026-11-13 续作已完成源码/离线验收：新增 14 项，全套默认并发/串行各 380 项通过；见 `SOL_BUY_QUOTE_QUERY_BOUNDARY.md`。以下保留本批交接时的基线与实施约束，最新后续任务为 TS-04-F1 preflight 隔离。

独立 POST quote / GET quote 仍在 Handler 读取具体 SDK 钱包和调用报价网络 helper。下一批通过项目借用输入、只读报价 source、已加载身份与缓存端口组合替换这两条入口的编排，保留鉴权/校验/身份/报价/绑定/错误状态顺序；fake HTTP 覆盖真实成功与故障。preflight/submit/reconcile 另按 TS-04-F 收口，不在本批改签名账本或资金政策。

### 已开始的基线

迁移新编排之前新增 `http_contract_tests/quote_query_tests.rs::sol_buy_http_parameter_baseline_precedes_missing_wallet_and_keeps_get_lookup_first`，在原生产 Handler 路径通过。覆盖 wallet/Mint/WSOL 输出、低于/高于 SOL cap、u64::MAX、滑点非法及多重错误的优先级；GET 的空/缺失钱包仍先返回 `wallet_id not found`，不提前返回缺失 Quote。没有实际钱包或网络请求。

加入这一项后全套为 366 项；它属于 TS-06-A 的迁移前证据，不代表新只读编排已实施。

续作复核：全套默认并发与 `-- --test-threads=1` 各 366 项通过；`http_contract_tests` 53 项通过。磁盘余量约 3.3 GB，未生成新的 release 制品或修改原依赖/制品指纹。

### 续作实现边界

1. 新增无 serde 的 4 字段借用报价输入；SDK/HTTP Request 的映射留 source adapter，可复用现有请求关联类型模式，避免 Application 依赖 wire DTO。
2. Application 只组合报价 source、已加载身份 directory 与 QuoteCache。POST 的原顺序必须是 map/validate → lookup → fetch quote → 取该 lease 公钥 → bind；fetch 失败不能提前取公钥或写缓存。实际 source 继续复用原 validate/quote（含原第二次校验），不改网络端点、5 秒超时或参数 cap。
3. 使用原只读 lease 保留已加载实例，不查钱包数据库冒充可执行钱包，不引入默认 ID 解析。GET 直接 lookup → 公钥 → cache read，不调用 source 或 POST 校验。
4. 项目失败类型区分参数、钱包、报价网络、缓存写与缓存读；Handler 仍映射原 400 / 502（原 URL 脱敏）/ 503 / 410，保留原响应 DTO。鉴权与 JSON/query 提取顺序不改。
5. bootstrap 注入相同 identity/cache Arc，不增加 Config 引用或新钱包/SDK 初始化；现有 preflight/submit/reconcile 仍通过兼容入口读取同一缓存，后续再迁移。
6. 补纯项目调用轨迹/失败/lease 回归、DTO/source 参数映射，以及真实 Router 的无 signer fake 成功、GET 读取、错误状态/脱敏、未经授权零调用和跨入口共享缓存。旧基线/108 组缓存字节/448 组普通聚合映射期待值不改。
