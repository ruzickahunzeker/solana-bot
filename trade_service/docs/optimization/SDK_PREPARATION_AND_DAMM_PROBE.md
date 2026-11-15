# NP-01-A2a 普通 SDK 准备与 DAMM 零签名报价

日期：2026-11-13。状态：本阶段源码/离线验证通过；未部署。不升级 SDK/vendor/依赖，不操作生产配置、钱包、数据库或服务，不签名/广播，不恢复 PumpAPI 对接。

本文保留 A2a 历史证据。后续 A2b 已替换 lane 回退/核验，B1 已将只读 DAMM probe 构建改为公开参数的固定 profile；分别见 `SDK_LANE_ENTRY_AND_UNCERTAINTY.md` 与 `DAMM_PUBLIC_PROBE_COMPILATION.md`。下文“仍接 payer”及“不手写替代 SDK”描述的是 A2a 当时的实现，不作为 B1 当前构建事实；最终 SDK 模拟/提交仍未完全隔离。

## 改动范围

| 文件 | 实际职责 |
| --- | --- |
| `adapters/sdk/ordinary_preparation.rs` | SDK 内部的准备能力接口与普通 buy/sell 准备顺序；只提供加载参数、余额、ATA 查询、DAMM 输出探测，无提交、签名或通用 RPC 访问方法 |
| `sdk_client/lane_preparation.rs` | 私有运行适配器，借用原 SdkTradeClient/选定 lane，继续调用原 loader、余额和探测实现；不再次解析钱包或新建 SDK client/RPC |
| `adapters/sdk/damm_probe.rs` | 仅接受 RPC 和公开 wallet/instructions/blockhash/output ATA/slot，编译原 legacy Message、创建零签名 transaction 并模拟；不返回可签名句柄或广播入口 |
| `sdk_client.rs` | buy_on_lane/sell_on_lane 在原 SDK 调用前使用准备函数；报价探测的签名构造改为调用上述无 signer 参数的 RPC helper；原最终预检和 buy/sell_by_percent 仍在原位置 |

项目 Application 仍使用 TradeCommand/已有执行 port，没有引入 SDK 参数。这里的 SDK 准备结果是可编辑请求数据，不是 VerifiedRoute、不可变授权消息或 signed artifact；不可因名字含 preparation 就允许广播。

## 兼容语义

- 买入仍先 load，再选 Quote 类型/检查金额，必要时查 Quote 余额，最后查 Mint ATA；不提前或重复读取，不改变 native/USDC/USD1/custom Quote/hop 的旧差异或错误。
- 卖出仍先检查百分比、读代币余额，余额为零则不加载协议；DAMM 才校验参数 variant、强制 exact-in、做原 checked-u64 `balance * percent / 100`。保留原 overflow 和 dust 错误，不暗改成 u128。
- DAMM 仍探测后应用原 minimum_output，最后映射 SDK 请求；透传原余额/百分比给 SDK sell_by_percent。非 DAMM 不新增探测/溢出政策，不改变 ATA/费用等 A1 参数。
- 当前与原 SDK sell 指令 builder 继续共用同一组公开指令和 blockhash。探测 RPC 的 sig_verify=false、replace_recent_blockhash=false、processed、Base64、输出账户、minContextSlot、inner_instructions=false 全部保留。
- RPC 结果/错误继续由原 root 逻辑检查 program error、输出 ATA、Token Program/Mint/wallet 和正余额增量；没有重写可信报价算法、增加强制超时、替换节点或改变金额单位。
- A1 的 2,016 组最终参数摘要 `166aa348712cd4516bccbe10f132caf91acf8a2fb5aa734f06a9d38806255261` 不重写，既有 API/hash/persistence 合同继续回归。

## 明确的安全变化

原报价探测使用 Transaction::new_signed_with_payer，然后以 sig_verify=false 调 simulate。现在使用同一 Message::new_with_blockhash 的编译结果与 Transaction::new_unsigned，仅有一个全零签名槽，不再把可广播的真实签名交给报价 RPC。适用于现有 DAMM v2 探测路径，不针对特定 Mint。

新的局部拒绝：若 SDK 指令意外要求其他 signer，先返回 `DAMM v2 quote probe requires exactly one wallet signer`，不调用 RPC。原单 payer 签名构造无法为这些额外账户提供签名；本轮不提供额外 signer、临时密钥或绕过检查。

## 仍然存在的边界

- SDK 的 InstructionBuilder/SwapParams 仍要求 payer 类型，root 运行适配器仍持有完整 SDK 资源；当前 vendored DAMM builder 的卖出及账户建立 helper 已核对为读取 pubkey/组装指令，不在这里签名或发送。没有据此声称准备阶段类型完全不含密钥、OS 沙箱或 SDK 升级后仍自动安全。
- 当前启动配置 use_seed_optimize=false 保持，未暗改账户生命周期；未来启用 seed 优化需独立核对输出账户/探测/最终交易一致性。
- **只修复报价 probe 的签名。** Live 最终预检中的 SDK sell_by_percent(simulate=true)，以及普通 SDK 模拟/发送组合路径仍可能在内部签名；禁止广播 guard 仍保守拒绝 opaque SDK 模拟，不扩大能力。
- lane 选择、结果核验/回退、多个签名只取第一个、middleware、原子账本和 legacy 普通聚合缺口未修复。不能把准备失败或短期未查到签名当作可以重新签名/补单的证明。
- 取消准备会 drop 当前 future，不新增后台准备；已经接受的 executor 生命周期不因此自动撤销。准备接口不持有新的状态副本或新的幂等权威。
- 本轮没有运行真实 SDK 客户端、供应商 RPC、真实池报价/模拟、签名或广播，不证明链上可成交、最终性或上线准入。

## 离线证据

1. 修改前全套 792 项通过；原 Root buy/sell 准备步骤逐语句核对后迁移，不把未执行的原 SDK 组合调用说成已做端到端对照。
2. 八项准备测试包含 144 组 Pool/Quote/hop/模式，类型化事件验证加载、余额和 ATA 顺序；另覆盖原失败优先级、零余额/错误 variant、DAMM 三种百分比/两模式、checked-u64/dust、min_out 和六个 await 取消点。source 没有 sign/send 方法，使用公开合成参数，不是可交易池证据。
3. 六项专用 probe 测试使用内存 RpcSender：实际捕获 RPC 编码，反序列化验证消息/原始零签名槽与全部模拟选项；额外 signer 在 RPC 前拒绝，transport/program 错误和取消无新增 send/retry。消息对照不生成旧实签名，也不手写 DAMM swap 指令替代 SDK。
4. 较宽的 `damm_probe` 筛选为七项，其中包含准备模块的一个探测错误测试，不能将 8+7 当成 15 个新增测试；本阶段实际新增 14 项。

当前依赖的 solana-transaction 4.1.6 源码已核对：旧 new_signed_with_payer 使用 Message::new 后由 sign 写入 blockhash/签名；新构造保留同一消息编译与 blockhash，只不产生签名。没有为对照执行旧签名方法。

最终验证：

```bash
cargo test --locked --offline --quiet ordinary_preparation
cargo test --locked --offline --quiet adapters::sdk::damm_probe::tests
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

准备八项、专用 probe 六项、A1 参数三项分别通过；默认并发/单线程全套各 806 项通过，HTTP 158 项通过，生产 check/格式/diff 检查通过；仍仅两条原兼容导出警告，无新增抑制或 ignored。vendor/原 migration 无 diff，本轮 manifest/lock 不变，原 release 二进制不更新。以下与本轮改动前相同：

```text
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b src/pumpapi_client.rs
2246675f820b79d4939efa655ff0d8237456651c945f157059a54d4baf56b28f src/adapters/sdk/ordinary_execution.rs
e232ecb1d46726de2db85407e3a302c69a113efe413ffdc05c438937c6d71677 src/executor.rs
59a0c51b5a8c833fe8f037c5e3f5e33615a06762c970dd0df282f231e7e287c1 vendor/sol-trade-sdk/src/instruction/meteora_damm_v2.rs
```

## 下一项

继续 NP-01-A2 剩余实际 lane/middleware/opaque SDK 调用副作用与错误保留契约，调查不依赖真实签名的 build/模拟接口，再进入完整消息授权与唯一发送前签名账本。A2a 的局部准备测试或零签名 probe 不代替 NP-01-B/C/D/E，也不减少整项优化的验收要求。
