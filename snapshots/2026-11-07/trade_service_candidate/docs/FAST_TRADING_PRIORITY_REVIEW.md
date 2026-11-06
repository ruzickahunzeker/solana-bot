# SDK 6 与 SDK 11 交易优先的最短落地路径

## 当前实施决定（2026-11-07，优先于下方历史记录）

用户指定先完成 **SDK 6.0.2 单池** 和 **SDK 11.0.1 单池及原生两跳**。这两条路线分别构建、验收和发布；Bot 的 `copy_trading` 是本机 `/root/solana/bot_trading`，与 Telegram 和外部 API 共用 Trade Service 订单入口。Jupiter 维持 quote-only，Router 与 DFlow 不参与本阶段构造、签名或发送；保留聚合器接口供 SDK 明确不支持的后续路线使用。

| 顺序 | 立即实施 | 必须留下的证据 | 当前状态 |
|---|---|---|---|
| 1. SDK 11 CPMM 两跳买入 | 保留最终 RPC 模拟的原消息；把短期决策、最低到手、唯一签名、发送前 durable 签名占位、单次发送和原签名回查接入默认关闭的 live 分支 | 错误输入拒绝、消息一致、传输超时及进程中断不重发、隔离 HTTP 买入模拟与重启回读 | 买入模拟已通过；发送路径尚未接线。卖出已完成默认关闭的相同状态机接线 |
| 2. SDK 11 单池与两跳卖出 | 对现有 Direct SDK 单池及两跳卖出补足发送故障注入、订单状态与签名回查；复核 Bot 同源订单回读 | 锁定测试、独立构建、Bot→Trade Service 模拟买卖和重启回读 | 单池及两跳卖出模拟通过；两跳卖出隔离发送代码已接线，尚无 live HTTP 故障注入和链上确认 |
| 3. SDK 6 单池 | 在**发送前**持久化本地签名；对传输超时及崩溃注入验证相同幂等键只查原签名 | 固定依赖锁定构建、单池模拟、SQLite 中断恢复、唯一发送和原签名回查 | 锁定构建、51 项测试和单池模拟已通过；传输超时能带回签名，但发送前持久化仍缺 |
| 4. 受限 live 验收 | 两版分别固定钱包、池、Mint、输入、滑点、最低到手和单个发送通道，再做小额买入与卖出 | 链上确认、交易签名、订单状态、Bot 回查一致 | 尚未进行；必须与模拟完成度分开报告 |

开发、离线故障注入和模拟验收连续推进，不以 Jupiter Shadow 的 24 小时窗口为前置条件。18887 继续 `simulate-only`；本阶段不切换 `current`、不改 18788、不广播交易。计划中的 live 验收是独立发布门槛，不能把报价、编译、模拟或 HTTP 200 当成链上成交。

## 当前执行计划（2026-11-07）

**本阶段只交付两条 SDK 路线。** `SDK` 指现有 `sol-trade-sdk 6.0.2` 单池基线；`SDK 11` 指 `sol-trade-sdk 11.0.1` 单池及其原生两跳。两者共用 Trade Service 的钱包、订单、风控、幂等和回查模型，独立构建和验收。Jupiter 保留 quote-only，Router 与 DFlow 不进入本阶段的交易构造或发送路径；聚合器接口继续保留。

| 交付项 | 目前可用的证据 | 下一项可执行工作 | 完成判据 |
|---|---|---|---|
| SDK 6 单池 | `current` 指向旧 release；发送不确定性修复位于隔离 `legacy_candidate`；锁定依赖、50 项测试和隔离 release 通过，PumpSwap 买入与 CPMM 卖出 HTTP 模拟、幂等及重启回查通过；产物 `d7adf52bb04e…` 未部署 | 补真实传输超时后的发送回查；已通过重启时 `submitting`→`unknown` 的 HTTP 幂等不重放验收，再做受限 live 预检 | 修复版小额买入及卖出分别链上确认，订单和签名回查一致 |
| SDK 11 单池 | 18887 已部署通过门槛的 simulate-only candidate；Bot Telegram handler 经本地 Telegram mock 和一次性 candidate 完成 PumpSwap 单池买入与 CPMM 单池卖出，两笔模拟及回查通过 | 用独立 Bot 进程复核单池买卖与重启回读；准备同源 live release 和受限交易预检 | 小额买入及卖出分别链上确认，订单和签名回查一致 |
| SDK 11 两跳买入 | CPMM→CPMM 的 Bot handler 模拟通过；CPMM→PumpSwap 的服务端模拟曾通过，2026-11-07 Bot handler 重跑因池价变化被最低输出保护拒绝 | 选实时有效的 PumpSwap 池和新快照补 Bot handler 验收；再将已验证的两跳指令接入单笔签名、唯一发送和订单回查，完成独立 live 预检 | 受限小额两跳买入链上确认，订单和签名回查一致 |
| SDK 11 两跳卖出 | candidate 已将 CPMM→CPMM 的 unsigned RPC 模拟绑定到短期 SDK 决策与 durable 模拟订单；隔离 HTTP 已验证持币钱包订单、参数篡改拒绝、幂等与重启回查，交易/聚合器报价/提交为 1/0/0；Bot 的 Telegram handler 经本地 mock 和一次性 candidate 验证 CPMM/USDT→WSOL 卖出模拟，订单为 1/0/0、`simulated`、无签名 | 将已验证的卖出指令接入同一笔交易的签名、唯一发送、未知状态回查和确认；再做受限 live 预检 | 受限小额两跳卖出链上确认，订单和签名回查一致 |

**执行顺序：** 优先完成 SDK 11 已支持的单池买卖和两跳买卖的 Bot 隔离模拟闭环；SDK 6 的发送安全修复独立完成锁定构建与验收。SDK 11 两跳当前只会构建和模拟，必须先实现单笔签名、唯一发送和未知状态回查，再做 live 预检。各版本分别经过“锁定构建 → 隔离 HTTP/SQLite 模拟与重启回查 → 发送路径实现及故障注入 → 指定钱包/池/金额的 live 预检 → 小额链上确认”。两版独立推进。开发和模拟无需等待 24 小时 Shadow；模拟通过不能记作实盘完成。Jupiter、Router 和 DFlow 暂不参与交易执行，保留既有聚合器接口。

**SDK 11 两跳执行接线的最小代码顺序：** 先把现有预览函数局部的 SDK 指令与编译后的 unsigned 消息提取成同一内部构造结果，模拟和后续发送必须消费相同的消息、区块哈希和保护最低到手；对 legacy 与 V1 都验证钱包是唯一签名者及报文大小。然后新增与 simulate-only 决策隔离的 live 决策，绑定钱包、公钥、两池、两 Mint、方向、输入、滑点、最低到手和幂等键。订单先持久化 `submitting` 与本地签名，再调用单个发送通道一次；发送超时或签名暂不可见时保持 `unknown`，仅按原签名回查，不重建交易或切换 lane。最后以故障注入、同幂等键重启回读和受限链上买卖验收。18887 的 simulate-only 配置与既有模拟凭据继续拒绝 live 请求。

**2026-11-07 构造函数增量：** SDK 11 两跳买入和卖出的 unsigned RPC 模拟现共用 `compile_sdk_two_hop_unsigned_transaction`，同一函数验证唯一钱包签名者，并按交易大小选择 legacy 或 V1 编码。新增两项测试覆盖版本选择与第二签名者拒绝；完整门槛为 155 项 Rust、20 项 Python、锁定 release 与 simulate-only guard，隔离产物 `845c41e9ee63…`。该产物的 CPMM→CPMM 买入隔离 HTTP 模拟为 `simulated`、938 字节 legacy、目标增量 23,818,799 ≥ 最低 23,580,611，订单/聚合器报价/提交 1/0/0；CPMM→CPMM 卖出隔离 HTTP 模拟为 `simulated`、无签名、幂等及重启回查通过，订单/聚合器报价/提交 1/0/0。构造函数目前仍只产出 unsigned 交易供模拟，live 决策、签名持久化与唯一发送尚未接入。

**签名与持久化接线增量：** 同一内部 unsigned 交易类型现在能在内存中用选定钱包签名原消息，拒绝换钱包，并验证签名确实覆盖该消息；尚无运行时调用者。durable 订单新增原子签名占位：仅 `building_tx`、`execution_method=sdk` 且明确 `simulate=false` 的订单可把签名和 `submitting` 一次写入；两个并发调用只有一个成功，重启将它恢复为带原签名的 `unknown`，模拟订单与空签名均被拒。159 项 Rust、20 项 Python 与锁定 release 门槛通过，隔离二进制 `29d22aec6bff…` 未部署。下一步是把 SDK 11 两跳执行入口连接到这个顺序：同笔构建及模拟通过 → 本地签名 → durable 占位成功 → 唯一发送 → 按原签名确认；18887 当前仍不提供 live 入口。

**2026-11-07 优先级与接线修订：** 继续只实现 SDK 6 单池和 SDK 11 单池及原生两跳，先闭合 SDK 自身的交易状态机；Jupiter、Router、DFlow 的交易执行均不作为前置条件。SDK 6 下一门槛是发送传输超时故障注入与按原签名回查。SDK 11 单池先对已有 Direct SDK 发送路径做同样的故障注入、唯一发送和订单回查，再做受限 live 预检。SDK 11 两跳卖出已将预览构建结果改为保留同一次 RPC 模拟所用的 unsigned 消息；公开预览仍只返回只读摘要。接下来把同一消息的签名接入受默认关闭的 live 入口，且必须在发送前 durable 占位；失败或超时保留签名与 `unknown`，只按签名回查。然后为两跳买入做相同接线，最后分别做受限 live 预检与小额链上买卖。开发、隔离模拟和故障注入可立即进行，无需等待 24 小时 Shadow；18887 继续 simulate-only，未通过链上验收前不计作实盘完成。该切片通过 160 项 Rust、20 项 Python 测试、锁定 release 构建与运行时保护门槛；隔离二进制 `7685fda7e90c…`，未部署。

**SDK 11 单次发送离线故障注入：** 新增对预先签名消息的 Default RPC 单次发送函数；发送前验证钱包签名和报文大小，固定 `maxRetries=0`、最低上下文槽位，并拒绝 RPC 返回不同签名。离线 RPC sender 验证成功与传输超时都只有一次 `sendTransaction`，传输超时后调用方仍持有原签名。执行器新增“先 durable 占位、后调用发送闭包一次”的组合函数；真实 SQLite 故障注入验证超时保留原签名及 `unknown` 状态映射，重复占位不会再次调用发送闭包。这两个函数尚未接入 CPMM 两跳运行时分支，18887 仍拒绝 live 请求；下一步必须把准备、签名、占位、发送和按原签名查询状态串起来，并做进程中断与 HTTP 回归。

**SDK 11 两跳卖出默认关闭的运行时接线：** 上一段记录的是接线前状态。当前源码已把 CPMM→CPMM 卖出的决策复核、fresh 双池构建、同消息 RPC 模拟、钱包签名、durable 签名占位、Default RPC 单次发送和按原签名状态回查串接；未见签名时保持 `unknown`，链上报告交易错误时记 `failed`。新增 `SDK_TWO_HOP_LIVE_ENABLED`，默认 false，且 live 还要求 `TRADE_SERVICE_SIMULATE_ONLY=false`、请求显式 `simulate=false` 和 durable SDK 决策。candidate 与 staging unit 都强制关闭这个开关；18887 不接受 live。此代码尚未做启用开关后的隔离 HTTP 故障注入或真实链上验收，不应部署为实盘 release。先验证现有模拟和 live 拒绝回归，再做可控的传输超时/进程中断测试，之后再评估受限 live 预检。

**当前边界：** 18887 已更新为二进制 `405494b7de87…` 的 SDK 11 simulate-only staging，HTTP smoke、持久化回读及只读 CPMM 两跳卖出决策通过。Bot 新二进制已在安装路径，隔离 SDK 模拟开关已启用且主提交关闭；Bot 服务仍 inactive/disabled，独立临时数据库进程启动验收通过。`current` 仍为 SDK 6 旧 release，18788 未改。SDK 6 固定 `zap-program` 提交 `064c58b3…` 已从公开 fork 取得，提交及树哈希核验通过，原锁文件未改；隔离副本补回冻结 release 的 migration SQL，固定 Rust 1.99 工具链的锁定离线 `cargo check`、50 项测试及 release 构建已通过，产物 `d7adf52bb04e…` 未部署。隔离 PumpSwap 单池买入及 CPMM 单池卖出 HTTP 模拟均达到 `simulated`、无签名，幂等与重启回查通过；人工注入的 `submitting` 中断在重启后转 `unknown`，同幂等键未重放；真实传输超时和 live 发送仍未验收。SDK 11 两跳卖出已完成候选服务的同源模拟订单和本地 Telegram handler 回查；真实 Telegram 外部 API、两版本链上买卖尚未验收。

**2026-11-07 执行修订：** SDK 6 单池和 SDK 11 单池及原生两跳是本阶段仅有的交易交付项。候选服务的 CPMM→CPMM 卖出已从只读预览推进到 `sdk_verified` 决策和 `simulated` 订单；钱包、目标池、桥池、卖出比例、滑点和模拟模式绑定同一短期凭据，重复请求与重启回读保持同一订单。Bot 新增默认关闭的 CPMM/USDT 卖出隔离入口，仅在明确目标池和经候选服务验证的 WSOL/USDT 桥池存在时生成 SOL 结算请求；主服务仍拒绝该形状。Bot 本地 Telegram handler 已通过一次性 candidate 验证：1 笔模拟卖出、零签名、零聚合器记录。下一步依次完成 SDK 11 同源 live 发送与确认预检、受限小额链上确认；SDK 6 已恢复固定依赖并通过隔离买卖模拟，后续独立走发送回查及 live 门槛。上述状态只计入代码与模拟，不计入链上交易完成。

**本次隔离复核：** 候选服务完整门槛 153 项 Rust、20 项 Python、锁定 release、simulate-only guard、审计清单与 `current`/18788 不变检查均通过；最终隔离二进制 SHA-256 `405494b7de8759977e0118d36e96732cb82e777786c1eee27798f27dfe80ea7a`。首次真实 RPC 复核遇到 `Minimum context slot has not been reached`，订单安全失败且未签名；对相同 RPC 的模拟调用加入最多 3 次短暂槽位追赶重试，保持 `min_context_slot` 和余额/最低到手校验。新产物复核为 `sdk_verified`→`simulated`，无签名，五种凭据篡改均 400，同幂等键和重启回读保持同一订单，交易/聚合器报价/提交 1/0/0。Bot 最终完整测试见本轮证据报告；安装态未变。

**最新 SDK 11 验收增量：** 隔离 Bot 用例现在允许指定新的 Mint、池 ID、协议及预期 Quote Mint，每次运行单独归档日志。默认 CPMM→CPMM 买入与单池卖出已在一次性 candidate 复验为两笔 `simulated`、零签名、交易/聚合器报价/提交 2/0/0；Bot 默认锁定测试 101 项通过。TTWO PumpSwap 历史池因最低输出保护拒绝，仍需以实时有效的非 WSOL Quote 池单独补测。证据见 `/root/solana/trade_service/evidence/staging-sdk11-20261106T151912Z/REPORT.md`。

**单池买卖补验：** 使用已知 WSOL 报价的公开 PumpSwap 池，Bot 的 `/pool` 与模拟回调完成买入；同一隔离用例完成 CPMM 单池卖出。新增断言确认 Bot 面板中的 Mint、池 ID、协议与链上核验的 Quote Mint 一致。两笔订单均回查 `simulated`、无签名、聚合器报价/提交为零。证据目录 `runs/20261106T162908Z-1958435`；它使用本地 Telegram mock，仍不代表独立 Bot 进程连接真实 Telegram API。

**两跳卖出开发增量：** 新的 CPMM→CPMM 反向报价严格按目标代币→Quote→WSOL 方向重新计算，拒绝错误池/Token Program/转账费；完整 candidate 门槛通过，独立产物哈希 `94f4abc183d3…`。该函数尚未接入交易入口，18887 仍运行先前 `f3110d074fca…` 产物。详见 `/root/solana/trade_service/evidence/sdk11-cpmm-reverse-quote-20261107/REPORT.md`。

**2026-11-07 最新卖出状态：** 上一段是反向报价阶段的历史记录。当前隔离源码已新增钱包绑定的目标代币 ATA 余额计算、fresh 双池快照、SDK 11 两跳卖出 unsigned 指令构建和可选的 `sig_verify=false` RPC 模拟。模拟代码要求目标代币余额精确扣减本笔输入、WSOL 增加达到最终最低到手，并用返回的两个账户数据核对前后余额；任何证据缺失都不设为已核验。只读 HTTP 始终 `executable=false`。完整 candidate 门槛为 **153 项 Rust、20 项 Python 测试及锁定 release 构建**，隔离二进制 SHA-256 `a8db7f79870994ce33e4373a6ffaa08c1cea107aed729f712f1badf611db4e29`，归档于 `/root/solana/trade_service/builds/candidate-a8db7f798709-*`。持币样本已经通过真实 RPC unsigned 模拟；尚无同源模拟订单或 Bot 卖出回调证据。18887 仍为 `f3110d074fca…`。此项属于 SDK 11 交付，不开启聚合器交易开发。

**一次性 HTTP 验证：** 新脚本 `scripts/verify_cpmm_sell_preview_http.py` 用钱包库副本、临时 SQLite 与随机回环端口启动最新隔离产物。未鉴权返回 401、未知钱包 400、额外字段 422、重复池 502；已知 CPMM→CPMM 池的卖出预览和 `unsigned_simulation=true` 均返回 502。只读 RPC 查询确认钱包 `2TE` 没有该目标 Mint 的代币账户，因此本次是缺少卖出资产时的失败闭合，不是成功构建或 RPC 模拟成功证据。结束后交易、聚合器报价和提交表均为 0。下一步需找到确有目标代币 ATA 余额且池仍有效的隔离钱包，再验收成功构建与 unsigned 模拟。

**有余额 RPC 模拟：** 同一隔离脚本改用钱包 `2TE` 已持有的 `3NZ9…`、链上核验的 `3NZ9…/USDT` 目标池和 `USDT/WSOL` 桥池，100% 卖出 234 最小单位。只读构建与 unsigned 模拟均返回 200；模拟交易为 906 字节 legacy 格式，目标代币扣减 234 单位，WSOL 增加 1,157,067 lamports，高于受保护最低 1,145,496 lamports，`balance_and_minimum_verified=true`。交易、聚合器报价和提交表仍为 0。证据见 `/root/solana/trade_service/evidence/sdk11-cpmm-sell-preview-20261107/REPORT.md`。这证明构建和 unsigned 模拟，不代表订单、Bot 或链上成交。

以下保留前期审查和实验记录；本节是后续执行与状态判断的准绳。

审查日期：2026-11-06

## 当前事实

- 本机 `bot_trading` 就是备份仓库的 `copy_trading`；已有 Telegram、跟单、钱包绑定、`/v1/trades` 客户端、幂等键与回查。无需再部署一个 copy_trading。
- `trade_service/current` 仍指向 SDK 6.0.2 legacy；SDK 11 candidate 的 18887 是隔离 simulate-only 实例。已有的 Jupiter 24 小时结果仅证明报价稳定，不能代表交易验收。
- SDK 11 candidate 已能对部分单池和 CPMM 桥接多跳进行决策、构建及有资金 unsigned 模拟；桥接路线仍仅能提交模拟请求。Router 未接入；Jupiter 不能提交。
- Bot 当前独立只读决策地址与交易提交互斥，`TRADE_SUBMISSION_ENABLED=false`，且启动前配置检查固定模拟模式。现有 Bot 源码中的独立决策结果只用于面板展示，不能当作 18788 的交易许可。
- 当前与备份分支的 SDK 发送代码均曾在“签名暂时不可见”时尝试另一条 lane，存在同一订单重新构造交易的风险。candidate 已改为每笔 live SDK 请求只尝试一条 lane，保留不确定状态为 `unknown`；旧 release 未改。
- 本次沙箱无法连接 systemd bus、netlink 和宿主回环监听，因此本轮无法确认宿主服务是否正在运行；不能从沙箱内的 HTTP 000 推断服务已停机。
- SDK 6 的发送修复已放入独立 `legacy_candidate`，但固定依赖 `MeteoraAg/zap-program` 的提交在本机缺失，公开拉取返回 HTTP 404；该副本尚不能编译验收。依赖未恢复期间继续推进 SDK 11，不把 SDK 6 的构建等待变成全项目等待。
- SDK 11 vendor 的 `find_pool_by_mint` 只实现 PumpSwap；candidate 的 Mint 单独查池只覆盖 Pump 与 PumpSwap。Raydium CPMM/AMM v4 需要 Bot 已观察到的池 ID，或手动 `/pool` 提供池 ID，再由 candidate 从链上验证池身份和真实 Quote Mint。没有池线索的动态 Mint 目前不能保证打开交易面板。

## 重新划定完成条件

不要再用一个“项目完成百分比”混合计算旧版单池、SDK 11 多跳、Jupiter、Router 与全部协议。按可交付交易能力验收：

本阶段明确交付两个版本：**SDK 6.0.2 是现有单池交易基线；SDK 11.0.1 是后续单池和原生多跳目标版本。** 两条线都沿用已有 Trade Service 钱包、订单、幂等与状态回查，不新建交易服务或 Bot。新功能先进入独立 release；冻结的 `current` 不在原地修改。

| 交付层 | 最小完成条件 | 现状 |
|---|---|---|
| A：SDK 6.0.2 单池交易 | Bot → Trade Service 同一钱包完成模拟、幂等回查；修复发送不确定性后，受限小额买卖链上确认 | 代码路径存在；运行态未核实；独立修复副本受缺失固定 Git 依赖阻塞，未通过构建 |
| B1：SDK 11.0.1 单池交易 | 在独立 release 复用同一订单状态机，完成单池模拟、唯一发送和链上确认 | 已通过有资金钱包库临时副本的 PumpSwap 单池模拟、幂等重试、重启回读及 live 拒绝；live 与 Bot 端到端未验收 |
| B2：SDK 11.0.1 原生两跳交易 | 已核验两池、保护报价、单笔构建、完整模拟、同一交易发送与确认 | 部分路线有有资金模拟；live 发送与 Bot 端到端未验收 |
| C：Jupiter 聚合交易 | 服务端证明适用 SDK 路线不支持，完成交易审计、模拟、提交与回查 | quote-only；Gate 2 未完成 |
| D：Router 特殊路线 | 原生 SDK 确有能力缺口，Router Program 匹配部署并完成专项验收 | 未接入；不在 A/B 的关键路径 |

## 2026-11-06 优先级修订：先交付现有 SDK 与 SDK 11

本阶段只推进 A、B1、B2 三个 SDK 交易交付项。**SDK 6.0.2 与 SDK 11.0.1 并行，SDK 6 的缺失依赖不得阻塞 SDK 11。** 完成顺序按可验证的路径决定：SDK 11 单池已经有有资金模拟证据，先接通 Bot 的隔离模拟；SDK 6 同时恢复锁定构建。Jupiter Gate 2、Router Program、DFlow、任意 Quote Mint 和全部协议覆盖移到 SDK 买卖验收之后；24 小时 Jupiter Shadow 已结束，也不再作为等待项。

这里的“现有 SDK”指当前 `trade_service/current` 使用的 **sol-trade-sdk 6.0.2**；“SDK 11”指隔离 candidate 使用的 **sol-trade-sdk 11.0.1**。执行顺序以能否形成完整交易闭环为准：先完成 SDK 11 单池 Bot 模拟与回查，再完成 SDK 11 原生两跳模拟；SDK 6 固定依赖的恢复和发送安全修复独立推进。两个版本的 live 小额链上验收各自单列，未完成前不得把模拟结果写成实盘完成。聚合器只保留接口与只读诊断，不占用本轮 SDK 交易实现的关键路径。

1. **B1：SDK 11 单池模拟端到端。** 在 Bot 增加默认关闭、只允许手动 `Simulate + SDK` 的隔离通道；路由决策、钱包 `/ready`、`/v1/trades` 和订单回查都使用同一个 18887 origin。服务端返回的 `sdk_decision_id` 必须与原钱包、公钥、交易参数和有效期绑定；本地仍保持 `TRADE_SUBMISSION_ENABLED=false`。模拟订单的回查必须明确归属 18887，不能让既有 18788 reconciler 错查或自动重提。先测单池买入，再用持有目标代币的隔离钱包测卖出；验收两个方向的模拟报告、幂等重试和重启回读，签名与广播均为零。
2. **A：SDK 6 单池基线并行恢复。** 在宿主核对 18788、`/health`、`/ready`、钱包公钥和现有 `simulate=true` 订单回查；恢复 `MeteoraAg/zap-program` 在 `Cargo.lock` 固定的准确提交后，对 `legacy_candidate` 做锁定测试与 release 构建。仅发布已通过测试的独立修复 release；`current` 原地不改。若固定依赖仍不可取得，记录阻塞并继续 B1/B2，不用可变版本或未审计的替代依赖绕过构建。
3. **B1/A：单池 live 准备。** 对每条 release 分别核对钱包、协议、池、金额、日额度、滑点、最低到手、签名前模拟和唯一发送。发送结果不确定时只按签名或订单 ID 回查；同一订单不换 lane、不重新构造、不切 Jupiter。形成具体钱包、池、金额和预检记录后，再安排受限小额买入与卖出；以链上确认和本地订单回读一致验收。SDK 6 的既有实盘能力不等于已修复 release 的验收。
4. **B2：SDK 11 原生两跳。** 从已经有资金模拟成功的 CPMM→PumpSwap 或 CPMM→AMM v4 池对选一个；逐跳验证池快照、保护报价和目标余额，确认实际交易版本及发送通道。沿用 B1 的同一订单状态机，先模拟买卖、幂等与重启回读，再做受限小额链上买卖。SDK 11 原生路线未明确不支持前不签发 Jupiter 交易候选。
5. **Bot 实盘接棒。** 只有对应 SDK release 的买卖链上验收完成后，才为 Bot 增加与该 release 同源的决策、提交和回查配置；跟单、Telegram 手动和外部 API 仍走 Trade Service 的钱包、订单、风控及幂等状态机。独立 18887 的决策 ID 不能拿到 18788 使用。

每项按“代码和测试 → 隔离 HTTP/SQLite 买卖模拟 → 具体 live 预检 → 小额链上确认 → Bot 回读”逐项验收；代码构建、报价、模拟和 HTTP 200 分别记录，不合并成一个笼统完成百分比。

SDK 6 单池、SDK 11 单池与 SDK 11 两跳各自以“链上确认的买入和卖出 + 订单/签名回查一致”为完成条件；编译、报价、模拟或 HTTP 200 不能单独算作已完成交易。Jupiter 的 Gate 2 与 24 小时 Shadow 不属于这三个 SDK 交付项的前置条件。

2026-11-06 SDK 11 单池增量：`/v1/trades` 现在对所有带 SDK 决策的请求先查询持久化幂等记录，原订单即使决策过期或进程重启也只回读；新请求仍必须通过当前决策校验，且决策交易必须使用 durable persistence。隔离 HTTP 回归使用公开 WSOL 报价 PumpSwap 池和 staging 钱包数据库的临时只读备份，得到 `sdk_verified`、`simulated`、签名为空；重复请求及重启后 GET 回查一致，live 被拒绝，交易/聚合器报价/聚合器提交记录为 1/0/0。该结果只证明单池模拟，不代表 live 或 Bot 运行级验收。

Bot 客户端新增 `submit_isolated_sdk_simulation`，只接受 `simulate=true` 的 SDK 请求，在 18887 同源核对 `/ready` 钱包公钥、取得服务端决策，再向该 origin 发送带 `sdk_decision_id` 的交易。随后已把 Telegram 手动 `Simulate + SDK` 单池分支接入，增加默认关闭的 `TRADE_ISOLATED_SDK_SIMULATION_ENABLED`；订单持久化记录 `trade_service_origin`，回查按 origin 选择 18887 或 18788，自动重提只针对 primary。临时 HTTP mock 验证 18788 不收到隔离模拟请求，live/非 SDK 在网络前拒绝，隔离订单只回查隔离 origin；Bot 锁定测试 91 项与格式/锁定检查通过。当前 `.env` 未启用该开关，Bot 运行态与有资金单池买卖尚未验收；本切片未更新服务、`current` 或 18788，也未广播交易。

Bot 隔离 release 构建与验收记录见 `/root/solana/trade_service/evidence/bot-sdk11-isolated-simulation-20261106/REPORT.md`。新产物仅保存在 `bot_trading_builds`，未覆盖 Bot 服务当前二进制；SDK 6 固定 `zap-program` 提交在本地 Git 缓存仍不存在。下一步以复制数据库和隔离进程验收 Bot 端到端单池买卖模拟，然后再考虑将新 Bot 产物与默认关闭的 opt-in 配置安装到宿主。

后续隔离 HTTP 已用 Bot 手动请求的字段形状和默认小费跑通 SDK 11 PumpSwap 单池买入：`sdk_verified`、`simulated`、重启幂等回读通过、签名为空、trade/aggregator quote/submission 为 1/0/0。原先 `main` 钱包失败由当前 SOL 余额不足导致；有资金的 `2TE` 钱包通过。`2TE` 已持有的 WSOL 在服务端核验的 WSOL/USDC CPMM 单池完成卖出模拟，同样取得 `sdk_verified`、`simulated`、重启幂等回读、签名为空和 1/0/0 记录；Bot 形状及默认小费也通过。Bot 源码已限定开放这条 CPMM/USDC 卖出形状，92 项锁定测试及新隔离 release 构建通过。Telegram 运行级买卖仍未验收，live 仍关闭。

Bot 面板复核发现普通单池没有 Simulate 按钮，WSOL mint-only 解析也到不了候选服务验证的 CPMM 池。现已在 opt-in 时由 18887 的只读 `/v1/pools/sol-hop` 填充 WSOL/USDC 卖出面板，并仅为符合 SDK 单池策略的草稿显示“模拟本笔（不广播）”按钮。回调仍走同一钱包和服务端决策绑定；94 项 Bot 锁定测试、配置 guard 与独立 release 构建通过。新产物 `sdk11-isolated-sim-437772be8f8f` 未安装，Telegram 运行级验收仍缺。

2026-11-06 最新回归：增加直接调用 Bot 手动交易入口的隔离 HTTP 测试，验证 Telegram 模拟回调所建订单状态为 `simulated`、归属 `isolated_sdk_simulation`、持久化请求含服务端 `sdk_decision_id`，且主交易入口没有收到请求。Bot 完整锁定测试现为 **95/95 通过**，格式检查通过。测试使用 mock Telegram 与 mock Trade Service；它补足代码路径和数据库证据，仍需以复制数据库的隔离进程完成实际 Bot 运行级验收。现有 release 二进制由这项仅在测试配置下编译的代码变更前构建，运行代码未变；不因此重装或切换服务。

SDK 11 两跳 Bot 接线增量见 `/root/solana/trade_service/evidence/bot-sdk11-bridge-simulation-20261106/REPORT.md`。客户端已接入观察、unsigned 预检、决策与同源模拟交易，面板允许明确池 ID 的非 WSOL Quote 买入模拟；Bot 字段形状的 CPMM→CPMM 已在隔离 candidate 真实 HTTP 上成功模拟并完成重启幂等回读。mint-only 池解析超时仍会发生，现有池线索可经 candidate 链上复核后回退；没有池线索的 Mint 仍可能打不开面板。Bot 运行级两跳验收、两跳卖出与 live 链上确认仍未完成，因此 B2 不能标记完成。

上述增量已通过 99 项 Bot 锁定测试、格式检查及模拟模式配置检查，独立 Bot release `sdk11-bridge-sim-0cec4e628ffa` 已构建并归档。当前安装的 Bot 二进制与 `current` 均未更改。下一步先用复制数据库和隔离 Bot 进程验收实际面板及订单回查，再评估安装默认关闭的 opt-in 二进制；SDK 6 仍须取得锁定的 `zap-program` 提交后完成测试与 release 构建。

隔离 candidate 随后用 Bot 字段形状完整跑通 CPMM→PumpSwap 的非 WSOL Quote 买入，得到 `sdk_verified`、`simulated`、unsigned V1 交易、签名为空和重启幂等回读；CPMM→CPMM 同样已通过。两个公开目标池都只能在本次只读检查中通过“已知池 ID → candidate 链上验证”打开，mint-only RPC 发现仍不稳定。动态 Mint 的 Bot 面板可达性取决于其缓存或 PumpAPI 事件是否提供目标池 ID，此项须在隔离 Bot 进程验收时核对。

Bot 现增加默认关闭的 `/pool <Mint> <Pool ID> <协议>` 显式池线索入口，candidate 只读解析并核验真实 Quote Mint 后打开模拟面板。隔离模式的面板准备不再依赖主交易服务钱包同步；最终交易仍由 candidate `/ready` 校验同一钱包公钥。101 项 Bot 锁定测试通过，真实候选 HTTP 的无 Quote Mint 池线索检查通过。该入口解决已知池 ID 的历史 Mint 可达性，**尚未解决没有池线索时的自动寻池**，不能作为全面协议覆盖验收。

最新独立 Bot 产物为 `sdk11-explicit-pool-sim-eefd697a9871`，未安装。candidate P0/P1 门槛在脚本更新后重跑，150 项 Rust 和 20 项 Python 测试、锁定 release、simulate-only、审计清单与 `current`/18788 不变检查均通过；产物归档 `candidate-f3110d074fca-d5c6c69fd634`。下一步在隔离进程中将真实池线索、面板按钮、钱包公钥绑定与订单回查连成同一测试，并继续恢复 SDK 6 的固定依赖。

## 2026-11-06 本轮执行顺序与检查点

**先完成 SDK 11 的已支持路线，再恢复 SDK 6 的安全修复构建。** SDK 6 已是 `current` 的旧单池基线；SDK 11 是当前可继续验证的新候选。两者不合并发布，也不让 SDK 6 的不可用固定依赖阻塞 SDK 11。Jupiter、Router 和 DFlow 本轮不进入交易构造或发送路径，聚合器接口保留供后续扩展。

1. **SDK 11 Bot 隔离运行级验收。** 使用复制的 Bot 与钱包 SQLite、独立 candidate 和关闭实盘的配置，逐一验证单池买卖、CPMM→CPMM 与 CPMM→PumpSwap 买入的实际面板按钮、钱包公钥一致、同源决策、模拟订单和重启回查。记录服务端订单 ID、模拟结果和零签名/零广播；失败项回到对应代码修复。
2. **SDK 11 补足卖出与池可达性。** 用持有目标代币的隔离钱包做原生两跳卖出模拟；对 CPMM/AMM v4 优先使用 Bot 已观察池线索并链上复核。若希望只输入 Mint 也能自动交易，须另加可信的 CPMM/AMM v4 查池能力并单独验收；手动 `/pool` 是已知池 ID 的回退，不算自动发现完成。
3. **SDK 6 构建恢复。** 取得 `Cargo.lock` 固定的 `zap-program` 提交 `064c58b317b9a85f212c0de72caea286fc72fdb4`，验证源码身份后对 `legacy_candidate` 运行锁定测试与隔离 release 构建；然后验收发送不确定时的订单回查。未取得准确依赖前，该修复副本不具备可发布条件。
4. **两版分别做 live 准备与验收。** 每版分别固定钱包、协议、池、金额、滑点、最低到手、单笔发送通道和失败回查证据；先通过模拟，再以受限小额买入和卖出的链上确认及订单回读判定该版交易完成。当前模拟证据不能替代 live 验收。

本轮 Bot 将已观察池 ID 先交给 candidate 链上验证，再尝试较慢的 mint-only 查询，避免 CPMM/AMM v4 先等待一个不支持它们的查池分支。格式检查、**101/101** 项锁定测试及独立 release 构建通过；新产物 `/root/solana/bot_trading_builds/sdk11-cached-pool-first-b30535a42d0b/bot-trading`，SHA-256 `b30535a42d0b87303b6de92b3099e36a8b234c35babe7d3cceb6782f4d426588`。该产物未安装，实际 Telegram 进程交易流仍待验收。

2026-11-07 更新：已将通过门槛的 `f3110d074fca` SDK 11 candidate 更新到 **18887 simulate-only staging**；升级前二进制与三份 SQLite 一致性备份、升级后 HTTP smoke、持久化回读及交易/聚合器报价/提交行数 122/0/0 不变的证据见 `/root/solana/trade_service/evidence/staging-sdk11-20261106T151912Z/REPORT.md`。`current` 与 18788 不变。Bot 测试现用真实 Telegram 消息/回调处理函数、本地 Telegram mock、一次性 candidate 与复制钱包库连通 `/pool` 面板、单池卖出和 CPMM→CPMM 买入模拟，两笔订单均经 isolated origin 回查到 `simulated`，一次性 candidate 三张表为 2/0/0。慢 RPC 时 candidate 可以重新核验池并刷新观察 ID；Bot 原先要求预检 ID 与旧观察 ID 相同，现改为接受格式有效的新 ID，同时继续要求后续 SDK 决策的 `route_id` 与预检 ID 完全一致。Bot 全量默认测试 101 项通过，1 项真实 candidate 集成用例显式运行通过；新独立产物 `sdk11-refreshed-observation-e594763f4b19` 未安装。该证据覆盖 Telegram handler，**不等于独立 Bot 进程连接外部 Telegram API**；CPMM→PumpSwap 的 Bot handler、两跳卖出及两版 live 买卖仍按上方顺序继续验收。观察 ID 修复、RPC 429/预检 502 的失败记录和通过证据见 `/root/solana/trade_service/evidence/bot-sdk11-observation-refresh-20261107/REPORT.md`。

## 停止条件

钱包公钥不匹配、池/Quote Mint 漂移、快照过期、模拟输出低于最低到手、交易字节超限、RPC 模拟失败、幂等状态不确定或发送后签名未查到时，当前订单只进入失败/待回查状态；不得换后端或换 lane 重下同一笔。
