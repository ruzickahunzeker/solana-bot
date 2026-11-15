# C3b7 原候选的受控签名核心

日期：2026-11-15。源码和受限类型检查完成，五项新增用例及完整回归待 CI。
正式 executor、真实钱包 getter 与供应商 sender 尚未接线，不代表上线或全协议授权完成。

## 签名入口

- Application 在原 owner 的 signing CAS 成功之后才构造 `SdkSigningPermit`。
  类型没有 Clone/Serialize/Deserialize/Debug；构造器只对控制器模块可见，绑定原 owner、
  完整命令、字面键、原 metadata 和整条 message，没有恢复或数据库反序列化入口。
- 新 SDK `DammSellCandidate::sign_original` 消费原不可变候选与一次性 permit。
  真正读取 signer 的同步闭包在 permit/原执行绑定、原 context/非模拟模式、raw 余额/报价时效、
  原 profile metadata、全指令和标准消息重构核对之后才调用；不包含 RPC/await/SDK worker。
- 签名核心核对规范化 MEV 声明和选定 provider，拒绝明确 MEV 请求却选 Default、指定 provider
  不匹配及原 wire 校验禁止的冲突。复用原 SDK adapter 的 provider 映射，只扩大其 crate 内可见性；
  没有改原选择器或 wire 归一化。Auto/全局模式的实际可用 lane 注册仍由后续真实 backend 验证。
- 当前已支持的 DAMM 卖出 profile 固定池/相反两端资产/Token Program、canonical 输入/输出、
  整数百分比输入/整数 min_out、CU/小费/接收者、账户权限、指令顺序、payer/唯一 signer 与 blockhash。
  使用之前通过读取/无签名 probe 生成的同一 evidence Arc，不把 nominal DB 行当新的链上证据。
- Getter 返回后再次检查公钥和原证据期限，然后只签原零签名交易的 message；不重建业务指令、
  取新 blockhash、换路由、换 key 或重新获取报价。签名用标准 Solana `VersionedTransaction::try_new`。
- 新 `DammSignedCandidate` 私有持有原候选、签名字节、原 request ID/hash，不能反序列化或改写。
  后续 `recheck` 验证同一原订单/上下文/不可续期证据和签名，用于每次发送门槛；原字节只读提供给账本。
- 已经生成签名之后的编码/核验错误附带 typed 原签名；控制器在返回未知结果时保留诊断，
  不因为 artifact 释放失败、丢确认或错误文字就变成可重试 failed。

这些检查约束该固定 profile 的签名前授权，不保证 RPC 诚实、链上程序未升级、未来状态/价格不变、
手续费/租金已足额、交易必成功，或所有协议/Quote/方向已支持。真实后端选择/发送仍须下一步验收。

## 实际测试边界

原十项控制器测试中的第一种后端现调用真实的 SDK 受控签名核心及 Ed25519 签名，
signer 为测试生成的临时 Keypair；另一后端保留不同的项目 Candidate 表示。
两者仍使用真实隔离 SQLite、假发送器，发送点核对原库字节，无生产钱包/RPC 或广播。

新增五项：替换 candidate/context/key 不能释放字节并计数 key 获取；实际签名后 artifact 失败保留签名；
签后产物不能换原订单；源证据在初始/获取 key 前/签后落盘后过期均不能被存储或签名续期。
另有十组规范化 MEV/Default/provider 组合及两项既有 wire 冲突拒绝；新临时钱包 helper 使用合法
`use_mev=false` 的 Default 夹具，原 384/576 ABI oracle 及旧 nominal fixture 不改。
测试通过仅 cfg(test) backdate 注入过期证据，生产没有时钟重置、可写候选或续期方法。
最终受限类型检查及专项/完整 CI 结果另记，不借用历史通过数字。

最终受限 test check 约 10 秒、RSS 1,216,508 KiB；bin check 通过且仍 21 组警告，
包含尚未接正式入口的 permit/受控核心，不压制或加假调用。fmt/diff、policy 七项及来源检查通过。

## C3b6 CI 与夹具修复

`bd39f04` / Actions `43678564856` 已终态 failure，artifact `14147911636` 下载核对：
类型检查/测试构建及 candidate 15 通过，prepared 21 通过/1 失败，控制器与完整套件尚未运行。
失败是损坏 Quote 行的测试 UPDATE 先被既有 CHECK 阻止，尚未到 reader 负例。

现在首先断言正常 SQLite 确实拒绝非法 Quote；随后仅在同一个隔离测试连接临时设置
`ignore_check_constraints=ON` 并以非缓存 UPDATE 注入损坏行，断言 reader 仍拒绝。
事务回滚后明确恢复 OFF。生产 DDL/约束/validator 没有放宽，原负例不删除。
此前 C3b5 prepared-only 读取修复已在本次 candidate 15 全部执行通过，不重复实施。

## 发送接线前的已知事实

- 固定 SDK 的高层 worker 会构建/签名/排队发送，取消调用者不一定取消后台工作；
  新路径必须绕开该 worker，而非只把它放进新 trait。
- `SwqosClientTrait::send_transaction` 接已签交易并返回 `Result<()>`，不返回供应商签名。
  Default RPC 配置 maxRetries=3；Temporal 可把同一编码批次从 QUIC 切 HTTP。
  这些是原字节传输行为，不是更换交易的许可，也不能宣称整个供应商只发一次。
- 部分 provider 的 response 检查并不严格，不能把 trait 的 Ok 当作已核验的远端签名或成交。
  下一步须明确接受证据与原签名单次/持续观测，保留 unknown，不伪造回执。
- 新 sender 仍需保留唯一 provider、固定 tip 接收者、原 configured minimum/built-in tip 规则、
  现有 wait/验证语义，并在字节落盘/独立读回/单次领取后只使用原 packet。
  需要有界取消/超时和原签名恢复，不能因失败调用旧 worker、换 lane 或另造 key 补单。

## 接续主线

完成真实钱包 getter/唯一 sender adapter 与原 executor 的实际接线，补原签名只读观测/恢复，
然后扩展其他方向/协议/backend/mode 与资源/升级演练。当前无 SDK/依赖更新、配置或迁移变化，
无部署/重启、生产数据或实盘交易操作；PumpAPI 开发继续暂停，整体目标 active。
