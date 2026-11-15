# Sol-Bot 当前实现与目标计划审查

日期：2026-11-14（Asia/Shanghai）。范围为本仓库三个独立服务，不包含工作区其他项目或 `trade_service-sdk9/`。这是源码、离线测试及只读运行检查，不是全协议交易验收。

## 结论

方向正确，但尚不能宣布优化完成或全协议可交易。Handler/DTO、项目模型、Application/port 与 SDK adapter 的拆分已有实际交付；剩余主要问题是资金执行安全、统一 RPC 治理、可复现升级与发布验收，不是继续增加空目录。

必须分开看三个状态：

- 源码：当前工作树包含大量尚未提交的优化和新增迁移；实际依赖仍是 vendored `sol-trade-sdk 6.0.2`，不是旁边 SDK9 目录。
- 离线验证：本轮 Trade Service 全套 1038 项通过；Bridge 10 项通过；Trade Service fmt 和仓库 tracked diff 空白检查通过。未重跑 Copy Trading Rust 全套，不把历史结果当成本轮验证。
- 线上：继续运行 11 月 12 日的旧 release。本轮只切换 RPC 配置并按用户明确批准重启 Trade Service，没有编译、替换二进制或上线本轮优化。

持续优化目标当前为 paused；本轮审查和 RPC 运维不代表恢复该目标。PumpAPI 交易对接开发保持暂停，现有 Bridge 和兼容代码保留。

## 按风险排序的发现

| 优先级 | 发现与位置 | 影响及处理建议 |
| --- | --- | --- |
| P1 | 普通 opaque SDK 仍先写 Submitting 再调用组合执行，之后才获得签名：`trade_service/src/executor.rs:195` | 进程在 SDK 广播后、结果写入前退出时，不能依靠原签名账本恢复所有交易。现有 unknown/不换 lane 保护有价值，但不是全路径广播前持久化；后续优先拆不可变构建、授权、签名落盘与发送。 |
| P1 | Durable Live 普通 SOL 聚合已接原 authority 的 signed journal；Memory/Shadow 仍落入 legacy：`src/adapters/sdk/ordinary_sol_buy.rs:97`、`src/application/legacy_quote_submission.rs:49`（均相对 Trade Service） | 新独立 quote 账本不自动保护所有普通交易。必须明确模式能力，不能把 Durable 子路径验收扩大为所有模式/backend 的崩溃保障。 |
| P1 | 钱包管理鉴权源码仍默认 legacy：`trade_service/src/config/security.rs:75`；导出经 `src/api/handlers/wallets.rs:113` 的管理鉴权 | legacy 中交易凭据也可取得管理权限；`.env` 未显式设置 separate，本轮未验证线上凭据分离或外部网络拒绝合同。独立管理令牌和 loopback 方案需调用方迁移后单独发布，不能随本次 RPC 切换改变。 |
| P1 | 没有统一的跨读路径 RPC 冷却/预算：`trade_service/src/sdk_client.rs:1406`、`src/adapters/rpc/blockhash.rs:22` | 两个 Helius 的寻池顺序随机，但下一次请求仍可选到限流节点；blockhash 每轮从主节点重试。已实测主 Helius 429，重启前日志反复出现该错误。配置移除失效节点止住当前问题，不等于根治以后限流。 |
| P1 | 制品和恢复门槛未收尾；磁盘 98%，仅约 1.7 GiB 可用 | 全量优化未上线，发布还涉及新迁移和调用方兼容；不能在此空间条件下盲目 release 或生产 VACUUM。需足够空间的构建环境、加密异机恢复证据、版本化制品与回滚清单。 |
| P2 | Bridge ID 缺指令/事件序号：`pumpapi_zmq_bridge/pumpapi_zmq_bridge.py:73`；PUB 和 observation spool 在同文件约 287、304 行 | 两个只在 instructionIndex 不同的样本得到相同 ID，已只读复现；PUB 无消费确认，单轮转 JSONL 也不等于可恢复交付队列。可能合并多事件或在断连时丢信号；保留在暂停的后续工作包中，不在本轮改变消息合同。 |
| P2 | 审查时主计划/概览停在 D1B1，而源码已有 D1B2；旧总计划的 immediate queue、目录行数和部分审计结论也已过时 | 本轮已补专用主计划/概览/执行记录的当前入口并标明历史基线。历史证据保留，后续验收须按源码/测试/部署分列，不能继续沿用旧数字。 |

本轮没有新增实盘交易，没有导出私钥、直接查询生产钱包密文、替换数据库或升级依赖。健康检查不证明某个协议能够成功成交。

## 与目标计划对照

| 工作包 | 已具备的交付 | 仍需完成 |
| --- | --- | --- |
| TS-02 / 内部模型 | 项目拥有的地址/整数金额/命令、V1 DTO/codec 与历史 hash 合同 | 剩余接受入口/旧资源耦合与防新增类型泄漏检查 |
| TS-03 / 协议能力 | 中央 Registry、九类既有交易映射、六类 observe-only；基础 loader/参数 fixture | 各协议双向、Quote、Token 扩展、账户/费用/迁移与真实模拟验收；不能靠 SDK builder 宣布支持 |
| TS-04 至 TS-08 / 边界 | 钱包余额/管理/靓号、Pool、Quote、交易查询等实际入口已接 Application/port；HTTP 组合文件仅 17 行 | 普通接受/执行、full Config、缓存、签名与原始 RPC 资源继续收口；`sdk_client.rs` 仍 1863 行 |
| TS-S1 / 执行安全 | 独立 quote、SOL 转账、WSOL 与 Durable 普通 SOL 聚合已有局部签名账本/故障恢复合同 | opaque SDK 和其余模式/backend、完整最终消息授权、真实故障/最终性验收 |
| NP-01-D1B2 / 普通聚合观测 | 默认关闭的后台只读扫描、keyset/年龄/批量限制、007 部分索引、原证据恢复、单次 HTTP/局部预算和 429 冷却已有源码；本轮全套 1038 通过 | 尚未部署/启用，不能覆盖全部 RPC 或全部交易；真实节点、进程信号/重启和生产迁移验收仍缺 |
| NP-03 / 基础设施 | 四个冗余生产 AppState 字段移除、资源复用已有合同 | 全服务并发/配额、缓存版本、指标及依赖方向检查 |
| NP-04 / 后端通用性 | 已有局部 fake port/runtime 测试，尚未引入新 Router 依赖 | 两种独立假构建后端跑同一金额/授权/取消/unknown 合同；不以新增空 adapter 代替验收 |
| NP-05 / SDK 升级机制 | 本地版本与历史导入线索已记录，本轮 vendor 无差异 | 可信上游 revision、可重放补丁账本、隔离升级和新协议接入演练 |
| TS-12 / 发布 | 当前旧服务健康；二进制身份可核对 | 新源码 release、迁移恢复、调用方兼容、部署/回滚和单独授权的链上验收 |

原总计划 `EXECUTION_PLAN.md` / `TRADE_SERVICE_AUDIT.md` 保留历史意义；当前 Trade Service 任务以专用主计划和本轮检查点为准。不要用目录数或测试数推算完成百分比。

## 本次 RPC 处理记录

每个配置入口串行执行一次 `getSlot`，没有交易请求。标识为 URL SHA-256 前十位，不是 key 后缀，不包含凭据：

| 入口 | 脱敏标识 | HTTP / 耗时 | 处理 |
| --- | --- | --- | --- |
| 原主 Helius | `ed4e19d0c6` | 429 / 0.040 s | 从当前活动配置移除 |
| 另一个 Helius | `37aad360c6` | 200 / 0.020 s | 设为唯一 Helius 主节点 |
| Chainstack | `d1967377fc` | 200 / 0.015 s | 第一备用 |
| Alchemy | `d40adf45df` | 200 / 0.034 s | 第二备用 |
| PublicNode | `298f383fee` | 403 / 0.015 s | 从当前活动配置移除，未把 403 解释为额度耗尽 |

轻量方法可用不代表 `getProgramAccounts` 有足够配额；本轮没有执行大范围扫描，也未查询账户控制台余额。两个 key 是否共享同一账户配额仍未核实，不能靠随机切 key 承诺提升限额。

原 `.env` 已备份为 `trade_service/.env.rpc-before-20261114`，两者权限均 0600，且均被 Git 忽略。备份含凭据，只可本地受控保留，不能提交仓库。

用户明确允许仅重启 `trade-service-db.service`。重启前运行程序与磁盘 release 的 SHA-256 相同：

```text
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f
```

20:25:55 CST 重启，约 37 秒后开始监听；启动日志显示 `rpc_lanes=3`。随后 `/health` 为 200 / `ok=true`，鉴权 `/ready` 为 200 / `ready=true`、`trade_persistence_mode=durable`、`trade_database_healthy=true`、8 个已加载钱包。Copy Trading 与 Bridge PID 未改变，三个服务均 active；未提交、模拟或自动重试任何订单。

## 建议的后续执行顺序

1. **读 RPC 治理**：项目层定义只读请求预算和节点状态；按 endpoint/method 对 429 冷却，支持受限 Retry-After、总 deadline、单次请求与有界串行备用；稳定去重所有 URL，不仅相邻去重。覆盖 blockhash、寻池、余额、确认及后台扫描，全部冷却时明确不可用，不继续撞限额。
2. **资金执行收尾**：优先补 opaque SDK 的最终不可变消息/权限/整数预算审计及广播前原签名 journal；明确 Memory/Shadow 限制。unknown 只查原签名，不能把读 RPC 的切换策略套到重建/重签交易。
3. **通用性与来源**：补两种假构建后端合同、依赖规则、SDK revision/补丁账本，再进行隔离升级/协议演练。不为此恢复 PumpAPI 或引入 Router SDK。
4. **受控发布**：在足够空间的环境产出带 source/lock/vendor 校验的制品；验证加密异机恢复、临时库迁移/回滚与调用方兼容；明确授权后逐项发布，不随配置重启混入未发布优化。

## 本轮验证命令

- `/root/http/sol-bot/trade_service`：`cargo test --locked --offline --quiet`，1038 passed，0 failed；`cargo fmt --all -- --check` 通过。
- `/root/http/sol-bot/pumpapi_zmq_bridge`：`python3 -m unittest discover -p 'test_*.py'`，10 passed。
- 仓库根：`git diff --check`、`git diff --exit-code -- trade_service/vendor/sol-trade-sdk` 通过。
- 本轮没有执行 release 构建、Copy Trading Rust 全套、SDK 升级或生产协议买卖验收；不作这几项通过的声明。
