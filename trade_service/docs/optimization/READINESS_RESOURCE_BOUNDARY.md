# NP-03 Readiness 设置与 SDK 诊断边界

日期：2026-11-15。源码及受限 test/bin 类型检查通过，六项新增 Rust 合同尚未执行，未部署。

## 实际接线

- RuntimeReadinessSource 不再直接持有 Config、WalletRegistry 或 SharedReadRuntime；
  只接三值 ReadinessSettings、项目 ReadinessWalletObserver、原 blockhash cache 和持久化。
- Settings 纯投影原 blockhash_max_age_ms、BroadcastPolicy 与 pumpapi.enabled，无凭据、
  钱包、数据库路径、RPC URL 或环境读取；是原不可变启动配置的值快照，不增加热加载语义。
- SDK adapter `SdkReadinessWalletObserver` 持原 Registry/runtime Arcs。原默认钱包的
  readiness_probe 在返回 future 前捕获，按原共享 profile/候选 URL 查询，之后按原顺序采样
  default ID/count/public_infos/metadata。没有 SDK signer、交易构建或发送方法调用。
- 项目 WalletReadinessObservation 只含原公共元数据和 rpc_ok，不含 RPC URL/SDK 类型；
  外部 ReadinessSnapshot/ReadyResponse 字段与判定不变。
- observe_health helper 逐字节保持，仍先 await RPC，再 await database，最后采样一次 cache
  freshness/age。V1 ready 仍仅 has_blockhash && rpc_ok；paused/database health 为独立诊断。
- bootstrap 复用原 Arcs 注入 SDK observer，只改该构造器接线，不新建客户端/资源/任务。
  970d835 的原组装字节对照是前序提取时的证据，不当作本批接线也逐字节不变。

## 六项新增合同

1. 三值配置投影覆盖年龄 0/u64::MAX、广播两态、PumpAPI 两态；快照不随后续合成 Config 改动。
2. 实际 RuntimeReadinessSource 使用假钱包 observer 与原 cache/store，释放合成状态后 Config
   Weak 不再升级；read 执行一次，最后 source 释放后原 cache/store 回收。
3. 实际 ReadinessService + 实际 source + 临时 Durable SQLite，32 组 freshness/RPC/DB/
   broadcast/PumpAPI 状态核对原 ready 规则、全部诊断字段与原元数据；关闭临时 pool 验证 DB 失败。
4. 实际 source 的钱包观测等待中取消，future 被 drop、一次调用且无重试、不写 cache。
5. 等待钱包观测期间成功刷新 cache，恢复后实际 source 看到新值，而不是提前缓存的 freshness。
6. SDK observer constructor 仅捕获原 Registry/runtime Arcs，不持 Config；释放最后 observer
   后回收原资源。使用空 registry，不初始化 SDK/Keypair，不调用默认钱包 getter。

旧五项 observe_health 合同及 SDK probe/shared runtime 证据保留。新测试只验证合成资源下
调用链/字段/资源交接，不证明非空真实 SDK registry、供应商 TLS/格式/额度或生产关停；
observer 通过 SDK adapter 间接持有 registry，不声称进程无密钥或 OS 级隔离。

## 当前验证

- 受限 test check：`target/dev-checks/20261115T084206Z-166970`，33 秒，
  RSS 2,130,684 KiB，exit=0，无新增测试警告。
- 受限 bin check：`target/dev-checks/20261115T084403Z-167501`，13 秒，
  RSS 978,984 KiB，exit=0；原七组生产警告保持，不抑制/假调用。
- 后批加入 readiness-settings 六项专项，保留原全部专项/全并发及串行；
  前序 bootstrap `970d835` / Actions `43695116074` 虽完整并发/串行各 1452 已通过，
  但不包含本批，不能冒用其结果。
- SDK/manifest/lock/vendor/release、运行配置和生产数据不变，无部署/重启或实际广播；
  PumpAPI 只保留既有状态位，不恢复其交易接入开发。

## 继续

AppState/其他 adapter 的 full Config、普通接受/index/journal、SDK 其他读取及所有方向/
协议/模式仍待继续；数据库健康 port、全局 RPC/cache/backend 与真实环境门槛未据此完成。
