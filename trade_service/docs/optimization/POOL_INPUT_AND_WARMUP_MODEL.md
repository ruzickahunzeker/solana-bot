# TS-02-C8 只读 Pool 输入与 Warmup 结果

日期：2026-11-13。状态：新增 5 项离线测试，全套 340 → 345，通过。未升级 SDK/依赖/vendor、构建 release、部署/重启、读取生产钱包/数据库、访问真实 RPC 或发送交易。

## 已实施

- `application/trade_input::WarmupInput` 是 4 字段无 serde 的借用视图；DTO 只映射原 pool/Mint/Pool ID/Quote，无新参数或默认值。
- `application/validated_trade::ValidatedWarmup` 只持有项目 Address；旧 `validation::NormalizedWarmup` 保留重导出路径，DTO→route hint 不再转换 SDK Pubkey。
- Warmup 输入政策归 `TradeValidator::validate_warmup`，共享原 parser port 和 Registry 政策。Mint → Pool ID → Quote/别名 → Quote 政策 → SDK Pool ID 政策的顺序不变。
- resolve/hop 的 DTO 不再直接解析 SDK 地址；HTTP 在原鉴权/feature-gate 之后调用兼容解析入口，入口注入同一个原 SDK parser。
- `parse_query_address` 仅解析原始语法，不检查 required、不应用 Quote alias。Warmup 原政策 helper 改为私有，避免边界按错误次序拼装规则。
- `api/dto/pools.rs`、兼容 `validation.rs`、项目输入/结果/政策模块都不直接引用 Solana SDK/链地址类型。具体原 parser 仍在 adapter，没有重写语法算法。

`ValidatedWarmup` 只表示输入政策通过，不表示链上池存在、owner/Quote/Token Program 正确、可签名或可执行。输出继续是 PoolRouteHint，不升级为 VerifiedRoute。

## 刻意保持的差异

| 场景 | 行为 |
| --- | --- |
| Warmup 空白 Mint/Pool ID | 原 `is required`，不调用 parser |
| Warmup 空 Quote | 原 `quote_mint must be a valid pubkey` |
| resolve/hop 空白地址 | 原 `must be a valid pubkey`，仍进入纯语法 parser |
| Warmup 的 Quote 为 SOL alias | 规范化为 WSOL；Mint/Pool ID 不改 |
| resolve Mint 或 hop Quote 为 SOL alias | 保留其字节，不变成 WSOL |
| hop 功能关闭 | 原 gate 优先，不解析地址或访问后端 |

没有修改寻池策略、RPC lanes/超时/fallback、缓存键/写入时点、SDK loader、MEV 或交易能力。

## 新增验证

1. 原生 Warmup 输入的三类地址保持独立，仅 Quote 按原规则归一化；不需要普通交易的金额/模式/settings。
2. Warmup Mint/Pool/Quote 错误顺序、空白池地址与 Quote 政策先于必需池政策的调用轨迹。
3. raw query parser 原始语法错误、原因链、调用参数以及不应用 Warmup alias/required 规则。
4. DTO 的 4 字段借用与 None 映射，项目结果直接复制到 route hint，不经过 SDK。
5. 真实 Router 中空白/缺省错误、零后端调用、resolve 原始 SOL 与 Warmup Quote 规范化的对比；继续使用 fake 后端，不访问真实节点。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet warmup
cargo test --locked --offline --quiet
git diff --check
```

结果：精准 16 项、全套 345 项，0 failed，无编译警告。冻结的 108 组 Registry/Quote/Pool ID 指纹与原真实 Router 期待值不变；金额/hash/持久化/loader/故障恢复回归通过。manifest/lock/现有 release SHA-256 保持，vendor/原迁移 diff 为空。

补充复核：`cargo test --locked --offline --quiet -- --test-threads=1` 同样 345 项通过；`cargo test --locked --offline --quiet http_contract_tests` 的 51 项全部通过。没有运行 release 构建或启动服务。

## 下一执行批次 TS-04-E3

普通 SDK/PumpAPI 主分支已用项目执行 port，但 Executor 的 `aggregator_sol` 分支仍读取具体 WalletRegistry 并直接调用 aggregator_quote 的 wire helper。下一批按以下顺序推进：

1. 冻结现有分支优先级、钱包 lookup、金额/滑点错误、BuildingTx 后的报价→绑定→提交顺序及旧状态分类。
2. 将此分支的 quote/binding/legacy-submit 收入已有 adapter 边界，使普通 Executor 不获取 SDK 钱包或引用 SOL-buy wire DTO；共享原钱包 Arc 和 QuoteStore，不每次重建 SDK/RPC。
3. 经 fake provider/钱包/报价源验证 simulate/live 选择、失败步骤停止、零自动重试、原持久化与 HTTP 幂等；必须继续允许真正离线且无 signer 的测试。
4. 不在目录提取中改成独立 quote journal、不改变 legacy unknown 的签名形态，不悄悄扩大安全承诺；发送前账本缺口保留单独 TS-S1 后续任务。
5. 通过完整合同后再推进普通 Application 编排与 Handler 拆分；协议证据与 raw RPC/signer 的其他路径继续按独立任务收口。

本批没有完成整体 Trade Service 优化、SDK 来源核实、升级演练或生产验收。上述缺口不阻塞下一批离线边界工作，也不构成自行上线的授权。
