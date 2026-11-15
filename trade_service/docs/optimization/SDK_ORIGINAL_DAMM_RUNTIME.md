# C3c1 原 SDK owner 的实际 DAMM 执行接线

日期：2026-11-15。C3c1 源码及完整离线 CI 已通过；C3c2 四项串联后于 `fd1c7cb` /
Actions `43685088919` 验收（完整并发/串行各 1398），C3d1 显式恢复也已验收（各 1416）。
当前 C3d2 扫描及 C3e 完整准备串联已于 `6c50aca` / Actions `43690435964` 验收，
完整并发/串行各 1437，通过五入口实际 runtime/16 项扫描/历史迁移专项，
见 `SDK_ORIGINAL_RUNTIME_INTEGRATION.md`。下方 C3c2 待验/恢复未接是当时历史，不覆盖最新状态。
没有部署/重启、SDK 升级、生产钱包/数据库操作或实盘广播；不是发布完成声明。

## 源码中的实际调用链

`executor -> original ordinary claim -> sdk_journaled -> prepare_original_damm`
`-> Application prepare/readback/signing CAS -> linear permit -> controlled signer`
`-> signed journal/readback -> broadcast CAS -> fixed sender -> receipt/readback`。

- 仅新 Durable、Live、SDK、直接 DAMM v2 卖出使用此链；依据原 `operation_for_claim`，
  不扩展 Memory/Shadow、模拟、买入、其他协议/多跳、聚合或暂停的 PumpAPI。
- SDK claim 在全局禁播检查之后、building/钱包解析之前取得，原 owner 按值传递，
  同键竞争/取消/重开不生成新的执行所有者。未取得 owner 不进入后端。
- 真实 `SdkOrdinaryExecutionBackend` 复用主程序的同一钱包 registry、共享 read runtime、
  原 `TradePersistence` Arc；没有新建 SDK、HTTP runtime、独立订单库或签名 authority。
- 同一已解析钱包贯穿全部 await，准备前核对其公钥与原 admission 公钥；准备沿用原
  `LaneExecution`、raw 输入/本笔报价和参数映射，只有原可用性准备错误可尝试下一 lane。
- 预览、候选核对或任何 journal 阶段失败均不换 lane。已提取的无签名 preview 验证
  同时供旧路径与新路径使用，原 RPC 参数、slot 下限、原 Arc/期限与模拟失败拦截保持。
- 冻结同一 sender 的小费接收者及原配置 minimum，构建不可变 live 候选。
  `sign_original` 读取原 wallet getter 前核对 permit、原消息/证据/钱包和规范化 MEV。
  新路径不调用 `sell_by_percent`、SDK builder/signing queue 或旧 opaque worker。
- signed 和父行 submitting 原子提交，再独立读回，单次领取已落盘原字节；最终 sender
  同时接收原私有签后 artifact，实际 I/O 前再次核对原订单、时效及逐字节一致。
- SDK 外层只用 `finish_journaled` 更新内存视图，绝不将旧 executor 快照补写覆盖原签名账本。

## 发送与观测语义

- 新 sender 只持原选定 SwqosClient、同一 RPC 与只读 observer，不持 Keypair 或重新构建能力。
  packet 只允许该固定 Legacy/V0-compatible profile，1232 字节、canonical bincode、
  一个 payer/signer、有效原签名；SDK Binary/Base64 序列化另有原 packet 对照测试。
- Default 使用原选定 RPC，保留 SDK 原 skipPreflight/processed/Base64/maxRetries=3/minContextSlot=0
  选项，3 秒外层 deadline；必须拿到匹配原签名的 typed 回执。
- MEV provider 只调用一次原 `send_transaction`、传入已签原交易，不进入高层 worker。
  SDK 传输可能有内部原字节重传或 QUIC→HTTP，项目不把它宣称为供应商 exactly-once。
- SDK trait 只返回 `Result<()>`，因此 provider Ok 不冒充可靠远端签名。
  只按原签名、原已注册 RPC 查询；已有 processed 且无错误的原签名可返回 submitted，
  否则为 unknown。传输错误后若明确观察到原签名成功，也不重发。
- `WAIT_TX_CONFIRMED=true` 由项目只读观测等待 confirmed/finalized，预算 15 秒；
  provider 内部 wait=false，避免其隐藏确认查询。提交尝试预算 3 秒，确认预算从之后开始。
  false 时 MEV 观测使用 `MEV_SUBMIT_VERIFY_MS`（最多 15 秒）；0 不查询且无可核验回执时
  保留 unknown/原签名，不自动普通 RPC 回退。Default typed 回执不受 0 的影响。
- observer 限制为选定且已注册的 URL（包括 key 身份），复用原 runtime 的 observation
  配额/250ms pacing/429 冷却/体积与单步预算；本地 reader 间隔 1 秒，不建第二个 HTTP client。
  查不到、忙、限流、错误 bank/slot、不同签名或链上错误都不授权重签/重播。

这里的 submitted 不是链上最终确认；unknown 不是没发送。GET/列表/重复 POST 仍只读存储，
没有新增 RPC 副作用。后续 C3d1/C3d2 已在原 SDK scope 接显式恢复/有界只读扫描，
见 `SDK_PENDING_OBSERVATION_SCAN.md`；不是旧聚合扫描天然覆盖全部 SDK/模式。
真实 provider 的接受语义、延迟、额度及节点格式仍待独立实网验收。

## 兼容与失败

- 外部路径、DTO、数量/单位、参数映射、旧 hash/JSON、wallet ID、公钥映射及幂等键不改。
- 新内部 rejected 阶段只用原文本列和原父行：空/过期 blockhash 仍为外部 rejected，
  钱包不存在及准备失败仍为 failed。只允许 claimed/prepared，无签名，原 owner/CAS/读回一致；
  signing/signed/broadcasting/submitted/unknown 都不能因此降级。没有新 DDL 或配置项。
- 未迁移协议/模式继续旧路径。全协议唯一签名前账本、动态全局安全门、全服务 RPC/wire
  预算、SDK 其他内部读取、完整资源隔离和真实生产恢复/回滚仍是剩余门槛。

## 验证与前轮 CI

新增 20 项：10 项 keyless sender（Default 原字节/选项、错误回执、无证据/传输错误、
坏观测、确认模式、packet/小费、取消、SDK 编码），8 项真实 executor + 临时原库的分流/
唯一 owner/非目标隔离/空 hash/钱包缺失/禁播/竞争/取消/拒绝写失败不覆盖原记录，
2 项选定 observer 注册/资源合同。另保留原签名后不能 failed，并补不能 rejected 的断言。
这些组合证明各层离线合同，不替代真实 SDK 初始化/全部准备 RPC→原 signer→供应商的集成验收。
所有 signer 使用临时测试 Keypair，发送器/RPC 为内存 fake，无生产身份或广播。

最终受限 test check 约 8 秒/RSS 1,242,528 KiB，bin check 约 6 秒/RSS 779,480 KiB，
fmt/diff、policy 七项及来源检查通过。生产警告为原五组；此前未接线代码由实际生产调用引用，
没有新增 allow、假调用或通过跳过测试消除警告。manifest/lock/vendor/原 release 身份保持。

C3b7 `83036a4` / Actions `43680602908` 终态 failure，artifact `14147664555` 已下载核对：
candidate 15、prepared 22、signing journal 11、controller 10 通过，真实受控签名 4/5；
后续完整套件未执行。失败为过期注入 helper 经过生产构造器而先被拒绝，不是签名已绕过时效。
现仅 cfg(test) 复制原字段后 backdate，保留原 raw 身份/slot，不修改生产构造器或时效门槛。
新批次必须重新运行五项/所有原专项/完整两线程与串行套件，不借用旧 1348 全绿。

该要求现由 `dd946aa` / Actions `43683583741` 完成：artifact `14149574413` 已下载核对，
所有步骤 exit=0，完整并发 1394（124.26 秒）/串行 1394（225.17 秒）零失败/忽略/过滤。
prepared 22、签名账本 11、controller 10、受控签名 5、sender 10、executor 8、observer 2、
HTTP 200、参数 3、候选 54 与 probe 59 均通过。旧 C3b7 失败为历史，不再当作当前阻塞。

## 后续

源码专用验证提交 `dd946aa` / Actions `43683583741` 已完成，结果见上方。
之后新增四项 `sdk_original_sender_journal_integration_*`，将真实受控 signer、原 SQLite
账本与本 sender/RpcClient 串联，内存传输在 egress 逐字节核对 broadcasting 已落盘的原 packet。
覆盖成功、错回执、receipt 写失败及发送中取消；仅受限 test check 通过（约 9 秒，
RSS 1,315,280 KiB），尚未运行，不冒用先前 CI 结果；不改生产代码/SDK/API。
完整准备 RPC→原 signer→provider 与原签名恢复仍未验收。

先完成本批 CI，再接 SDK 原签名的只读观测/恢复与端到端故障注入，覆盖其他方向/协议/backend/mode，
并完成剩余资源边界、升级/新协议演练及发布前备份/迁移/回滚验证。实际生产动作另需明确授权。
只读恢复的源码缺口、实施顺序和验收门槛见 `SDK_ORIGINAL_SIGNATURE_RECOVERY_PLAN.md`。
