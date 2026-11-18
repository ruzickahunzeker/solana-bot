# NP-03 参数缓存写入口与资源所有权

日期：2026-11-15。源码及完整离线 CI 通过，未部署。`449731c` / Actions `43691952757`，
artifact `14153850007` 下载核对：param-cache-port 两项、全部步骤通过；完整并发
1443（133.72 秒）/串行 1443（236.47 秒）零失败/忽略/过滤。

## 实际变化

- 复用已有 `ParamCacheWriter` port，不建立第二个缓存或新的缓存接口。普通执行结束的
  `after_trade` 现在只传项目 `PoolRouteHint`；不直接依赖 DashMap 或具体缓存方法。
- `ParamCacheWriter` 的原 Arc 实现委托给同一具体 ParamCache 的实现，原 cache_key/
  字符串转换/插入/时间采样保持顺序与含义；不修改 TTL、缓存 key 或 SDK 参数缓存。
- bootstrap 向 PoolQueryService 和普通执行 writer 注入原来的同一个 Arc<ParamCache>；
  没有重建服务、复制表项、引入第二个写入权威或改变查询/交易执行顺序。
- 生产 AppState 移除具体 `params` 字段，只暴露项目 writer；原 `params` 名称仅 cfg(test)
  保留以兼容已有夹具。缓存模块改用原领域 Pool，不经 API models 兼容重导出。
- 这仍是候选元数据缓存，`PoolRouteHint` 不代表链上验证或可签授权。原实现从不保存 Quote，
  writer 不将其升级为 VerifiedRoute，也不把缓存改成生产寻池权威。

## 两项测试合同

1. 实际普通 executor + 原 fake execution backend 覆盖 Memory/Shadow/Durable：替换旧
   fixture handle 后，完成交易仍只写原 writer 持有的缓存，原字段/键保持；替换的缓存为空。
   无 SDK 运行时/Keypair/网络，释放状态后 Weak 不能升级。
2. PoolQueryService warmup 和实际 executor 共用原缓存，沿用同一键；移除 fixture handle
   不影响写入，最后只保留查询服务时缓存仍存活，释放最后服务后回收。

这些合同不证明真实后台任务关闭、缓存全局一致性/额度、全部 SDK 资源隔离或生产运行验收。
原 HTTP resolve/warmup 及执行合同保持，不改原期望值或增加 real RPC。

## 当前验证

- 最终受限 test check：`target/dev-checks/20261115T074510Z-143335`，9 秒，
  RSS 1,229,316 KiB，exit=0，无新增测试警告；初轮未用导入已删除，不抑制警告。
- 受限 production check：`target/dev-checks/20261115T074419Z-143028`，5 秒，
  RSS 701,196 KiB，exit=0。共七组警告：原五组，加移除公开字段后暴露的既有缓存
  metadata fields/get 未读两组。保留原存储及测试读取语义，不用 allow/假读取掩盖。
- `43691952757` 的 param-cache-port 两项、command-defaults 四项与所有原专项/完整
  并发/串行均通过，identity 指纹匹配；旧 `43690435964` 不包含本项，不作验收。
- 后续 API 鉴权/普通校验配置五项不在本快照，仍需其独立 CI 证据。
- SDK/manifest/lock/vendor/release/配置不改；无生产数据操作、部署/重启或实际广播。

## 剩余

full Config、接受/index/journal、blockhash/SDK 参数内部缓存和 bootstrap 仍待继续；
不能把一个 port/字段收口称为基础设施隔离全部完成。PumpAPI 功能开发保持暂停。

## 2026-11-19 Candidate Metadata Bound

Local original NP-03 continuation, not yet independently Rust-tested or
published. Current source usage confirms this map is advisory metadata written
by pool resolve/warmup and ordinary `after_trade`; production has no consumer
of its `get`. It holds no quote, SDK parameters, signed bytes, idempotency or
authorization. An unbounded sequence of distinct Mints therefore retained
metadata indefinitely without helping the trading path.

The same map now retains at most1024 entries. A synchronous write-admission
mutex serializes distinct-key admission, chooses the oldest `updated_at_ms`
(key order breaks ties), drops the map read guards and evicts before inserting.
Updating an existing key does not evict another entry. The lock is never held
across an await or network call; cloned reads retain the original interface.
The original fields, key formatting, timestamp source, writer port and shared
Arc stay unchanged. No TTL, runtime configuration or SDK internal cache is
added/changed. This is a count bound for address-sized metadata, not a general
byte bound or a new route cache/verification policy.

Three unexecuted Rust contracts cover actual default capacity and metadata,
existing-key updates, oldest/tie eviction, four concurrent writers with no
overshoot and final Arc release; the actual pool service and ordinary executor
then run under Memory/Shadow/Durable after advisory eviction. Original route
results/key, one execution, Submitted record and resource release must remain.
No signer, SDK client or external RPC is used. Existing cache authority tests
are retained unchanged. The original three-layer compiler gate also forbids
the concrete ParamCache type; new callers must keep using the existing port.

Read-only source reconstruction verifies the original map/metadata/insert/get
after removing only bounded admission, retains the old executor tests and
checks unchanged writer/query/ordinary runtime/bootstrap/journal/SDK loader
sources. Five deliberately weakened bound/lock/eviction mutations reject.
Format and policy checks do not substitute for the pending Rust execution.
This is carried with the provider boundary batch after independently accepting
0530bed/run44363743712/artifact14457878477 (full2070 each). That accepted
snapshot does not contain or validate these three cache contracts.

First provider/cache snapshot3721cf1/run44371523601 failed only two capacity
fixtures, not a service type error: actual compiler/link and74type/225field
gates passed; provider5 and original cache-port2 passed, capacity1 passed/2
failed before full suites. Matching artifact14459603617 was downloaded.
The existing CPMM key uses the supplied pool ID, so varying only Mint while
reusing one pool produced one key and concurrent overwrites. Correction varies
the public fixture pool address per index and explicitly asserts unique keys.
Original cache implementation/key/writer and every old assertion remain;
later independent-backend source is held out. Do not rerun the terminal job
or treat this partial result as complete2078 acceptance.
