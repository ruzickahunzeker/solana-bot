# Router Backend 灰度方案

目标：降低已缓存池交易的路由和构建延迟，同时保留 Jupiter 作为未知池和复杂多跳的备用路径。

## 边界

- 不修改现有 `/v1/trades` 请求格式。
- 不修改钱包数据库、签名边界、幂等记录和 Telegram 面板。
- 不替换 PumpAPI Local Transaction 执行器。
- 不在未部署匹配 Router 程序前开放实盘 Router 路径。
- Jupiter 继续负责通用自动寻路。

## 阶段

### 1. 探针和依赖隔离

- `router_sdk_probe/` 固定 Router SDK commit。
- 当前 `trade_service` 的 `sol-trade-sdk 6.0.2` 不升级。
- 探针只验证编译和公开 API，不加载钱包、不签名、不广播。

### 2. RouterBackend 接口

在交易服务内部抽象四步：

```text
quote -> build unsigned transaction -> simulate -> sign/send
```

Router 后端只接受已经由 PumpAPI/ZMQ 或数据库缓存提供的完整池快照；禁止在热路径调用 `getProgramAccounts`。

### 3. 第一批协议

按风险顺序：

1. PumpFun native SOL 单池。
2. PumpSwap WSOL 单池。
3. Raydium CPMM WSOL 单池。
4. StonkFun Via-SOL，要求完整的内盘/桥接池快照。

暂不接入 Meteora DBC、复杂分拆路由和未知自定义 Quote。

### 4. 双跑验收

同一信号同时生成 Router 和 Jupiter 的只读结果，但只允许一个后端提交：

- 输出数量、最小输出、滑点一致性。
- 构建耗时、模拟耗时、广播耗时分别记录。
- 指令、账户、目标 ATA、费用和最终输出 Mint 审计。
- Router 失败时只记录并转 Jupiter，不自动重复提交同一订单。

### 5. 灰度开关

建议配置：

```text
ROUTER_BACKEND_ENABLED=false
ROUTER_BACKEND_LIVE_ENABLED=false
ROUTER_BACKEND_ALLOWED_POOLS=
ROUTER_BACKEND_PROGRAM_ID=
```

只有 `enabled=true`、`live=true`、匹配的 Router 程序 ID、允许的 Pool、完整热缓存和模拟通过时，才允许 Router 实盘。

## 速度目标

目标不是承诺固定毫秒数，而是分解测量：

- `signal -> snapshot ready`
- `snapshot -> quote`
- `quote -> unsigned tx`
- `unsigned tx -> simulation`
- `simulation -> submit`

Router 只有在快照已热缓存时才应进入极速路径。冷路径仍使用 Jupiter 或原 SDK，避免为了追求速度绕过池状态和安全审计。

## 上线门槛

- Router 程序已部署且 program ID 与 SDK commit 匹配。
- 至少完成买入、卖出、滑点失败、余额不足、过期快照和重复幂等测试。
- 每个协议至少完成模拟和小额实盘买卖。
- 失败订单不会被两个后端重复广播。
- 连续观察成功率和 P50/P95 延迟后，才扩大 Pool 白名单。
