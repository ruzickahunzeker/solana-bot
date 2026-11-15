# TS-S1-G4-C WSOL 报价与未签名模拟边界

日期：2026-11-13。接续 `WSOL_STATUS_RECORD_AND_ERRORS.md`。仅 Trade Service 源码、离线测试和文档；不升级 SDK/依赖、不构建 release、部署重启、访问生产钱包/数据库、调用真实 RPC 或发送交易。

## 先复现的问题

两项先失败测试复现：应用接受同 owner 但 wallet_id 不匹配的 metadata；也会把 adapter 返回的零金额等不可能的成功估算存成 Quote。原 quote/unsigned simulate 又把参数、RPC、存储和数据完整性错误都映射为 400，调用方无法区分真正的参数拒绝。

本阶段保护实际边界，不新增 Router/SDK 依赖、不另建 journal，不改 Quote 的 13 字段、外层 quote/executable、simulate 的四字段、整数单位、60 秒 TTL、原 V1 JSON/hash、wallet_id/owner 的字面含义或表结构。

## Metadata 读取

- WalletRepository 新增实际使用的 find：与原 get 相同的七列无私钥投影、相同字面 ID 查询和整数布尔规则，但返回 Option，所有列用 try_get，不把不存在与 SQL 故障混在一起，也不对损坏列 panic。
- 两个 WSOL SDK adapters 复用 persistence/wsol_wallet_metadata，只有明确 None 被标为 wallet not found 参数拒绝；结构化 SQLx 解码错误标为完整性异常，数据库关闭/连接故障继续是存储错误。不匹配错误文本，不更改全局 get 或其他钱包管理接口。
- 不查询 encrypted_private_key，不解密、不导入/导出钱包。测试用无效合成密文仍可读 metadata，六列损坏及关闭数据库都有单独证据。
- Quote 保留 store -> metadata/owner/enable -> all 参数 -> parser -> 已加载 reader -> estimate -> 保存的顺序；新增 metadata ID/非空身份/解析公钥的一致性核对。disabled/owner/参数错误保持既有消息和优先级，不为了提取层次提前做 RPC 或钱包加载。
- unsigned simulate 经相同 metadata adapter，并在绑定处核对返回的钱包 ID。它与 Live 复用原一次 resolve/公钥/Quote 检查，保留原 wallet/RPC，不获取新的 signer 或重新选择节点。

## 成功估算的应用校验

QuoteEstimate 继续是项目拥有的整数类型，不携带 SDK 或私钥。adapter 声称成功后，Application 在生成 ID/保存前验证：

1. amount 为正且不超过 source；固定请求精确等于原 amount，all unwrap 精确等于来源余额，不静默截断或更换数量。
2. fee + temporary rent + 原 100,000 lamports reserve 使用 checked arithmetic，不接受不可表示的成功预算。
3. wrap 不得带 temporary rent/refund，来源 SOL 至少能覆盖 amount + fee + reserve。
4. unwrap 必须有来源 ATA；全量不垫临时账户租金，部分不把原 ATA 租金当作净返还。

这里不是复制 SDK/RPC 预算算法。wrap 缺失 ATA 的建户租金仍由原 helper 读取并检查；unwrap 的 source 是 WSOL，不能把网络费/租金 SOL 成本拿去减 WSOL 余额。SOL 侧余额、真实账户布局/租金、当前余额与 canonical Message 仍由准备/授权边界验证。报价不是原子链上快照或签名许可，executable 继续为 false。

## 类型化错误及 HTTP 兼容变化

quote 增加 quote_store / quote_metadata / quote_estimate / quote_save 阶段；simulate 增加 unsigned_admission / unsigned_quote / unsigned_input / unsigned_metadata / unsigned_wallet / unsigned_prepare / unsigned_rpc。

| 情况 | HTTP / code |
| --- | --- |
| 明确不存在、owner/enable/all/key/模式/Quote 绑定/过期、金额或余额不足、需重新报价、模拟明确返回程序错误 | 400 / wsol_rejected |
| RPC/运行时/数据库不可用，或尚无 typed 证据的原始错误 | 503 / wsol_unavailable |
| metadata/公钥/账户布局/标准消息/成功估算或存储读回不一致 | 503 / wsol_integrity_error |
| quote 保存或提交回执不确定，包括写入后晚到 rejection marker | 503 / wsol_outcome_unknown |

原生 helper 在既有校验位置标记已知拒绝，不移动缺少金额/余额/租金/费用等原校验顺序。SPL 账户 owner/布局、标准消息不匹配标为完整性异常，原始 RPC 传输错误不伪装成参数拒绝。业务错误原文本保持；不根据文本包含 wallet not found、expired 等词猜类型。

所有应用错误复用 C3-A/G4-B mapper，仍只有 error 字段，保留 URL 脱敏、no-store、阶段/错误码、禁止自动重试说明，不发送 Retry-After。quote/simulate 不设置 submission ID 或 signature 恢复头；同 key 可能另有实盘操作，不能把预览失败当作实盘未执行证明。成功响应只加 no-store，JSON 不变。

这是显式安全行为修正，不称为纯文件迁移：原来服务故障/坏数据的 400 改为 503，七项旧 HTTP 合同按具体原因更新，明确参数拒绝仍 400。鉴权、JSON/Query 提取器、方法和禁止广播的原入口顺序保持。调用方兼容仍需发布前独立验收，本轮未改 Bot。

## 取消与恢复交叉清单

| 路径/阶段 | 已有证据 | 允许的结果与禁止事项 |
| --- | --- | --- |
| Quote estimate 取消 | 新 Notify gate + 临时文件库重开 | 无 Quote、无 Live key，不自动重新估算 |
| Quote 已提交但响应前取消/回执丢失 | 新保存后 gate、提交后原始/typed 错误；读回完整原 Quote | Quote 可能存在，但不是订单，不伪造成功报价、不创建或重开 Live key |
| unsigned prepare / RPC 取消 | 新确定性 gate、文件重开、执行事件计数 | 只有原 Quote，无 reservation/sign/send；构造临时 signer 不等于使用长期钱包签名 |
| unsigned 正常或失败，原 key 已有 Live reservation | 新四字段 HTTP/应用回归及原记录比较 | 预览不改原 Live 记录/指纹/签名，也不据此释放 pending |
| Live reservation / prepare / signed 写入 / 模拟 / 广播 / 结果写入取消 | G3-B2 八阶段文件测试、C2 unsigned failure 与提交回执丢失测试 | reserved/signed 及原字节按真实阶段保留，不换 key、blockhash 或重签/重发 |
| 原签名 lookup 取消/超时、观测回执丢失 | C1/G4-B 原签名/3 秒 deadline、文件恢复及迟到终态回归 | 网络不明保留状态；已存终态不再查节点；不自动补单 |
| 原 key 纯账本恢复 | C3-B 真正 read_only SQLite、文件恢复、越权/坏数据合同 | 只读原记录，无节点/观测写入；404 不证明原请求已停止 |

新增预览测试只用无签名的 SDK Message/Transaction、进程内 RPC mocks、假 runtime 和临时 SQLite。runtime trait 仍同时声明 Live 与 unsigned 方法，不能把未调用 sign 的测试说成整个 runtime 在类型上不具备 signer 能力；全服务能力隔离仍有后续门槛。测试取消也不等于操作系统断电或远端节点共识验证。

## 离线验证与后续

相对 G4-B 新增 19 项：报价 Application 5、unsigned Application 4、metadata persistence adapter 2、阶段分类 1、原生 SDK/memory RPC 1、真实 Router 6。覆盖固定/all 金额、全量/部分租金、最小预算、损坏/关闭 metadata、原始与 typed 相同错误文本、保存回执不明、取消/文件恢复、实盘 key 与预览隔离、原四/13 字段和 HTTP 鉴权优先级。

最终命令和计数以 `EXECUTION_STATUS.md` 本阶段检查点为准，不把此前 676 项或局部 205 项结果当作全项目验收。

本阶段最终全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 695 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 695 passed
cargo test --locked --offline --quiet http_contract_tests  # 152 passed
cargo test --locked --offline --quiet wsol                 # 205 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仍只有 models/persistence 两条既有兼容重导出警告，没有用 allow 隐藏。Cargo.toml、Cargo.lock、既有 release SHA-256 仍分别为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原迁移无 diff。两个复现失败及中间七项旧 HTTP 期望失败均已解决；没有忽略测试或将它们算成通过。

WSOL 的这些局部工作不是整个优化完成：生产历史签名字节尚未重新验证、没有断电/真实节点/调用方迁移或发布证据；不自动修复坏行或部署。下一步转入 PumpAPI 外来 Message 的签名前审计、unsigned simulate 隔离及普通交易发送前 journal，之后继续 VerifiedRoute/协议扩展、剩余 Handler/基础设施/依赖规则、SDK 来源和隔离升级演练。
