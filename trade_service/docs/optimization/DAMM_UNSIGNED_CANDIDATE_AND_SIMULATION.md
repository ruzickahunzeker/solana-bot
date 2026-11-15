# NP-01-B4c1 不可变 DAMM 产物与零签名模拟

日期：2026-11-15。仅主 Trade Service 源码/离线验证，不升级 SDK/依赖、不改 vendor，不构建 release、不部署/重启、不访问生产钱包/数据库、不签名发送。PumpAPI 交易开发仍暂停，Copy/Bridge/sdk9 不修改。接续 `DAMM_FINAL_CANDIDATE_GUARD.md` 的 B4b，完整目标 active。

## 本批交付与实际接线

1. 将 B4b guard 内的 full 构建/编译提取为真正拥有交易的 `DammSellCandidate`。私有字段保存原 profile、完整指令和完整零签名 VersionedTransaction；不提供 Clone/Serde、任意交易构造或可写 transaction getter，也不持 signer/RPC/sender。可取得的 message bytes 是独立副本，不能修改原产物。
2. 真实 Live guard 的 Frozen 状态现在持该产物，不再仅存一段 bytes；仍逐项比较实际 SDK 指令、按同一 message 验返回签名，保留 B4b 正式费用/recipient/原指令/单次/取消语义。正式 Live 仍使用 opaque SDK，尚未从该产物直接签名/发送。
3. 真实 `invoke_sell` 改用 profile 的 `execute` 分派：普通 DAMM 卖出的 `simulate=true` 从产物只调用只读 RPC，绝不调用传入的 SDK 执行闭包。旧 `invoke` 也明确拒绝 simulation profile，避免从旧方法绕回 SDK simulate。其他协议/买入保持原路径。
4. 真实 DAMM Live preview 经 `after_unsigned_preview`：核对同一 context、完整 intent（只允许 simulate 差异）、同一原余额 Arc 和同一业务指令；只读模拟成功后重查原期限，才释放原正式 submit 闭包。失败/超时/取消/身份变化/过期不进入正式 SDK，也不切 lane。

```text
原准备/净报价/余额证据
  -> 冻结 profile
  -> DammSellCandidate（完整消息 + 唯一全零签名占位）
  -> 只读 simulateTransaction（没有 SDK executor / signer）
  -> 模拟请求：simulated / failed，signature=null
  -> Live preview：成功且原证据仍有效才调用正式 SDK 闭包
  -> 正式 SDK 的原私有 guard 再冻结同类型产物并验返回签名
```

生产只读分支不调用 SDK signed builder，也不调用 `sell_by_percent`。SDK client 仍由原执行端持有，传入的正式闭包可能捕获原 client，但只读分支既不调用也不轮询它；本批不是整个进程/钱包初始化已无 signer 的声明。

## 编码、RPC 与时效

- 原金额/minimum、swap2 ABI、ATA 生命周期、CU price=1000/limit=300000、V0-compatible/no ALT/no nonce（实际 Legacy）和 1232 wire 上限保持。预检仍是原 Default 零 tip profile，不把正式 relay tip 加进原预检，不声称它已模拟正式付款全预算。
- 只读 RPC 独立入口再次拒绝非 Legacy、非单 payer、默认 hash、非唯一零签名或超大交易。方法固定 `simulateTransaction`；不提供签名、send、确认或另查签名能力。
- 原 SDK 模拟配置保持：processed、Base64、sig_verify=false、replace_recent_blockhash=false、inner_instructions=true、accounts/minContextSlot 均 None。唯一有意交易编码差异是从真实签名改为全零占位；message 不变。
- 使用原 lane 的同一个 RpcClient，未建新 transport/节点或扩大候选。增加单次逻辑模拟 4 秒 deadline；原 3 秒余额期限从最初读取开始，模拟前/后均重查，不因候选构造或请求返回续期。context slot=0 和坏响应拒绝。
- 本函数只有一次逻辑 RPC 调用，不在项目层重试或换节点；底层 SDK RpcClient 的内部 HTTP 重试/缓冲仍存在，**不是一次 wire 请求/流式体积上限/第七类 SharedReadRuntime profile**。真实供应商、HTTP body 限制与统一 429 配额尚待治理。
- 本地取消会丢弃当前只读 future，不说明远端节点停止处理。它不提供提交能力；正式 SDK worker 的取消/签名/发送不确定性仍是独立未完成门槛。

现有 quote-only DAMM probe 仍用于净输出计算；新 RPC 替代的是最终 SDK 模拟/preview，不新增一笔该阶段模拟。整条准备流仍有其他账户读取和原 quote probe，不能将这里的一次逻辑调用写成整笔交易只有一笔 RPC。

## API、权限与持久化兼容

公开路径/字段/状态枚举、validator、原整数/舍入/hash/key/JSON 和数据库格式不变。`TradeResponse.signature` 原本就是可空字段；该源码中的普通 DAMM 卖出 simulation 成功为 `simulated`、链上模拟拒绝为 `failed`，signature 都是 null，不伪造或持有可误认作实盘的 preview 签名。旧原样 GET/同 key 重复不重新执行。API 格式与三个持久化模式有真实 Router + fake backend/临时库合同，不冒充真实 SDK/节点交易验收。

原 `TRADE_BROADCAST_DISABLED` 的 SDK 方法准入保持，即使该子路径现在安全地无签名模拟，也不借本批扩大原权限许可。尚未部署，调用方及发布验收需显式检查 `simulated/null`；其他普通 SDK simulate 仍可能签名。Live 的最终签名、回执与 unknown 不换 key 语义保持；取消/超时不授权重签或补单。

本批没有修改或写入生产库，也没有新增 migration、Quote、owner 或幂等账本。零签名产物不是 signed journal，不能因为它可编码/模拟就宣称广播前持久化完成。整个未部署优化树已有 004–007 等迁移，未来发布仍需其原迁移/备份/恢复门槛，不能因本批无新 migration 直接上线。

## 离线证据

基线 B4b 完整并发/串行各 1269。本批新增 20 项：SDK 产物/分派/preview 12、RPC 7、HTTP 1（该用例含 Memory/Shadow/Durable 与成功/拒绝六个场景）。旧 B4b 用例不删除，Frozen bytes 断言机械改为产物 message 副本；取消测试使用 Live profile，因为 simulation 现在明确禁止进入旧 SDK invoke。

最终实际结果：`damm_unsigned_simulation` 8 项（RPC 7 + HTTP 1）、`damm_sell_message` 37 项（含旧 25）、完整并发 1289（142.51 秒）/串行 1289（246.58 秒）、HTTP 200、原参数合同 3（2,016 组）、damm_probe 48、ordinary_lanes 19 与 production check/fmt/diff 全部通过，仍五组既有生产警告，无新增警告。新用例只使用公开地址、零签名、内存 RpcSender、临时库及 fake 执行后端，不生成签名。旧 576/384 SDK instruction-only oracle 与已公开 RFC 向量保留，不将它们当作本批实际签名测试。

覆盖：完整消息/零签名与副本不可变性、禁止 formal profile 被当 simulation、禁止 simulation tip、模拟不进入 SDK 闭包、只读链上拒绝无签名、不匹配/过期 preview 无 RPC/无 submit、成功 preview 一次读后一次 fake submit、RPC 不可用/迟到/取消不释放 submit、4 秒真实逻辑 timeout/Drop、固定方法/配置/编码、坏上下文/响应、URL 脱敏、原 Live 不确定签名完整保留、HTTP null/状态/同 key 不重执行。

内存 sender 只证明逻辑请求/编码/取消，不证明真实 HTTP 次数、供应商限额或节点诚实。HTTP 用例 fake 成功不代表 SDK/链上成功。没有运行真实 RPC、SDK executor 或签名器。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet -j 1 damm_unsigned_simulation
cargo test --locked --offline --quiet -j 1 damm_sell_message
cargo test --locked --offline --quiet -j 1
cargo test --locked --offline --quiet -j 1 -- --test-threads=1
cargo test --locked --offline --quiet -j 1 http_contract_tests
cargo test --locked --offline --quiet -j 1 ordinary_param_contract_tests
cargo test --locked --offline --quiet -j 1 damm_probe
cargo test --locked --offline --quiet -j 1 ordinary_lanes
cargo check --locked --offline --quiet -j 1
cargo fmt --check
git diff --check
```

## 指纹与不变项

manifest/lock/vendor/release、原准备/参数/input guard、executor/钱包 registry/PumpAPI、root SDK 与 lane preparation 和 B4b 相同；有意变化仅为 lane execution 的无签名分支、profile/guard 的产物复用、RPC module 声明和新组件/测试/文档。本批未编辑 migration 或原 claims/签名账本。

| 本批主要源码 | SHA-256 |
| --- | --- |
| `src/sdk_client/lane_execution.rs` | `99a5d310beddca5ef09a2d94fcf4506cd1bde09e52b496acc84ad54cec51a897` |
| `src/adapters/sdk/damm_sell_message.rs` | `c7aa5b68e22a379dc7311fa332d79fcda37fca3c5ac6295b9b59b88ab29e21cb` |
| `src/adapters/sdk/damm_sell_message/candidate.rs` | `41840fbcbd47cd711a2f7cacbf36fbfae164410dbcbfcada658126e8b46f5bd5` |
| `src/adapters/sdk/damm_sell_message/simulation.rs` | `2c61e3a28dba4d7c2ff1c1379a6664b56ec3a6bbf7fc45fedb605972e15a6119` |
| `src/adapters/sdk/damm_sell_message/guard.rs` | `63e6b12616f7a7f12c2eb3ddaac9ea46fd8f86fcfc2058b66297ef5a3dfcc095` |
| `src/adapters/rpc/damm_sell_simulation.rs` | `088e0f4c4d5978dc2e0e1d0c701fa4bf47e6a909931c03c7f8fad47702c476ca` |

原 release 仍 `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`。这些只是本地内容指纹，不冒充可信 SDK revision、已部署制品或真实节点验收。

## 账本接续与完整未完成项

后续 C3a 已完成下列操作分型/008 兼容迁移与原 owner 绑定的源码及离线子项，见 `ORDINARY_SDK_CLAIM_OPERATIONS.md`；SDK metadata/完整授权/CAS/readback/受控正式发送仍未完成。下文保留 B4c1 当时的表约束和接续要求，不应据此重复实施已验证的 C3a。

已核对当前 `ordinary_execution_claims` 的 operation CHECK 固定 `ordinary-sol-buy`，`OrdinarySubmissionJournal` 的 prepare/authorize 绑定 BoundQuote/QuotePreflight，scope 仅接受 Live aggregator SOL buy。不能把 DAMM 卖出伪造成 quote、复用它的 metadata 或另造一个恢复 owner 来“接通”。

下一主线按原 ordinary authority 扩展操作类型及对应持久化 metadata/完整性/CAS/readback/恢复合同，保留旧行与 hash/JSON，再让正式不可变产物进入该原 owner 的受控 sign -> durable record/readback -> 原字节 sender。正式 recipient 必须在授权前选定并冻结，不能依赖 SDK worker 再随机选择或重建。当前产物在模拟和正式 guard 实际使用，为此提供已编码边界，不等于已完成授权/账本。

| 接续任务 | 必须保留的权威/验收 |
| --- | --- |
| 操作分型与 scope | 原 request/owner/key/hash 不变，明确区分既有 aggregator SOL buy 和 SDK 操作；旧接口不能因 scope 放宽接受别的资金路径 |
| 原 claims 表兼容扩展 | 若扩展 operation CHECK，必须事务化迁移/标记/回滚/逐列旧行保留，临时库验证 FK、索引与损坏拒绝；不得修改旧 migration 来掩盖升级过程 |
| SDK prepared metadata | 独立类型记录原意图/具体候选/账户与金额/费用证据，不伪造 BoundQuote/QuotePreflight，也不新增交易或恢复 owner；未知/混合 metadata fail closed |
| 原执行入口接线 | 在原 ordinary owner 下保留并传递新操作权威，准备/签名前 CAS、签后落盘及独立读回，最后只发原字节；不能只建表而仍从 opaque SDK 直接广播 |
| 恢复与跨模式 | 旧聚合扫描/对账不得误解 SDK 行；新操作按原签名只读恢复，不新 key 重签；Memory/Shadow 不冒充 durable，取消/崩溃/竞争/跨文件重开各自验收 |

此表是剩余执行任务，不是已迁移/已签名或可以发布的声明；涉及生产动作仍需另行授权。

仍须覆盖独立可信 Pool/账户/费用版本/min_out/权限与 blockhash 有效期证据、DAMM 买入/其余八类普通 Pool/wrapper/多跳、全部 backend/mode/Memory/Shadow、SDK worker/多签名、剩余 RPC/资源/依赖方向、双 fake backend、可信 SDK 来源/补丁账本/隔离演练，以及真实节点/生产迁移/恢复/部署验收。暂停的 PumpAPI 项不剔除；持续目标保持 active。
