# TS-S1-H2-B2-A PumpAPI 消息账户与 ALT 视图

日期：2026-11-13。接续 `PUMPAPI_OWNED_EXECUTION_CONTEXT.md`。本批完成 H2-B2 的消息/ALT 子任务，**不代表完整池核验、指令授权或发送前账本完成**。只改 Trade Service 源码、离线测试和文档；不升级 SDK/依赖、不构建 release、不部署重启、不访问生产钱包/数据库、不调用真实 HTTP/RPC、不签名或广播。

## 先确认的复用边界

- 原 PoolReadBackend 返回 PoolRouteHint，不含完整 owner/Token Program/slot/账户权限证据。warmup/dex_params 的成员校验和 SDK loader 不能直接改名为 VerifiedRoute。
- 原 aggregator ALT helper 检查 owner/布局/停用状态，但丢弃读取 slot，也不检查同 slot 新增地址的可用前缀。此处不改聚合行为，而为 PumpAPI 建立实际接入的账户视图，后续池/指令审计以此定位账户。
- 不建第二套 RPC 客户端、钱包、缓存、reservation 或 journal。复用原选中的 Default lane RPC，所有 SDK/链类型留在 `adapters/sdk/pumpapi_accounts.rs`。

## 实际接线

```text
unsigned simulate:
  原 mode/config/Local 请求/严格解码
  -> 原模拟钱包与单零签名槽校验
  -> resolve_unsigned_accounts(original RPC, public wallet, original transaction)
  -> ensure_bound(original wallet/message)
  -> original simulateTransaction, minContextSlot=ALT view slot if present

Live:
  原禁播/mode/config/Local 请求/严格解码
  -> resolve_unsigned_accounts(original RPC, public wallet, original transaction)
  -> ensure_bound(original wallet/message)
  -> 原 try_new/sign
  -> 原 submit, minContextSlot=ALT view slot or legacy 0
```

Live 在新校验之后才可能进入原签名调用；本轮未构造真实 signer 或验证成功广播，该接线由源码、编译和无 signer helper 测试佐证，不能把它称为完整 Live 安全验收。消息中的未知程序/资产/金额仍须 H2-B2-B/B3/B4 处理。

## 账户视图

模块首先复用 packet/sanitize，核对唯一零签名槽和 payer；拒绝零/超过 32 条指令、超过 4 张 ALT、重复 ALT、重复 static keys。Legacy 和无 ALT 的 v0 完全不做新增账户 RPC。

带 ALT 时：

1. 一次逻辑 getMultipleAccounts 读取所有表，Base64、processed、无 dataSlice，整次 await 5 秒上限；不逐表读取或新增应用层回退/重试。
2. 保留 context.slot，严格核对响应数目、非 null、ALT Program owner、非 executable、非零 lamports。
3. 只接受 Base64 数据；在解码前限制编码长度，解码后限制为 metadata + 最多 256 个地址，核对对齐及可选 space。拒绝 Base58/JsonParsed/Zstd 等意外格式，不调用可能对坏 UI 数据 expect/panic 的便利转换。
4. 使用既有 AddressLookupTable 结构解码；拒绝所有 deactivating/deactivated 表、比 context 更新的 last_extended_slot，以及不可能的扩展起点。
5. 使用现有 Solana 库 lookup_iter 判断当前 slot 的可用前缀和索引，不另写一套生命周期算法。同 slot 仅允许扩展前旧地址；因为已拒绝 deactivating 状态，不需要额外 SlotHashes RPC。使用库接口推导 SlotHashes 类型，未为依赖树中的多版本新增依赖或强制跨版本转换。
6. 按运行时顺序拼接 static + 所有表的 loaded writable + 所有表的 loaded readonly；不能按“每表 writable/readonly”交错拼接。loaded 均非 signer。
7. 限制总地址最多 256，拒绝跨 static/loaded/表的地址别名。保留消息编码请求的 signer/writable 权限；这不是 runtime 对 reserved/program keys 降权后的最终权限，更不是池角色许可。

原 RPC transport 自身是否进行物理 HTTP 重试仍由库决定，本批只保证不追加应用层逻辑调用且总 await 有限。响应字段/解码受限不等于底层 RPC HTTP 流已全面限流；通用 RPC 体积/并发等基础设施仍属后续工作。

## 消息绑定与兼容变化

ResolvedPumpApiAccounts 是私有字段的自有视图，包含公开 wallet、原消息内部 SHA-256、解析后的地址/请求权限和可选 ALT slot。调用方使用前重新核对钱包、单零签名槽、消息摘要、payer 权限、数量与指令索引；换 blockhash、指令数据、lookup 索引、账户或签名都不能复用原视图。

这个 hash 只绑定本次内存对象，不写数据库、不替换旧请求 fingerprint、不称为协议授权摘要。构造阶段和使用前分别计算一次，避免在每层反复序列化；完整 command 仍由原 opaque PumpApiTrade 持有。

有意收紧：Live 不再忽略供应商的非零签名，重复账户/过多指令或表/不可用 ALT 在签名前失败；错误可能与原 try_new/RPC 返回不同，这是资金路径安全修正，不伪装成纯目录迁移。API 字段、validator、旧 key/hash/schema、后端选择和默认模式不变，无新开关/能力发布。

模拟仍不签名、不返回签名、不替换原消息/blockhash；仅带 ALT 时新增一次有界只读调用并设置 minContextSlot。无 ALT 的模拟 minContextSlot 仍为空，Live 仍为原 0；原 retry/preflight/确认配置保持。

minContextSlot 只防止后续请求使用落后于视图的节点上下文，**不锁住 ALT、不证明同一分叉，也不保证随后链上状态不变**。本批没有墙钟 TTL/可信最新 slot 对照、池/token 证据或原子快照。后续协议审计和发送前账本仍必须绑定实际来源、slot/时效、原消息及原签名字节。

## 离线证据

新增 16 项：

- Legacy/v0 无 ALT 的零 RPC、原 static 权限和消息字节；两张表的一次批读、完整配置和正确拼接顺序。
- 真正内存 RPC 的 simulate 接线：先读 ALT，再调用 simulate，原 unsigned bytes/零签名槽不变，minContextSlot 为读取 slot；不使用真实 signer/发送接口。
- 同 slot 旧/新前缀、下一 slot 放行；375 组 context/extension/prefix/index 与现有库的结果对照。
- inactive/future/坏起点、12 类 owner/state/布局/编码/长度错误、missing/partial、五类重复、四类钱包/签名错误、4/5 张表、0/33 指令、完整 256 地址表和 256 resolved keys。
- 六类消息/钱包/签名篡改导致绑定失败；URL 脱敏、Notify gate 确定性取消、真实 5 秒逻辑超时，均无应用层第二次读/模拟/发送。

样本只有固定公开地址、零签名槽与合成 ALT；内存 sender 拒绝 GetMultipleAccounts/SimulateTransaction 之外的方法。无真实节点或供应商端到端验证。取消测试直接 drop/abort helper future，不代表取消 HTTP 请求会停止已 spawn 的 ordinary Executor；客户端超时/取消不作为未签/未发、换 key 或释放钱包许可。

最终验证全部通过，`EXECUTION_STATUS.md` 已同步：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 744 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 744 passed
cargo test --locked --offline --quiet http_contract_tests  # 153 passed
cargo test --locked --offline --quiet pumpapi_h            # 49 passed, includes B2-A's 16
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍为两类 models/persistence 兼容重导出警告，无新 allow/忽略测试。Cargo.toml、Cargo.lock、既有 release SHA-256 仍为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原 migration 无 diff，磁盘约 2.2 GB 可用。没有 release 构建、生产 RPC、数据库迁移或部署操作。

## 下一阶段

H2-B2-B 继续实际池与资产证据：先使用显式 CPMM/StonkFunSwap 的既有布局/fixture，再补其余协议；未显式给 poolId 时只能从预期协议的正确指令位置/PDA 得到候选，再链上验证，不用任意消息账户或缓存 hint 冒充真实池，也不追加无界 GPA 寻池。核对 owner、两端不同 Mint、Token Program/扩展、账户角色、状态、slot/时效。

续作已完成上述 CPMM 两枚举的直接路线子集，见 `PUMPAPI_CPMM_ACCOUNT_EVIDENCE.md`。H2-B2-B 的最终 bank slot 还会推进 minContextSlot；其他协议未完成，原 B2-A 的无 ALT 不读账户性质仅指通用 ALT resolver，不代表后续协议检查也零 RPC。新增 17 项，全套 761，不能把局部检查当作完整授权。

接着 B3/B4 核对全部指令/方向/整数支出/min_out/费用/收款账户及 ATA/WSOL 生命周期，拒绝未知或额外副作用后绑定授权消息。H3 复用唯一普通 reservation/hash，完成签名前后证据、发送前持久化、取消/恢复/不确定结果只查原签名；H4 继续 SDK/legacy 组合入口。不能只凭本账户视图通过就宣布任何协议已完成授权、开放实盘或自动重签重发。
