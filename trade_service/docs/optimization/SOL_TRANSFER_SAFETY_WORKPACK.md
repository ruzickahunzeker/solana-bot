# TS-S1-F2 SOL 转账安全执行工作包

状态：F2-A 阶段分离、F2-B 账本、F2-C 原 API/Application 安全接入均已源码/离线通过，见 `SOL_TRANSFER_SDK_STAGES.md`、`SOL_TRANSFER_JOURNAL.md`、`SOL_TRANSFER_API_EXECUTION.md`。不等于生产/链上验收。只改 Trade Service，不升级 SDK，不部署，不操作生产钱包/数据库，不进行真实 RPC、签名或广播。以下为工作包原要求，具体完成证据以上述阶段记录为准。

## 目标和兼容边界

保留现有 internal 转账路径、V1 金额模式、owner/系统钱包权限、地址和 key 规则、11 字段响应，以及手续费/租金算法。金额冻结指纹见 `SOL_TRANSFER_CONTRACT_AND_AMOUNT.md`。

以下是明确的安全行为修正，不伪装成无行为变化的目录移动：同 key 异内容返回冲突；签名落盘失败绝不发送；结果写入失败不能伪造完成；发送结果不明保留原签名与 unknown，不返回可盲目补单的 failed。unknown 和新增冲突错误需要发布前的调用方兼容验收，但本轮不改 Bot。

## F2-A：SDK 准备、签名、发送分离

阶段函数现已由 F2-C Application 编排；发送后 unknown 和持久化由 B/C 共同接入，原无 journal 组合入口已移除。

1. 把 native SOL 转账的 RPC 准备移到 SDK adapter/子模块，原钱包运行实例与 Default lane 复用，不新建客户端或默认钱包。
2. prepare 仅取公开身份、balance、blockhash、fee、rent，调用 F1 政策并构造未签名消息；保留原每 Default lane 5 秒准备超时和 fallback 顺序。
3. 返回不可外部编辑、仅 adapter 可构造的 PreparedSolTransfer；绑定原 source、destination、整数金额、费用/余额快照、blockhash 与选择的 RPC。API/Application 不拿 Keypair 或 raw RPC。
4. sign 再核对原钱包身份、payer、所需签名、唯一 System transfer 指令及精确目标/金额；不能签外部任意 Message。尽量复用已有 SignedSubmission 字节/签名模型，不复制通用提交系统。
5. send 必须发送已签名的同一字节，不换 blockhash、不重新计算百分比、不重新签名。已核实锁定 RpcClient 会拒绝不同回执签名；B/C 须先记录本地原签名，再把这类错误按 unknown 处理，不能因此推断未发送。
6. 保留原发送 12 秒、确认 15 秒边界；确认未达成仍为 submitted，发送超时/错误为待对账 unknown。禁止在不确定发送后切 RPC 自动补单。
7. 先用内存 RPC/fake 副作用验证准备零签名/零发送、opaque handle 校验和所有超时；该阶段本身不算发送前持久化已接入。

## F2-B：发送前账本与旧格式兼容

以下账本层工作已实现并通过 18 项离线测试。F2-C 已补 fake 端到端签名/发送/取消故障与原 Handler 替换，不代表真实签名或广播验收。

1. 从 WalletRepository 提供受限 journal 工厂，不公开连接/私钥；获取既有 handle 不触发初始化、恢复或新池创建。
2. 使用钱包库的增量 attempt 表或等价兼容扩展，保留 wallet_sol_transfers 的旧列/解码。建立保留、已签名、已提交/unknown、已确认/失败的明确阶段，不建立第二套会相互覆盖的幂等权威。
3. 原 idempotency_key 是全局唯一，不能悄悄改成 wallet-scoped。新 intent/fingerprint 绑定操作/版本、literal key/owner、wallet_id、source pubkey、解析后的 destination 和请求的固定金额/百分比；不绑定当次余额或重新报价，以免重试生成不同操作。
4. to_wallet_id 和 raw address 经权限核对后，同一实际目标用规范公钥绑定；label 变化不改变身份。50% 与等额 fixed 仍是不同请求意图。
5. 首次 projection 与 attempt 保留要在同一事务完成；唯一约束处理竞争，不能只靠内存锁。相同内容重放，异内容冲突，无法核实旧数据则 fail closed。
6. 完整签名、signed bytes 和准备快照必须持久化成功后才发送。针对钱包池核对 synchronous=FULL 的全部连接初始化，记录性能/发布门槛；不能把 WAL 默认值当作已具备所需落盘保证。
7. 新 intent/元数据保留精确整数，不盲用 u64 as i64。旧列维持兼容表示；新账本可用规范 JSON 或经范围检查的存储字段。
8. 未有 attempt 的旧记录逐项核对原 source/destination/金额/owner；保留已有签名/终态，不因过期、重启或账本升级自动重发，不覆写历史记录来消除冲突。processing/无签名且来源不明需明确拦截/人工核对。
9. 启动恢复只做状态观测，不发送；签名已保存但结果缺失保留原签名并进入 unknown。进程取消和各写入故障都要留可恢复证据。

## F2-C：Application 与原 API 接入

已完成源码/离线接入，新增 22 项 Application/runtime/Router 测试。复用既有 metadata、F1 金额输入、F2-A 原实例/不可编辑 artifact 和 F2-B journal，没有新建保留表或恢复流程。生产构造检查中的 5 条未使用账本接口警告已消除。验证详情见 `SOL_TRANSFER_API_EXECUTION.md`。

1. 保留原提取/鉴权、禁播、key、source/owner、destination、金额错误顺序；不通过提前 durable 检查改变旧错误优先级。
2. 通过项目输入、metadata directory、受限执行与 journal ports 编排，原 Handler 仅做 DTO/HTTP 映射。接受时核对 loaded wallet 与 source 公钥，不因默认钱包或注册表替换换人签名。
3. 执行顺序：授权/校验 → 原子保留或重放 → 保留原 runtime → unsigned prepare → 授权签名 → record_signed → 一次发送 → 结果/确认持久化 → 读回。
4. prepare/签名前失败可明确 failed；已有签名后出现任何不确定结果保留原签名，不能当未提交重试。所有 finish/写入错误必须显式传播，不能用 let _ 忽略。
5. 完成新路径合同后移除无 journal 的旧组合发送入口，不保留隐藏 fallback 继续绕过新流程。不要为了兼容内部函数保留生产可调用的不安全资金路径。
6. 不新增通用转账 API、不改普通交易/WSOL 语义、不扩展 Telegram UI。只读对账可后续按原签名 port 接入；不引入自动重签/重发或默认后台定时器。

## 离线验收门槛

- 完整旧 V1 请求/响应、权限、默认值、金额及错误顺序回归；F1 1,890 组金额指纹不变。
- native unsigned prepare 的账户/指令/费用/方向检查，不能触达 signer/send。
- 同 key 同内容、同 owner 异金额/钱包/目标、不同 owner、50% 与 fixed、literal key，均有真实 Router/临时库证据。
- 跨任务/连接竞争，只一笔可以签名和发送；先落 signed bytes 后进入 fake send。
- reserve/sign/record_signed/send/结果/确认写入每阶段失败及取消、文件重开、迁移重复执行与恢复；原签名/字节不丢失、不重签/重发。
- 错误返回、签名不一致、超时、确认未知不误判未发送；无自动补单及新 key。
- 旧记录无 attempt、损坏/冲突、已有签名、旧 projection 字节兼容和新同步策略证据。
- 默认并发/串行全套、HTTP、fmt/diff 检查；manifest/lock/vendor/release 身份不变。

所有数据库改动只在测试库验证。真实启动、迁移生产库、钱包/密钥操作、部署和实盘验收仍属独立授权窗口，不以 mock 测试替代。
