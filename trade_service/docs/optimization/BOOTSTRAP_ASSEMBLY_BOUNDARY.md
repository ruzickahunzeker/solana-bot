# NP-03 启动组装与资源交接边界

日期：2026-11-15。源码及受限类型检查通过，四项新增 Rust 合同尚未执行；未部署。

## 实际提取

- `bootstrap::initialize` 接原 Config Arc，按原顺序完成观察模式门槛、钱包仓储/运行配置/
  WalletRegistry、共享 read runtime、blockhash refresher、普通持久化、WalletOps 与可选
  conversion store。没有重建 SDK/RPC、换节点、变更钱包加载参数或修改 schema。
- 私有 Resources 把这些原句柄移入 `complete`，只组装原服务/缓存和 ObservationPlan，
  不加载钱包、监听或启动 observation worker。所有 API/application 仍使用原 Arcs。
- Bootstrapped 返回 AppState、原钱包/仓储日志句柄、原 BlockhashRefresher owner 与
  原 ObservationPlan；main 保留 Router/TraceLayer、监听、日志、worker start 和 shutdown。
- 两段原初始化/服务组装语句在提取前后逐字节比较完全一致；仅新增资源包/函数交接，
  原错误 context/顺序不变。main 现在 233 行，bootstrap 277 行，不以行数当完成度。
- 临时 runtime_wallets 配置 Vec 现在在 initialize 返回时释放，而非保留到 main 退出；
  SDK/Config 所持原身份/密钥不变，不声称 Drop 会清零秘密或已完成 signer 资源隔离。
- refresher 仍在原阶段启动，失败路径的原 guard Drop 取消；成功后必须由 main 原 owner
  协调 shutdown。观察扫描仍在成功 bind/日志之后 start，不改默认关闭配置。

## 四项新增合同

1. 空 registry、内存仓储/持久化及待决只读 fake reader 通过实际 complete：Config/
   WalletRegistry/repository/persistence/cache 的 Arc 指针不变；已提供的 refresher 仍在途。
   移交后释放状态回收服务资源，最后原 refresher shutdown 取消 read 并释放 cache。
2. 可选 conversion store 的 Some/None 与原 Arc 保持；释放状态后回收，不创建第二个 store。
3. 实际 ObservationPlan 因非 Durable 存储拒绝组装时，原 refresher guard 被 drop，
   待决 read 取消，资源全部释放；不创建第二个 reader 或重试。
4. initialize 对 Memory/Shadow + observation enabled 在打开隔离钱包 SQLite 路径前
   返回原错误；临时目标文件不存在。测试不运行 SDK 初始化，不读取 .env/生产数据库。

这里只证明合成资源下的实际组装/所有权交接与明确失败点，不等于真实 WalletRegistry 初始化、
provider/TLS、数据库所有迁移、进程崩溃/刷盘或生产关停均已验收。

## 当前验证

- 受限 test check：`target/dev-checks/20261115T082114Z-159090`，约 14 秒，
  RSS 1,551,888 KiB，exit=0，无新增测试警告。
- 受限 bin check：`target/dev-checks/20261115T082256Z-159553`，约 4 秒，
  RSS 658,692 KiB，exit=0，前批七组生产警告保持，不抑制/假调用。
- CI 新增 `bootstrap-assembly` 四项专项并保留原全部专项/并发/串行；
  前序 `cb94bdd` / Actions `43693519994` 虽完整并发/串行各 1448 通过，但不包含
  本批，不能冒用其结果。
- SDK/依赖/vendor/release、运行配置与生产数据不变，不部署/重启或实际广播。

## 继续

AppState 仍有 Config/接受/index/journal/blockhash 等具体依赖，SDK 其他方向/协议/模式、
全局 RPC/cache/backend 边界与升级/协议演练仍须完成；PumpAPI 新交易对接保持暂停。
