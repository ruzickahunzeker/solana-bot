# 聚合提交内部类型边界

日期：2026-11-13。TS-02 的第一批，仅覆盖独立聚合提交及共享预检边界。源码和离线验证完成，未升级 SDK、构建 release、部署、修改生产配置/数据库或发送交易。

## 本轮结构

```text
HTTP JSON -> api/dto/sol_buy -> QuoteSubmissionCommand
  -> application/quote_submission
  -> ports/quote_execution (BoundQuote / QuotePreflight / opaque Prepared)
  -> adapters/sdk/quote_submission -> 原私有预检/SDK 签名发送实现
  -> QuoteSubmissionResult + typed Outcome -> api/dto/sol_buy -> HTTP JSON
```

| 文件 | 当前责任 |
| --- | --- |
| `src/api/dto/sol_buy.rs` | 原有 7 个 wire DTO 与显式 From 转换；字段、必填性、空值和状态字符串不变 |
| `src/domain/quote.rs` | 独立报价/路线/绑定报价/预检值类型；无 SDK、HTTP 类型；serde 形状冻结为 journal-v1 快照 |
| `src/application/quote_submission_types.rs` | 非 serde 的内部命令、结果和 typed Outcome；不会直接作为 JSON 输出 |
| `src/application/ports/quote_execution.rs` | 后端边界，只暴露内部类型与 opaque Prepared；保留禁止广播检查、签名与发送分离 |
| `src/application/quote_submission.rs` | 仅依赖内部命令、结果、报价和 ports，不再 import aggregator API DTO |
| `src/adapters/sdk/quote_submission.rs` | 内部类型进入私有预检，具体 VersionedTransaction 只存在于 adapter/backend |
| `src/aggregator_quote.rs` | 旧 DTO 路径兼容重导出；私有 prepare 使用内部类型，旧公开 preflight/submit 包装处转换 wire 类型 |
| `src/http.rs` | 在请求解析后转 command，输出前将 result 转成 wire response |

内部类型是真正独立的结构，不是给 API DTO 换一个别名。生产转换是逐字段映射，不使用 JSON 往返来隐式适配。预检 32 个字段明确保留，报价数量一直是 u64，价格影响字符串不转浮点数。

Outcome 在应用内为 Pending/Simulated/Submitted/Confirmed/Finalized/Unknown/Failed；只有 API 转换映射为既有 submitting/simulated/submitted/confirmed/finalized/unknown/failed。数据库状态及转换规则不变。

## 旧合同与账本

- 先用改造前 API 类型运行 3 项基线测试，再启用内部类型；不是用新实现重新生成预期值。
- 新 `tests/fixtures/contracts/sol_buy_v1.json` 含绑定报价、完整预检和显式 confirmed/simulate 输入；包含 u64::MAX、超过 2^53 的整数、精确字符串与 null。
- 旧代码对该合成意图计算的固定指纹为 `0a0a5408fabdf8c4aa7d5ce1187245ebc3e37f7482ab8ffe11f08a87adf60bdb`，新内部快照必须保持同值。
- journal 中的 intent_json/preflight_json 继续保持 V1 形状；没有新 migration。内部报价仍 flatten 估算字段，不能无意变成嵌套 quote 对象改变指纹。
- API 和 Domain 的字段名即使当前相似，所有权不同：后续 HTTP/SDK 字段变动先改对应转换，不能顺手改历史快照。
- serde 仍服务于存量 journal-v1；未来若调整该表示，需显式版本化 codec/迁移和旧数据回归，不能只更新 fixture 哈希让测试通过。

样本只验证数据合同，不代表真实 Mint、有效预检或允许的购买金额；其中大整数故意超过实际单笔限制，不会送入 RPC。

## 验证

新增 10 项测试，当前全套 142 项通过，无编译警告；格式、差异检查通过：

1. 旧 API 报价/预检/提交响应的 3 项基线。
2. 内部报价往返、原始数量/路线顺序、原指纹不变。
3. 预检逐字段扰动检查，避免两向映射恰好抵消错误；覆盖布尔值与数值。
4. null 及旧可选字段缺失仍保持 None，不伪造模拟证据。
5. 内部命令保留四种显式布尔组合和原样幂等身份。
6. 7 种 typed Outcome 的公开字符串及完整响应字段不变。
7. 用旧格式合成样本写入临时文件账本，关闭重开后通过新 Application 读取，保持签名、字节、状态和幂等指纹。
8. 损坏的旧预检快照报错，不回退成新交易。

恢复测试使用 ReplayOnlyBackend：任何许可检查、重新报价、签名或发送调用都会 panic；成功恢复必须只读取原记录。这里不是复制生产库，也不是使用旧 release 的实盘记录验收。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline contract_tests -- --nocapture
cargo test --locked --offline --quiet
git diff --check
```

依赖边界另用源码检索确认：application 提交编排、内部类型、执行 port 和 domain quote 不 import API/aggregator/SDK/Axum 类型；具体 SDK 类型仍留在 adapter。当前这是源码检查与类型签名约束，不冒充完整 AST 架构 lint。

## 升级收益与剩余任务

SDK 的预检和 Prepared 类型变化现在主要落在 adapter/legacy backend，应用命令、结果及 HTTP 合同不必一起改。这个边界没有触碰 SDK vendor，也没有改变原报价、审计、签名、广播、MEV 或重试算法。

但整个服务尚未完全解耦：普通 TradeRequest/NormalizedTrade、QuoteStore、聚合获取/审计实现、钱包/RPC/signer 等仍有旧耦合。应用的 journal port 仍传递 V1 JSON 快照，尚未单独提取版本化 persistence codec。

下一离线批次 TS-02-B：为普通交易提取内部命令与金额/模式值，先冻结别名、默认值和 Quote 原始单位合同，不顺带改变 API、Pool 规则或开放协议。之后推进 TS-03 Registry；TS-04 收口原始 RPC/signer。真实 RPC、release、运行配置切换与部署继续单独验收。
