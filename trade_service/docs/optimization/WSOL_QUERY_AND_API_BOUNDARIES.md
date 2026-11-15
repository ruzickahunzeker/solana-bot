# TS-S1-G2-C WSOL 报价、状态查询与 API 边界

日期：2026-11-13。源码已接入；未部署、未重启、未访问生产钱包/数据库、未进行真实 RPC 或签名/广播。仅 Trade Service，不升级依赖或 SDK。前置阶段见 `WSOL_SUBMISSION_RUNTIME.md`。

## 已接入的职责

- `application/wsol_queries.rs` 接管报价与原签名状态查询。只接收项目输入、只读 source、journal 和 address parser，不依赖 AppState、Axum、SQLx、serde 或链类型。
- `application/ports/wsol_queries.rs` 分开 QuoteReader、绑定原签名的 StatusReader 与 query journal；没有 signer、消息、Live reservation 或广播方法。
- `adapters/sdk/wsol_queries.rs` 复用原 registry/repository，仅查询公开 metadata。报价 reader 在解析 owner 后获取原 RPC handle，只私有持有 RPC，不保留 SDK 钱包/签名能力；状态 reader 先按原 SDK 规则解析签名，再获取已加载钱包的原 RPC。
- `adapters/rpc/wsol_status.rs` 私有持有原 RPC 与已解析原签名，仅执行一次带 history 的查询。无重签、发送、备用节点广播或后台任务。
- `sdk_client/wsol_conversion/queries.rs` 负责原 WSOL 余额、账户、预算和手续费估算；`instructions.rs` 接管原四种账户形态的指令 builder，字段仅 SDK 模块内部可见。没有重复 builder，也不再反向导入 conversion 的 wire enum。
- `api/dto/wsol_conversions.rs` 接管原 DTO、Query 与内部转换；`api/handlers/conversions.rs` 接管四个薄 Handler；`api/routes.rs::wsol_conversion_routes` 合并原四个路径。Handler 不取 SDK、RPC、SQL 或 wallet repository。
- `adapters/persistence/wsol_conversion.rs` 保留唯一旧 V1 fingerprint/绑定兼容规则，query/submission 端口复用原 store Arc。DTO 映射归 API，不把内部模型重新序列化成一个新 hash。
- 旧 `conversion.rs` 只保留原 store/schema、必要 V1 类型重导出和测试 oracle；四个旧业务包装函数已经移除，没有隐藏 fallback。内部 submit/simulate 禁播测试改为直接调用 Application，仍不经过 HTTP。

## 保留的行为

报价顺序仍为 store → metadata/owner → all 互斥/方向 → owner 语法 → loaded wallet → RPC/账户/余额 → amount → rent/fee/budget → 成功后取时间/ID → 保存。缺金额仍是读取账户后的错误，不在目录迁移中提前。Quote 仍 13 字段、60 秒 TTL、返回 executable=false；不因 Live 开关或只读查询成功扩大交易能力。

状态顺序仍为 store → record → owner → 仅 signed/submitted/unknown 且有签名时解析原签名和获取 reader → 查询 → 必要时保存 → 再读回。终态/unsigned 不访问 runtime，解析错误先于 missing wallet；RPC 失败/无结果保留原记录，不自动补单。

旧 WSOL 确认映射明确保留：err 存在为 failed，confirmations=None 为 confirmed，其他为 submitted。这不是通用 ChainObservation 的最终性算法，也不是已完成可信链上确认验收；迟到结果、异常 RPC 形态和外层 deadline 属于 G3 安全修正。

四个 URL/method、JSON/Query 先提取、loopback/Bearer、禁播/Live gate、400/403/422、七字段提交/四字段模拟响应、错误脱敏及默认值保持。OwnerQuery 只迁移 WSOL 的 DTO，其他钱包管理入口不重构。只读报价/状态查询仍可在禁止广播时执行。

## 字节兼容证据

在 API 移动前先跑全套 551 项与针对性 61 项，再迁移 DTO/Handler。冻结旧 fixture 的完整 `(submission, quote)` JSON 字节及 SHA-256：

`ce2b1bb83be3c1f5bc569a1bd8842846469e71d3051ad9e28b8b0777ee30700e`

旧方法与外部 `sha256sum` 对相同固定字节一致；移动后继续用同一黄金断言。原字段序列、flags、literal key/owner 和 Quote 时间/费用参与 hash 的方式不变。API 类型移动不创建新 schema 或重新解释历史 key。

迁移前后逐字比较原 MIGRATION 和完整 ConversionStore 实现（7,458 字符），均完全一致；状态 SQL、连接策略、幂等唯一范围和恢复行为没有借重构改变。该证据不代表其既有安全缺陷已消除。

## 离线覆盖

相对 G2-B 新增 20 项；原 builder 测试只移动，不计新增：

- 原 Handler 状态错误优先级 1 项，拆分前先验证 owner/签名解析/空 runtime。
- Application 9 项：原身份/TTL/save、验证顺序、all/missing amount、disabled store、终态无需 RPC、owner/reader 构造顺序、RPC 错误/无结果、旧确认映射/读回、写失败。
- RPC adapter 2 项、SDK query source 1 项：旧确认/错误文本、空/缺失结果、故障及原 handle、metadata 不解密、签名先解析。
- 真实 SDK quote helper 2 项：新/旧 ATA、全量/部分解包、租金返还、100,000 预留边界、溢出与账户身份错误；只使用内存 RpcClient mock，不连接节点。
- 真实 Router query 3 项：完整 wire/同 store、禁播仍可读、owner/原签名/终态、保存故障；使用无密钥 fake source，查询不会提交订单。
- V1 黄金字节/hash 1 项；独立 route group 与主路由对照 1 项：四路径、method、auth、提取器、disabled policy 和无钱包副作用。

本批曾修正两处测试假设：借用 JSON Value 的比较需解引用；SDK 自定义错误文本不能假定以十进制显示，改为与同版本 TransactionError::to_string 精确比较。没有为测试放宽生产逻辑。

## 检查命令

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet wsol
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

最终默认并发/串行全套各 552 项、HTTP 114 项、wsol 筛选 62 项通过，无测试构建警告；fmt/diff 通过。生产 check 通过，只有既有 models/persistence 两条兼容重导出警告，未加 allow 隐藏。manifest/lock/原 release 指纹与 `SOL_TRANSFER_API_EXECUTION.md` 一致；不构建 release，不执行生产迁移或修改 SDK vendor。

## 下一项：G3 安全修正

1. 基于现有唯一 ConversionStore 设计精确单向 CAS。迟到 unknown 不能降级已观测 submitted，迟到回执不能覆盖终态；竞争失败后读回权威记录，不重新执行。
2. 明确签名与模拟的持久化顺序及失败阶段，补 canonical WSOL 指令全集、账户/金额/费用与 Quote 授权的签名前审计。签过的交易不能简单以“未签名失败”丢弃证据；保留原签名/字节，不清空或换 key 补单。
3. 覆盖 reserve 提交、prepare、sign/record_signed、simulate、send、结果/对账写入各处取消和文件重开；reserved/signed 恢复只观测或保守标记，绝不自动重发或释放不确定 pending wallet。
4. 原签名查询的 deadline、RPC 形态/确认级别验证及读取/更新竞态单独作为行为修正验收，不把旧映射的测试当成未来安全标准。
5. 存储/结果不明的错误分类和“不换 key”提示需明确调用方合同；不能把旧 400 当成已证明未发送，也不能在目录移动中暗改 Bot。

G2 的分层不等于完整资金安全、生产验收或全协议支持完成。之后仍需 PumpAPI 外来消息审计、普通交易发送前账本、协议/扩展证据、基础设施/依赖规则、SDK 来源/升级演练与受控发布回滚；本轮约束持续有效。
