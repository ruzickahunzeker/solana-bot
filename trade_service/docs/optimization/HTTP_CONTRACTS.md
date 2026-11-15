# HTTP 合同测试边界

更新：2026-11-13。入口：`src/http_contract_tests.rs`，由 `main.rs` 的 `cfg(test)` 注册。

## 已完成

使用实际 `http::router` 与 Tower `oneshot`，在进程内发送请求，不启动 HTTP 监听器。

| 测试 | 验证范围 |
| --- | --- |
| 公共入口与方法 | `/health` 无需认证；未知路径 404，不支持的方法 405 |
| 默认 legacy 保护 | 20 个操作传入合法形状的请求，无 Bearer 返回 401 JSON；包含新增原签名只读状态 GET |
| 默认内部保护 | 10 个操作在 allowlist 中的非 loopback 地址，即使有正确 Bearer 也返回 403；loopback 无 Bearer 返回 401 |
| IP 优先级 | 不允许 IP 返回 403；允许 IP 仍需 Bearer；不信任 `X-Forwarded-For`；IPv4/IPv6 loopback 可鉴权 |
| Bearer 格式 | 拒绝错误 token、错误 scheme、多余字段；接受大小写不同的 Bearer scheme |
| 请求解析 | 缺 JSON Content-Type 返回 415；缺字段返回 422；非法 JSON 返回 400；目前这些发生在 Handler 鉴权前 |
| 钱包选择失败 | 默认/显式钱包不存在均返回 400，内存订单仍为空 |
| 订单读取 | 缺失订单 404，空列表 200，非法 limit 400 |
| 能力输出 | schema、时间字段与 observe 协议不开放买卖的 HTTP 响应 |
| 兑换关闭 | 关闭 Live 时，合法形状的 WSOL submit 在访问钱包/RPC 前被拒绝 |
| 原签名状态路由 | 新 GET 的鉴权、IP、方法、缺参数和缺钱包拒绝路径；不访问真实 RPC |
| separate 管理 | 8 个管理操作拒绝交易凭据/非 loopback 管理凭据；管理凭据不能直接调用交易；正确凭据通过实际 PATCH 鉴权后由空仓储拒绝 |
| 禁止广播 | 转账/WSOL 在访问钱包或数据库前拒绝；模拟默认/请求覆盖/MEV 不能越过策略；绕过 HTTP 的 executor 仍拒绝 |

首批 17 个测试函数包含多条请求断言，不等同于仅测试 17 个路由。TS-04-D 另新增 `src/http_contract_tests/readiness_tests.rs` 的 7 项真实路由测试，包含完整成功响应、48 组状态组合、字段/null/大整数及鉴权零探测；见 `READINESS_QUERY_BOUNDARY.md`。TS-05-A 新增 `pool_query_tests.rs` 的 13 项（含 1 项序列化基线），覆盖真实 resolver/warmup/hop 成功/失败、缓存顺序、feature gate/别名/脱敏；见 `POOL_QUERY_BOUNDARY.md`。

## 隔离与限制

- Config 显式构造，不读取 `.env`、环境中的密钥或生产路径。
- `WalletRegistry::empty_for_http_tests` 只在测试构建中存在，没有 SDK 客户端或 signer；生产 `new` 不变。
- 钱包仓储使用 File 模式的无 I/O 构造路径，交易仓储 Memory 模式，兑换仓储未初始化。
- 不启动 blockhash 刷新；后续已通过显式注入无 SDK/signer 的 source 测试实际 `/ready` 成功路径，未测试真实 runtime SDK 或成功下单/管理钱包。
- `ConnectInfo` 由测试请求扩展模拟，验证的是应用对 peer 的使用，不是反向代理、TLS 或真实 socket 集成。
- `enable_simulate=true` 不是测试安全保证。安全来自无真实 signer/RPC 和只执行拒绝/只读路径。
- 默认 legacy 仍是回归基线，不代表权限迁移已完成；新增 separate 模式有独立管理凭据和 loopback，需显式配置及调用方迁移。测试不自动启用线上模式。
- 尚未验证 Handler 层签名次数、并发 reservation、DB 写入失败后零广播、取消/崩溃恢复；这些依赖后续 ports 和 fake transport。

## 运行与依赖

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline http_contract_tests -- --nocapture
cargo test --locked --offline --quiet
git diff --check
```

结果：HTTP 模块当前 37 项通过（含序列化基线），全套 236 项通过，格式/差异检查通过。TS-05-A 为全套 226 项，安全轮为 HTTP 17 项/全套 132 项，TS-02-A 为全套 142 项，最初 HTTP 轮为 10 项/全套 76 项，对账轮为 11 项/全套 116 项；各阶段另见对应验收记录。

新增的唯一直接依赖是 dev-dependency `tower = "=0.5.3"`（util）；该版本已在原锁文件中。锁文件只给本包增加 `tower 0.5.3` 依赖边，没有升级 SDK 或任何包版本。`cargo metadata --no-deps` 未更新此依赖边，之后补齐该边并用 `--locked --offline` 验证。
