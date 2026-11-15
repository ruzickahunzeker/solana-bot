# C3b5 原 SDK owner 的签名字节账本

日期：2026-11-15。源码及受限类型检查阶段；C3b5 首轮 CI 失败，修复与新增 Application 见下。
这是原 `ordinary_execution_claims` 的 SDK 子路径，不是新的订单、Quote 或幂等权威。
正式 executor 尚未切换，不表示已经提供完整业务授权或可上线。

## 原 authority 与阶段

- 继续要求原 `AdmittedTrade` 和不可 Clone/反序列化的 `ExecutionOwner`；完整命令、
  钱包/公钥、字面键、hash、operation、nonce、父行请求 JSON 在每次事务前后检查。
- 复用既有 005/008/009 列，无新 migration、配置、公开 API 或存储密文变化。
- 原 `claimed -> prepared` 保持；新增 `prepared -> signing -> signed -> broadcasting`
  `-> submitted | unknown`，全部单向 CAS。记录签名时父行在同一事务变为 submitting。
- `begin_sdk_signing` 仅取得本次账本阶段，不是完整业务授权，不返回通用 signer。
  同一候选的 live quote/raw 证据、时效和完整消息政策仍须在实际 signer 入口再次检查。
  存储 metadata 重构不能代替这些证据，也不能续期或生成恢复 owner。

## 签名字节与一次性领取

- 新 SDK adapter 纯核验函数使用现有 bincode/Solana 库，1232 字节上限、拒绝尾随字节、
  sanitize、canonical 重编码、完整原消息/payer/一个 signer、签名字符串及 Ed25519 验签。
  即使重算 digest，也不能以另一笔消息、另一钱包、零签名或坏签名替换。
- 写签名和父行一起提交，事务内核对后还有独立 `read_sdk_signed`；独立读回是证据，
  不是可重用的发送许可。`take_sdk_broadcast` 对原库执行 signed -> broadcasting CAS，
  只有成功提交后才能返回已落盘的原字节，同一 owner 竞争只有一个领取者。
- 领取后取消、提交错误、回执保存失败或重启均不把阶段恢复为 signed。
  新 owner 不能从库中取得；本模块没有 RPC、signer、重建、重新签名或自动广播方法。
- 回执只能引用同一签名/原字节，不接受成功同时附错误；不确定回执为 unknown，
  保留原签名与脱敏错误。后续只读链上观测另接，不把 unknown 改成可重试失败。
- 这只约束项目账本的一次性领取，不证明供应商内部零重发、全链路 exactly-once、
  刷盘/断电安全或链上成交；真实发布与供应商发送语义仍待验收。

## 验证与范围

新增 11 项离线用例：正常阶段/原字节；跳步与 prepared 变化；十类签名/消息/wire 变异；
错 owner/六类父身份变化；unknown 保留；四阶段三类写失败/读回篡改回滚；
单 pool 竞争；digest 重算/部分字段；回执替换拒绝；五阶段文件重开；两个独立 pool 竞争。
测试仅在隔离临时库生成临时 Keypair 并离线签公开合成消息，不获取生产 signer 或发送交易。
原 [C3b1 测试](SDK_CANDIDATE_PREPARATION.md) 不删除，原 [参数合同](ORDINARY_UNSIGNED_BUILD_AUDIT.md)
与完整 HTTP/并发/串行测试继续由 CI 执行。本机不重复大链接。

首次受限 test/bin 类型检查约 9/5 秒，RSS 1,270,136/652,524 KiB；
新增双 pool 用例后 test 复检约 9 秒、RSS 1,189,688 KiB，通过；fmt/policy 七项和来源检查通过。
生产目标为 12 组警告：原七组和本模块尚未接正式路径的五组，没有新增抑制或假调用。
manifest/lock/vendor/原 release 身份保持；不改其他服务、不升级、不部署、不操作生产库。
PumpAPI 开发仍暂停。

已推送独立验证分支提交 `3bd979bca10aaced601b4544db35252e80384862`，
[Actions 43676492857](https://github.com/ruzickahunzeker/solana-bot/actions/runs/43676492857)
已终态 failure，artifact `14146766179` 核对：candidate 14/1，失败为原 prepared-only reader
仍接受 signing，后续 11 项账本专项/完整回归未执行。本轮补精确 prepared 阶段检查，
合法 signing 的 claim 必须 AlreadyHeld，其他损坏数据拒绝不放松；修复与新增十项应用层合同
见 [C3b6](SDK_SUBMISSION_APPLICATION.md)。不重复等待旧运行，不将前序 1348 当本轮验收。

## 接续主线

1. 实际 Application 编排从原 executor 取得并保留原 owner，接 prepared/完整授权/阶段 CAS；
   不让旧 `finish_trade` 覆盖签名账本，不伪造 `BoundQuote` 或新恢复执行者。
2. 完整政策复核之后，受控 signer 只接原不可变产物；最终签名前重新检查不可续期证据。
   失败/取消保留 signing 不确定性，不走 opaque SDK worker 重建发送。
3. 固定 sender 仅使用领取的已落盘原字节，核对回执签名；审查 provider 内部重试、费用、
   编码及确认行为后接正式路径。签名后不换 lane/路线/blockhash/key。
4. 扩展只读观测与恢复、其他方向/协议/backend/mode，以及真实节点/迁移/回滚验收。

以上四项仍未完成，不能因账本子模块通过而删除或宣称整体优化完成。
