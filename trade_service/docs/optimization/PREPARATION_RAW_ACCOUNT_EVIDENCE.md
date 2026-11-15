# NP-01-A3b1 原始余额账户证据

日期：2026-11-15。范围仅 Trade Service 源码及离线测试；尚未部署。不升级本地 SDK 6.0.2、不更改依赖、公开 API/金额/hash/存储合同、不访问生产钱包/数据库、不构建 release、不重启服务、不签名发送。PumpAPI backend 开发仍暂停；复用既有通用 token validator 不修改该 backend。

本项延续 `PREPARATION_BALANCE_ENTRY_GUARD.md` 的 A3a，不把单个余额读子项当作整项 A3、完整最终消息授权或共享 RPC 治理完成。

本页保留 A3b1 的原始账户政策/测试/指纹检查点。后续 SDK 输入绑定的实际进度见 [A3b2 入口绑定](PREPARATION_SDK_INPUT_BINDING.md)；本页原“绑定待完成”的说明属于 A3b1 当时状态，完整最终消息授权和 A3c 仍是独立门槛。

## 真实准备路径

`LanePreparation::token_balance` 仍通过 `PreparationBalanceRead`，原普通买卖准备顺序/参数投影不变。`RpcPreparationBalanceRead` 从只接受 parsed 元数据改为每次必须核验原始输入账户：

适用范围仍遵循原准备谓词：全部普通卖出，以及 PumpAmm/StonkFun/StonkFunSwap/RaydiumCpmm 的直接非 WSOL Quote 买入；未借此扩大到其他买入、SOL-hop 资金检查或独立兑换操作。后续全量最终输入/消息授权仍须覆盖这些尚未进入本 port 的路径。

1. 当前 lane 上执行原 processed/jsonParsed owner scan。合法解析保留 A3a 的钱包/Mint/program/精度/状态/canonical/重复/混合和数量校验；坏扫描直接终止，不靠 fallback 掩盖。
2. 合法完整扫描记录数量、程序、精度及非零 slot；全未解析记录 slot 但不采信其余额；正常不可用不采信扫描也不伪造为零。只有后两者允许继续原始账户回退。
3. 同一 RPC `getMultipleAccounts` 一笔读取 `[Mint, classic ATA, Token-2022 ATA]`，显式 base64/processed、完整 data；有扫描 slot 时使用 `minContextSlot`，并在返回后检查非零/不回退、三项数量及位置。
4. 根据实际 Mint program owner 选择唯一 canonical 输入账户。Mint 与实际输入来自这一次 raw response；验证程序/初始化、钱包/Mint、状态、权限和扩展后才取得 raw u64/decimals。
5. 合法 parsed 扫描与 raw 的数量、程序、精度不一致直接拒绝；合法空扫描也不能隐藏 raw 正余额。只有 fallback 分支可完全依据 raw canonical 输入；明确不存在该输入才是零，Mint 不存在不是零。
6. snapshot 仍从整次读取开始计时，3 秒新鲜度，scan + raw 合计共享原 4 秒逻辑 deadline；进入 SDK 的身份、数量、比例/溢出和不回退合同继续保持。

正常及 fallback 均是最多两次逻辑 RPC，而非 wire 次数：SDK 仍可能内部重试、预先缓冲 HTTP body。原 1000 扫描账户限额在 SDK 解码后生效；raw 单账户 base64 解码上限 8192 字节也不是总 HTTP body 流式上限。A3c 仍须接 main 原共享 transport/runtime，不新建平行预算框架。

## 原始账户政策

复用未改的 `pumpapi_assets::{decode_data, check_mint, check_vault}`，在普通 SDK adapter 的 `preparation_assets` 中补具体准备政策。没有复制 token 二进制布局、手写 PDA 或修改 vendor。

- envelope 必须 base64、非 executable、lamports 非零、owner 为实际 Token Program；space 若存在必须与解码长度一致。库解析初始化、SPL/Token-2022 类型和 TLV 框架，重复/未知或未审计扩展拒绝。
- Mint owner 只允许 classic SPL 或 Token-2022；WSOL/USDC/USD1 的固定 Mint 额外要求 classic program 和 9/6/6 精度，显示简称不作为判断依据。
- 实际输入只能是该 program 的 canonical ATA。原始 token Mint、钱包 owner、initialized 状态必须匹配；frozen、delegate/close authority 和残留 delegated_amount 拒绝，不能用扫描的另一个账户合计代替。
- 未使用的另一个 program 的 ATA 通常缺失；合法系统空账户预存 SOL 不造成 token program 冲突，也不计入 token units。其他异常账户仍拒绝，不将其解释为本次付款账户。
- 非 WSOL 的 native 标记拒绝，WSOL 必须有 native reserve 且 lamports 足以覆盖 reserve + raw units（checked add）；非原生输入不能超过同次 Mint supply。
- Token-2022 保留既有 Mint 扩展白名单，TransferFee 两份费率均须不超过 10000 bps 且 epoch 有序；active TransferHook 和未审计扩展（如 NonTransferable）拒绝。实际 token account 必须有 Mint 所要求的扩展；fee/hook/immutable owner 等固定扩展用库 getter 验证长度/状态。
- raw amount 不包含单列 withheld_amount，也不使用利息/UI 浮点金额。TokenMetadata 只作非执行数据保留，检查的是 TLV framing，不声称解释其业务内容；没有推导 epoch 输出费率、min_out 或最终可交易报价。

这是来自同一 RPC 的原始账户布局/身份/状态证据，不是独立共识证明。scan 与 raw 可能不同 slot：只保证 raw 不早于 scan，不宣称相同 bank；发生数量变化会保守拒绝，不能重新打时间、自动换路线或重签。缺失/不可用/坏证据不能相互混用。

## 离线验证

本阶段新增 16 项（raw asset 10、RPC 6），不是把原用例重新计为新增。最终 preparation_ 54（包含既有准备顺序 8）、preparation_assets 10、preparation_balance 36、完整并发/串行各 1199、HTTP 199、shared_read 71、quote_balance 25、ordinary_param_contract_tests 3 项全部通过。完整并发 129.08 秒、串行 214.07 秒。production check/fmt/diff 通过，仍五组既有警告，未新增抑制。所有新 RPC 测试使用内存 RpcSender，无真实节点/HTTP/签名，不能当实际 wire/供应商证据。

新增 raw 夹具覆盖 SPL/Token-2022 的 raw 大整数、canonical 派生、缺失/错位/程序/权限/状态、base64/长度、扩展/费率/required account extensions、withheld 不累加、native reserve/残留委托/supply 与系统预存款。RPC 用例捕获精确键/配置与最小 slot，覆盖 parsed/raw 差异、坏扫描零 fallback、不可用/未解析安全恢复、原始结果错误/取消/整次 deadline。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet preparation_
cargo test --locked --offline --quiet preparation_assets
cargo test --locked --offline --quiet preparation_balance
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet shared_read
cargo test --locked --offline --quiet quote_balance
cargo test --locked --offline --quiet ordinary_param_contract_tests
cargo check --locked --offline --quiet
cargo fmt --check
git diff --check
```

## 剩余门槛

**A3b2：SDK 实际输入程序与证据绑定** 仍需完成。现在 raw port 已验证 Mint 与 canonical 输入，但 SDK 最终参数/指令可能采用另一 Token Program，不能把它们视为天然一致。例如 vendored Pump builder 的 default/`*.pump` Token-2022 规则，以及 StonkFun/多跳 wrapper 的实际 meme 端点需要逐项核对。后续须在真实 pre-invoke guard 中将捕获的 program/decimals/输入与实际 SDK 参数绑定，覆盖各协议和 wrapper、默认/反向/错误身份的无 signer fixture，保留原有效参数。不可直接改写 raw owner 或重写 SDK 默认来让测试通过。

**A3c：实际共享 HTTP 治理** 仍需完成，不能把本项的 4 秒逻辑 timeout/两次方法调用称作无隐藏重试/流式 body 上限或第六类共享 profile。

其他完整目标不缩减：不可变最终消息/账户权限/费用和 min_out 授权，opaque SDK 原 authority 广播前 signed journal、取消/多签名与其他 mode/backend、剩余读取治理、双假构建后端、依赖方向、可信 SDK 来源/升级演练、真实节点及生产迁移/发布/恢复验收。既有 PumpAPI 暂停项继续计入未完成。

本批无 migration/配置变化；整个尚未部署树已有 004–007，后续发布仍须独立备份/迁移/回滚授权。不能据此直接上线 A3a/A3b 的中间方案。

## 指纹与范围核对

manifest `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、lock `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、原 release `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f` 与 A3a 一致，vendor 无 diff。

原普通准备/参数文件 SHA 仍为 `b49859e0cb32fd4a01e381d6f356b85f5f40470e877087b00505ac6ef720b438` / `9cb1bee04e18894d3a2090d24019e42099d6dbca5dde4467419b4065a3841374`；实际 lane_execution/lane_preparation SHA 仍为 `7decf8dec935095d861f135c876fed88cd65b0e99139a6b44bd2810dc4610507` / `77ed94b962d712847c4a0e74ac4b6125227e0fe0c836f4f972cc967f1e20c291`。

被复用的 pumpapi_assets SHA `e75da06c2a9ca20b09b5c7f50488b60a3d0c43f2fe280dfb273a1279e4ea87f0` 不变；root SDK `581c01153eb702486e23ab8647304189f9eead8370653e3b819dd1a5e9b25206`、executor/钱包 registry/PumpAPI root 与 A3a 相同。未修改 Copy/Bridge/sdk9；这些目录的既有脏改动不在本轮处理。
