# NP-02 / TS-08-D Pool 与系统查询 API

日期：2026-11-13。状态：本阶段源码和离线验证完成，未部署。范围：仅 Trade Service 的入口归属与离线合同；不修改 SDK/vendor、依赖、交易能力、运行配置、生产数据或服务状态。PumpAPI 交易对接开发仍暂停。

## 交付

- `src/api/handlers/pools.rs` 接管原 warmup、resolve、SOL hop 三个 Handler；继续调用原 Application 查询对象和校验 facade。
- `src/api/handlers/system.rs` 接管原 health、ready、capabilities 三个 Handler；沿用只读 readiness query 和原能力响应生成器。
- `src/api/routes.rs` 唯一注册两组路由；总路由仅 merge 并注入原 AppState，`http.rs` 不保留这六个函数或旧位置转发层。
- DTO、validation facade、Application、cache、SDK/RPC adapter 均沿用原实现，不因文件移动另建服务或仓储。

| 方法 | 路径 | 原语义 |
| --- | --- | --- |
| GET/HEAD | `/health` | 公开存活检查，不读取 readiness 或 SDK |
| GET/HEAD | `/ready` | 原交易凭据/IP 检查后调用只读 query；仍返回原 ready 真值及 200 |
| GET/HEAD | `/v1/capabilities` | 原交易凭据/IP 检查，配置决定原能力字段；保留动态生成时间 |
| POST | `/v1/cache/warmup` | 原 DTO/校验，后端成功后写原参数缓存并返回原 key |
| POST | `/v1/pools/resolve` | 原地址解析、查询/缓存顺序，成功 DTO 与失败 502 保持 |
| POST | `/v1/pools/sol-hop` | 原功能开关先于地址语法校验；查询失败仍为 200/unavailable，reason 保留 URL 脱敏 |

## 兼容与合同

迁移前新增三项真实进程内 Router 合同并通过：Pool 的 HTTP 方法/Allow、415/400/422 提取错误先于 Handler 鉴权、有效请求的 401/403；独立管理凭据及禁播策略不改变查询凭据；系统 GET/HEAD、公开 health、受保护 HEAD 的空响应体和 readiness 调用次数。

迁移后将这三项同时运行于独立路由组和总路由，覆盖 Pool 成功时的后端/缓存顺序；另新增两项独立组合同，验证 Pool 失败响应及不写缓存、系统完整响应/功能开关、两个组均不暴露钱包或交易等其他操作。已有 Pool、readiness、协议响应冻结指纹继续回归，不重新生成黄金期望。

首次全套回归发现新测试直接比较了两次 `generated_at_ms`，产生毫秒竞争。已将该测试改为单独验证响应时间在本次请求的时间区间内、其余完整 JSON 保持相等；不修改生产时钟或原能力快照。

测试仅使用公开地址、fake readonly source、原内存缓存/临时 SQLite；SDK 钱包为空，不加载 `.env`、真实密钥，不建立真实 HTTP/RPC 连接，不签名或广播。

## 验证

旧位置三项新增合同通过；另两项在新组装完成后添加，总测试数从 871 增至 876。上述动态时间测试修正后的最终结果：

```bash
cargo test --locked --offline --quiet                         # 876 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 876 passed
cargo test --locked --offline --quiet http_contract_tests     # 165 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

check/fmt/diff 全部通过，生产 check 仅保留原两组兼容导出警告（models 领域类型和 persistence request_hash）；不新增警告抑制。旧参数/消息/状态/协议指纹测试随全套通过，未重写期望。原迁移/vendor 无新增修改；manifest/lock/release 与拆分前相同：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

受保护的 PumpAPI client/assets、ordinary execution adapter、executor 和 vendored DAMM 指令文件 SHA 也与本阶段开始前一致。持续任务保持 active；修改计划或补这份验收记录不构成暂停指令。

## 仍未完成

- 钱包管理、余额及 vanity Handler 仍留在 `http.rs`，请求作用域 AppState/资源组合和 validation facade 仍需继续收口。
- 这次不改变寻池算法、RPC 配额/并发/超时/回退、缓存 TTL/版本或协议能力；PoolRouteHint 仍不是 VerifiedRoute。
- 普通 SDK 最终消息授权、唯一广播前签名字节账本、原字节受控发送及 ordinary SOL 聚合 legacy 迁移仍未完成；不能把入口移动当作该安全工作包验收。
- 不升级 SDK、不构建 release、不部署/重启、不操作生产钱包或数据库、不重提历史订单。源码测试通过不等于真实节点或生产发布验收。
