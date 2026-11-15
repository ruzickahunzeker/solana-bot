# RPC-01B2 Readiness 有界只读探测

日期：2026-11-14。范围：Trade Service 源码/离线验证，未部署；不升级 SDK、不操作生产配置/钱包/数据库、不签名或发送交易，不恢复 PumpAPI 交易对接开发。最终验证结果以 `EXECUTION_STATUS.md` 为准，不以计划存在代替测试通过。

后续 RPC-01C1 已将原生 SOL 批量余额作为第四类独立容量接同一 runtime；本页三类/1103 等为 B2 历史检查点。最新原生余额预算、分批、数量异常修复及验收见 [C1 后续记录](NATIVE_BALANCE_SHARED_RPC_RUNTIME.md) 与 `EXECUTION_STATUS.md`，Quote/token 余额仍未迁移。

## 实际接线与候选权威

- main 将 RPC-01B1 的同一个 `Arc<SharedReadRuntime>` 传给 `RuntimeReadinessSource`，不建立第二个 HTTP client、冷却表或 RPC registry。
- `SdkTradeClient::readiness_probe()` 仍按原 `execution_lanes()` 顺序冻结当前钱包的公开 metadata 和 RPC URL；不再把 `Arc<RpcClient>` 放入 probe。初始化、lane 选择、交易构建/签名/发送均不改。
- probe 只能向共享 runtime 请求本次执行候选的 `getLatestBlockhash(processed)`；不能扩大为全部配置入口。URL canonical 别名合并，保留不同凭据和候选首次顺序；缺失、无效或不在 registry 的候选不自动取得另一节点。
- 第一笔有效响应即停止；失败只在本次候选内串行尝试。沿用 B1 的 JSON-RPC ID/版本、响应形态、hash 长度/固定解码及 64 KiB 限制，不把“HTTP 成功”当作探测成功。
- 本次探测不会写 blockhash cache。即使 RPC 成功，旧/空 cache 也不会被伪造为新鲜。

原 `src/adapters/rpc/readiness.rs` 连同三项 mock 测试原字节保留为 `readiness/legacy_tests.rs`，SHA-256 为 `827d1d22832946e122bd1f98e8c53afad72d214309d3808d0bb780ac82113675`。还原 root SDK 中唯一 `.url()` 为原 `.clone()` 后，其 SHA 精确重现本批前的 `bbc65da4622288c4c343c67e0b3e647510e6e6a2a81143c12a02b3ffce3767b9`；不是把整个长期 dirty SDK 文件与 Git HEAD 的差异误记作本批改动。

## 容量、截止与冷却

| 类别 | 同时客户端 future | 最小发送起点间隔 | 一次操作 |
| --- | --- | --- | --- |
| 关键 blockhash 刷新 | 1，原独立 FIFO | 100 ms | 单节点 500 ms，总 1500 ms |
| 可选普通聚合状态观测 | 1，原局部 gate/FIFO | 1000 ms 或原更慢配置 | 单节点单次，2 秒 |
| Readiness | 1，忙时立即 false，不排队 | 250 ms | 单节点 500 ms，总 1500 ms |

Readiness 的间隔包括本次备用节点与后续探测，不是每个节点各有额度。三类独立容量/起点预算，已迁移范围最多三笔客户端 future 在途，理论起点合计上界 10 + 1 + 4 次/秒；不是服务商账户、其他进程或整个 SDK 的配额。取消只释放本地 future/名额，不返还已用起点，不能撤回上游已经收到的请求。

任一类别的 HTTP 429 都冷却同一个 canonical endpoint；整数 Retry-After 默认 30 秒、限制 1..300 秒，并发响应不缩短已有更长冷却。后续探测跳过已冷却入口，不新增请求或延长冷却；如果执行候选全部冷却/无效，`rpc_ok=false`，不借健康的全局非候选节点兜底。忙、超时、坏响应、预算不能准入均不能伪报成功。

这里刻意去除原 SDK probe 的内部 429 重试，并引入明确 deadline/频率；并发 `/ready` 可能立即得到 `rpc_ok=false`，不新增“最近成功缓存”。关键刷新容量独立，不代表 CPU/网络/供应商物理资源可抢占或真实延迟 SLA。

## 缓存时序修复与 V1 合同

先在生产 source 实际调用的 `observe_health` seam 保留旧采样顺序，新回归复现两项失败：

1. RPC 等待前缓存新鲜、返回时已过期，旧 `has_blockhash` 仍为 true。
2. 数据库检查等待期间缓存过期，旧 age/新鲜标记都采于等待之前。

修复后保留原 RPC -> 钱包公开 metadata/list/count -> 数据库检查顺序，在所有 await 完成后，从同一 cache 条目、同一次时钟读取得到 age 与新鲜标记。cache 年龄仍按 wall-clock 饱和减法，包含等于 max-age 的边界；不引入 block height、共识或跨默认钱包更新的原子快照承诺。

公开字段/17 字段 fixture、鉴权、HTTP 200 及 V1 `ready = has_blockhash && rpc_ok` 不变。`paused`、持久化模式和数据库 healthy 仍是独立诊断；不把数据库故障默默折入 ready 公式，不自动启用 Live、恢复或重发交易。

## 测试范围

- Source 健康采样：慢 RPC/DB 过期、空/旧缓存不改时间、取消不继续检查 DB、探测期间成功刷新可被最终快照观察。
- Cache：一次条目/时钟读、精确到期边界、回拨饱和、不更新原条目。
- Runtime/probe：metadata/processed 请求、候选顺序与隔离、canonical 去重/不同 key、坏响应、两方向共享冷却、忙拒绝/三类独立容量、取消不返还预算、配额拒绝不伪报成功、500/1500 ms 串行边界。
- 真实 HTTP：沿用 B1 exact-filtered 子进程与 loopback fixture，核对候选子集/429、hash 冷却反向可见、并发/取消/关键刷新及坏 ID/JSON/RPC/HTTP/体积失败。子进程清除代理，不启动 main、不实例化钱包 SDK、不访问外部节点。
- 原 readiness 三项 oracle、B1/D1B3、信号与 HTTP 合同继续回归。

本批实际新增 20 项：source 5、cache 1、runtime/probe 10、wire 父用例 4（沿用原 B1 exact helper，不重复计数）。先保留旧采样顺序时四项 health 测试中两项失败；修复后的最终 readiness 33、shared_read 39、完整并发/串行各 1103、HTTP 198 全部通过。Production check、fmt/diff 通过，仍五组既有警告，没有新增抑制。旧 age-only accessor 已无生产调用，保留在 cfg(test) 为原 oracle/fixture 使用，不作为额外生产入口。

最终指纹：manifest `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、lock `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、release `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`、executor `bfeebf9e5aa8ce78ef157406d954189a589d8e73d63802a62af6b25fdbfb9bec`、PumpAPI `acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b` 均与本批前一致，vendor 无差异。Root SDK 本批后为 `4c1e4c950976a5c379b4cabff7eb7ee56bb39b0ca077321f75220fba828d6bc9`，唯一只读投影变化及可还原校验见前文。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet --no-run
cargo test --locked --offline --quiet readiness
cargo test --locked --offline --quiet shared_read
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --quiet
cargo fmt --all -- --check
```

## 剩余目标与发布门槛

余额、寻池、独立 quote/WSOL、模拟及 SDK 初始化/内部读取仍未全部接共享预算。新探测失败并不能确定供应商账户额度、执行发送可用性或节点诚实性；真实节点/TLS/HTTP2、负载、配置/启动与生产发布仍需另行验收。

本批没有新增 migration，但整个尚未部署优化树已有 004–007，未来发布仍需备份、迁移、恢复与调用方验收。完整最终消息授权、opaque SDK 广播前签名账本、Memory/Shadow 等模式、双假后端、依赖检查及可信 SDK 来源/升级演练仍未完成，不能以本项代替完整优化。
