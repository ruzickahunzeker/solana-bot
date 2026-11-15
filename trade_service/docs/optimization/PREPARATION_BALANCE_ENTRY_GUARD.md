# NP-01-A3a 交易准备余额与 SDK 入口检查

日期：2026-11-15。仅 Trade Service 源码及离线测试，尚未部署。SDK 仍为本地 6.0.2，不升级依赖、不构建 release、不重启服务、不访问生产钱包/数据库、不签名发送，不恢复 PumpAPI 交易对接。A3a 不是 A3 全部完成，也不是完整签名前授权。

本页保留 A3a 当时的政策与验收证据。后续原始账户核验及安全 fallback 的实际进展见 [A3b1 原始余额证据](PREPARATION_RAW_ACCOUNT_EVIDENCE.md)，不能把本页历史“全未解析拒绝/无 raw”当作后续源码现状，也不能将后续 raw 子项当作 SDK 最终输入程序已绑定。

## 实际调用边界

```text
ordinary_preparation::buy/sell
  -> LanePreparation::token_balance
  -> PreparationBalanceRead（公开钱包/Mint，无 signer）
  -> RpcPreparationBalanceRead（当前 lane 的 SDK RPC）
  -> PreparationBalanceSnapshot
  -> PreparedLaneRequest
  -> OrdinarySdkLane::validate_prepared
  -> SDK invoke
```

原 `ordinary_preparation` 与 `ordinary_params` 文件逐字保持：买入先加载参数，仅四类协议的直接非 WSOL Quote 买入读取付款币余额，SOL-hop 不额外读取；卖出仍先读目标币余额，再加载参数与必要的 DAMM 只读 probe。不插入查询 API 的 supply/SOL 额外请求，不改变有效 SDK 参数、公开金额、hash/JSON 或数据库格式。

旧 `SdkTradeClient::token_balance` 私有转发已移除，`wallet_reads::token_balance` 与数字 ATA fallback 仅保留为测试 oracle，不再是生产准备入口。C2 查询 API 自己的 fallback/显示余额政策不变，不能与本项交易准备政策混用。

## 余额元数据与拒绝规则

- 只执行当前 lane 的 `getTokenAccountsByOwner(wallet, Mint, jsonParsed/processed)`。精确 RPC 配置与 SDK 输出对照；reader 不持有 TradingClient、payer 或签名能力。
- 扫描核对钱包/Mint、实际 program owner 与解析标签、账户地址与唯一性、initialized 状态、统一 decimals/Token Program、raw u64。冻结账户不得作为输入余额。
- 正余额必须位于对应 Token Program 的 canonical ATA；不能把 SDK 无法消费的非 ATA 正余额累加为可支出量。元数据一致的零余额非 ATA 账户可以共存，不改变数量。
- 完整合法空扫描为零；混合未解析、重复、错误身份或格式均拒绝。全未解析也拒绝，不通过裸 `getTokenAccountBalance` 数字回退伪造账户证据。
- 每次最多 1000 个账户，但当前上限在 SDK 已缓冲/反序列化后检查，不是 HTTP body 内存上限。测试以 1001 个可成功解码的账户验证真实分支。
- snapshot 绑定钱包/Mint/原数量、元数据和非零 context slot；从操作开始计时，最多 3 秒，未来时间也拒绝。响应慢不能重新赋予新鲜时间；逻辑读最多 4 秒。
- canonical-only：`use_seed_optimize=true` 的余额准备输入账户尚未审计，直接拒绝，不猜测 seed 地址。

这些数据是 RPC 解析元数据，不是链上共识/原始账户证明。不独立验证 Mint owner、原始 Token Account、TLV/TransferFee/其他扩展，也不保证节点返回了完整持有集合或同 bank 原子快照。不能据此承诺 Token-2022 可支出量或完整最终消息授权。

## SDK 入口与回退

Prepared 保存同一次余额 snapshot。入口 hook 对照买卖方向、目标 Mint、实际执行 client 的 payer 公钥、买入数量/卖出比例、已捕获卖出余额以及快照身份/期限。所有卖出在进入 SDK 前用 checked u64 乘法验证 SDK 后续百分比运算，拒绝溢出及舍入为零，不更改 SDK/vendor 或重写旧参数投影。

类型化错误分三类：

| 类别 | 行为 |
| --- | --- |
| Unavailable | 仅 SDK invoke 之前允许按原顺序尝试下一 lane |
| Invalid | 终止该操作，零 SDK invoke，不靠另一 lane 掩盖坏证据 |
| Expired | 终止该操作，不更新原快照时间，不自动重新读取后交易 |

进入 invoke 时再查快照；DAMM 最终 SDK preview 成功后、正式调用前再次检查，拒绝时保留 preview 已知签名。进入 SDK 后仍按既有 typed uncertainty 处理，不尝试其他 lane、不重新签名或补单。取消只证明本地 future/control flow 被丢弃，不证明 SDK 自建 worker 或链上执行已取消。

此处仅是准备余额及输入检查。默认 hook 兼容既有 fake lane，实际 LaneExecution 显式实现检查；尚无完整 immutable message、协议/附加指令/费用/权限授权，也没有 opaque SDK 广播前签名账本。

## 离线证据

新增 30 项：domain 4、SDK 纯输入规则 7、内存 RPC 11、真实 lane controller 配 fake read port 6、SDK 入口后快照/签名保留 2。原样保留旧准备顺序与参数 oracle。

RPC 测试只使用内存 RpcSender，没有 HTTP/真实节点；证明的是逻辑方法、参数、取消与结果分类，不是 wire 次数、隐藏重试或真实供应商行为。controller 使用 fake invocation，不初始化 SDK 钱包或 signer；不将 fake invoke 次数当作实际广播证据。

最终 preparation_balance 30、完整并发/串行各 1183、HTTP 199、shared_read 71、quote_balance 25、ordinary_preparation 8、ordinary_param_contract_tests 3 项全部通过。完整并发耗时 125.65 秒、串行 208.46 秒。production `cargo check`、fmt 与 diff check 通过；仍五组既有警告（models/persistence 兼容重导出、QuoteStore 兼容方法、WsolQuote 兼容方法、wallet_balance），未添加抑制。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet preparation_balance
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet shared_read
cargo test --locked --offline --quiet quote_balance
cargo test --locked --offline --quiet ordinary_preparation
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo check --locked --offline --quiet
cargo fmt --check
git diff --check
```

## 不变项与后续

manifest `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、lock `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、原 release `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f` 不变，vendor 无 diff。executor、钱包 registry、PumpAPI root 指纹与 C2 一致。

原准备顺序 `ordinary_preparation.rs` SHA `b49859e0cb32fd4a01e381d6f356b85f5f40470e877087b00505ac6ef720b438`；参数投影 `ordinary_params.rs` SHA `9cb1bee04e18894d3a2090d24019e42099d6dbca5dde4467419b4065a3841374`。旧整文件 oracle `wallet_reads/legacy_tests.rs` SHA `d721589eac8295e32387abc5c1aa8ed146d199c0d9206677b9c5c04b94e2a666`。root SDK 仅删除旧余额转发，lane 与旧 helper 的 cfg(test) 变动是本项必要接线，不将这些文件误称原字节未改。

当前 root SDK SHA `581c01153eb702486e23ab8647304189f9eead8370653e3b819dd1a5e9b25206`；在只读内存流中补回删除的私有转发后，精确重现阶段前 SHA `012ef3263592a209566b48474c29c8fddf710c0480b024926a4272119eabcdad`，未向源文件写回旧实现。核对的是本阶段必要接线范围，不以整个脏工作树对 Git HEAD 的 diff 冒充本轮差异。

1. **A3b 原始账户/安全回退**：核验 Mint owner/decimals/初始化、Token Account 的实际程序/owner/Mint/state 与 canonical 输入、扩展白名单及可支出量。明确缺失/坏证据/不可用分类；若恢复 ATA fallback，必须读取并验证原始账户，不能只信数字。保留原有效参数与无 signer 测试。
2. **A3c 共享预算**：准备读取接同一受限 HTTP runtime，单独容量/总 deadline/体积/请求数/共享冷却及取消验收。现在仍使用 SDK RpcClient 的缓冲和内部重试，不是第六类共享 profile，不能把 4 秒逻辑 timeout 称为完整 RPC 配额治理。
3. 继续完整最终消息授权、opaque SDK 原 authority 广播前 journal、其他 mode/backend、剩余读取治理、双假构建后端与可信 SDK 来源/隔离演练。PumpAPI 暂停项仍算未完成；三服务兼容及真实节点/部署验收不由离线测试替代。

本批没有新增 migration，但整个尚未部署树已有 004–007，后续发布仍需完整备份、迁移、恢复与回滚验收。本阶段暂时拒绝未解析/非 canonical/seed 输入，部署前必须明确兼容性影响，不自动上线。
