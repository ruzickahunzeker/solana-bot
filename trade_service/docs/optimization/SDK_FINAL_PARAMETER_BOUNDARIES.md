# NP-01-A1 普通 SDK 最终请求参数与手续费

日期：2026-11-13。状态：本阶段源码/离线验证通过；未部署。不升级 SDK/依赖/vendor，不恢复 PumpAPI 对接，不初始化 SDK 客户端、读取生产数据、签名或广播。

## 实际改动

- `src/adapters/sdk/ordinary_params.rs`：集中普通 buy/sell 最终请求投影、Pool -> SDK DexType、确认等待映射、Default/选定 lane 费用策略与模拟费用策略。
- `src/adapters/sdk/trade_command.rs`：拥有原 `ExecuteTrade` SDK 执行上下文及项目 command 的转换。字段/类型保持，`sdk_client::ExecuteTrade` 原路径保留 re-export，适配器不再反向依赖 root SDK client 的类型定义。
- `src/sdk_client.rs`：原 `buy_on_lane`/`sell_on_lane` 在原阶段调用参数函数；DEX 参数加载、Quote 余额/ATA 查询、DAMM probe/minimum_output、预检、SDK buy/sell_by_percent、lane 核验和回退保持原调用关系。
- `src/sdk_client/ordinary_param_contract_tests.rs`：公开合成参数的完整字段快照与费用隔离测试，不调用协议指令 builder 或 SDK 交易方法。

这是最终请求映射，不是 unsigned transaction builder、VerifiedRoute、消息授权或新的 Live 能力。API DTO、普通接受、幂等 hash、存储和执行默认值不改变。

## 基线来源与范围

1. 修改前全套 789 项通过。
2. 先将原 inline 初始化提取成 root 局部纯函数，保留原流程的金额检查、余额/RPC 和报价顺序；仅移动无副作用的 DexType / Some(1) 常量映射。这里没有执行原 SDK 组合方法或真实交易来采样。
3. 在局部函数仍位于 root 时，三项新测试通过，采集 2,016 组参数/错误的 SHA-256。然后把该值写成固定断言，再移动函数/上下文所有权，禁止用新摘要适配后续差异。

```text
166aa348712cd4516bccbe10f132caf91acf8a2fb5aa734f06a9d38806255261
```

矩阵使用九类既有 Pool 标签、八组公开 Quote/hop/模式/确认/ATA/金额/滑点场景、七种现有配置 lane；每组一笔 buy 投影与 25/99/100% 三种 sell 投影，包含原拒绝错误。字段快照穷举 TradeBuyParams/TradeSellParams，SDK 增加字段会先导致编译检查失败，而不是被 `..` 忽略。费用按买/卖、lane/策略排序后记录整数配置和 f64 原始位，避免 Map 迭代顺序或浮点显示影响基线。

协议参数使用完整公开 BonkParams 哨兵，所有字段/Token transfer fee 透传不变；部分 Pool 标签与哨兵故意不组成可执行路线。因此这是投影矩阵，不是九类协议的有效池、builder、报价、模拟或链上成功证据。此前实际 loader 证据仍以原协议 fixtures 为准。

## 保持的关键规则

- 买入金额由原调用方先取得 u64；参数函数不重新进行 SOL/Quote 换算。快照包含 2^53+1 与 u64::MAX，避免最终请求映射引入浮点舍入。
- 卖出请求的 input_token_amount 仍为 0，实际数量仍由 SDK sell_by_percent 使用原余额/百分比计算；只有 100% 设置 close_mint_token_ata。不能把这个 0 当作可广播的有效输入数量。
- 原 ATA create/close、空 ALT、无 nonce、use_exact_sol_amount、确认和 wait_for_all_submits=false 保持，不更换 SDK Simple API 或账户生命周期策略。
- DAMM 买入仍映射原 fixed_output=Some(1)，这是旧行为，不是可信报价/min_out 验收。卖出透传原 probe + minimum_output 结果，精度/报价安全没有在此重写。
- 每笔请求单独创建费用策略，Default 买卖均保留 CU=300000、price=1000、tip=0；非 Default lane 再加入该 lane 的原小费。没有启用其他 SDK provider 或修改 MEV lane 选择。
- DAMM simulate 的最终 sell 请求使用 Default 模拟策略；Live 最终预检 clone 替换费用后不改变原请求 Temporal 小费。额外测试验证不同请求间的 fee map 不共享修改。
- 该阶段不改变 SDK simulate 内部签名、DAMM probe 的签名行为、多 lane 重构/回退、签名丢失或普通 signed journal 缺口，不可据此宣称普通提交安全已完成。

## 验证

根位置三项基线测试通过，移动后同三项及固定摘要通过。最终验证：

```bash
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

全套默认并发/单线程各 792 项通过，HTTP 158 项通过，生产 check/格式/diff 检查通过；仍仅两条原兼容导出警告，没有新增抑制或 ignored。vendor/原 migration 无 diff，本轮 manifest/lock 与 release 二进制未改，SDK 仍为 vendored 6.0.2。以下与改动前核对相同：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b src/pumpapi_client.rs
2246675f820b79d4939efa655ff0d8237456651c945f157059a54d4baf56b28f src/adapters/sdk/ordinary_execution.rs
e232ecb1d46726de2db85407e3a302c69a113efe413ffdc05c438937c6d71677 src/executor.rs
```

原禁止广播 SDK wrapper 合同继续保留；离线 SDK 参数数据不等于获准调用组合交易接口，运行二进制未更新，不能据此宣称线上已生效。

## 后续

NP-01-A 的异步准备事件/失败顺序、实际 lane 副作用与 middleware、独立无签名构建能力仍待审查和契约化；随后按工作包推进授权、唯一广播前签名账本及 legacy 普通聚合。PumpAPI 开发保持暂停，不借此增加对接或改默认 backend。

后续阶段 A2a 已接入准备 source 和 DAMM 零签名报价，详见 `SDK_PREPARATION_AND_DAMM_PROBE.md`；本文件 792/158 是 A1 的冻结记录，参数摘要不重写，不能替代后续全量结果或 SDK 最终签名/发送审计。
