# RPC-01C2 Quote 余额共享只读运行时

日期：2026-11-15。范围：Trade Service 源码/离线验证，未部署。不升级 SDK、不操作生产钱包/数据库、不改公开 API、存储/hash/金额合同、不构建 release 或重启服务，不签名发送，不恢复 PumpAPI 交易对接。实际最终测试和指纹见 `EXECUTION_STATUS.md`。

## 实际接线与范围

`RpcWalletReader::quote_balance` 已接 main 创建的同一个 SharedReadRuntime，不再通过 SDK RPC 方法查询余额。Reader 只保留项目 Address、原 URL 列表与共享 runtime，构造时将传入 SDK RPC handles 投影为 URL 后释放；真实 HTTP 测试用 Weak 验证不保留 handle，不实例化 SDK 钱包。

查询仍只选择该钱包原首个 Default RPC，失败/429 不扩大到其他节点，也不修改 SDK 初始化、lane 选择或交易发送 client。原生批量余额仍使用原全部 Default 候选；这两种政策不能混用。没有增加第二个 HTTP client、RPC registry、余额缓存或新数据库。

尤其注意：`wallet_reads::token_balance` 仍由 `SdkTradeClient::token_balance` -> `LanePreparation::token_balance` -> 普通买卖准备调用。本批保留该 helper 与 ATA fallback 的原字节（片段 SHA `f086b7b4fa750fa35502b7fdaee70ec42304911edc91ab5572f7a2ff81d04fb9`），不把查询 API 的新校验偷偷套到交易执行。SDK/root executor、manifest/lock、vendor、PumpAPI、release 均不改。

## 查询与原语义

请求为固定只读 enum，不开放任意 RPC：

1. `getTokenSupply(mint, processed)`，保留原 decimals/raw amount 类型。
2. `getTokenAccountsByOwner(wallet, Mint, jsonParsed/processed)`，编码包含原 `dataSlice:null`、`minContextSlot:null`；纯测试与 SDK RpcAccountInfoConfig 的序列化对照，而非猜测格式。
3. 合法空扫描直接得到零；完整合法解析按原 u64 饱和相加，不经过 f64。
4. 节点/传输不可用或全部无法解析时，按原 SPL -> Token-2022 两个规范 ATA 顺序查询 `getTokenAccountBalance`。第一个正余额即停止；任一 ATA 不可用且没有正余额仍报错，不伪造成已知零；两次成功且零才是零。
5. 得到 token 结果后才 `getBalance(wallet, processed)`；SOL 读取失败不返回部分 Quote 快照。

常规三笔、ATA 路径最多五笔实际 HTTP，没有 SDK 的隐藏 429 重试。HTTP 302 本身不会跟随，仍可按旧“scan 不可用”政策查询同一入口的规范 ATA；重定向目标零连接已验证。规范 ATA 由现有 interface 库派生，与原 SDK fast helper 多组对照，无自制 PDA 算法或 signer。

ATA 回退仍是历史两地址政策，不承诺覆盖所有非 ATA 账户或提供完整可支出余额证明。保留该政策不等于完整 scan 证据；这里没有新增来源/银行 slot 等公开字段。各步可能处于不同 slot，不承诺同 bank 原子快照或节点诚实性。

## 拒绝不可信或不完整数据

- JSON-RPC 版本/ID、context slot、结果/error 互斥、类型或 JSON 无效直接拒绝；合法 RPC error 与传输不可用才能进入旧 ATA 回退。错误不回显 URL、provider body/message。
- 扫描最多 1000 个账户；不分页、不截断、不返回前半合计。
- 账户地址/owner 必须是固定 32 字节，不能重复；program owner 与 `spl-token`/`spl-token-2022` 标签一致，账户非 executable、parsed type 为 account。
- 已解析 info 的钱包、Mint 必须与本请求一致，state 是 initialized/frozen，decimals 与 supply 一致，raw amount 是有效 u64；冻结账户仍可显示持有量，不因此授权支出。
- 混合“有效解析 + 无法解析”不返回部分合计，也不以 ATA 成功掩盖坏扫描。全未解析分支保留旧回退政策。
- typed/identity/数量/响应上限错误不当成“零余额”或普通节点不可用，不自动刷新或重试交易。

上述是来自同一 RPC 的语法/元数据绑定与完整性检查，不是原始账户/共识证明、独立 Mint owner 验证、费用扩展审计或最终交易授权。Token-2022 的 uiAmount/扩展字段不替代 raw amount，不用显示余额承诺可转账金额。

## 预算与响应上限

Quote 余额为第五类独立容量：一笔操作在途，忙时立即错误；发送起点至少间隔 250 ms（所有 supply/scan/ATA/SOL 步骤共享），单步含等待最多 2 秒、整次含全部步骤最多 8 秒。继承发送前后 deadline 检查，取消释放本地 future/名额，不返还已用间隔，无后台续查。

只对 TokenAccounts 扫描开放 512 KiB 声明/流式上限，其他请求维持 64 KiB；所有请求仍禁止 HTTP 重试/重定向。新增类型化 ReadBodyLimit，Quote adapter 不靠错误字符串猜测超限：超大扫描明确拒绝，不返回截断结果，也不转入 ATA 后伪造成功。普通网络中断/timeout 则仍按原回退政策处理，整次不超过总预算。

五类读取独立名额/起点，共享 canonical endpoint 的 HTTP 429 冷却。Retry-After、不同 key/别名、取消和冷却不延长等原政策不改；scan 返回 429 后 ATA 实际零新增请求，其他类别随后也跳过该入口。JSON-RPC -32005 等正常错误按原不可用处理，本批未声称实现所有供应商的账户配额/错误码治理。

已迁移范围最多五笔客户端 future，持续起点理论速率 10+4+1+4+4 次/秒，不是任意一秒滑窗硬额度、供应商/跨进程或全部 SDK 配额；独立容量不等于 CPU/网络物理抢占或线上延迟 SLA。

## 测试证据

本批新增 23 项：六项 codec、十项 runtime、一个与旧 helper 的差异 oracle、六项真实 HTTP 父用例；复用原 exact-filtered child，不重复计算 helper。`quote_balance` 筛选 25 包括两项已有合同，不把它们当新增。

范围包括精确大整数/饱和/零、Token-2022/frozen、字段/身份/精度/重复/过量/混合不完整拒绝；规范 ATA 顺序与零/错误矩阵；首节点/不同 key 不扩大、五类容量、取消/不返还间隔；三步慢成功及五步被总 deadline 取消，不返回部分快照。

真实 loopback 使用生产 RpcWalletReader/Reqwest/runtime，核对原三笔请求、空/全未解析/合法 RPC error/两个 ATA、512 KiB declared/chunked 边界与超限零 fallback、坏 ID/JSON/Mint/混合解析、headers/body timeout 无隐藏重试、redirect 零连接、429/首节点范围及 busy/cancel 不饿死 hash。只在隔离 child 清除代理，不启动 main、不读取生产 `.env` 或钱包。

差异 oracle 实测旧 helper 会接受扫描内错误 Mint 的数值，新查询 codec 拒绝。旧整文件/八项 oracle 与现行 helper 测试保留；这不是宣称普通交易 helper 已安全或已接共享预算。

最终 quote_balance 25、shared_read 71、完整并发/串行各 1153、HTTP 199，production check/fmt/diff 全部通过，仍五组既有警告；没有新增 warning 抑制。新增 23 项与筛选/总数口径分开，不用它们推算整体完成度。

阶段前后保持：manifest `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、lock `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、release `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`、executor `bfeebf9e5aa8ce78ef157406d954189a589d8e73d63802a62af6b25fdbfb9bec`、SDK/root `012ef3263592a209566b48474c29c8fddf710c0480b024926a4272119eabcdad`、registry `3d1672cab1ae19b5d61db30878689375ff0c92b04a1d0e15ba4164afd42b7657`、PumpAPI `acf58f89e90d0400c4890ec08adb1157da7cced0d4ee3d942e07991f8efca78b`；vendor 无差异。旧 helper 的 SHA 见前文，必要改变只在查询 adapter/共用 transport、测试与文档。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet --no-run
cargo test --locked --offline --quiet quote_balance
cargo test --locked --offline --quiet shared_read
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo check --locked --offline --quiet
cargo fmt --all -- --check
```

## 后续门槛

优先 NP-01-A3 普通交易准备的 token 余额证据：不能直接拿查询 Snapshot 替换旧准备 helper、增加与交易无关的 supply/SOL 读取，或改变有效金额/协议参数及 lane fallback 的时点。先核对 caller/钱包/Mint/Token Program/整数与 malformed 数据是否允许下一 lane，再接受控只读 port，独立验收取消、错误和零 signer/invoke。

寻池、账户/模拟和 SDK 初始化/内部读仍未全迁移；完整消息授权、opaque SDK 广播前 journal、其他 backend/mode、通用双假后端、依赖检查和可信 SDK 来源/升级演练继续保留。PumpAPI 开发暂停项仍计入完整目标。

真实供应商格式/512 KiB/1000 账户、负载、生产配置/恢复/调用方验收仍缺；本批无新 migration，但整体未部署树已有 004–007，未来发布仍需完整备份/迁移/回滚门槛。未 release 构建、部署、重启、签名或广播。
