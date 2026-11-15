# TS-02-B 普通交易内部模型边界

日期：2026-11-13。状态：源码和离线回归通过；未构建 release、未部署、未做链上模拟或实盘。

## 已实施

- `api/dto/trades.rs` 接管普通与批量交易的请求/响应 DTO；`models.rs` 保留原导入路径重导出。
- `domain/trade.rs` 保存项目拥有的协议、方向、执行方式、MEV、状态及结算身份。共享枚举沿用原 serde 名称和别名，不新增协议或公开字段。
- 新增内部 `TradeAmount`：SOL 整数 lamports、绑定 Quote Mint 的原始 units、卖出百分比；不凭空推断自定义 Quote 的 decimals。
- 地址使用项目拥有的 32 字节 `Address`，仅边界转换为现有链类型。Domain 不依赖 Solana/SDK、Axum 或 SQLx；base58 展示使用项目已有的 bs58。
- `application/trade_command.rs` 保存已解析钱包 ID、原样幂等键、路由提示和执行参数。内部命令不实现 serde，不成为新的 API 或存储快照。
- Handler 在既有校验/禁止广播检查后、幂等保留前转换命令；普通和批量交易继续共享同一入口。转换缺失规范化金额时返回错误，不 panic、不保留键、不启动执行任务。
- Executor 不再接收普通 `TradeRequest` 或 `NormalizedTrade`；SDK/PumpAPI 参数分别由 `adapters/sdk/trade_command.rs` 和 `adapters/pumpapi/mod.rs` 显式映射。

## 保留行为

1. `validation.rs` 的原金额、Pool、Quote、MEV、Via-SOL 校验流程未改动。
2. `NormalizedTrade` 仍作为边界规范化和旧持久化指纹输入，不是假称已完成全服务领域化。
3. `persistence::request_hash`、请求 JSON 格式、数据库 schema 和 reservation 顺序未改动；旧模拟字段 `None/false/true` 继续参与旧 hash，不能用解析后的二态模式替换。
4. 内部模式仅解析执行时的布尔值，`ENABLE_SIMULATE` 仍是默认值，不是强制禁播。Executor 与 SDK wrapper 的既有禁播检查保留。
5. 原 `amount_lamports` 名称曾承载 Quote 原始单位；SDK adapter 为兼容仍填写该字段，内部金额明确区分 SOL/Quote。WSOL 路径保持原付款语义。
6. PumpAPI 默认 priority fee 与 SDK 默认 tip 分开保存；不能直接使用规范化 SDK tip 代替 PumpAPI 的旧默认值，显式 0 和负零也保持原浮点位模式。
7. `aggregator_sol` 优先于 PumpAPI 的旧分支顺序、缓存键、日志身份与交易状态转换保持不变。路由仍需既有链上核验，内部 Route 不等于 VerifiedRoute。

## 冻结与测试

`tests/fixtures/contracts/trade_normalization_v1.json` 包含 18 组成功请求与 19 组错误请求，均为合成数据。包含超出正常交易限额的精度样本，不可用作真实交易参数。

在迁移 DTO/激活命令前，使用旧类型和旧校验函数捕获：请求 serde 值、全部 NormalizedTrade 字段、旧 durable hash 的联合 SHA-256：

```text
cf2a091b670a8e19841659134a122f1a42f6817371f96c7ae4759ff19b838322
```

迁移后相同指纹通过。不得重新生成这个值来接受行为变化。

新增 8 项离线测试覆盖：冻结合同、原错误顺序、全部 SDK/PumpAPI 参数映射、SOL/Quote 分类、模式默认值及 hash 三态、独立手续费默认值、身份/金额别名/metadata 的旧 hash 语义、非有限值和缺失规范化数量。原 Executor 禁播回归已改用内部命令，并继续通过。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：150 passed，0 failed，无编译警告。没有加载生产 `.env`、创建 RPC 客户端、签名或发送交易。Manifest/lock、vendor 与原 release 指纹保持上一轮值。

## 尚未完成

- 普通交易接受/校验、TradeStore 与持久化仍有旧 DTO 耦合；没有把整个普通交易编排迁入 Application。
- Executor 仍依赖 AppState、具体钱包 SDK wrapper、blockhash 和聚合器旧 API 类型；两个新 adapter 仅负责参数映射，不是完整 Backend ports。
- 普通交易内聚合发送仍走旧 submit，不因独立 Quote journal 已完成而宣称拥有同样的发送前签名账本。
- TradeRoute 只承载语法已校验提示；owner、slot、Token Program、路由时效与 VerifiedRoute 留待后续 resolver 边界。
- 没有改变 SDK、协议能力或实盘开关。后续推进 TS-03 Registry，再推进 TS-04 raw RPC/signer 收口。
