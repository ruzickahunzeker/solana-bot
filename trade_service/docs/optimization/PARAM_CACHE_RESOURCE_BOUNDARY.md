# NP-03 参数缓存写入口与资源所有权

日期：2026-11-15。源码及受限类型检查通过，两项新增运行合同尚未执行，未部署。

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
- `param-cache-port` 两项专项加入后续 CI，保留 command-defaults/HTTP/原参数及完整
  并发/串行；当前 `43690435964` 不包含本项，不能当成本项验收。
- SDK/manifest/lock/vendor/release/配置不改；无生产数据操作、部署/重启或实际广播。

## 剩余

full Config、接受/index/journal、blockhash/SDK 参数内部缓存和 bootstrap 仍待继续；
不能把一个 port/字段收口称为基础设施隔离全部完成。PumpAPI 功能开发保持暂停。
