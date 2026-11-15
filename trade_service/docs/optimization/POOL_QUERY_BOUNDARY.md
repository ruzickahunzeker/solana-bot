# TS-05-A Pool 查询与预热边界

日期：2026-11-13。状态：源码与 226 项离线测试通过；未构建 release、未部署、未访问真实 RPC/钱包或发送交易。

## 已实施

- `domain/route.rs` 新增项目拥有的 `PoolRouteHint`，仅有 Pool、Mint、Quote 和可选 Pool ID。
- `application/ports/pool_queries.rs` 分离 resolve、warmup、sol-hop 只读 backend 与 ParamCacheWriter。
- `application/pool_queries.rs` 负责成功查询后的元数据缓存记录。失败不写、不清空或延长旧缓存；hop 发现不写此缓存。
- `PoolQueries` 作为三个 HTTP Handler 的可注入 façade，使用与 readiness 相同的 boxed-future 组合方式；内部 backend 保留原生异步 port，不增加依赖。
- `adapters/sdk/pool_queries.rs` 复用 WalletRegistry 的现有默认客户端，只在 adapter 把项目 Address 转换成 SDK Pubkey，不重新初始化 SDK/钱包/RPC。
- `adapters/cache/params.rs` 复用现有 Arc<ParamCache> 与 cache_key；不另建缓存、数据库或 reservation。
- `api/dto/pools.rs` 接管 6 个旧请求/响应类型及边界转换；models 保留旧路径重导出。
- SDK resolve_pool 现在返回项目提示类型，不再依赖 API ResolvePoolResponse；3 个成功分支仅转换既有 Pubkey 字节，探测和错误算法不变。
- Handler 不再获取 SDK 客户端；保留鉴权、输入校验、feature gate 和旧响应/错误映射。

## 保留行为

| 项目 | 本轮保留 |
| --- | --- |
| 自动 resolve | 原 Pump/DBC 探测及 PumpSwap fallback；只调整返回类型，不增加自动支持的协议 |
| RPC 顺序/超时 | 原两个 Helius 的随机优先/串行 lane fallback、每 lane 3 秒 timeout；lane 内 Pump+DBC 原并发保持 |
| SOL hop | 原 Mutex 串行发现、Default lanes、两种 Mint offset 过滤、5 秒 lane timeout、成功 30 秒缓存、两次完整空扫描的拒绝规则 |
| Warmup | 原 execution_lanes、每协议参数读取/校验、错误 lane 前缀与脱敏；不改成交易构建逻辑 |
| 查询缓存 | 原 key、Pool/Mint/Pool ID 字符串和成功后的记录时机；先 backend，再 cache，再响应 |
| Feature gate | STONKFUN_VIA_SOL_ENABLED=false 在 pubkey 校验之前返回原 200/available=false；不会访问 backend |
| 错误 | resolve/warmup 为原 400/502；hop 失败为 200/available=false/reason，继续脱敏 URL |
| 别名 | Warmup 仍按原 validator 把 SOL quote 别名归一化成 WSOL；sol-hop 仍直接传入字面 Quote 地址，不顺带改变其旧行为 |

## 证据

激活前运行旧实现 4 项基线：完整响应 fixture、无效参数、关闭 gate 的优先级、JSON/Pool 枚举解析顺序。fixture 位于 `tests/fixtures/contracts/pool_queries_v1.json`，地址仅为合成公钥，不是可用池或链上交易证据。

迁移后新增 9 项真实 `http::router` + fake backend/cache recorder 测试：

- Pump、PumpSwap、DBC 的旧返回/nullable Pool ID 与成功后缓存顺序。
- Mint/Pool ID/Quote 三个不同地址不被交换。
- Warmup 原成功 key、PumpSwap 缺省/显式 Pool ID、SOL quote 和 StonkFun 历史别名的归一化。
- resolve/warmup 失败为原 502，旧缓存内容/时间不变，零 cache 调用。
- hop 成功/失败的原 shape 和 200 语义、URL 脱敏、不写元数据缓存。
- 未授权/feature 关闭时 backend/cache 调用均为 0。
- sol-hop 的字面 Quote 与 warmup 别名语义不被混用。

所有成功 HTTP 用例的 WalletRegistry 都为空；Query façade 调用实际 Application 与 cache adapter，但 backend 是无 SDK/signer 的 fake。若 Handler 退回直接读取默认 SDK 钱包，测试会失败。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：226 passed、0 failed，无编译警告。旧普通交易 hash、独立 Quote journal、readiness、Registry/校验指纹同时通过；SDK vendor、manifest/lock 与原 release 不变。

## 尚未完成

- PoolRouteHint 不包含完整 owner、上下文 slot、Token Program、有效期和报价证据，**不是 VerifiedRoute**。SDK 部分账户检查没有被删除，但不能凭这个提示类型自动批准交易。
- SDK 参数 builders、账户校验与 GPA discovery 仍在 sdk_client 的现有实现；backend 私有持有 Registry，不等于完全无私钥的进程或 SDK 运行时。
- ParamCache 仍只存元数据，key 不含 Quote/Token Program/SDK 版本；本轮不升级为可执行路由缓存，也不把预热记录当作安全授权。
- 未做真实链上 resolver/warmup/hop 的验收、RPC 配额/并发预算优化或寻池性能证明。保留了原可能较慢的调用策略。
- SDK warmup 与 dex_params 存在不同校验/错误语义，例如 warmup 的 Pump complete 检查、构建时的 Mint 成员校验等。下一批需分别冻结并提取，不能为了复用直接合并。

下一批 TS-05-B：先冻结纯参数/Quote 类型/池 Mint 检查，再把真实 SDK 参数映射集中到 adapter 模块；保留 warmup 与交易构建差异及所有错误，补不带 signer 的账户/RPC fixture。不得扩大协议或修改 vendor。
