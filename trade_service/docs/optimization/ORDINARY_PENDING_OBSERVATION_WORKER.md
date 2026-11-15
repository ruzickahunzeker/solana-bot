# NP-01-D1B2 普通聚合待确认扫描

日期：2026-11-14。状态：源码与离线专项/完整回归通过，未部署、未启用。仅覆盖 Durable 普通 SOL 聚合买入的已有签名账本，不代表全部交易的恢复完成。

后续 D1B3 已补生产 Reqwest transport 的本地 HTTP/1.1 集成证据，RPC-01A 补跨配置变量稳定去重，见 [后续检查点](ORDINARY_OBSERVATION_HTTP_TRANSPORT.md)。下文 D1B2 测试数字为当阶段历史，真实供应商/生产验收和全服务预算仍未完成。

## 目的与不变量

在进程重开后，不依赖旧 AdmittedTrade、ExecutionOwner 或钱包运行实例，从原 SQLite authority 枚举可核对的 signed/submitted/unknown 记录，只查询原交易签名。明确 finalized 成功/错误可按原 006 规则推进父订单为 confirmed/failed；查不到、弱确认、429、超时或坏证据均不产生执行许可。

- 没有重新获取 claim/owner、构建、签名、广播或换业务幂等键的接口。
- 原请求 JSON/hash、报价/预检、owner、消息、签名/字节均继续绑定；候选 SQL 行不是观测授权，必须经过 D1B1 完整 capture/read/CAS 核对。
- 这里的“只读”指链上 RPC；本地原账本仍可保存核实后的最终性。GET、列表及重复 POST 不因此触发新的 RPC 或执行。
- 不处理未取得完整签名证据的 preparing/prepared/signing 中断，不迁移 Memory/Shadow、opaque SDK 或 PumpAPI backend。

## 接线与边界

| 位置 | 职责 |
| --- | --- |
| `src/config/ordinary_observation.rs` | 默认关闭的开关、批量/时间范围验证；enabled + 非 Durable 在 main 初始化钱包前拒绝 |
| `src/application/ordinary_observation_scan.rs` | 有界 keyset/cycle cutoff、全页验证、逐项 capture 与原签名观测，取消保留已完成游标 |
| `src/adapters/persistence/ordinary_observation_scan.rs` | 复用原 pool，部分索引查询、limit+1、不使用 OFFSET，不自行修复坏 schema |
| `migrations/007_ordinary_observation_scan.sql` | 增量部分索引，经已有迁移账本一次应用；004/005/006 不改 |
| `src/adapters/rpc/ordinary_observation.rs` | 独立单次 HTTP 状态查询、两节点局部预算、串行/间隔与 429 冷却，不使用 SDK 内置重试 |
| `src/adapters/sdk/observed_quote.rs` | 原 prepare/sign/send 完全委托既有适配器，仅将本次发送后的状态查询接入共享 fail-fast reader |
| `src/adapters/ordinary_observation_worker.rs` / `src/main.rs` | plan 不发请求；TCP bind 成功后才启动；stop/join/Drop abort；Ctrl-C 与 Unix SIGTERM 关闭信号 |

后台和发送后前台观测共享同一 reader；配额忙时前台保留原订单并报告查询不可用，不退回未限额的 SDK status reader。后台等待使用 FIFO slot，前台不能抢占它已预留的请求间隔。

## 配置

| 参数 | 默认值 | 合法值/范围 | 含义 |
| --- | --- | --- | --- |
| `ORDINARY_OBSERVATION_ENABLED` | `false` | `true/false/1/0` | 明确 opt-in；仅 Durable 可开，默认不启动扫描 |
| `ORDINARY_OBSERVATION_BATCH_SIZE` | `8` | `1..64` | 每批最多处理的候选数，不保证每批都能在 deadline 内处理完 |
| `ORDINARY_OBSERVATION_POLL_INTERVAL_MS` | `30000` | `1000..300000` | 上一批结束/超时后的等待时间，不是固定墙钟频率 |
| `ORDINARY_OBSERVATION_MIN_AGE_MS` | `30000` | `1000..3600000` | claim 创建与父行最后更新时间都必须不晚于 cycle cutoff |
| `ORDINARY_OBSERVATION_REQUEST_INTERVAL_MS` | `1000` | `1000..60000` | 此 reader 请求起点的最小间隔，前后台共用，并非全服务 QPS 配额 |

参数在读取配置时验证，错误值不会被静默截断。每批总 deadline 为 20 秒，单次 HTTP 最多 2 秒，响应体最多 65536 字节；较长间隔会让一批跨多轮完成，游标不跳过正在处理的记录。轮内截止时间冻结，完成一轮后刷新；重启从头枚举仍未终结记录，而不是恢复执行权。

RPC 选择保持配置顺序，选前两个**不同**的 HTTP(S) URL。回归先证明“先 take(2) 再去重”会丢失第二个独立备用，现改为去重到两个后才停止。429 后只冷却对应入口，Retry-After 仅解析整数秒，默认 30 秒并限制到 1..300 秒；全部冷却时不发请求、不顺延到第三节点、不重提交易。发行前仍须验证实际供应商格式与配额。

虽然默认关闭扫描，未来部署该源码的 Durable 初始化仍会应用 007 索引；不能把关闭开关当成无数据库迁移。生产迁移、备份和回滚须另行授权，本轮只在内存/临时文件库执行。

## 验收证据

前检查点已有 20 项专项；本轮新增七项，总专项 27、完整回归 1045 项通过：

1. 重复前两项 URL 的回归先在旧实现失败（只选到一个节点），修复后两个独立节点依次冷却，第三节点绝不接管。
2. 五种坏页面（重复、倒序、超 cutoff、超批量、空页却 has_more）整页拒绝，capture/RPC 均零次。
3. 文件库 signed/submitted/unknown，各覆盖 finalized 成功和错误；销毁旧 owner/执行对象/backend 后重开扫描，再重开一次。原请求/owner/报价/消息/签名字节不变，终态不再查。
4. 三种文件库状态各覆盖查询无结果与故障，两个扫描周期保持原父行和证据不变；fake 构建/签名/发送事件没有增加。
5. 临时库触发器禁止新订单、新 claim、owner/原消息/签名字节改写；最终性 CAS 不靠新的执行许可。
6. 两个专用测试子进程分别接收 SIGTERM/SIGINT；实际 `shutdown_signal()` 唤醒后停止在途 fake 查询，join 完成、写入零次、原状态仍 unknown。子进程通过 exact test filter 进入 helper，不运行服务 main、不加载 `.env`，只对自建子进程发信号。

本轮受限沙箱中的两个信号用例超时，宿主同一源码的 SIGTERM 单项及 27 项专项均通过，最终完整回归也在宿主通过；没有跳过失败后声称全绿。信号测试不证明 opaque SDK 自建 worker 可以取消，也不代替生产 systemd/HTTP 排空验收。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet ordinary_scan
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --quiet
cargo fmt --all -- --check
```

结果：专项 27；完整并发/串行各 1045；HTTP 198；production check/fmt/diff 通过。production check 仍为此前五组警告（models 重导出、request_hash 重导出、QuoteStore 兼容方法、WSOL 兼容方法、SDK wallet_balance），没有抑制或新增警告组。命令只使用测试入口、fake RPC/HTTP 与临时 SQLite，不提交链上交易。

本轮开始/结束的 manifest、lock、release SHA-256 一致；vendor 相对 HEAD 无差异：

```text
Cargo.toml  67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89
Cargo.lock  4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5
release     b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f
```

## 仍未完成

- 此 reader 预算仅限普通 SOL 聚合观测，不覆盖寻池、blockhash、余额、独立 quote/WSOL、多进程或服务商共享账户额度。
- D1B3 已补真实本地 HTTP 请求计数/429/重定向/体积/超时/取消；真实供应商/TLS/HTTP2 与最终性响应兼容尚未验收，不能把本地或 fake 证据扩大为这些门槛。
- 全协议最终消息授权、opaque SDK 广播前 signed journal、Memory/Shadow legacy 迁移、生产 schema/刷盘/断电和端到端调用方验收仍待续。
- 本轮没有修改生产配置、钱包/数据库、SDK/vendor/依赖、服务状态或发送交易，也没有恢复 PumpAPI 交易对接。
