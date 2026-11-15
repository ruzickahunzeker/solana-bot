# NP-01-A3c 准备余额共享 HTTP 运行时

日期：2026-11-15。仅 Trade Service 源码/离线验证，未部署。不升级 SDK/依赖、不构建 release、不改公开 API、校验/金额/hash/存储格式，不访问生产钱包或数据库、不重启、不签名发送。PumpAPI 交易接入开发继续暂停，不停止 Bridge/Bot。

## 实际接线与单一权威

main 既有 SharedReadRuntime 通过 `SdkOrdinaryExecutionBackend` / 已解析钱包执行上下文传到普通 SDK buy/sell、LaneExecution/LanePreparation，再到 `RpcPreparationBalanceRead`。不改变钱包注册/加载/管理、钱包 Arc 身份或初始化顺序，不为每钱包/每 lane 新建 HTTP client、RPC registry、缓存或预算实例。

内部 Rust buy/sell 方法显式接收惰性运行时提供者，禁播政策与 lane 选择仍先于取得该资源；旧零 signer/transport fixture 额外用 panic closure 证明禁播时不会取得 runtime。方法收窄为 crate 内可见，不更改 HTTP 请求/响应或公开执行方式。

Reader 现在只持公开钱包地址、当前 lane 的 URL 和共享运行时，不再持 SDK RpcClient。LanePreparation 只投影 URL，不调用 SDK 余额方法；其他 SDK 参数/ATA 存在性/probe/签名核验等读取尚未因此迁移。PumpAPI 的交易构建 backend 不走本次新功能。

## 保留的读取与账户规则

1. 原当前 lane URL 必须已注册；不扩大到 main 全局候选或其他 Default 节点，也不修改原 lane 选择顺序。不可用仍只允许普通 controller 在 SDK 入口前尝试下一 lane，进入 SDK 后绝不借只读切换重新执行。
2. 第一笔为 owner scan，原 processed/jsonParsed/dataSlice:null/minContextSlot:null。复用 A3a/B1 的 canonical/钱包/Mint/program/精度/initialized/重复/混合/1000 账户检查，坏扫描不 fallback。
3. 正常不可用、明确 RPC error 或全未解析扫描可继续第二笔 raw 读取；它必须合法核验原始 Mint 与两个规范 ATA，不能裸数字回退或把错误当已知零。
4. 第二笔固定 `[Mint, classic ATA, Token-2022 ATA]`、base64/processed/完整 data；scan 成功时保留 minContextSlot，返回非零 slot 不得回退；parsed/raw 数量/程序/精度必须一致。
5. raw 账户和 snapshot 仍使用原 A3b1/B2 的程序/初始化/权限/扩展/native reserve/supply/canonical 输入与 SDK 参数/模式/preview 绑定。仍从整次操作开始计时，3 秒新鲜度，不因 fallback/等待重新赋时。

不额外添加供 UI 显示的 supply/SOL 查询。完整合法空扫描与 raw 明确无余额可得零；坏或不完整的证据不变成可支出量。不宣称同一 scan/raw bank、独立节点共识或最终可签消息已授权。

## 容量、请求与失败政策

准备余额是第六类 profile：一笔操作在途，忙时立即返回类型化 Unavailable，不抢占其他五类名额；两笔实际 HTTP 的起点至少间隔 250 ms。单步含 pacing 最多 2 秒，整次含两步/等待最多 4 秒。普通成功最多两笔客户端 HTTP，无 SDK 的五次内部 429 重试，不分页、不截断、不后台续查。

仅 scan 允许 512 KiB，raw 请求维持 64 KiB；Content-Length 和 chunked 流式累加均有上限。类型化 ReadBodyLimit、超大/坏 JSON/版本/ID/context/身份或结果形态错误属于 Invalid，不通过 raw 回退隐藏；正常网络/状态不可用属于 Unavailable。错误不回显 URL/key、provider message/body。

HTTP 429 在同一 runtime 的 canonical endpoint 上共享冷却，Retry-After 仍 clamp 为 1..300 秒，默认 30 秒；该 key 冷却后本次 raw 不再发出，其他 profile 后续也跳过，准备操作不直接换另一 URL。不同 key 保持不同入口，不把别名当新配额。

取消释放本地 future/操作名额，不退还已用发送间隔；取消等待 pacing 不产生请求。取消不能证明节点不再处理已收到请求。六类独立容量/间隔不是 CPU/网络抢占，也不是供应商账户/跨进程的完整配额。新增 profile 的持续起点上界 4 次/秒，整个已迁移运行时六类理论合计 27 次/秒，不是任意一秒滑窗硬额度或线上 SLA。

相对原 SDK 逻辑读有意改变了单步时间预算：旧内存 RPC 可以等待 3.2 秒 scan 后因快照过期拒绝；新单步在 2 秒超时后若 raw 很快合法返回，可以在原 3 秒期限内恢复。此时未知 scan 被当作不可用，而非伪装成已验证扫描。两个步骤合计超过 3 秒仍过期，不能把 4 秒总 deadline 当作余额有效期。HTTP 302 不跟随，只能按不可用政策对原 URL 尝试 raw，目标地址零连接。

## 兼容 oracle 与验收

旧 SDK reader 和原 17 项内存 RpcSender 用例完整移到 `preparation_balance/legacy.rs` / `legacy_tests.rs`，只在 cfg(test) 编译，生产没有旧余额读取 caller。只移除 tests 的 path 注解在内存流中计算，精确重现 A3b2 reader SHA `fb4e1e293e56e681229cc0c07f2a1fd3bb8d0196986f75da1ed8690fc255de1c`；不以改写 oracle 掩盖新预算差异，也不把迁移用例重复计作新增。

首轮旧 Quote fake 对新增请求枚举没有穷尽匹配，已显式增加拒绝分支，不让 Quote fixture 接受新操作。第一次专项 88/89 通过，一项新断言遗漏了 Url 的末尾 `/` 规范化，修正断言后再跑；新增 private_bounds 警告通过收窄内部方法可见性处理，不加抑制。最终源码 preparation_ 90、完整并发 1235（139.07 秒）、完整串行 1235（237.37 秒）、HTTP 199、shared_read 89、quote_balance 25、原参数合同 3（2,016 组）全部通过。production check、fmt、diff 检查通过；仍五组既有警告，无新增警告。

实际新增：12 项 runtime/codec + 6 个真实 HTTP 父用例 + 1 项 runtime Arc 复用，共 19 项。复用既有隔离 child，只有 child 清除代理环境；使用公开地址、内存响应和临时 loopback，不创建 SDK wallet/真实 signer，不访问生产节点。

覆盖精确大整数/Token-2022/零、坏/混合扫描、raw 差异/slot、已注册 lane/别名/不同 key、不扩大候选、busy/取消/保留间隔、单步 timeout 安全恢复/总 deadline/不续期，以及实际两笔/512 KiB 与 64 KiB declared/chunked、坏 ID/JSON、headers/body timeout、302 零目标连接、429 共享冷却与 blockhash 独立容量。fake fetch 次数和真实 HTTP 证据分别记录，不将它们混同。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet -j 1 preparation_
cargo test --locked --offline --quiet -j 1
cargo test --locked --offline --quiet -j 1 -- --test-threads=1
cargo test --locked --offline --quiet -j 1 http_contract_tests
cargo test --locked --offline --quiet -j 1 shared_read
cargo test --locked --offline --quiet -j 1 quote_balance
cargo test --locked --offline --quiet -j 1 ordinary_param_contract_tests
cargo check --locked --offline --quiet -j 1
cargo fmt --check
git diff --check
```

## 不变项与完整目标

manifest/lock/vendor/原 release、executor、钱包 registry、PumpAPI root、原准备顺序和参数投影指纹与 A3b2 相同。本批无 migration/运行配置变化；只在普通 SDK adapter/main 增加同一 Arc 的必要传递，不改 PumpAPI 调用参数或业务。其他服务和 sdk9 工作树既有改动不动。

原 release SHA `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；原准备/参数 SHA `b49859e0cb32fd4a01e381d6f356b85f5f40470e877087b00505ac6ef720b438` / `9cb1bee04e18894d3a2090d24019e42099d6dbca5dde4467419b4065a3841374`。A3b2 input guard `def6e630cf63f38d4f065778a96810ab8a993165072a693219688349940351ff`、query decoder `e97b35cb1070a0a8be7b3e532503b41a8d740cc3e229a08417b724483fb6b9cb` 均不改。

root SDK 必要注入后 SHA `2dfb4dc5c3eae433fb6750db44cda2e34b3f7e86649747bb99d1abda018a3fc2`；在只读内存流还原 buy/sell/execute_sdk 三个方法块后精确重现 A3b2 SHA `581c01153eb702486e23ab8647304189f9eead8370653e3b819dd1a5e9b25206`，未写回旧实现。实际 lane execution/preparation 的变化只增加原 Arc 的传递/URL 投影，不能将这些文件误报为未变。

下一主线继续普通 SDK 的不可变构建与最终消息/费用/min_out/权限授权，以及原 authority 广播前签名账本/其他 mode/backend/worker 取消和多签名恢复。A3 子项不等于这些门槛完成。其余参数加载/寻池/独立 quote/WSOL/模拟和 SDK 内部读的共享治理、双 fake 构建后端、依赖方向、可信 SDK revision/补丁账本与隔离升级/协议演练仍保留。

该检查点后 B4a 的九项入口合同和 B4b 的 DAMM guard 均保留各自历史数字/指纹，见 [入口审查](ORDINARY_UNSIGNED_BUILD_AUDIT.md) 与 [候选检查点](DAMM_FINAL_CANDIDATE_GUARD.md)。当前 [B4c1](DAMM_UNSIGNED_CANDIDATE_AND_SIMULATION.md) 已接不可变零签名产物/模拟；该只读模拟仍用原 SDK RpcClient，不是第七类共享 profile，不将它或后续新接线误记为 A3c 已完成全服务 RPC/全部源码不变。

真实供应商/TLS/HTTP2/延迟负载、生产备份/迁移/刷盘恢复/调用方兼容与部署回滚没有在此验收；完整未部署树已有 004–007，不能因本批无新 migration 就跳过其发布门槛。PumpAPI 暂停项继续计入未完成。
