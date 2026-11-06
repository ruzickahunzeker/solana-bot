# 本地 Trade Service 迁移检查清单

更新日期：2026-11-05

## 当前结论

本机尚未搭建 Trade Service。当前可见的是 `bot_trading` 客户端，交易请求仍指向
`http://127.0.0.1:18788`，由 `trade-service-tunnel.service` 转发到
`10.9.0.15:18787`。本机没有 Trade Service 源码、二进制、数据库或监听端口，
因此不能仅修改 `TRADE_SERVICE_URL` 切换到本地。

## 已核对项目

- `bot_trading`：已有钱包绑定、幂等、报价、模拟、提交和状态回查客户端。
- `bot_trading` 已接入聚合器只读诊断：启动前读取 capabilities 和 Shadow metrics；它不使用
  这些接口选择 provider、构建交易或提交交易。旧 Trade Service 若返回 404，诊断信息只显示
  为可忽略的 capability/Shadow 错误，不阻断现有 readiness 判定。
- `pumpapi_zmq_bridge`：只负责 PumpAPI/ZMQ 数据流，不负责签名和广播。
- `position_service`：持仓捕获/跟踪服务，不是 Trade Service，不能直接替代。
- `trade-service-tunnel.service`：当前交易入口的 SSH 转发层。

## 本地迁移必须取得的材料

1. 远端 Trade Service 完整源码和固定版本（包括路由/SDK 依赖）。
2. 远端 Trade Service 的数据库及迁移脚本，特别是钱包表、幂等记录、订单状态。
3. RPC、报价服务、交易服务和鉴权配置；密钥只能通过受控迁移导入，不能从日志复制。
4. 当前钱包公钥与远端 `wallet_id` 的映射清单。
5. Router/协议配置和已验证的 pool resolver；未知协议先保持 observe/simulate。

## 切换顺序

1. 在本机以独立端口启动 Trade Service（建议先 `127.0.0.1:18787`），不改现有隧道。
2. 导入数据库和钱包后，只做 `/health`、`/ready`、钱包公钥映射和报价只读检查。
3. 用 `simulate=true` 验证 `/v1/trades`、幂等重放、状态回查和失败恢复。
4. 停止 bot，备份本地 SQLite 与远端交易数据库，记录旧隧道状态。
5. 将 `TRADE_SERVICE_URL` 临时切到本地地址，保持 `TRADE_SERVICE_TOKEN`、钱包 ID 和 API 契约不变。
6. 先恢复 dry-run；确认观测、首两笔跟随和订单审计正常后，才允许人工开启 Live。
7. 连续稳定观察后再停用隧道；出现 readiness、钱包映射或幂等异常时立即回退隧道。

## 当前禁止动作

- 不把 `TRADE_SERVICE_URL` 改成 `127.0.0.1:18787`（该端口目前无人监听）。
- 不删除或停用 SSH 隧道服务。
- 不直接开启 Alpha Shadow/Live 或 Router Live。
- 不覆盖 `bot_trading` 的钱包、SQLite、`.env` 和现有首两笔策略。
- 不把聚合器 capabilities 或 Shadow metrics 当作交易授权；Jupiter/DFlow provider 选择仍由
  Trade Service 服务端负责，Bot 只发送既有业务请求。

## 只读聚合器迁移验证记录

- pinned Cargo 工具链下 Bot 测试结果：74 passed；8 个需要本地 `TcpListener` 的测试被当前
  沙箱以 `Operation not permitted` 拒绝，未执行到业务断言。
- Trade Service HTTP 错误响应不再把响应 body 回传到 Bot 错误文本；Bot 只保留操作名和 HTTP
  状态，避免 provider URL、内部诊断或凭据进入日志/UI。
- `TradeServiceClient` 现在拒绝带 userinfo、query、fragment 或非 HTTP(S) scheme 的
  `TRADE_SERVICE_URL`；认证凭据只能通过 Bearer header 传递。URL 边界回归测试通过。
- 当前 `.env` 已固定 `TRADE_SUBMISSION_ENABLED=false`、`DRY_RUN_DEFAULT=true`，并保留
  `TRADE_SERVICE_URL=http://127.0.0.1:18788`；本次只调整提交 guard，没有启动或修改隧道。
- `PreflightReport::is_live_ready()` 现在把提交开关关闭作为明确阻断项，避免状态面板在
  simulate-only 配置下误报可实盘；新增回归测试通过。
- 新增 `scripts/verify_simulate_only_config.py`，无网络、无 token 输出地校验 Bot 的隧道地址、
 提交开关和 dry-run 默认值；当前配置验证通过。
- `bot-trading.service` 已将该脚本加入 `ExecStartPre`，并通过 `systemd-analyze verify`；
  Bot 在 simulate-only 配置不满足时不会启动。
- 在独立 staging 允许本地监听前，不声称 HTTP smoke、DFlow unavailable 持久化或 24 小时
  Shadow 已完成。

## GitHub 更新边界

GitHub 分支 `backup/sol-bot-20261102` 的更新应先导出为补丁并逐项审阅；不能直接覆盖
本机目录。当前环境无法解析 `github.com`，且本机目录没有 Git 元数据，因此本轮未执行
远程同步。可联网后先比较 Trade Service、协议 rollout、Alpha 和 Router adapter，
再按文件迁移并运行现有测试。
