# RPC-01C1 原生 SOL 批量余额读取

日期：2026-11-14。范围：Trade Service 源码/离线验证，未部署。当前子项不等于全部 WalletReadPort、RPC 配额或整个优化完成；不升级 SDK、不改公开 API 或持久化格式、不触碰生产钱包/数据库、不签名发送、不恢复 PumpAPI 交易对接开发。

2026-11-15 后续 C2 已迁移余额查询 API 的 Quote/supply/token/ATA 操作；本文的四类/1130 及“查询 Quote 未迁移”属于 C1 历史检查点。最新第五类运行时与验证见 [Quote 查询验收](QUOTE_BALANCE_SHARED_RPC_RUNTIME.md)。真实交易准备仍用旧 token_balance helper，不能把查询迁移当成它已验证或已统一限流。

## 实际接线

- main 将同一个 `Arc<SharedReadRuntime>` 传给 `SdkWalletBalanceRuntime`；观测 plan 接线改为 clone 同一 Arc，不增加 HTTP client/冷却表/节点 registry。
- 原 registry/SDK 的 read_access 只增加只读 runtime 注入参数；仍从已加载钱包提取原公开地址与 Default RPC 候选，不更换 SDK 初始化、选择、签名或发送实现。
- `RpcWalletReader::wallet_balances` 不再调用 SDK getMultipleAccounts；URL 仍从本钱包原 Default RPC 客户端投影，交给共享运行时选择本候选集合的节点。
- `quote_balance` 及其供给量、token scan、两 ATA fallback 逻辑不迁移。本批不能宣称 Quote 余额也已去除 SDK 内部重试。
- 原 `wallet_reads.rs` 整文件/八项测试原字节保留为 `wallet_reads/legacy_tests.rs`，SHA-256 为 `d721589eac8295e32387abc5c1aa8ed146d199c0d9206677b9c5c04b94e2a666`。当前 Quote/token 方法也继续受原 mock 测试覆盖，不只依赖旧实现的 oracle。

Quote 方法片段 SHA 为 `1f1b9a785652a4ea10bca346b3226c3b8eda20fac742f71bf249cf410b1ba19d`，token scan/ATA 函数片段为 `f086b7b4fa750fa35502b7fdaee70ec42304911edc91ab5572f7a2ff81d04fb9`，均与原文件相等。Root SDK 还原唯一 read_access 注入参数/构造参数后，SHA 精确重现 B2 前值 `4c1e4c950976a5c379b4cabff7eb7ee56bb39b0ca077321f75220fba828d6bc9`；registry 还原只读参数/传递与换行后也精确重现原 SHA `deec15c00f4933c16758b8165b19d2a3be49e4380ab803ffb585adff654a5c41`，没有改变 CRUD/密文/SDK 初始化。不是把与 Git HEAD 的全部长期 dirty 差异记作本批改动。

## 请求与响应合同

只允许 typed `getMultipleAccounts`，参数固定 `processed`、`base64` 和 `dataSlice:{offset:0,length:0}`。每笔最多 100 个地址，整次最多 1000 个，保留输入顺序及重复地址；空请求直接返回空结果，不发 RPC。超过上限明确失败，不静默截断，不引入 signer 或任意 RPC 方法入口。

响应沿用共享 HTTP 的禁止重试/重定向、2 秒单次 HTTP、64 KiB 声明/流式上限和脱敏错误。解码还核对 JSON-RPC 版本/ID、context slot、精确结果数量、账户 lamports/executable/rentEpoch 类型、固定 32 字节 owner 和空 base64 切片；不把忽略 dataSlice 的响应当作已验证结果。仅字面 null 按缺账户补零，合法账户自身的零余额仍正常保留，坏对象、缺字段、负数、字符串、浮点或溢出金额不能变成零；有效 lamports 保持 u64 整数。

同一节点按页串行读取，任意页失败则丢弃该节点的全部临时结果，在允许的下一节点从第一页开始。不能把不同节点的半批余额拼成一个成功结果。即使同节点，不同页的 bank slot 也可能不同；本批没有声称这是链上同 bank 原子快照，也没有声称节点诚实性。

## 预算与生命周期

原生余额为第四类独立容量：最多一个客户端 future 在途、忙时立即报错、不排队，发送起点间隔至少 250 ms。一个候选节点整批最多 4 秒（包括所有页/间隔），整次最多 8 秒（含串行备用），实际每笔 HTTP 仍最多 2 秒。所有页面共用间隔，不是每页/每钱包各有一份预算；到期不进入下一笔 HTTP。

与关键刷新、readiness、普通聚合观测共用 canonical endpoint 的 429 冷却，不并行探测多个 key。不同凭据保持不同入口；冷却节点被跳过，全冷却或没有合法 Default 候选明确失败，不扩大到全局非候选节点。取消释放本地名额/临时页，不返还已经使用的发送间隔，不在后台续查或重试；不能撤回已到达上游的只读请求。

收尾先对共用 read 复现“响应已 ready 但被延迟 poll 到截止之后仍返回成功”的失败回归；最终用例还包裹与生产相同的外层 timeout，确认不能只依赖该 timeout。现共用 read 先记录可能的 429 冷却，再显式拒绝迟到响应；Native 还在页面解码后复核节点期限。该通用修复覆盖实际共用的四类读取，不能让该迟到响应续新缓存/返回余额，也不返还已用间隔；这不是硬实时或上游停止计算保证。

已迁移四类总计最多四笔客户端 future，持续起点速率理论为 10 + 4 + 1 + 4 次/秒（刷新/readiness/观测/原生余额），正常刷新更低；这不是任意一秒滑窗的硬计费额度，也不是全部服务、SDK 内部、供应商账户或跨进程配额。独立容量不证明 CPU/网络可抢占或线上时延 SLA。

## 数量不匹配修复

先新增实际 Application 回归，原两个可见钱包/空返回在旧实现中变成零行，测试失败复现。原 `.zip(values)` 也会静默丢弃过多结果。

修复后 Application 在 zip 前再核对返回长度。任何读取后端返回数量异常，都保留全部可见钱包、原顺序与 `fetched_at_ms`，每行 `balance_lamports=null` 并报告读取错误；不会把异常金额错配或补零。原 200/逐行错误、owner/visibility/metadata/解析先后、正常字段和整数语义不变。这个异常结果的行为是有意修复，不继续以旧 zip 的截断断言作为期望；原旧 RPC 实现与测试保留为历史 oracle。

真实 HTTP Router 回归使用临时钱包元数据和 fake reader，核对缺少/多出返回的三行可见钱包、HTTP 200、原字段与零订单/零审计写入；不加载真实钱包。

## 验证范围与命令

- Codec：u64 边界、顺序/重复/null、数量/ID/slot/类型/owner/dataSlice 错误及 64 KiB。
- Runtime：201 与 1000 地址分批、丢弃半批后整批备用、空/过量/非法候选、双向 429 冷却、busy/取消/不返还起点、其他三类容量、单节点/总 deadline。
- 真实 HTTP：生产 RpcWalletReader 的 100/100/1 页面、同节点半批故障/完整备用、共享 429、错误 ID/JSON/账户数量/HTTP/超体积/重定向零连接、pending 余额不饿死 hash 与取消。
- 沿用 exact-filtered 测试子进程/代理隔离和 loopback fixture，仅增加固定只读方法；请求 body 界限从原 4 KiB 调整为 6 KiB、总请求仍 8 KiB，以容纳最多 100 个地址，不构造真实 SDK 钱包或启动 main。
- 原 Quote/token mock、原始 native oracle、Application/HTTP 身份和可见性、其他 profile/信号/持久化合同继续回归。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet --no-run
cargo test --locked --offline --quiet native_balance
cargo test --locked --offline --quiet shared_read
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --quiet
cargo fmt --all -- --check
```

最终新增行为回归 21 项：Application 1、HTTP 1、codec/runtime 14、真实 HTTP wire 5。保留旧文件八项 oracle 且现行 Quote/token 六项继续覆盖，测试计数因此净增 27，不把重复覆盖当成全新独立场景。初轮完整 1129 通过后又新增迟到响应失败回归并修复；最终 native_balance 21、shared_read 55、完整并发/串行各 1130、HTTP 199，production check/fmt/diff 全部通过，仍五组既有警告，未新增抑制。测试数不代表整体完成度或生产验收。

最终 manifest `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、lock `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、release `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`、executor `bfeebf9e5aa8ce78ef157406d954189a589d8e73d63802a62af6b25fdbfb9bec`、PumpAPI `acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b` 与阶段前相同，vendor 无差异。SDK 文件当前为 `012ef3263592a209566b48474c29c8fddf710c0480b024926a4272119eabcdad`，registry 为 `3d1672cab1ae19b5d61db30878689375ff0c92b04a1d0e15ba4164afd42b7657`，只读注入的可还原校验见前文。完整检查点与下一段 C2 队列记入 `EXECUTION_STATUS.md`。

## 后续与发布边界

Quote 余额的 supply/token account/ATA 读取、寻池、独立 quote/WSOL、模拟及 SDK 内部/初始化请求仍未统一预算。生产供应商对空 slice 的兼容、真实负载/页间一致性、生产启动/迁移恢复、调用方验收仍需另外窗口。本批无新 migration，但整个未部署优化树已有 004–007，发布仍须总体备份/迁移/回滚门槛。

完整最终消息授权、opaque SDK 广播前签名账本、Memory/Shadow 等模式、通用双假后端、依赖检查、可信 SDK 来源/升级演练等仍属完整目标，不因本批完成而删除。
