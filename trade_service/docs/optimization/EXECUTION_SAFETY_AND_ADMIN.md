# 禁止广播策略与管理权限

日期：2026-11-13。范围仅为本地 Trade Service；不修改 Copy Trading、Telegram、Bridge、SDK vendor 或生产配置。实现的是现有已审查资金入口的服务端策略与可选管理鉴权，不宣称完成网络沙箱或整个服务的能力类型隔离。

## 配置与默认兼容

```dotenv
TRADE_BROADCAST_DISABLED=false
WALLET_ADMIN_AUTH_MODE=legacy
WALLET_ADMIN_API_TOKEN=
```

| 参数 | 行为 |
| --- | --- |
| `TRADE_BROADCAST_DISABLED=false` / 缺失 | 保留原有执行行为；不因此额外开放任何路径，仍需原参数/确认/持久化等条件 |
| `TRADE_BROADCAST_DISABLED=true` | 拒绝下面列出的发送入口及 SDK 完整买卖流水线；请求参数不能覆盖 |
| `WALLET_ADMIN_AUTH_MODE=legacy` / 缺失 | 保留旧 Bearer/IP 行为，尚未完成运行时权限迁移；预置的管理 token 不会自动改变模式 |
| `WALLET_ADMIN_AUTH_MODE=separate` | 独立管理 Bearer + IPv4/IPv6 loopback；交易凭据不能管理钱包，管理凭据不能直接调用交易接口 |
| `WALLET_ADMIN_API_TOKEN` | separate 必须非空、与交易凭据不同，且是无内嵌空白/控制符的单个可打印 ASCII Bearer token；建议独立生成至少 32 个随机字节，安全保存，不能复用钱包 passphrase/MASTER_KEY_HEX |

布尔值支持 true/false、1/0、yes/no、on/off（忽略大小写和两侧空白）；空值或无效值报错，不能静默当作允许广播。模式只支持 legacy/separate。安全配置读取对无效 Unicode 报错，不当作缺失。测试通过纯参数解析，不修改进程环境或读取生产 `.env`。

策略在进程启动时固定，经 WalletRegistry 传给每个已有/新导入钱包的 SDK wrapper，没有 HTTP 请求级别的重新开启选项。切换需要受控重启，不会终止已广播交易。`ENABLE_SIMULATE=true` 仍不是硬保护；旧文档出现过的通用 `TRADE_SUBMISSION_ENABLED` 仍未实现，不可依赖它。`CONVERSION_LIVE_ENABLED` 的原门槛保留，但不能覆盖禁止广播策略。

## 已接入的资金入口

| 路径 | 提前拒绝 | 包装/发送边界 |
| --- | --- | --- |
| 普通/批量 SDK 交易 | 合法参数归一化后、幂等 reserve 前；executor 再检查 | SDK wrapper 的 buy/sell 在选 lane/构建交易前检查；simulate=true 也不能进入 |
| 普通交易内的聚合分支 | executor 根据真实 simulate 模式检查 | 原 aggregator submit 在预检/签名前检查，SDK aggregator send 再检查 |
| 独立 quote submit | 已有记录仍可读；新 Live 在 prepare/reserve 前检查 backend 策略 | SDK adapter sign 与 aggregator send 再检查；模拟不签名/发送 |
| PumpAPI Local | executor / SDK wrapper / PumpAPI execute 在 Live HTTP 请求前检查 | private submit 在 send_transaction_with_config 前检查；显式模拟返回 RPC simulate，不进 submit |
| SOL 转账 | HTTP 在数据库/钱包操作前检查 | SDK transfer wrapper 在 lane/金额/RPC 准备前检查；固定数量和 50/100% 均不能覆盖 |
| WSOL submit | HTTP 和 application 入口检查 | 保存签名后的直接 RPC send 前再检查；独立 simulate 接口保留 |

报价、寻池、预热、余额、健康、能力、订单查询和原签名对账不广播，继续可用。普通请求若还没通过身份或参数校验，可能先返回原先的 400/401，而不是策略 403；批量接口维持逐项错误合同。已存在的幂等记录仍可读取，不执行第二次交易。

成功加载新版本后，`/ready.paused` 在禁止广播模式下为 true；`ready` 本身仍表示依赖健康，不能把 ready=true 当作发送许可。启动日志只记录策略布尔值与鉴权模式，不打印管理 token。

当前 wrapper 两处 TradingClient 初始化均设置 `create_wsol_ata_on_startup(false)`。本地 vendor 6.0.2 的已查初始化路径以此控制启动 ATA 发送；本轮未运行带真实 SDK/钱包的初始化验收。升级 SDK 必须复核启动行为与新增发送方法，不能只信版本号。

## 管理接口分组

separate 模式下以下 8 个操作使用 `Authorization: Bearer <WALLET_ADMIN_API_TOKEN>` 且 peer 必须是 loopback：

| 方法 | 路径 |
| --- | --- |
| GET | `/v1/wallets/manage` |
| POST | `/v1/wallets`（创建/导入） |
| PATCH / DELETE | `/v1/wallets/:wallet_id` |
| POST | `/v1/wallets/:wallet_id/export` |
| POST | `/internal/wallets/vanity-jobs` |
| GET / DELETE | `/internal/wallets/vanity-jobs/:job_id` |

普通 `/v1/wallets` 公钥列表、交易/报价/状态等继续使用交易凭据。内部余额、SOL 转账、WSOL 兑换继续要求交易凭据与 loopback；资金动作另外受广播策略约束。管理凭据不能直接调用交易，但能导出私钥，本质上仍是高权限钱包控制凭据，必须最高级别保护。

不信任 X-Forwarded-For。loopback 指应用的 TCP peer，不证明反向代理后真实来源是本机：公网代理必须拒绝转发管理接口，不能只靠 loopback 判断。远程管理使用受控 SSH 隧道，而不是把管理员令牌暴露在公网 HTTP 上。导出成功响应增加 Cache-Control:no-store；禁止把真实导出结果加入测试日志或文档。

## 实际分层

- `domain/execution_policy.rs`：SDK 无关的不可请求覆盖策略。
- `config/security.rs`：纯参数校验、兼容默认值、管理秘密的 Debug 脱敏。
- `api/middleware/auth.rs`：原交易/内部鉴权与可选管理鉴权；旧授权入口委托到这里。
- `validation.rs` 与 executor：请求模式解析和进执行器前的复核。
- WalletRegistry、SDK/PumpAPI wrappers、聚合 adapter 与 conversion：按上述清单检查策略。

没有修改 vendor、钱包格式、数据库 schema 或 HTTP 请求/响应字段。READY 的既有 paused 在显式禁止广播时反映 true，导出响应多了 no-store 头。默认 legacy 下全部旧合同测试保留。

## 离线验证

新增测试覆盖配置缺失/显式值/无效值、独立凭据为空或重复、秘密 Debug 脱敏、8 个管理操作的错误凭据/远程拒绝、管理凭据不能交易、legacy 兼容、请求模拟默认/覆盖/MEV 无法突破策略、executor 绕过 HTTP 仍被拦截、转账/WSOL 的 HTTP 和应用入口、独立聚合在预检前被拦截与已有记录继续可读。

SDK wrapper 测试使用完全没有 signer、SDK client、RPC 或 lane 的对象。若策略检查遗漏，测试因不同错误或 panic 失败，不会连接真实网络。直接验证 buy/sell（含 simulate）、聚合发送/签名、SOL 转账及 PumpAPI Live 在资源访问前拒绝。没有真实钱包导入、创建或导出。

本轮新增 16 项测试；全套 132 项通过，格式与差异检查通过，构造函数可见性警告已修正。依赖、SDK vendor 和原 release 指纹不变，未构建 release 或部署。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

## 兼容迁移与发布门槛

1. 先在无生产密钥的环境验证构建和临时数据库；保持默认配置，不替生产启用新模式。
2. 管理凭据由操作者安全生成、保存和分发；如预置 token 但仍用 legacy，只是准备阶段，不代表权限已隔离。
3. 受信任调用方按路由选择交易/管理凭据，并先验证 loopback/SSH 隧道；本轮不自动修改 Bot。
4. 独立窗口改为 separate 并重启，验证普通交易凭据不能导出，管理凭据不能下单，管理调用仅能走可信本机入口。避免在核对时向日志/聊天打印真实私钥。
5. 禁止广播模式需独立窗口启用。用无生产密钥配置验证已知所有资金入口拒绝与只读/明确模拟继续可用；不以 SDK simulate 参数代替隔离，不用实盘测试猜测保护是否生效。
6. 如回退 legacy，是明确降低权限隔离，不是自动容错；只允许操作者在受控窗口决定。不得通过放宽来源 IP/关闭广播保护解决客户端报错。

## 仍未完成

- 两种设置都未在线启用；legacy 默认仍共享管理和交易凭据，历史风险没有因新增可选模式自动消失。
- 不是 OS 网络沙箱或无密钥模式：仍可能读取钱包、连接 RPC/MEV 服务，明确 RPC 模拟可能签名，管理导出未因禁广播而关闭。
- 后续 TS-04-B2 已移除公开原始 signer getter，并为 WSOL 增加 unsigned 模拟；见 `SIGNER_ACCESS_AND_UNSIGNED_WSOL.md`。底层 rpcs()/SDK TradingClient 仍有具体能力，继续以受限 ports 收口；此保证范围是已审查入口，不是阻止恶意 Rust 代码或未来未经审查的新增入口。
- SDK 模拟流水线在禁止广播模式下整体拒绝；要开放，需要先分离只读 builder/simulator 并独立验收，不在此次夹带 vendor 修改。
- 非多租户用户授权：全局交易凭据仍是可信服务级权限；未做每用户 scope、凭据热轮换、统一秘密类型、管理审计及跨请求限流。
- 转账幂等指纹/发送前签名保存、PumpAPI 指令审计、真实 SDK 初始化与生产故障演练仍有各自门槛。
