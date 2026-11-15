# TS-04-B1 独立报价的不可伪造构建句柄

日期：2026-11-13。状态：源码和 181 项离线测试通过；未构建 release、未部署、未做真实 RPC/签名/广播。

2026-11-14 后续 NP-01-B3a 为实际待签消息释放补充原 Quote 到期快照与签名前重查，先复现原缺口，再通过三项新增测试及并发/串行各 989、HTTP 196 回归；见 `QUOTE_SIGNING_FRESHNESS.md`。本页保留早期 B1 交付与风险语境，不等于最新完整安全状态。

后续 B2 已完成原始 signer getter 迁移与 WSOL unsigned 模拟，当前全套 193 项；见 `SIGNER_ACCESS_AND_UNSIGNED_WSOL.md`。下方保留 B1 检查点的实施记录和遗留清单，不代表后续未推进。

## 实施

独立 Quote 提交的 `SdkQuoteSubmission::Prepared` 不再直接是 `VersionedTransaction`，改为 SDK adapter 拥有的 `PreparedQuoteTransaction`：

- 字段和生产构造函数为模块私有；没有 serde、Clone、原始交易 getter 或可由 API 填写的字段。
- 只有 adapter 在原 `prepare_transaction` 完成审计/模拟后构建，内部保存未签名交易、绑定的钱包公钥和预检授权结果。
- Application 只拿不透明句柄，不能构造任意 SDK 交易交给该 `sign` 方法，也不能在预检后替换 message。
- `sign` 先检查服务端禁止广播策略，然后核对预检授权、绑定钱包、payer、单一必需签名及全零原始签名，再进入原钱包签名逻辑。
- 授权结果要求 Quote ID/Wallet ID 一致，以及报价有效、已构建、聚合模拟、本地模拟、指令审计、executable 六项都成立。
- 预检不可执行仍返回原 preflight，交给既有应用层处理；不会在 prepare 中提前签名或占用新的 key。

这是一项防止内部调用误用的能力边界，不是把一个布尔字段当成全新可信报价器。实际报价/指令/余额/费用检查仍由原 prepare 流程执行，未修改算法和白名单。

## 兼容性

- HTTP 路径、字段、状态、quote/preflight JSON 与 journal 指纹不变。
- Application 的保留、过期检查、签名保存、广播和 unknown 恢复顺序不变。
- 正常预检成功对象继续使用原 `VersionedTransaction::try_new`、签名字节和发送方法，不改变 provider 或费用策略。
- 模拟和历史订单重放不会调用 sign；原离线计数/恢复测试继续通过。
- SDK wrapper 禁播测试使用仅在 `cfg(test)` 下存在的未授权对象构造器，验证策略检查先于 SDK wallet/signer 访问；生产代码没有该构造器。

## 新增测试

4 项无 signer/RPC 的测试验证：

1. 正确钱包/unsigned 形状允许，错误钱包或 payer 拒绝。
2. 六项预检授权任一关闭都拒绝，不能靠应用层遗漏检查绕过。
3. 其他 Quote ID 或 Wallet ID 的 preflight 无法授权。
4. 缺失、预填、多余签名或多签名消息拒绝。

原 4 项无 signer 的 SDK 禁播测试、普通交易兼容修复与全部历史合同同时通过。

## B1 遗留清单

- `conversion_signer()` 仍向旧聚合路径和 WSOL 模块返回 Arc<Keypair>；不能宣称所有签名权威已经收口。
- 普通 `/v1/trades` 内部聚合分支仍有旧发送路径，缺乏独立 Quote journal 的发送前签名持久化保证。
- WSOL 的准备函数目前会签名，独立 simulate 路径也复用它。必须先拆成 unsigned prepare，再为明确 Live 的已保留操作提供受控签名；不应把它称为纯只读模拟。
- 原 SDK buy/sell 仍是组合流水线，保守禁止广播策略继续拒绝它们的 simulate；不能在未证明内部无副作用时放开。

## B1 后续顺序

1. 把聚合签名留在持有私钥的 SDK 封装内，只接收受控 Prepared 对象；外部不再取原始 signer。
2. 旧内部聚合分支复用 adapter 的 prepare/sign/broadcast，保留其旧 HTTP 合同，并明确它仍不是独立签名账本。
3. WSOL 分离 unsigned prepare、unsigned simulate 和已保留 Live 的签名步骤；需要完整 wrap/全额 unwrap/部分 unwrap 的离线 fixture，以及费用/租金/钱包/幂等回归。
4. 所有原始 signer 消费点迁移、搜索与离线验收完成后，再移除/私有化 getter；不只改方法名掩盖密钥能力。

以上都保持 SDK/vendor、API 与生产数据不变；若要改变模拟合同、持久化或广播语义，单独记录行为变更和验收范围。
