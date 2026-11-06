# 全协议交易与可替换 SDK 落地规划

更新时间：2026-11-03

## 总体架构

```text
PumpAPI / ZMQ
  -> bridge: raw / observation / normalized
  -> bot_trading: 识别、首两笔跟随、Alpha、策略、风控、幂等
  -> Trade API: 稳定请求和订单模型
  -> route registry: program、pool、mint、quote、token program 验证
  -> execution backend: sdk6 / sdk9 / sdk11 / sdk-next / dbc-legacy
  -> 签名、模拟、广播和状态回查
```

Trade API、钱包、数据库、订单状态和幂等键属于稳定层。SDK 版本只存在于执行
backend 中，未来升级 SDK 不改变上层业务和数据结构。

## 协议能力分层

| 协议 | 首阶段 | 后续阶段 | 备注 |
|---|---|---|---|
| Pump | SDK backend | canary/live | 首两笔跟随优先 |
| PumpSwap | SDK backend | canary/live | 需要 pool 验证 |
| Raydium CPMM | SDK backend | canary/live | 先验证 Quote 和 Token Program |
| Raydium AMM V4 | SDK backend | canary/live | 保留旧路径回退 |
| StonkFun/LaunchLab | 独立 adapter | canary/live | 按 platform config 区分 |
| Meteora DAMM V2 | SDK backend | canary/live | 独立账户布局 |
| Raydium CLMM | observe/simulate | canary/live | tick arrays 和 bitmap |
| Meteora DLMM | observe/simulate | canary/live | bin arrays 和状态过期 |
| Orca Whirlpool | observe/simulate | canary/live | tick arrays 和 sqrt price |
| Meteora DBC | dbc-legacy/observe | canary/live | 不依赖 SDK 11 的缺失类型 |
| DAMM V1 | observe | simulate/canary | 需独立布局审计 |

事件里的协议字符串只作为候选提示。Trade Service 必须用 program ID、pool owner、
池状态、mint、quote mint 和 token program 二次验证。

## 稳定内部接口

执行 backend 统一实现：

```text
capabilities()
resolve_pool(hint) -> VerifiedRoute
quote_exact_in(route, request) -> Quote
build_buy(route, quote, params) -> Instructions
build_sell(route, quote, params) -> Instructions
simulate(instructions) -> SimulationResult
submit(instructions, idempotency_key) -> Submission
```

`DexParamEnum`、`MeteoraDammV2Params`、`RaydiumClmmParams` 等 SDK 类型只能在
对应 backend 内使用，不能进入 HTTP JSON、SQLite 或 Telegram 状态。

## 版本和目录管理

```text
/root/solana/trade_service/
  current -> releases/sdk-<version>-<commit>
  releases/
    sdk-6.0.2-legacy/
    sdk-9.x/
    sdk-11.0.1-<commit>/
    sdk-next-<commit>/
  data/
  env/
  secrets/
```

每个 release 使用固定 Cargo.lock、Rust toolchain 和依赖来源。`data/`、`.env` 和
passphrase 位于版本目录外。升级通过切换 `current` 完成，失败时切回旧 backend，
不做数据库降级。

## SDK 11.0.1 的特殊处理

官方 SDK 11.0.1 使用新的 parser/streamer 版本，并移除了旧 DBC/Zap Git 依赖。
当前 Trade Service 仍引用 `MeteoraDbcParams`，因此 DBC 必须走独立 legacy backend
或重新实现专用 adapter。不能将 SDK 6 的 DBC vendor 文件混入 SDK 11。

SDK 11.0.1 先负责 Pump、PumpSwap、Raydium、DAMM V2、DLMM/Whirlpool 的编译和
模拟验证；DBC、DAMM V1 和未确认 Launchpad 保持 observe/simulate，直到对应 adapter
完成。

## 路由和金额边界

每个 `VerifiedRoute` 保存：

```text
protocol_id, program_id, pool_id, mint, input_mint, output_mint,
quote_mint, token_program, validated_slot, expires_at,
supports_buy, supports_sell, execution_backend
```

SOL 买入、Quote 买入和 Token 卖出使用不同的整数单位。每次 quote 必须返回输入、
预计输出、最小输出、费用和过期时间。过期路由、Quote 不匹配、余额不足或模拟失败
直接拦截。

SOL 两跳只在两条池路由都通过验证、报价绑定和单笔模拟成功时启用：

```text
SOL/WSOL -> Quote Mint -> Target Token
```

Router SDK 是可插拔 backend，不能接管钱包、幂等、审计和订单状态。

## 发布阶段

1. **Observe**：Bridge 记录全部协议，未知协议进入 observation。
2. **Simulate**：Trade Service 提供 route resolve、quote 和 `simulate=true`。
3. **Shadow**：单协议、单钱包、固定幂等键，连续观察至少 24 小时。
4. **Canary**：单协议、单 Pool、单钱包、小金额和频率限制。
5. **Live**：按协议逐个开启，能力不满足时自动降级到拦截。

Alpha 继续保持 Paper；首两笔钱包跟随继续使用现有规则。Alpha Shadow、Router Live
和全协议 Live 不与 SDK 首次升级同时开启。

## 验收矩阵

每个协议至少完成：

- pool owner/program 验证；
- 买入模拟；
- 卖出模拟；
- Quote 不匹配拦截；
- 过期路由拦截；
- Token-2022（协议支持时）；
- 幂等重放不重复执行；
- 服务重启后订单回查；
- 小额 canary 买入和卖出。

SDK 升级必须通过 `cargo check --locked`、`cargo test --locked`、API JSON 回归、
钱包映射回归、订单持久化回归和每协议模拟回归后，才移动 `current`。

## 当前执行顺序

1. 保留 SDK 6.0.2 和 DBC legacy 基线。
2. 建立 SDK 11.0.1 adapter 平行 release。
3. 先完成 Pump、PumpSwap、Raydium CPMM/AMM V4、DAMM V2 模拟。
4. 加入 CLMM、DLMM、Whirlpool 的 observe/simulate。
5. 独立处理 DBC、DAMM V1、Launchpad。
6. 完成能力接口和协议 allowlist。
7. 通过 24 小时 Shadow 后，逐协议 canary。
8. 最后再评估 SDK 下一版本和 Router backend。
