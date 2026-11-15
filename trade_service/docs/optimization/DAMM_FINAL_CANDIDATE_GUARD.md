# NP-01-B4b DAMM 正式候选消息冻结与签名前核对

日期：2026-11-15。接续 `ORDINARY_UNSIGNED_BUILD_AUDIT.md`。仅主 Trade Service 源码/离线验证；不升级 SDK、不改 vendor、API、原校验/hash/JSON/存储格式，不部署/重启、不访问生产钱包/数据库、不签名广播。PumpAPI 开发仍暂停，Copy/Bridge/sdk9 不修改。

本页保留 B4b 历史。后续 [B4c1](DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md) 已将完整产物接实际无签名 simulation/Live preview，正式 guard 也改持产物；本页的两次 SDK 调用/preview 签名描述与源码指纹不代表最新接线，历史测试数字不重写。

## 实际接线与范围

真实 `LaneExecution.prepare` 在原 DAMM 卖出准备完成后，以原 balance/percent、请求参数、公开钱包与原余额证据生成私有 `DammSellMessageProfile`；正式请求与已有模拟 preview 分别冻结。`validate_prepared` 和实际 SDK 调用前再次检查请求/context，实际 `invoke_sell` 只为该本次 SDK client 安装项目私有 guard。其余普通协议和买入仍走原路径，不是另建未使用的 builder。

原 lane 的任意 middleware 仍拒绝，原单 sender/单策略约束不放宽；已加载钱包和 client Arc 权威不变。guard 的构造/纯指令投影/编译不接 Keypair、RPC、sender 或生产配置，SDK client 只在既有 invocation 闭包中使用。源码仍调用 opaque SDK，**尚未隔离最终签名能力或加入普通 SDK 广播前账本**。

```text
原准备/本笔净报价 -> 原 request + 原余额 snapshot
    -> 冻结 nominal protocol profile（正式 / preview 各自独立）
    -> pre-invoke request/context/input 再核对
    -> SDK protocol callback：逐项完全相等，只原样返回
    -> SDK full callback：具体 tip + CU + 原业务指令 / 编译 / wire 上限
    -> 原 SDK 编译签名/模拟或发送（未在本轮运行）
    -> 用原钱包和被冻结 message 核对全部返回签名
```

这补上了 DAMM 卖出 nominal candidate 一致性检查，不代表已获得独立可信的协议/费用授权：profile 仍来自现有加载/报价证据，不应将“与请求一致”误写成全池/全扩展/全费率版本已审计。

## 冻结与拒绝规则

| 阶段 | 实际规则 |
| --- | --- |
| 原意图 | Pool/Mint/Quote/原 pool_id、仅卖出、无 hop、原钱包/provider/版本/seed、balance/percent、simulate、滑点、tip、hash 与等待规则绑定 |
| 原 request | 保留中间 `input_token_amount=0`，最终量使用原 checked-u64 `balance * percent / 100`；原零/溢出拒绝，不改另一种舍入 |
| 净输出 | 原正式 fixed minimum 必须非零且冻结；不能以 probe 固定 `1` 替换；probe 原有三种模式与指纹保留 |
| 资产/程序 | 对向池两侧、canonical ATA/program、原输入 raw/SDK 证据复用。WSOL/USDC 输出必须 classic Token；不证明输出 Mint/Pool 全部 raw 证据或 Token 扩展已完成 |
| 生命周期 | 原创建输出 ATA、保留输出账户；仅 percent=100 关闭输入。不会因显示 SOL 添加解包或改变金额 |
| 费用 | sell strategies 必须原 Normal：Default 零 tip，正式非 Default 的唯一 provider 使用原 tip，CU limit=300000/price=1000；原 simulate 仅 Default。clone 后更改不能改变冻结预算 |
| 完整指令 | 正式非 Default 正小费为一个原 System transfer，目的地址属于当前 vendor 的对应 provider 列表；其后 CU price、CU limit、原业务指令。Default/simulate 无 relay tip。不删/改/重排实际指令 |
| 消息 | 当前 V0-compatible/no-ALT/no-nonce/canonical profile 实际产生 Legacy；编译后只允许一个原 payer，零签名完整交易编码不超过 1232。绑定原 hash，但独立链上有效期证明仍待续 |
| 回调/取消 | cloned middleware 共享 Pending -> Protocol -> Frozen；乱序/重复/篡改变成拒绝，不能第二次释放 full。RAII 在返回或本地取消时撤销迟到 callback；每次核对原 3 秒余额，不续期 |
| 返回证据 | 成功/已知签名必须对应冻结的同一 message 与原钱包；缺失/额外/不匹配签名拒绝，保留全部已知签名。SDK 构建前拒绝且无签名维持原失败回执，不伪造已验证消息 |

保留原浮点 tip 到 lamports 的 round/saturation，包括正 dust 可能生成零 lamport transfer；不为整数化悄悄改语义。provider 最低 tip 的原 SDK 过滤不绕过。recipient 映射已核对当前 17 个实现及 Node1 QUIC 的常量；它们仍是 vendor 本地证据，**不是独立可信名册或上游 provenance 已完成**。未更换 provider transport/节点/策略，没有网络查询。

## 结构与兼容

- 新 `damm_instructions::swap_instruction` 只投影原 swap2 ABI，由原 quote-only probe 和新普通 profile 共用，避免重复维护账户/data。原 probe 校验/顺序/固定输出不变；原 576 消息冻结摘要不重生成。
- `ordinary_preparation.rs` / `ordinary_params.rs` 原字节与 2,016 组参数摘要保留，不修改 SDK builder。实际 lane 的卖出 holding 改为同一 snapshot 的 Arc 共享，不改变起点/内容或新增读取，买入不因此迁移。
- 新安全拒绝：最终请求/指令/费用/签名不符、重复回调、过期证据、异常 canonical 输出程序、超大 wire。当前服务未提供 V1/ALT/nonce/seed 的这一 profile，不能静默降级；未来必须扩展相应冻结/验收，不以删协议或拒绝全部请求完成目标。
- SDK 入口之后仍按既有 uncertainty/不换 lane 处理。没有新 HTTP 字段、hash/key、表、缓存或恢复 owner；preview 仍是模拟签名证据，不当作正式执行签名。

## 离线验收

前序 B4a 为并发/串行各 1244。本批新增 25 项，最终专项 25 项（3.13 秒）、完整并发 1269（139.95 秒）/串行 1269（239.16 秒）、HTTP 199、原参数合同 3（2,016 组）、ordinary_lanes 19、damm_probe 48 以及 production check/fmt/diff 通过，仍五组既有生产警告，无新增警告。覆盖 96 方向/程序/比例/模式组合、完整 intent 与 request/全账户/费用变更、实际 SDK 对照、17 provider 全部本地 recipient、单次/并发/取消/原期限、tip dust/saturation、wire codec、公开 RFC8032 验签与签名保留。

首轮错误是调用 SDK 私有 `TradeTokenType::mint`（E0624，无测试结果），已改为 adapter 内显式映射，不改 SDK 可见性。修正后中间 17 项通过；增加 canonical 输出程序检查与 oracle/其他用例后 24 项通过，再补完整执行意图绑定得到最终 25 项，未用中间源码结果代替最终全套。

新生产 builder/guard 和新多数测试只使用公钥/零签名。实际 SDK oracle **复用既有固定未注资测试 payer 的 SwapParams**，只调用 `MeteoraDammV2InstructionBuilder.build_sell_instructions` 读取测试公钥；不调用 executor、signed builder、sign 或 RPC。原 576 消息不变；另对 exact-in、合法 canonical 输出、两种原数量、50/100%、三种 minimum 共 384 个正式 profile 对照。极大 minimum 仅验证 ABI，不是可信报价/可成交证明。

RFC8032 只有公开公钥/已有签名/空消息，不生成签名或包含私钥，不是 Solana 交易成功验收。wire 测试用完整零签名交易对照本地 bincode 与 SDK 生产 wincode serializer，不启动 provider 或创建 signer。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet -j 1 damm_sell_message
cargo test --locked --offline --quiet -j 1 damm_probe
cargo test --locked --offline --quiet -j 1 ordinary_param_contract_tests
cargo test --locked --offline --quiet -j 1 ordinary_lanes
cargo test --locked --offline --quiet -j 1
cargo test --locked --offline --quiet -j 1 -- --test-threads=1
cargo test --locked --offline --quiet -j 1 http_contract_tests
cargo check --locked --offline --quiet -j 1
cargo fmt --check
git diff --check
```

已核对 manifest/lock/release、原准备/参数/input guard、executor/钱包 registry/PumpAPI、root SDK 与 lane preparation 指纹和 B4a 相同，vendor diff 为空；有意变化为 lane execution 的实际接线、共用 ABI/原 probe 投影、新 guard 及测试。本轮不构建 release，不修改线上二进制。

| 有意变化的主要源码 | SHA-256 |
| --- | --- |
| `src/sdk_client/lane_execution.rs` | `ae68e4b8dbef9da0d55af8e9943cf10a2bc0c6187cb2d2aa5824f18f3ca4a123` |
| `src/adapters/sdk/mod.rs` | `00540437fa5fe5b9433535b163ada31bd71d9cbcbcc360b647f9a0713e868ff2` |
| `src/adapters/sdk/damm_instructions.rs` | `dcdf4fdaa978d65d9389061d8ccd475323c1364cfb824a30b5cb7401feb16d24` |
| `src/adapters/sdk/damm_probe/plan.rs` | `5c67be8fff4a50cd21d47106b61e32d4ea7d28d7e30a90df61ab81c497e42f13` |
| `src/adapters/sdk/damm_probe/sdk_oracle_tests.rs` | `90f2725b04aa435c3e2c6633328ff9f734ae13f3e5519509bd728f440e0cf3a2` |
| `src/adapters/sdk/damm_sell_message.rs` | `c49c1cafd1bc45d4b3578ddbe4846decd26571f3b439f98dea94b7d069c05207` |
| `src/adapters/sdk/damm_sell_message/instructions.rs` | `abcfe12f78f888740f6319d002ff0596c06224e396772b99077944b01f046432` |
| `src/adapters/sdk/damm_sell_message/guard.rs` | `da1962f77f6c700bff50d7e88d12cfe2e15919ddf25092c05e01250b815e3f40` |

原 release 仍 `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；当前 vendor recipient 常量文件 `a92f7a7341262a9e2b20e9c0ebd78d4f225795d2792303e571452d2ad7bc5d60`。内容指纹不冒充可信上游 revision。

## 接续主线

B4c 下一步应将此冻结候选收敛成唯一可签名产物：在公开地址/已核验账户输入下选定具体 provider recipient 并编译，补完整证据授权；签名端只接该不可变产物和原 ordinary owner，而非任意 SDK request/transaction。继而通过原 authority 的 journal/CAS/独立读回和原字节 sender 取代 opaque SDK 的内部重建/签名/广播。复用现有普通交易 owner/账本与后端边界，不开平行交易 API、Quote authority 或恢复 owner；然后扩展全方向/协议与其他存储模式。当前 guard 是过渡一致性检查，不应继续只堆中间参数检查冒充这一主线。

## 未完成与接续

1. 本 profile 仍经 opaque SDK 内部编译/签名/发送，guard 看不到签名后广播前控制权。B4c/C/D 必须把签名与原 authority 的持久化/独立读回/原字节发送接上，不能用本批候选 bytes/返回验签冒充 journal。
2. DAMM Pool/费率版本/全部账户与 Token 扩展、可信 output/min_out/权限预算授权继续，含 DAMM 买入和其余八类普通 Pool、wrapper/多跳。probe/sell profile 不代替全方向保护。
3. 取消只阻止尚未通过检查的迟到回调，不证明 worker/已接受的签名或网络发送被取消；已知签名完整落盘与未返回签名中断仍有缺口。超时/503/unknown 不授权新 key/重签/补单。
4. 绑定依赖已审查的 SDK compile/sign 顺序；继续受控不可变产物、可信 revision/补丁账本、双 fake build backend、升级/协议演练及真实节点/生产恢复/发布验收，不以测试/目录数量估算完成度。
5. 剩余读取、其他 mode/backend、Memory/Shadow、资源/依赖边界保持完整范围；暂停的 PumpAPI 门槛不删除。目标 active，不部署、不升级、不动其他服务。
