# NP-01-B2b 暂停检查点与待执行计划

日期：2026-11-13。本文保留当时用户暂停时的中间状态与恢复清单。随后用户明确恢复 active，本清单 1-8 的本阶段源码/离线验收已完成：新增 19 项，全套并发/串行各 871、HTTP 160，check/fmt/diff 通过，新增弃用警告已移除。现状与限制见 `DAMM_SIMULATED_SWAP_OUTPUT.md`。下文 paused、未补测试及四条警告都是历史记录，不作为当前停止指令或当前未验收事实；未部署。

## 暂停时的历史中间状态

- 最近完整验证的阶段为 B2a：全套并发/串行各 852、HTTP 160 通过。该数字不能用于证明当前 B2b 工作树通过。
- 暂停前已修改 `src/sdk_client.rs`、`adapters/sdk/damm_probe.rs` 和 `damm_probe/evidence.rs`，新增 `damm_probe/output.rs` 与 `damm_probe/output/trace.rs`。
- 新生产入口拟以本笔 CPI transfer 及模拟后的八账户状态计算净输出，不再减去较早 RPC 的 output ATA 余额。innerInstructions=true，仍为固定 Legacy 消息和零签名；已有原模拟 helper 仅保留在测试条件下。
- 已启动的 `cargo check --locked --offline --bin pump-trade-service` 在暂停收尾时通过。只有 dev-profile bin check，不是 release 或测试验收；除两类原兼容导出警告，新增两条 deprecated TokenInstruction::Transfer 警告，尚未处理。
- `output.rs` 已声明测试子模块，但 `output/tests.rs` 尚未创建。没有执行 B2b 测试、完整回归或链上模拟。当前不作为可交付版本，也未替换运行中的二进制。

## 已有源码草稿的设计

1. 重用 B2a 的私有 prepared/RPC/slot 绑定；模拟请求返回同次执行后的 pool、两 Mint、两 Vault、Clock、input/output ATA。
2. 共用 snapshot 身份/扩展校验，模拟后的 input 余额不再要求仍大于原投入；必须存在 output ATA，Clock.slot 与模拟响应 slot 一致。
3. 草稿 trace parser 支持 compiled、partially decoded、fully parsed 的受限 Token 转账/输出账户初始化。只认可固定两个 root 指令组，限制组数、内层数量、索引和数据长度；无额外转账、authority 变更或任意程序旁路。
4. 要求一个精确 input transfer 与一个 output transfer，绑定用户 ATA、pool vault、authority、Mint/program/精度；当前生产 profile 必须 exact-in。Token-2022 费用用同次模拟 Mint/Clock epoch 的库实现，核对显式 declared fee 后算实际净到账。
5. 如果节点提供完整 pre/postTokenBalances，再逐账户交叉核对本笔流量、原始整数、身份及 post-account 数据。缺失或矛盾不能回退为跨 slot 余额相减；新输出 ATA 的缺失 pre 行必须有本次初始化证据。

以上为未验证草稿，不是已实现保证。暂停时没有声称 parser、费用、RPC 格式或当前根调用已通过验收。

## 恢复后必须完成

| 顺序 | 工作 | 验收 |
| --- | --- | --- |
| 1 | 复核当前 diff 与 SDK/RPC/SPL parser 格式；处理 deprecated Transfer 支持方式 | 按真实 SPL/Token-2022 语义区分 checked/unchecked，不靠全局 allow 隐藏警告；不修改依赖/vendor |
| 2 | 补 `output/tests.rs` 的真实内存 RpcSender 与公开账户/trace 样本 | 捕获新八账户/innerInstructions 请求、原零签名消息及 RPC 方法白名单；不初始化真实 SDK/signer 或连接节点 |
| 3 | 验证跨 slot 外部转入/转出不污染本笔输出 | 早期 output snapshot 与实际模拟初始余额差异很大时，结果仍只由本笔 transfer 与同次模拟状态决定，禁止旧减法 fallback |
| 4 | 验证所有 trace 编码与账户生命周期 | compiled/partial/parsed、ATA 新建/已有、两方向、SPL/Token-2022、缺失/重复/错组/坏索引/错 authority/数量/精度/程序、额外指令及上限拒绝 |
| 5 | 验证 Token-2022 净输出 | 无费用/费用舍入/最大费用/100%/epoch 切换/显式 fee 不符、Mint/Clock/所需账户扩展；使用库整数算法，不使用 uiAmount 浮点 |
| 6 | 验证可选 pre/post 同次模拟余额 | 两数组俱无可用完整 trace；只有一侧、坏 raw amount、缺 metadata、重复/越界/缺行、post-account 或 transfer 不一致均拒绝；新 ATA 缺 pre 需初始化证据 |
| 7 | 验证失败与取消不扩展发送能力 | program/transport/slot 回退、返回账户不全、取消，无 sign/send、无新 key、无自动重签；B2a 读取 deadline 保持 |
| 8 | 完整离线回归与文档更新 | 保持 B1 576 组消息、A1 2,016 组参数和旧 98 样本分类摘要；check/fmt/diff、HTTP、并发/串行全套通过后才记录完成 |

## 后续验证命令（仅记录，暂停时不执行）

工作目录：`/root/http/sol-bot/trade_service`。

```bash
cargo test --locked --offline --quiet adapters::sdk::damm_probe
cargo test --locked --offline --quiet ordinary_preparation
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo test --locked --offline --quiet existing_executor_status_mapping_fingerprint
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

新模拟选项与原 B1/B2a wire 的差异要独立记录、增加新请求报文测试，不重写历史摘要来迁就新实现。验证公开响应字段、金额、钱包/hash/存储与状态分类兼容，不把内部 RPC 变化说成对外 API 升级。

## 持续边界

- PumpAPI 交易对接保持暂停，不停止 ZMQ、不改已有 backend 配置，不修改其交易代码。
- 不升级 SDK/依赖/vendor，不构建 release、不部署/重启、不访问生产钱包或数据库、不执行真实 HTTP/RPC/签名/广播，不自动重发历史订单或推送 GitHub。
- 即使本阶段通过，也仍不是完整池开放/费用版本/全协议权限审计、DAMM 买入保护、最终 SDK 无 signer 授权或普通 signed journal；SDK worker/取消/legacy 缺口继续保留。
- 此为暂停时的历史约定：paused 不等于目标完成。用户随后已明确恢复 active，现按持续队列执行；不会因为本计划更新或阶段汇报重新暂停。
