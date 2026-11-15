# NP-02 / TS-08-H 靓号 Application 与 Worker 边界

日期：2026-11-14。源码与离线验收通过，未部署。范围为原三个 Handler、创建政策/取消政策及既有 worker 归位，不新增生成能力或更改钱包生命周期。

## 实际实现

- `application/vanity.rs` 定义项目输入、只读 getter 的非 Clone/Serde 私有创建命令、原 pattern/预算/label 政策、GET owner 检查、错误种类与编排。
- `application/ports/vanity.rs` 仅提供 create/get/cancel 的任务控制，没有私钥、signer、raw transaction/RPC、SDK 客户端或生成循环。
- `adapters/sdk/vanity_worker.rs` 归位原 WalletOps；根 `wallet_ops.rs` 只兼容重导出。实际生成、保存与加载的 `run_vanity_job` 连同原 impl 结束部分已与阶段前源码逐字节比较，完全相同。
- 实际 create 只接 App 政策构造的 owned command；不再接 API DTO。原 keypair/Base58、30M 上限、单 Semaphore、每 1000 次取消检查、保存/加载/失败补偿不在本阶段改算法。
- 取消纯政策移至 Application，但仍在原 worker 一次 get 后的位置调用；不额外查询或改变 Atomic 标记、写库、最终读取顺序。
- Domain 持有原 VanityMatchSide（原 Serde lowercase 与 as_str）和 VanityJob（原字段/顺序/派生）；models/repository 原名兼容重导出。原十二字段任务 JSON 保持。
- 三个 Handler 仅保留原提取器/管理鉴权、调用、HTTP 映射。AppState 删除 `wallet_ops` 句柄，换成项目 `VanityTasks`；bootstrap 构造原 worker 一次并交同一个 Arc 给服务，不按请求重建或增加启动恢复动作。

## 保持的合同

| 子项 | 原语义 |
| --- | --- |
| pattern | 1..4 ASCII Base58；长度/ASCII 先于非法字符错误；不修剪/大小写归一化或新增字符 |
| max_attempts | None=30,000,000；0→1；超过 30M 含 u64::MAX→30M；不新增配置/默认值 |
| 身份/label | owner/type/ID 字面保持；仅 label 用同一 Rust trim；匹配方向只 prefix/suffix |
| 创建 | 校验在 worker/仓储前；原仓储 owner/label 检查、活动计数、插入、get、登记与 spawn 顺序保持；成功 202，错误原 400 |
| GET | 原任务读取与实时 attempts 覆盖后严格 owner 相等；不匹配 403，读取错误 404 |
| 取消 | owner 判断先于所有状态；queued/running/cancel_requested 才请求取消，其他含未知状态保持原记录；不匹配和读取/取消错误仍 400 |
| 鉴权/提取 | 原 legacy/separate 管理凭据、loopback、Query/JSON 提取优先级保持；禁播不是靓号开关 |
| 启动 | 原 SQL 只把 queued/running 改 interrupted/service restarted；不悄悄把 cancel_requested 当已恢复 |

没有新增 URL、字段、配置、数据库或密文迁移、SDK/依赖更新、权限或默认生成模式。

## 源码精确证据

生成/保存/加载主函数从旧源码截取后，与移动后同一区间（至 impl 结束）字节相等；新测试声明和 port 转发未进入该区间。创建归一化与取消 guard 的必要接线变化独立记录，不把整文件冒称不变。

仓储当前仅用 domain 重导出代替原 VanityJob 定义。将这一行在自己创建的临时源码中恢复为原定义后，得到阶段前精确 SHA，临时文件随后删除；实际仓储未被“还原”编辑：

```text
还原后 6741458361130d8fd66cd407f3a2adf1d75a63722c1ea53e3613636d25b982b9
当前   e27da5136e51d835f62746078b53c7c3b67ca81955498ac893e661a6517843fe
```

因此 SQL、验证/保存/审计/加密、启动恢复及更新方法其余字节未变；不由此推出原实现所有风险已解决。

## 离线验证

原四断言校验测试从 worker 移到 App，不计新增。新增七项 App、三项适配器、三项 HTTP，共十三项；vanity 筛选 17 包含既有测试。样本仅公开任务元数据、计数器、fake port 与临时 SQLite：

- 原 alphabet/长度/Unicode 错误次序、两匹配方向、七类预算边界、Unicode 空白 label trim、字面 owner。
- 只调用一个 port、GET/取消不同 owner 错误、各错误状态、非最终请求取消不等于已取消/释放队列。
- 原 worker 构造 Arc/单槽与启动状态范围；真实 get/cancel 的公开记录、终态/未知状态与 owner；手工植入 Atomic runtime 后的 attempts 覆盖及 cancel 标记/写库。
- 实际进程内 HTTP 接项目服务 + fake port，覆盖 202/200 完整任务 JSON、进度、取消、400/403/404、422/Query 提取、separate/loopback、禁播独立。

没有调用实际 worker create/run 或生成循环，没有创建/解密/导出真实钱包、SDK/RPC 初始化、网络、签名或广播。适配器测试只种公开记录/计数，不安排任何后台生成任务；fake 取消不证明实际 CPU/SDK 工作已停止。

初轮串行 983/984 暴露一项旧 C2b HTTP 测试的时序假设，确定性补强见下节。补强后的最终源码回归如下，不以先前中间结果冒充结束：

```bash
cargo test --locked --offline --quiet vanity                 # 17 passed
cargo test --locked --offline --quiet                        # 984 passed
cargo test --locked --offline --quiet -- --test-threads=1     # 984 passed
cargo test --locked --offline --quiet http_contract_tests    # 194 passed
cargo check --locked --offline --bin pump-trade-service      # passed
cargo fmt --check                                           # passed
git diff --check                                            # passed
```

仅原 models/persistence 两组兼容导出警告，无新增抑制。旧 V1 hash/JSON、2,016 组 SDK 参数、98 组状态映射、576 组 DAMM 消息与原持久化/WSOL/转账/钱包合同均随全套通过，不重写黄金期望。源码真实接线已完成，不是未使用的预留接口；真实生成/SDK/节点不由 fake 回执证明。

### 既有 C2b 测试同步补强

普通 Durable GET/等待读取数据库，不能假定 HTTP 已见 Confirmed 时执行器的内存副本也同步完成。旧测试立即读内存，串行回归时观察到合法的 BuildingTx 窗口而失败。

仅改测试 fake 的返回门闩与 wallet Drop 完成通知：强制数据库 Confirmed、HTTP 已返回时执行器仍被阻止返回，先断言内存 BuildingTx/数据库 Confirmed；放行并等待执行器完成后，严格检查内存和数据库均 Confirmed、原签名不变、fake sign/broadcast 各一次。增加临时审计 trigger，确认数据库变为 Confirmed 后的写入数为零，不能靠触发器把错误降级重新修回而蒙混通过。

没有 sleep、弱化状态断言、改生产超时/交易实现或重写黄金值。最终带门闩/写审计版本单项连续十次通过，随后完整并发/串行和 HTTP 全部通过；测试数未因此增加。executor/ordinary SDK/PumpAPI 的生产实现指纹保持。

## 指纹

必要变更（新 Worker 路径、根兼容桥、仓储类型重导出及 bootstrap）：

```text
da8961af210b12d79f65807510114039829ac9d06ab58e985223d9e47d0d06f8 src/adapters/sdk/vanity_worker.rs
62147a44c75a26e96cd50109b71ad724355883e4ea13a46953ffd0280424dace src/wallet_ops.rs
e27da5136e51d835f62746078b53c7c3b67ca81955498ac893e661a6517843fe src/wallet_repository.rs
ea659348cc51dfe7152b7b518bc8c17c36c73a356644dce4c53f2b5b7dc97991 src/main.rs
```

原 registry、交易生产实现、manifest/lock/release 的本阶段前指纹保持：

```text
162ab6d5c1926bc8378d600a7c5a7f186b8f60c408a7703fa0d768ad65c5f0df src/wallets.rs
bfeebf9e5aa8ce78ef157406d954189a589d8e73d63802a62af6b25fdbfb9bec src/executor.rs
2246675f820b79d4939efa655ff0d8237456651c945f157059a54d4baf56b28f src/adapters/sdk/ordinary_execution.rs
acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b src/pumpapi_client.rs
67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89 Cargo.toml
4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5 Cargo.lock
b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f target/release/pump-trade-service
```

## 仍未完成

1. cancel_requested 启动恢复的原缺口、取消与成功生成/加载并发、取消轮询延迟、队列总量和 owner count+insert 的原竞争未修；不提供 exactly-once、原子取消或跨仓储/SDK 回滚保证。
2. 真实生成正确性/速度/CPU、生成后 SDK 加载、密钥日志/内存与异常隐私、生产恢复/发布仍须独立验收；私有命令/port 不等于完整安全授权。
3. AppState 其他运行资源、bootstrap、接受入口/依赖方向仍需收口；普通最终消息授权、其他模式/backend 账本与只读原签名对账等 NP-01 继续，不用目录归位代替资金安全完成。
4. 普通订单 GET 不加入隐式 RPC/重发，PumpAPI 对接保持暂停；不停止 ZMQ 或改配置。

持续目标 active，修改计划/汇报不暂停。不升级 SDK、不构建 release、不部署重启、不读生产配置/钱包/数据库、不提交推送、不发送交易。
