# TS-S1-H2-A PumpAPI 无签名模拟

日期：2026-11-13。接续 `PUMPAPI_REQUEST_AND_CODEC_BOUNDARIES.md`。仅 Trade Service 源码、离线测试和记录；不升级 SDK/依赖、不构建 release、不部署重启、不访问生产钱包/数据库、不调用真实 HTTP/RPC、不签名或广播。

## 本批实际修复

H0/H1 核实的旧流程是外来 bytes 解码后立即用 payer 签名，再判断 simulate。H2-A 将实际调用拆成两条：

```text
SdkTradeClient::pumpapi_trade
  simulate=true  -> simulate_trade(RPC, public Address, trade)
                    -> 无 signer 的共用 Local HTTP/结构解码
                    -> simulate_unsigned(RPC, public Address, transaction)
  simulate=false -> execute_live(TradingClient, trade, BroadcastPolicy)
                    -> 原 Live 签名/submit
```

分支发生在调用持有 signer 的 execute_live 之前，不再以“签名但不 send”冒充 unsigned。模拟入口和 RPC helper 参数中没有 TradingClient、Keypair、Signer、WalletRegistry、repository 或签名回调；SDK 分发器仍持有原 client，但只取同一 Default lane 的公开 payer 和原 RPC 引用。全服务并未因此获得无 signer 沙箱。

- simulate_trade 拒绝 simulate=false，且配置/Lightning/请求金额检查继续先于 HTTP。
- execute_live 保持禁播检查先于准备/签名，另拒绝 simulate=true，避免内部误接线。
- 原 Local Transaction 配置、请求内容、错误顺序和 HTTP 超时复用，无新开关、后端或网络重试。
- mapper 的公开 wallet 参数由 SDK Pubkey 收窄为项目 Address；80 组旧请求黄金字节保持，币种/精度/百分比规则不变。

## 进入模拟 RPC 前

复用 SDK codec 的结构检查，而非复制一套不同的格式规则。即使内部直接调用模拟 helper、没有经过 HTTP decoder，也检查 Legacy/v0、1,232 字节 packet 和 Solana sanitize。随后核对：

1. required signatures 为一，签名向量恰好一个零签名槽。
2. fee payer 的静态地址与本次公开 wallet 全字节相同。
3. 非零签名、额外 signer、错误钱包和畸形账户索引在 RPC 前失败；不“清零后信任”外来已签交易。

继续使用原 simulate RPC 配置：sigVerify=false、replaceRecentBlockhash=false、processed、Base64、innerInstructions=true，accounts/minContextSlot 为空。发送的 unsigned bytes 与解码后的原消息相同，不替换 blockhash 或自动换节点/路线；本 helper 只调用 simulateTransaction，没有 sendTransaction 或签名操作。

这不是 Live 授权：对池/资产/方向/整数预算/min_out/费用/收款账户/Token Program/ALT 的验证仍未完成。RPC 模拟成功也不能把外来消息升级为可签对象，不能把此 helper 用作任意交易签名接口。

## 明确记录的兼容行为

公开 API 字段、八字段普通响应、可空 signature、validator、请求 key/hash、SQL/schema、默认后端不变；不重写旧记录。普通 classify_result 函数及其原 98 组指纹未改。

实际 PumpAPI 模拟结果改为 signature=None，包括成功及程序失败。不会生成真实签名，也不把 111... 零签名串返回为交易签名。成功仍为 simulated；RPC/构建错误仍走原错误分类；程序模拟失败由于不再携带真实签名，现按原分类函数成为 failed，而非旧的 unknown。这是有意的无签名修正，不声称实际返回值与旧签名模拟完全相同；调用方兼容仍需发布前验收，本轮不改 Bot。

普通模拟仍沿用既有普通交易接受与记录流程，可能保存 Accepted/BuildingTx/Submitting 等阶段；没有将 WSOL 的“预览不创建 Live key”性质冒称为普通路径性质。helper 取消测试仅证明该次内存 RPC 不签名/广播/重试，不代表普通记录的跨重启恢复或生产断电安全已完成。H3 必须复用唯一 reservation/journal，不能为解决取消而换 key 自动补单。

## 离线证据

新增 10 项测试：

| 层次 | 覆盖 |
| --- | --- |
| 8 项真实 SDK helper/入口与内存 RpcSender | Legacy/v0 原消息与零签名槽、全部原 RPC 配置、两版本直接超限、七类钱包/签名/形态错误的零 RPC、程序失败无签名、RPC 错误 URL 脱敏/无重试、畸形回执不成功、五类入口错误优先级、Notify gate 确定性取消 |
| 1 项 SDK 分发守卫 | 禁播环境下 unsigned 仍到原 Default lane 检查，Live 原禁播测试保持；空 client 无 payer/SDK/网络，不把此项冒称为成功真实交易 |
| 1 项真实 Router -> 普通接受/执行/幂等/临时持久化 | memory/durable × buy/sell × 成功/程序失败的 null signature、simulated/failed、原八字段、同 key 重查不执行第二次；运行时为 fake，非供应商成功验收 |

内存 sender 记录每次 method 与完整参数，拒绝除 simulateTransaction 外的任何方法。样本只由公开固定地址/区块哈希和零签名槽构造，未创建或调用 Keypair/Signer。RPC helper 的真正请求字节通过 base64 解码逐字核对；HTTP 层使用现有 fake backend，不将两者组合成未执行过的端到端供应商验收。

最终验证全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 721 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 721 passed
cargo test --locked --offline --quiet http_contract_tests  # 153 passed
cargo test --locked --offline --quiet pumpapi_h2a          # 10 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仅两条既有 models/persistence 兼容重导出警告。Cargo.toml、Cargo.lock、既有 release SHA-256 仍为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原 migration 无 diff，磁盘仍约 2.4 GB 可用。`EXECUTION_STATUS.md` 已同步。中间测试的 Debug 约束、配置字段路径和 UiTransactionError 展示期望均已修正，没有改生产结果类型、增加 allow、忽略测试或放宽验证迎合测试。

## 后续 H2-B / H3

H2-A 只完成 unsigned 子任务，不缩减原工作包：

1. H2-B1：沿原 TradeCommand/同一钱包持有期传递不可变意图及候选 route，保留现有金额、协议、wallet/key、模式和 fee/MEV。H2-A 时点 PumpApiTrade 没有期望 Pool 类型；后续 B1 已通过完整自有只读快照解决上下文丢失，见 `PUMPAPI_OWNED_EXECUTION_CONTEXT.md`。快照仍不是可信整数费用上限或链上证据，不能用远端 poolId/回包冒充本地意图。
2. H2-B2：复用现有只读池/Token Program loader 和项目 ports，核实真实池、两端 Mint、owner、账户权限、ALT、slot/时效；RouteHint 不能当 VerifiedRoute。
3. H2-B3：逐协议解析完整指令及账户，核对 ExactIn/min_out、费用/小费、ATA/WSOL 生命周期；拒绝未知/多余指令、错误方向/数量/目的账户、approve/delegate/authority/close 等未授权副作用。不是单纯 Program ID 白名单。
4. H2-B4：将授权摘要绑定原意图与消息，签名前再次核对；没有可信 Quote/协议证据不能降为 min_out=1 或绕过校验。用恶意/篡改 fixture 证明签名前拒绝，不升级或修改 vendor。
5. H3：绑定原普通 key/hash、授权消息和同一原签名字节，发送前持久化、写入失败不得发送、回执不明只查原签名；随后 H4 继续处理 SDK 组合与 legacy 聚合的同类缺口。

H2-B2-A 续作已在 unsigned/Live 前接入真实消息账户/ALT 视图，带 ALT 时新增一次有界逻辑批读并将其 slot 作为后续 minContextSlot；无 ALT 的模拟参数仍保持本阶段原值。新增 16 项、全套 744，通过不代表池/Token/完整指令或 journal 已完成，详见 `PUMPAPI_MESSAGE_ACCOUNT_VIEW.md`；上文本阶段的 RPC 配置与 721 项保留为历史检查点。

所有后续工作仍受原边界约束，不自动切换实盘、重签重发、读生产数据或部署。整个优化目标未完成。
