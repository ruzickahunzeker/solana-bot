# TS-02-C5 校验设置与内部命令构造

日期：2026-11-13。状态：新增 7 项离线测试，全套 318 → 325，通过，无编译警告。未升级 SDK/依赖/vendor、构建 release、部署/重启、读取生产钱包/数据库、访问真实 RPC 或发送交易。

## 已实施

- `application/trade_settings.rs::TradeValidationSettings` 仅包含普通校验真正使用的 5 个值：默认 SOL 买入、默认滑点、默认小费、Via-SOL 开关/金额上限。没有钱包、RPC、凭据或完整 Config。
- 旧 `validate_trade(request, config)` 入口保持，只复制上述值，再调用 `validate_with_settings`；数字/金额/Via-SOL helper 改收窄设置。
- `TradeCommand::from_validated` 拥有原金额种类、模式、路由和执行参数选择。`CommandContext` 只携带 wallet/key/side/pool、默认模拟和已解析 PumpAPI priority fee，不接收 DTO/Config/SDK。
- 旧 DTO `to_command` 签名保持，变成参数映射桥，不再保存业务分支；原 missing buy/sell amount 错误仍来自内部 builder。
- `application/trade_execution::ensure_execution_allowed` 只接项目校验结果、默认模式和 BroadcastPolicy；旧 validation 执行检查入口仍作兼容映射。

没有新增 .env 参数、配置格式或默认值。保留原 parser、Quote/Pool 许可、金额/滑点算法、校验顺序、hash/JSON 和 schema。

## 刻意保留

- 默认值只在原对应分支使用，没有为了创建 settings 而提前校验所有默认值。
- simulate 在校验结果中保留 None/false/true，内部 builder 仍在原阶段应用默认模式。
- SDK tip 与 PumpAPI priority fee 不合并；显式 tip 与两种默认值的映射仍由旧边界规则决定。
- 命令构造不是再次执行 wire validator，不增加地址/余额/比例/协议政策；手工内部数据不因此成为交易授权。
- 禁播时完整 SDK 流水线含 simulate 仍拒绝；PumpAPI/旧聚合的独立 simulate 分支保留原策略。这个政策提取不证明其内部签名路径已全部隔离。

## 新增测试

`validation/settings_tests.rs` 3 项，无完整 Config、钱包或节点：

1. 5 字段 settings 的金额/滑点/小费默认值与显式覆盖。
2. Via-SOL feature gate 先于金额上限；原错误及恰好等于上限的接受行为。
3. 被覆盖且未使用的 NaN 默认值不引入新的提前拒绝；真正使用非法 tip/slippage 时保留原错误。这里只验证校验调用的旧惰性语义，不改变 Config::from_env 的启动配置验证。

`application/trade_command/tests.rs` 4 项，仅项目类型：

1. Quote/SOL settlement/aggregator/WSOL 与卖出比例的原金额种类，literal key、wallet 和路由保持。
2. 模拟三态 × 默认模式 × method，SDK tip 与 PumpAPI fee 保持独立，MEV/滑点原样映射。
3. 缺失买入/卖出金额保留原错误。
4. 48 组 method/aggregator/模拟三态/默认/禁播组合，原允许规则和拒绝文本保持。

旧 18 成功/19 拒绝与 hash、37 入库字节、720 记录字节、98 状态映射、48 HTTP 和实际 loader/故障/恢复回归均不更新期待值。

## 验证命令

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：325 passed、0 failed、无编译警告。Cargo.toml、Cargo.lock、现有 release SHA-256 与前批一致，vendor 与现有迁移文件未改；没有运行或替换生产二进制。

## 下一批 TS-02-C6

1. wire validator 仍直接读取 TradeRequest，且在同一模块使用原 SDK Pubkey parser；settings 收窄不等于输入/解析器已完全隔离。
2. 先审查字段使用与原校验顺序，再建立无 serde 的借用型项目输入视图；边界只映射实际使用的交易字段，不复制钱包/来源元数据或整个 DTO。
3. 评估地址 parser 的项目端口/adapter，仍调用原 SDK parser 并转字节；不能为方便解耦而改解析算法、Quote alias 或先后顺序，尤其不能把地址解析提前到原金额字段检查之前。
4. 普通验证规则逐步迁到 Application，与 wire/SDK parser 分离；Warmup 与历史 replay 也需保留各自政策，不把当前默认值重新施加到旧请求。
5. rejected 编排、发送前签名账本、VerifiedRoute、协议资产扩展/报价、SDK provenance 和真实验收继续独立推进，不因命令 builder 提取就扩展 Live 许可。
