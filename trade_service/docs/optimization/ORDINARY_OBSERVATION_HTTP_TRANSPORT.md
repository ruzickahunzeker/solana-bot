# D1B3 HTTP 验收与 RPC-01A 入口去重

日期：2026-11-14。范围：Trade Service 源码/离线验证；不改变 SDK 版本、公开 API、请求/持久化格式或交易许可，不操作生产配置、钱包、数据库或服务。

后续检查点：本文的指纹、1058 及“未接共享 runtime”是 D1B3/RPC-01A 当时的历史证据。RPC-01B1 已有意提取共用 HTTP transport/fixture，并让刷新与观测复用同一 runtime；原九项场景保留，最新 1083/专项 25 见 [共享读取验收](SHARED_READ_RPC_RUNTIME.md) 与 `EXECUTION_STATUS.md`。不将旧“transport 未改”表述套到后续源码。

上一轮为实际进展：D1B2 修复局部 reader 的备用丢失，并补文件恢复/坏页/信号测试。本轮继续验证当时明确缺失的真实 HTTP transport，并实施统一 RPC 治理的入口去重子项；不把它们代替完整资金安全目标。

## D1B3 真实 HTTP Transport

`src/adapters/rpc/ordinary_observation/tests/transport.rs` 使用生产 `SingleShotStatusReader::new` / `ReqwestStatusTransport::new`，没有另建一套测试专用 client 配置。测试服务绑定 `127.0.0.1:0`，逐个核对连接数和完整 JSON-RPC 请求，而不是只数 fake transport 的方法调用。

八个父测试各运行 exact-filtered 测试子进程，一个 helper 测试在普通全套中无 flag 时直接返回；因此 Cargo 计数为九项，不把内部响应场景再算成额外测试。

| 父测试 | 本地 HTTP 证据 |
| --- | --- |
| 原签名/ID | 两次独立逻辑查询，实际收到原签名、固定 `getSignatureStatuses`/history 参数，ID 为 1、2 |
| 429 与后续节点 | 本次 429 只有一个请求，不立即访问备用；下一次逻辑查询使用健康备用，随后继续避开冷却节点 |
| Retry-After | 实际响应头的 0/超大/正常整数被限制为 1/300/原秒数；HTTP-date、坏值及缺失用默认 30 秒；全冷却时零新增请求 |
| 302 | 返回明确 HTTP 302 错误，Location 目标连接/请求数均为零 |
| 响应体边界 | Content-Length 与 chunked 的 65536 字节合法 JSON 可读，声明/流式 65537 字节在解析前因上限拒绝 |
| headers/body 超时 | 等待响应头及只给部分 body 均在原 2 秒预算内终止；不重试、不返还 60 秒请求间隔 |
| 在途并发/取消 | 第一笔已到达服务端时第二笔前台查询立即拒绝、服务端仍只有一笔；取消原查询后释放 in-flight/semaphore，仍保留原 next_allowed，立即再次调用不产生 HTTP 请求 |
| 坏响应/断开 | 错 ID、非 JSON、JSON-RPC 错误、HTTP 503 和完整请求后断开均只发一笔；URL、供应商消息及 fixture secret 不进入返回错误 |

fixture 故意使用有界 raw HTTP 生成 chunked、超大长度及半截响应；只解析自己收到的固定 JSON POST，不作为可复用/生产 HTTP server。每个成功场景显式停止 listener 并取消/join 其连接任务；失败时 Drop 和测试子进程生命周期清理。

### 进程隔离

`src/adapters/test_support.rs` 仅经 `#[cfg(test)]` 编译。它复用原信号测试需要的 child/stdout/超时/回收逻辑，并仅在 child 中删除六种大小写代理变量，设置 NO_PROXY/no_proxy 为 `*`。父进程不使用 set_var/remove_var，不修改 `.env`，也不加载服务 main。HTTP helper 再核对代理隔离，所有 RPC URL 与重定向目标均为本地 fixture。

原 SIGTERM/SIGINT 测试改用同一 helper；它仍只向自建子进程发信号，不对 systemd/业务进程发信号。宿主执行是因为受限沙箱不能代表真实 socket/进程信号；没有把跳过或超时当作通过。

生产 `ordinary_observation.rs` transport/gate 实现本轮不改，开始/结束 SHA-256 均为：

```text
02db13ec49d189d4737784fba0753fb09048a37bc97b45b3150a145203018b3c
```

## RPC-01A 配置入口稳定去重

原 `urls.dedup()` 仅删除相邻项；同一 URL 跨三个备用变量出现时仍会建立重复入口。先提取不改变合并语义的 `src/config/rpc.rs` 并复现：预期四个入口，旧实现仍有七个。随后用 membership set + 原 Vec retain 稳定去重，保留首次出现的位置，不用 set 遍历决定顺序。

- 合并顺序仍为 `RPC_URL`、`BACKUP_RPC_URL`、`BACKUP_RPC_URLS`、`RPC_BACKUP_URLS`。
- 仍调用原 required 校验主节点，原缺失/空值错误不改；主值保留字面值，仅 backup comma entries trim 并过滤空项。
- 只合并精确相同字符串，不按域名合并、不改 query、不规范化或改写凭据；同一 Helius 上不同 key 的 URL 仍是不同入口。
- 环境读取与纯合并分开，四项测试不修改进程环境：跨别名非相邻去重、不同 key、原空白处理、空备用列表。
- 没有新增配置项、运行时热切换、供应商配额探测、发送策略或交易重试；源码未部署，不据此声称线上节点选择已再次改变。

## 验证命令

在 Trade Service 目录，先用离线 no-run 检查编译，再在允许本地 socket/自建子进程信号的环境运行：

```bash
cargo test --locked --offline --no-run --quiet
cargo test --locked --offline --quiet ordinary_transport
cargo test --locked --offline --quiet rpc_config
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --quiet
cargo fmt --all -- --check
```

最终源码的 transport 9、配置 4、完整并发/串行各 1058、HTTP 合同 198 通过；production check/fmt/diff 通过，仍五组既有警告，没有用单项通过宣称全链路完成。完整检查点见 `EXECUTION_STATUS.md`。

本轮开始/结束的 manifest、lock、release、executor、root SDK 与 PumpAPI 指纹一致；vendor 没有变更。除上述 transport 指纹外，可复核：

```text
Cargo.toml  67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89
Cargo.lock  4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5
release     b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f
```

## RPC-01B 前置源码核对

- 锁定的 `solana-rpc-client 4.2.2/src/http_sender.rs` 在 HTTP 429 上有五次内部重试，缺有效 Retry-After 时默认等待 500 ms；因此一个 SDK 逻辑调用最多可能产生初次加五次 HTTP 请求，外层 deadline 可能更早取消，不能把一次方法调用当成一笔 wire 请求。
- 当前 `src/adapters/rpc/blockhash.rs` 仍直接调用 SDK RPC，尚未复用观测 reader 的预算。配置默认刷新 400 ms、最大年龄 2000 ms（不是本轮读取的线上值）；不能简单把普通观测的 1000 ms/FIFO 间隔强加给所有读操作而饿死 blockhash。
- 后续先冻结 commitment/备用顺序/旧 hash 保留与 readiness 行为，再为具体只读 profile 接单次 transport、总/节点 deadline、共享冷却和并发/配额；必须把关键刷新与后台扫描的优先级/拒绝规则纳入测试。
- SDK 6.0.2 提供 `TradingClient::from_infrastructure` 和公开 infrastructure，可能用于适配器内注入；但初始化本身也有 RPC/潜在 ATA 设置路径，不能为只读实验实例化真实钱包 SDK，或改变发送 client/factory 后声称仅改查询。本轮未初始化或替换它，未来需单独验证生命周期与资金边界。

## 剩余门槛

1. 这里只验证固定 loopback HTTP/1.1，不证明生产 TLS/HTTP2、代理、负载均衡、供应商错误格式、银行最终性或共享账户额度；不是线上时延 benchmark。
2. 当前冷却/预算仍只覆盖普通聚合观测。RPC-01B 必须继续统一各 read port 的 deadline、单次发送、并发、配额和备用节点政策；SDK 内置 429 重试、跨钱包/后台竞争尚未全部收口。
3. read fallback 不得扩展到未知交易结果的换路线、重建、重签或新 key 补单；opaque SDK 的广播前 journal 与完整最终消息授权仍是独立未完成大项。
4. 未启用新的观测功能，没有升级 SDK、改变 PumpAPI 交易对接暂停范围、迁移 Copy/Bridge、构建 release、发布或发送交易。
