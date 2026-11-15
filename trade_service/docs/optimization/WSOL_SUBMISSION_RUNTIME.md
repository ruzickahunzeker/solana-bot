# TS-S1-G2-B WSOL 提交、模拟与原实例边界

日期：2026-11-13。状态：源码接入及离线验证；未部署、未初始化生产兑换库、未读取生产钱包/密钥、未签名或发送真实交易。继承 `WSOL_CONVERSION_SAFETY_WORKPACK.md` 的全部约束，未升级 SDK。

## 实际接入

- `application/wsol_submission_types.rs` 定义不依赖 serde/SQLx/SDK 的 QuoteSnapshot、SubmissionInput、结果及原提交 read-model。Application 不接收原 API DTO 或 AppState。
- `application/ports/wsol_conversion.rs` 定义公开 metadata、原钱包执行句柄、只读 artifact 和 journal 能力。runtime 不返回 Keypair、RpcClient 或裸交易。
- `application/wsol_submission.rs` 接管 submit/simulate 顺序、模式 gate、历史重放、准备/签名/模拟/落盘/发送和最终读回。旧 `conversion::submit_wsol`/`simulate_wsol` 仅映射输入、policy 与结果，没有保留第二份执行逻辑或 fallback。
- `adapters/sdk/wsol_conversion.rs` 复用原 WalletRegistry/WalletRepository。metadata 只读，不解密/加载钱包；Created 后只 resolve 一次原 client Arc。prepare 只选原 RPC 列表首项，PreparedWsol/SignedWsol 私有携带同一个 Arc<RpcClient>，模拟和发送不再查注册表或重新选择节点。
- RPC unsigned prepare 与 UnsignedWsolTransaction 移到 `sdk_client/wsol_conversion.rs`。只使用 owner、原 RPC 和项目 quote；不再从 AppState 获取钱包/数据库。原四项 unsigned 回归随实现移动，旧类型仅测试兼容重导出。
- `adapters/persistence/wsol_conversion.rs` 复用原 `Arc<ConversionStore>`；项目快照/输入转回旧 V1 DTO 后调用原 hash/SQL。没有另建 reservation、schema、连接池或恢复逻辑，也不改变 key 唯一范围。
- `main.rs` 只组装新增 façade，并保留原 conversions handle 供尚未迁移的报价/状态入口使用。HTTP 空 runtime fixture 同样显式组装；替换临时 store 的测试会同步重建该 façade。

## 保留的合同

1. submit 先禁播与 simulate/confirmed/Live gate，再检查 store、Quote、reserve/历史；仅新 reservation 访问 metadata/runtime。历史可在过期后重放，仍先读取原 Quote，不改为独立 key 查询 API。
2. prepare 前保持 metadata enabled → Quote expiry/身份/金额 → loaded runtime → 公钥匹配 → 原 RPC/账户查询。不会因为提前查 runtime 改变旧错误优先级。
3. 四种 wrap/unwrap 账户形态、整数金额、ATA/余额/费用/租金重检、100,000 lamports 预留不变。unsigned 模拟仍零签名、sig_verify=false，且不占 Live key。
4. Live 仍 prepare/sign → signed simulation（8 秒）→ record_signed 成功 → 禁播复核 → send（12 秒）→ 保存结果 → 读回。签前/准备/模拟失败回执和 HTTP 200/400/403、七字段提交/四字段模拟响应保持。
5. 发送错误/回执不匹配/超时仍 unknown 并保留原签名；无补单、无自动重签、无节点广播切换。普通交易、native transfer、Quote 独立账本均未改变。
6. V1 Quote/Submission 字段、flags 和序列化顺序留在兼容边界；fingerprint 沿用原函数。项目 snapshot 不带 serde，不能意外以新内部序列化替换历史 hash。

唯一有意收紧的运行行为：同一请求不会因后续注册表变动而换钱包/RPC。此时继续持有接受的原 client，不是改用默认钱包或失败后重新 resolve。钱包删除/禁用后如何撤销已接受执行仍需独立政策，不能在重构中暗改。

## 新增离线证据

新增 15 项（旧四项 unsigned 测试仅移动，不计新增）：

- Application 8：原句柄在 prepare 时模拟注册表替换后仍被 sign/simulate/broadcast 使用；unsigned 不签名/不保留；metadata/owner/pubkey/runtime/prepare/sign/simulation 错误；签名/结果/失败回执写入错误；unknown 不重发；16 个竞争请求仅一个执行者；gates 与绑定顺序。
- Journal adapter 1：原 store Arc 指针、V1 wire 往返、原 fingerprint、跨新/旧接口读取同一条 reservation。
- SDK runtime adapter 1：原 registry/repository Arc、仅公开 metadata、无真实 loaded wallet，也不解密 fixture 密文。
- SDK prepare 2：真实 unsigned prepare 经内存 RpcClient mock 覆盖新/旧 ATA wrap、全量/部分 unwrap；断言指令数、所需 signer、blockhash 和零签名；ATA program/Mint/owner/native、余额/费用/租金/状态变化拒绝。
- Router 3：模拟/提交完整响应、16 请求并发、真实 Application 与临时库上的存储故障及重复不重发。SDK runtime 为空，注入的是 fake 执行端口，签名为合成字符串、字节为无效占位符。

fake broadcast 在实际临时 ConversionStore 中核对 status=signed、相同原签名和字节，才允许返回 fake 回执。实际 SDK client/RPC Arc 的保留由生产 adapter 的私有字段与编译保证；没有借此宣称真实钱包签名、节点或链上成功。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet wsol
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

默认并发/串行全套各 532 项、HTTP 109 项、wsol 筛选 40 项通过，无测试构建警告；格式/diff 检查通过。生产 check 通过，仅剩原 models/persistence 两条兼容重导出警告。manifest/lock/release SHA-256 仍与 `SOL_TRANSFER_API_EXECUTION.md` 一致，没有生成 release 或修改 vendor/原迁移。

## 后续门槛

本文件记录 G2-B 检查点，532 项等计数为历史结果。后续 G2-C 已完成 Quote query/status、API DTO/Handler/route group 和 builder 分拆，旧业务包装移除；见 `WSOL_QUERY_AND_API_BOUNDARIES.md`。这不等于 G3 安全修正或全服务分层完成。

G3 刻意尚未改状态 SQL/恢复/错误映射：迟到 unknown 可降级 submitted、reserved/signed 重启保留原状、signed simulation 在落盘前、存储错误仍为 400 等风险继续保留并明确标记。现有缺陷基线不是未来安全合同。下一阶段按独立安全修正验证单向 CAS、取消、原签名对账及调用方错误兼容；不得释放不确定 key、清空数据库或重新签名来规避旧记录。

生产迁移、FULL 写入延迟、真实签名/RPC、制品上线仍是独立授权窗口。整体计划的 PumpAPI 审计、普通发送前账本、协议/扩展证据、SDK 来源及升级演练、基础设施与发布回滚均未因此完成。
