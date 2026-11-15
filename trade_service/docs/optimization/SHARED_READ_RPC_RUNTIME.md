# RPC-01B1 共享读取运行时

日期：2026-11-14。范围：Trade Service 源码/离线验收，未部署。本批从两个实际只读 profile 接入，不代表全服务 RPC 配额已经完成，也不改变交易发送 client、重签规则或 PumpAPI 交易对接暂停范围。

后续 RPC-01B2 将 readiness、RPC-01C1 将原生 SOL 批量余额、RPC-01C2 将 Quote 余额查询作为独立容量 profile 接同一 runtime；本文的两个 profile/25 项/1083 是 B1 历史检查点。最新接线、预算与结果见 [Readiness 后续验收](READINESS_SHARED_RPC_RUNTIME.md)、[原生余额后续验收](NATIVE_BALANCE_SHARED_RPC_RUNTIME.md)、[Quote 查询后续验收](QUOTE_BALANCE_SHARED_RPC_RUNTIME.md) 和 `EXECUTION_STATUS.md`。

## 实际接线

| 位置 | 职责 |
| --- | --- |
| `src/adapters/rpc/read_http.rs` | 只允许 LatestBlockhash / SignatureStatus 两种请求；Reqwest 禁止重试/重定向，2 秒 HTTP timeout、64 KiB 声明及流式 body 上限，错误不回显 URL/供应商 body |
| `src/adapters/rpc/read_runtime.rs` | 两类保留容量/起点间隔、共享 endpoint 冷却、发送前 deadline 核对；无 signer/钱包/订单库或 sendTransaction 能力 |
| `src/adapters/rpc/blockhash.rs` | 原项目 LatestBlockhashReader port 的实际实现，不再借钱包 SDK RPC 来刷新 |
| `src/adapters/rpc/ordinary_observation.rs` | 保留原局部 FIFO/foreground gate 与状态解析，只将生产 HTTP 委托共享 runtime |
| `src/adapters/ordinary_observation_worker.rs` | 注入同一 runtime，不新建独立生产 HTTP client/冷却表；扫描仍 opt-in |
| `src/main.rs` | 单一 runtime 的 Arc 同时交给刷新和观测；钱包 SDK 工厂/发送实例不替换 |
| `src/blockhash_cache.rs` | 显式持有刷新任务，停止时 abort/join，Drop 请求 abort；错误不更新旧 hash/时间 |

本批删除 WalletRegistry 已无调用的只读 RPC 快照 getter；不改钱包创建、导入、删除、加解密及存储流程。新 blockhash port 输出仍是项目拥有的 32 字节 Blockhash，不把 SDK 类型重新传给 Application。

## 预算与兼容

| 类别 | 保留容量 | 请求起点间隔 | deadline / 选择规则 |
| --- | --- | --- | --- |
| blockhash | 1，不能被观测占用 | 至少 100 ms，跨本类全部节点 | 单节点最多 500 ms；一次操作含排队/备用总计 1500 ms；原顺序串行、跳过冷却/不可用节点 |
| 普通聚合观测 | 1，不能被刷新占用 | 共享层至少 1000 ms，原 reader 配置/FIFO 可更慢 | 保留原外层 2 秒且共享层也有 2 秒边界；同一查询仅选一个节点，不自动在查询内部换节点 |

保留容量不是抢占正在发送的请求。同一查询不并行探测多个 key；两类独立读取可以各有一笔客户端 future 在途，因此已迁移范围最多两笔。单个 runtime 中起点预算的理论合计上限为 10 + 1 次/秒，正常 400 ms 刷新更低；这不是服务商账户配额，也没有覆盖尚未迁移的 SDK/其他进程流量。

- main 仍使用 `processed`，刷新默认 400 ms，最大年龄默认 2000 ms；没有新环境变量。原 0..49 ms 刷新值现在最少等待 50 ms，防止所有节点快速拒绝时形成忙循环；实际 HTTP 仍受 100 ms 类别间隔约束。
- 429 只冷却对应已注册 endpoint，整数 Retry-After 默认 30 秒并限制 1..300；并发响应不能缩短已有更长冷却。另一类别随后也跳过此入口。
- registry 按解析后 Url 等价性合并默认端口/host 大小写等别名，保留不同凭据和首次位置；不是按域名或服务商账户合并。原 Config 的 required/空白/别名规则不变，坏配置入口仅不可用，不增加启动校验规则。
- 先检查 deadline，再等待起点；等待后在同一预算锁内复核 deadline 和冷却，之后才记账并进入 HTTP。已过期/未准入请求不消耗起点预算；已进入请求即使失败/取消也不返还。
- 关键总 deadline、节点 deadline 与每次发送前检查并用。只给 future 套 timeout 不足以证明不会在继续 poll 时发下一笔；本批先复现过期 admission 仍成功，再补前后检查和备用循环的剩余时间约束。
- 成功响应核对 JSON-RPC 版本/ID、结果形态、context/lastValidBlockHeight 字段类型和 base58 hash；保持 SDK Hash 的最多 44 字符/32 字节解码规则，使用固定输出缓冲区，不对 64 KiB 级长串做无界 base58 分配/解码。不把这些字段当作节点诚实性/链上共识证明。

错误、缺字段、错误 ID、超时、超体积和配额拒绝均不能把旧缓存变“新鲜”。GET/重复提交没有新增链上查询/重发入口，原普通订单/hash/JSON/签名账本不变。

## 任务生命周期

刷新任务从返回即被 guard 持有；后续持久化/绑定等初始化错误会使 guard Drop 请求 abort。正常关闭时先通过 AbortHandle 停止刷新，再显式 shutdown 等待 join；已有观测 stop/join 保持。Drop 本身不等于同步证明已退出，显式 shutdown 完成才是 join 证据。

两种系统信号的专用测试子进程现同时挂起 fake hash 和 fake signature 查询，使用实际 shutdown_signal 唤醒后都取消并 join；不启动服务 main、不对业务进程发信号。取消不会撤回已经到达上游的只读查询，也不证明 opaque SDK worker 或服务器端计算停止。

## 验收证据

- 阶段前 blockhash 筛选 14 项通过；原 SDK adapter 连同三项 oracle 原字节迁至测试专用 `blockhash/legacy_tests.rs`，SHA-256 仍为 `52e329866d151a6726dc14ccc208c8bdfa0e13e4d3ec98c3182f3c452c962e6e`，没有删除基线以迁就新实现。
- 新专项 25 项：14 项 runtime/codec、8 项 wire（7 个父用例与 exact child helper）、3 项刷新生命周期。固定 hash 的 256 种字节填充值及坏编码/60,000 字符输入与 SDK oracle 对照，不涉及密钥或签名。
- 真实 loopback HTTP 核对两个方向的共享 429、后续跳过冷却入口、pending status 不饿死 hash、500 ms 慢节点备用、坏 ID/JSON/错误/断开、64 KiB 边界、坏响应不刷新旧/空缓存、全冷却零新增请求且不延长冷却。
- 纯运行时测试覆盖 1.5 秒总边界、取消释放保留位且不返还已用起点、60 秒观测等待不阻塞关键刷新、canonical URL/不同 key 和到期零发送；等待后过期用例延迟 executor polling，确认再次检查而不是只看进入时刻。
- 子进程复用代理隔离 helper，只有 localhost 随机端口；raw HTTP fixture 扩展为两个固定 read 方法。原 D1B3 九项及原信号用例继续回归，没有另造一套测试 client 配置。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet shared_read
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --quiet
cargo fmt --all -- --check
```

最终专项 25、完整并发/串行各 1083、HTTP 198，production check/fmt/diff 通过，仍五组既有警告。manifest/lock/release、executor、root SDK 与 PumpAPI 指纹保持本批开始值，vendor 无变化；具体接线和历史证据见 `EXECUTION_STATUS.md`。独立节点/TLS/HTTP2、实际供应商配额/格式、生产启动与迁移/回滚、真实负载/最终性仍未验收，本地时间边界不是线上 SLA。

## 剩余工作与发布边界

1. **RPC-01B2**：本检查点时尚未迁移的就绪探测，现已接独立容量/共享冷却与单次预算，具体合同及验证见后续文档；不再重复作为待实现项。真实节点/线上负载仍待验收。
2. 余额、独立 quote/WSOL、寻池、模拟与 SDK 内部读取仍待逐 profile 收口；不能只套逻辑 semaphore 就宣称消除了五次内部 429 HTTP 重试。复杂 GPA/模拟另需容量/体积/错误与账户数据校验。
3. provider 账户级/跨进程配额、可观测性、完整最终消息授权、opaque SDK 广播前签名账本、其他 backend/mode、依赖方向和升级来源/演练仍属于完整目标。
4. 观测开关默认关闭仍只控制扫描；新的 blockhash reader 将在未来部署该源码时生效，开关 false 不会回退旧 SDK 刷新器。本批没有 release 构建、改线上 `.env`、重启、生产库操作或真实交易；本批没有新 migration，但整个尚未部署优化树已有 004–007，不能据此跳过总体备份/迁移/恢复门槛。PumpAPI 交易对接继续暂停。
