# NP-03 / TS-09-A 生产 AppState 资源所有权

日期：2026-11-14。状态：源码与离线验证通过，未部署；不是完整 AppState 或基础设施隔离完成。

后续 NP-01-B3a 已完成下述 Quote 签名边界过期复现与最小修复，见 `QUOTE_SIGNING_FRESHNESS.md`；其并发/串行各 989、HTTP 196 不改本页 986 的历史检查点含义。

## 实际变更

- 逐项核对生产读取点后，将 `AppState.wallets`、`wallet_repository`、`aggregator_quotes`、`conversions` 四个旧句柄限定为 `cfg(test)`。普通生产构建不包含这些字段；测试保留原名称以兼容已有夹具。
- bootstrap 仍创建并注入原 registry/repository/cache/store Arcs，未重新建立运行时、缓存、数据库或幂等权威，未调整服务初始化次序。
- 实际请求通过已接入的项目服务使用原资源；四个冗余字段不再作为生产 AppState 的绕过入口。
- full Config、普通 admission/index/persistence、参数/blockhash cache 和项目服务 ports 仍在 AppState。此项不能冒充这些剩余资源或接受入口均已隔离。

## 两项新增合同

`src/http_contract_tests/state_resource_tests.rs`：

1. 替换测试专用字段后，实际 HTTP Router 仍从原钱包服务返回原默认钱包；管理入口仍保留原 File 模式拒绝，而非误用替换的数据库仓储。原四资源在服务持有期间存活，释放整个 Router 后 Weak 均不可升级。只用空 registry、临时库与内存 store，不初始化真实 SDK/RPC。
2. 仅给夹具字段填入 ConversionStore 不会启用未配置 journal 的兑换服务；原 503、阶段/错误码/no-store 保持。最后 Router 释放后该测试 store 也释放。

这些测试证明当前组装的原引用和请求权威没有被替换，不证明实际 SDK 加载、生成、生产关停、后台任务或操作系统资源均已验收。普通生产编译另行核对，不能只靠包含测试字段的 test build。

## 验证

```bash
cargo test --locked --offline --quiet state_resource          # 2 passed
cargo test --locked --offline --quiet                         # 986 passed
cargo test --locked --offline --quiet -- --test-threads=1      # 986 passed
cargo test --locked --offline --quiet http_contract_tests     # 196 passed
cargo check --locked --offline --bin pump-trade-service       # passed
cargo fmt --check                                            # passed
git diff --check                                             # passed
```

本轮 production check 实际观察到五组警告：models/persistence 的兼容重导出，以及 QuoteStore 的 insert/get、WsolQuote 的两个兼容校验方法、旧 SDK wallet_balance 未使用。后三组所涉实现没有在本阶段修改，未用 allow 或删方法掩盖；历史检查点记录的“两组”不能替代本轮实际输出。全套 test build 没有新增警告。

对照前阶段，wallets、wallet_repository、executor、ordinary_execution adapter、vanity worker、wallet_ops、PumpAPI 实现、Cargo.toml/lock、现有 release 二进制 SHA256 均未变化。只修改 main 字段暴露、测试接线/新增测试及本轮记录；未升级 SDK/vendor、构建 release、接触生产配置/钱包/数据库、联网或重启。

## 续作

- NP-01 继续最终可签消息授权、普通原签名只读对账、其他 backend/存储模式的唯一 signed journal；不在现有普通 GET 中悄悄加 RPC，也不自动重签或重发。
- 当前共享聚合签名适配器没有在取出待签消息时重新检查 Quote 有效期；授权期间的异步等待可能使旧检查过期。先补无 signer 的失败复现，再实施最小局部安全修复，不扩大到暂停的 PumpAPI。
- NP-03 后续继续资源/接受入口、依赖方向与兼容桥核查；真实运行验收独立授权。

持续目标保持 active，文档更新不暂停。PumpAPI 交易对接仍暂停，不停止 ZMQ，不改线上配置。

## 2026-11-15 原资源所有权核对

本节落实主计划 3.3 的资源所有权补充，归入原 NP-03/TS-04/09，不创建新任务或资源。
依据当前 `bootstrap::initialize/complete`、`main`、`AppState::trade_intake/trade_queries`
和普通 Handler；只作源码核对，不能替代真实初始化或生产退出测试。

| 资源 | 创建/共享位置 | 当前退出责任与已有证据 | 尚缺的范围 |
| --- | --- | --- | --- |
| 钱包 Registry/Repository | initialize 创建原实例，SDK/钱包服务共享原 Arcs；main 留日志句柄 | 原资源替换/Weak 释放与 bootstrap 合成组装合同 | 真实 SDK 客户端/密钥内存清理、钱包原子激活和后台任务全部释放 |
| SharedReadRuntime | initialize 创建一次，现有十二类读取及转账准备/确认使用同一 Arc | 转账准备复用原 WsolQuote 配额，`a5420ba` 独立验收；确认复用原 Observation，待验收 | 转账广播所有权、SDK opaque 内部/全服务配额与生产门槛，不重做已验收读取 |
| BlockhashCache/Refresher | 原缓存由 refresher、readiness、AppState 持有，main 接原 refresher owner | 后继普通任务生命周期将原资源保留到 drain 完成/超时后才停止和 await；c6f2745 独立专项与完整各 1498 验收，不借用旧生命周期证据 | 链上有效期/真实节点、异常进程退出与 SDK 内部等其他任务 |
| ParamCache | complete 创建一次，PoolQueryService 与 executor writer 共享 | 同实例写入/保留键、最后服务释放后回收，见 PARAM_CACHE_RESOURCE_BOUNDARY.md | 缓存版本/全局一致性与 SDK 内部参数缓存 |
| TradeStore/IdempotencyStore/TradePersistence | initialize 打开原持久化，complete 创建原内存资源；接受/查询/claims/执行共享 | 接受与普通执行服务借用原实例/项目 ports；26be717、26445c5 已验收，原 reservation/hash/JSON 和文件恢复合同继续适用；普通任务与 Shadow 生命周期见下两行 | 根具体资源隔离、全部 backend/mode 的 signed journal 及生产刷盘 |
| QuoteStore/ConversionStore | complete 创建原报价缓存，initialize 按配置打开可选 conversion store | 原服务持有和 bootstrap Some/None/Arc 交接合同 | 不合并独立操作账本；真实节点/恢复与发布仍独立验收 |
| ObservationPlan/Worker | complete 只 prepare，main 成功 bind 后按原开关 start | 普通 drain 之后 request/await 原 worker；另一资源报错也完成 join；c6f2745 四项 helper 合同和全套已通过 | 真实部署调度、断电与全系统 RPC 预算 |
| 普通交易后台任务 | bootstrap 创建一个共享 TradeTasks；原 Handler 接受 await 前登记，Created 移交原 executor | 线性 ticket 只记录生命周期；17 项新合同与完整各 1498 在 c6f2745 验收，原 ledger/owner 不变 | SDK opaque worker、其他独立操作/靓号任务与真实进程退出尚未覆盖 |
| 原 Shadow writer | 原 TradePersistence 创建单个 1024 项 FIFO，私有组件持有原 pool/handle；main 留原 persistence Arc | 普通 drain 和原 refresher/observer join 后关闭/等待原队列；796ba50 的专项 13/4、完整各 1515 验收，超时/取消保留同一 handle，无重放 | 仅原 SQL 返回及 FIFO 完成证据，不是 fsync/断电、全部生产者停机或新增 durable authority；不重复实施 |
| 原靓号任务 | 原 WalletOps 保留唯一 semaphore/map，私有句柄 owner；Bootstrap 把同一 Arc 交原服务和 main | 仓储 await 前登记，信号关闸/原取消标记并行 join；超时/取消不丢句柄。3d72a14 / Actions44245588737独立验收十三新合同/完整各1939/101步 | 原生成/import 主函数保持；不是原子取消/加载回滚、完整 SDK supervisor 或生产恢复；见 VANITY_TASK_LIFECYCLE.md |

最初核对时仅有 `axum::serve(...).with_graceful_shutdown`，不能证明脱离 HTTP 的任务完成。
后继本批源码接入原 NP-03 的普通任务登记/有界等待，但不把它当全服务/SDK worker supervisor，
也不释放 owner 或换 key/路线/backend 重签。新关闭窗口和参数的实际边界见
`ORDINARY_TRADE_TASK_LIFECYCLE.md`；17 项新测试已由 c6f2745 独立 CI 验收，不改线上配置或服务状态。
