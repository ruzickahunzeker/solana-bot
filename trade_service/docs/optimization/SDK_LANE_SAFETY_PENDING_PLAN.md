# SDK 通道安全待执行计划

日期：2026-11-13。本文记录用户要求暂停时的计划与中间状态；随后用户明确将持续任务恢复为 active，源码/离线工作已恢复，实际进度以 `EXECUTION_STATUS.md` 为准。不授权上线或恢复 PumpAPI 交易对接。

## 暂停时检查点（历史）

- 最近完成源码及离线验证的是 NP-01-A2a：准备流程边界与 DAMM 零签名报价 probe。历史全套并发/串行各 806 项、HTTP 158 项通过，不代表当前未完成工作树通过。
- NP-01-A2b 已有部分未验证源码：`application/trade_execution.rs` 中 typed SDK uncertainty，`adapters/sdk/ordinary_lanes.rs`，`adapters/rpc/sdk_signature.rs`，`sdk_client/lane_execution.rs` 及相关模块声明/导入。
- 旧 `sdk_client.rs` buy/sell、lane fallback、结果映射和签名核验仍在；新 controller 尚未接入。新模块引用的测试子模块尚未创建，旧调用还存在导入衔接缺口。
- 当前不是可交付构建检查点。暂停时不补代码、不测试、不回滚，不把草稿当作已生效修复；运行中的服务版本本轮未核对也不更换。

## 恢复后的实施顺序

恢复后检查点：步骤 1-6 已完成本阶段源码/离线验证，新增 25 项，全套并发/串行各 831、HTTP 160；证据与仍未解决的限制见 `SDK_LANE_ENTRY_AND_UNCERTAINTY.md`。步骤 7 尚未完成，不能将本记录理解为完整优化完成或允许部署。

| 顺序 | 任务 | 验收要求 |
| --- | --- | --- |
| 1 | 复核当前 diff、SDK sender 初始化、worker 与费用策略；明确准备和 opaque SDK 调用的边界 | 保留现有参数/校验顺序；查明隐藏 Default fallback、middleware 与任务扇出风险；不升级 vendor |
| 2 | 完成 lane controller 与 buy/sell 接入，移除被替代的旧私有循环和 helper | 仅准备阶段失败可换 lane；进入 SDK 后的任何返回、错误、超时、未查到签名均不自动调用下一 lane |
| 3 | SDK 入口校验真实 sender、middleware 和实际选用的费用策略 | sender 恰为目标 provider；拒绝隐藏 fallback/额外 sender/未审计 middleware；选用策略为单一 Normal；模拟仍保留 Default 费用规则 |
| 4 | 完成 typed uncertainty 与只读签名核验 | Live SDK 调用后结果不明使用既有 unknown；保留已知原签名，多签名保留在诊断信息；核验使用总 deadline，弱观测不冒充最终成功 |
| 5 | 新增 fake lane、内存 RPC、分类与 HTTP/临时数据库合同测试 | 覆盖准备回退、调用后禁止回退、多签名、核验错误、取消、幂等重复、文件重开及 Memory/Shadow/Durable；不访问生产钱包或 RPC |
| 6 | 完整离线回归和兼容核对，更新检查点及公开 API 状态说明 | 固定参数摘要与旧分类摘要不变；check/fmt/diff 与默认并发/串行全套通过；单独说明新的安全状态语义，不改公开字段/hash/数据库格式 |
| 7 | 另行推进 NP-01-B/C/D/E | 不可变构建与签名前授权、唯一 signed journal、受控发送/对账、legacy 聚合迁移；不能以完成 lane 修复替代这些工作 |

## 必测场景

- 准备失败允许按既有顺序尝试下一 lane；所有准备失败保持可诊断信息并脱敏 URL。
- 一旦 invoke 开始，无论成功、Err、无签名、多个签名或观察失败，都不准备/调用下一 lane。
- 单签名成功、未见签名、链上错误、身份不匹配、slot 为零、RPC 错误及整体超时分别验证。
- typed uncertainty 覆盖零/一/多个签名、外层 context 和模拟差异；旧未分类错误及 PumpAPI 分类保持原合同。
- SDK 返回签名过滤零占位并按顺序去重；多个有效签名不得静默只保留第一个而丢失其余诊断。
- 取消测试只证明本地控制流没有继续另一 lane，不能证明 SDK 后台 worker 已取消或链上未执行。
- HTTP 保持原响应字段；unknown 的 GET、同 key 重复 POST、临时数据库重开保留原请求/签名/错误，不再次执行后端。

## 验证命令（仅记录，不执行）

工作目录为 `/root/http/sol-bot/trade_service`。先完成代码与测试接入，再运行：

```bash
cargo fmt --check
cargo check --locked --offline --bin pump-trade-service
cargo test --locked --offline --quiet ordinary_lanes
cargo test --locked --offline --quiet sdk_signature
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

旧 98 样本分类摘要及 A1 的 2,016 组参数摘要不得重生成来迎合新行为；新 typed uncertainty 另增测试。复核 manifest/lock、vendor、PumpAPI 和 release 二进制相对前检查点的指纹，不覆盖已有用户改动。

## 不变边界与剩余风险

- 2026-11-15 C2 查询差异 oracle 已证实旧 `wallet_reads::token_balance` 会接受错误 Mint。后续 A3a 已将它和裸数字 ATA fallback 限于测试，真实准备改用公开地址 read port、parsed metadata snapshot 与 pre-invoke guard；原准备顺序/参数文件不改。preparation_balance 30、完整并发/串行各 1183、HTTP 199、shared_read 71、quote_balance 25、准备流程 8、参数合同 3 项以及 check/fmt/diff 已通过，仍五组原警告；最终结果见 `PREPARATION_BALANCE_ENTRY_GUARD.md`。本项不把查询 Snapshot 的 supply/SOL 请求引入热路径，不以元数据检查冒充原始账户/完整授权或共享 RPC 预算。
- PumpAPI 交易对接继续暂停；不删除既有代码、不修改运行配置、不停止 ZMQ 接收。
- 不升级 SDK/依赖，不新增 Router SDK；通用边界保持项目类型和 adapter 隔离。
- 不改公开 API 路径/字段、金额单位、钱包映射、请求 hash 或存储兼容规则。SDK 不确定结果改为 unknown 属于有意安全语义变化，必须独立说明。
- 不执行 release 构建、部署/重启、实网请求、签名或广播，不读取生产密钥/数据库，不提交或推送 GitHub，不清理磁盘。
- 单 sender/单策略只能约束已知扇出，不能保证 SDK 内部普遍恰好执行一次。SDK 后台任务可能在调用取消后继续。
- 历史截至 B4b，DAMM 最终 SDK 预检仍可能签名，保留预检与最终提交两次 SDK 调用。后续 B4c1 已将该 preview 与卖出 simulate 改为零签名只读 RPC；正式 Live 仍调用 SDK，其他方向/协议仍不得宣称无 signer。
- 本阶段没有广播前 signed journal；多签名诊断也不等于逐签名持久化账本。未知状态不得自动换 key、重签或补单。
- 恢复开发需用户明确指令；生产迁移、部署及真实交易验收另设明确窗口，不从本计划自动授权。

## A3 后续执行拆分

A3a/B1/B2 的 raw/SDK 输入与模式后，A3c 已将实际准备读接 main 同一 HTTP runtime 的第六类 profile，reader 不再持 SDK RpcClient。新增 19 项（含六项真实 HTTP），preparation_ 90、完整并发/串行各 1235、HTTP 199、shared_read 89 通过；其他专项/check/fmt/diff 最终见 `PREPARATION_SHARED_RPC_RUNTIME.md`。原顺序/参数/输入绑定保留，下一主线回到 NP-01-B 不可变 SDK 构建与最终消息授权，不把预算或 A3 子项当完整资金安全目标完成。本节不授权生产操作，不恢复 PumpAPI。

| 子项 | 后续实际交付 | 必须验证 |
| --- | --- | --- |
| A3b1 原始证据与准备 port | 已接原 processed scan + 同次 raw Mint/两个规范 ATA，核对最小 slot/数量/身份/状态/扩展/整数；扫描不可用或全未解析只能依据合法 raw 输入恢复，坏扫描不可隐藏，snapshot 不续期 | 新增 raw/RPC 16 项、preparation_ 54、完整并发/串行各 1199 与专项/check/fmt/diff 已通过，详见验收页。原准备/参数文件不变；不是全账户共识/费用/签名前授权 |
| A3b2 实际 SDK 输入绑定 | 已接真实入口，核对当前九协议的实际程序/方向/default、Pump 后缀、StonkFun wrapper、canonical 输入/声明精度/模式，DAMM preview 同样检查，实际 seed 标志保持拒绝 | 新增 domain 2 + SDK 15，preparation_ 71、完整并发/串行各 1216 及专项/检查见验收页；无 signer fixture/fake invoke，原参数 oracle 不改。不是最终 SDK message/真实 signer 路径或全方向授权 |
| A3c 准备共享读取 | 已接 main 同一 runtime，第六类一笔操作/250 ms 起点/2 秒单步含等待/4 秒整次，当前注册 URL、两笔 scan/raw，流式 512 KiB/64 KiB 与共享 429 冷却、取消 | 新增 runtime/codec 12、真实 HTTP 6、Arc 复用 1，preparation_ 90、全套并发/串行各 1235；其他专项/检查见验收页。旧 SDK reader/17 测试只作 oracle，不冒充全 SDK RPC 治理 |

A3c 保留原 3 秒新鲜度，并增加真实 HTTP 单步/整次预算；scan 超过新 2 秒单步后可按不可用政策以合法 raw 恢复，但不得续期。坏证据/过期拒绝、程序/模式绑定和 SDK 入口后不回退仍保留。其余参数/ATA/寻池/模拟/SDK 内部读不在本 profile 内，供应商/跨进程/真实延迟与生产验收仍缺，不能直接部署部分方案。

完成 A3 之后仍须推进 NP-01-B/C/D/E：全量最终不可变消息授权、opaque SDK 广播前 journal、Memory/Shadow 与其他 backend、SDK worker 取消不确定性，以及只读治理剩余路径。余额子项不替代这些门槛。

## B4a 无 signer 构建入口前序检查点

2026-11-15：A3c 最终专项/参数/check/fmt/diff 已补齐，仍五组既有生产警告。接续 B4a 已核对实际 vendor 6.0.2 的 Prepared/SwapParams/模拟/worker/编译/签名调用链，并新增九项公开地址的底层编译/费用共享状态合同；专项 9、完整并发/串行各 1244、production check/fmt/diff 通过，详见 `ORDINARY_UNSIGNED_BUILD_AUDIT.md`。

本批不改真实执行路径、不装 middleware、不改变 fee 或交易行为。公开 `compile_message` 可以不持 signer，但它不授权额外 signer/账户权限/预算/min_out/新鲜度/wire 大小；高层 Prepared 和 simulate 仍可能签名。下一 B4b 冻结实际最终协议指令/消息，再 B4c 不可变授权和真实路径接线，全部方向/协议和原 authority 广播前账本仍必须继续，不以审查/新增测试代替实现。生产/SDK 升级/PumpAPI 暂停边界不变，完整目标 active。

## B4b DAMM 候选消息前序检查点

2026-11-15：普通 DAMM 卖出/preview nominal profile 已接原 prepare/pre-invoke/invoke；私有单次 middleware 比对实际 protocol/full 指令、原 Normal 预算/具体 provider tip、单 payer/wire 和返回签名，拒绝篡改/乱序/重复/过期与迟到 callback。任意原 middleware 不放开；原参数/比例/hash/JSON/生命周期、余额原 3 秒起点与既有 SDK 入口后不换 lane 保留。共有 swap2 ABI 提取后原 576 probe 消息不变。

新增 25，专项 25、完整并发/串行各 1269、HTTP 199、probe 48、lanes 19、参数合同 3 及 check/fmt/diff 通过，仍五组既有生产警告；见 `DAMM_FINAL_CANDIDATE_GUARD.md`。实际 builder oracle 复用原未注资测试 payer 只生成指令，wire/RFC 向量测试不生成签名；没有运行真实 SDK signed builder 或节点。

仍是 nominal consistency，不是独立可信全池/费用/权限授权，也没有取得 SDK 签名后广播前控制权。B4c/C/D 继续唯一不可变可签产物和完整证据、原普通 owner 的 signed journal/CAS/独立读回/原字节 sender，扩展全部方向/协议与其他 mode/backend。取消不能撤销已经通过 callback 的签名/发送；不确定结果不新 key 重签/补单。不部署、不升级、不动生产数据或其他服务，PumpAPI 开发仍暂停，目标 active。

## B4c1 不可变产物与模拟（前序）

2026-11-15：`DammSellCandidate` 已拥有私有完整零签名交易/指令/原 profile，无 signer/RPC/sender 或可写交易出口；正式 guard 也持该产物。实际 DAMM 卖出 simulate 与 Live preview 已改固定 readonly RPC，不调用 SDK executor/sign。前后都检查原 3 秒余额，失败/取消/过期不释放正式闭包；原 SDK 方法禁播准入不放开，原参数/hash/JSON/状态与可空 signature 格式保持，模拟返回 null。

新增 20（SDK 12、RPC 7、HTTP 1），候选 37、readonly/HTTP 8、完整并发/串行各 1289、HTTP 200、probe 48、lanes 19、参数合同 3 与 check/fmt/diff 通过，五组原警告，见 `DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md`。固定 4 秒是一次逻辑 RPC deadline，底层 SDK HTTP 重试/缓冲仍在，不冒充共享 profile/一次 wire。未签名/发送/部署，不访问生产数据，不升级/修改 vendor/其他服务。

正式仍 opaque SDK，原 claims CHECK/metadata 仅适用于 aggregator SOL buy。下一按操作分型与兼容迁移、独立 SDK metadata/CAS/readback/恢复扩展原 ordinary owner，再以该原权威受控签名/原字节发送；不能伪造 Quote 或新 owner。完整独立证据授权/所有方向/协议、其他 backend/mode/worker/多签名、RPC/资源/来源及真实验收继续，目标 active，PumpAPI 开发仍暂停。

## C3a 当前原 owner 操作分型

2026-11-15：已接原 claims 的 SDK 操作分型、008 事务兼容迁移与完整命令/钱包/字面键 owner 绑定。仅 Durable Live 直接 DAMM 卖出取得新类型占用；旧 quote journal 不放宽，SDK metadata 未接，正式 SDK 入口未切换。新增 15，专项 26、并发/串行各 1304、HTTP 200、参数 3、候选 37 及 check/fmt/diff 通过，仍五组原警告，详见 `ORDINARY_SDK_CLAIM_OPERATIONS.md`。旧 DDL/行/storage class/FK/索引/依赖、失败回滚、双 pool 竞争及文件重开有隔离证据；没有生产迁移/部署/签名发送，不升级 vendor 或改其他服务。

下一 C3b 独立 SDK prepared metadata、完整授权证据和原 owner 下 CAS/独立读回，再接受控签名及落盘原字节发送/原签名恢复。不能把 C3a 占用当作 SDK signed journal，也不能重做已验证的分型以替代下一接线。全方向/协议/backend/mode 与真实环境门槛继续保留，目标 active，PumpAPI 开发仍暂停。
