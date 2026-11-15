# TS-04-E0 普通执行结果隔离

日期：2026-11-13。范围：项目执行结果与原状态分类；没有接入新的执行 port，没有改变 SDK/PumpAPI 执行、签名或广播流程。

## 已实施

- `application/trade_execution.rs` 拥有 `TradeExecutionResult`：`success`、`signature`、`error` 三个原字段，不增加 serde/外部 DTO 或链类型。
- SDK wrapper 的 `ExecuteResult` 路径保留为同一类型的重导出，已有内部调用不需转换或重新构造。
- 普通 Executor、PumpAPI 不再从 SDK wrapper 导入结果类型；执行依赖方向改为 backend → Application 的项目结果。
- 原分类函数移入 `classify_result`。Executor 保留很薄的 `map_status`，仍在原模块记录同一条调用错误日志，再委托纯函数。
- 交易状态更新、持久化、参数缓存、钱包解析、blockhash、分支优先级和调用数量均没有在这次提取中修改。

这不是完整执行后端隔离：Executor 仍使用 SDK/PumpAPI 参数 mapper 和 WalletRegistry 返回的 SDK client；后续需要注入 port 与独立故障测试。

## 迁移前指纹

先在旧 `executor::map_status` 上添加并运行测试，捕获 98 组输入/输出，再固定期待值并提取函数。不是从新函数倒推新的“兼容”期待值。

输入包含：成功/失败、无签名/普通签名/空签名、无错误/警告/三类现有失败标记/大小写差异/嵌入标记/空错误、Live/模拟，以及两组带 anyhow context 的调用错误。

```text
017be4bb035ed56cdf073b646057ba8d8c92407a144e979d5f6be5c77d3f5227
```

`executor/contract_tests.rs` 共 3 项：上述联合指纹；无明确失败标记的带签名不确定结果及无签名成功报告不提升为 submitted；已知失败标记保留原签名/错误。

## 刻意保留的旧语义

| Backend 报告 | 原分类，提取后保持 |
| --- | --- |
| success=true，simulate=true | simulated，不额外校验签名 |
| success=true，Live，signature=Some | submitted，包括旧空字符串情况 |
| success=true，Live，无签名 | unknown |
| success=false，原三类失败标记 | failed，原签名/错误保留 |
| success=false，无上述标记，有签名 | unknown，即使 simulate=true 也不提升为 simulated |
| success=false，无签名 | failed |
| 调用返回 Err | failed、无签名、保留原 `error.to_string()`，原 Executor 日志不变 |

这些是兼容事实，不是新的链上真实性或最终性证明。尤其字符串标记、空签名、调用 Err 不能代替持久化签名账本和原签名对账。不能将本函数复用于新的 reconciliation，也不能据此重发历史失败订单。

## 验证与边界

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet executor::contract_tests
cargo test --locked --offline --quiet
git diff --check
```

定向 3 项、全套 270 项均通过，无编译警告；格式/diff 检查通过。原 267 项 API/hash/journal/参数回归保留。本轮没有新依赖、vendor 修改、release 构建、部署、RPC、真实钱包/数据库操作或交易；manifest/lock/原 release 指纹不变。

## 下一批 TS-04-E1

2026-11-13 续作：普通 SDK/PumpAPI 执行 port 和解析后句柄已接入，新增 12 项、全套 282 项通过，详见 `ORDINARY_EXECUTION_PORT.md`。以下保留本批结束时的执行要求；真实 HTTP 接受身份查询、AppState 解耦和发送前账本仍未完成。

1. 定义项目类型的普通交易 backend port/解析后句柄，将 SDK/PumpAPI 参数转换收回 adapter；在应用编排不公开 signer、raw RPC 或 SDK params。
2. 保持一次钱包解析后持有原对象的语义，不用“先检查存在，执行时再查”替代，避免钱包删除/替换竞态。legacy 聚合原分支在独立规划前不强行归并。
3. 固定 building/submitting 写入、缺失钱包、stale blockhash 和调用 backend 的原先后顺序；durable 写失败不得调用资金 backend，shadow 行为保持原合同。
4. 注入无钱包/无 RPC 的 fake，验证 SDK 买卖/PumpAPI 选择、参数、结果映射、禁播/持久化错误和调用次数；普通 API 与数据库格式不变。
5. legacy 聚合签名前账本、真实协议验收及完整 VerifiedRoute 仍是独立工作，不因引入 port 就宣称安全目标全部完成。
