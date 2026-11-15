# TS-S1-F2-C Native SOL API 安全执行

日期：2026-11-13。状态：源码接入、离线验证完成；未部署、未迁移生产库、未读取生产钱包或执行真实 RPC/签名/广播。前置阶段见 `SOL_TRANSFER_CONTRACT_AND_AMOUNT.md`、`SOL_TRANSFER_SDK_STAGES.md`、`SOL_TRANSFER_JOURNAL.md`。

## 实际调用链

- `api/routes.rs` 合并原 `POST /internal/wallets/:wallet_id/transfer-sol`；`api/handlers/transfers.rs` 仅负责鉴权、DTO、政策和 HTTP 映射，不访问 SDK/RPC/SQL。
- `application/sol_transfers.rs` 负责输入/权限验证、原子保留、执行和读回；`ports/sol_transfer_execution.rs` 提供公开 metadata、受限 runtime/wallet 与只读 artifact。
- `adapters/sdk/sol_transfer.rs` 复用原 WalletRegistry/WalletRepository，不初始化或解密钱包。只在 Created 后获取一次原 loaded wallet Arc，后续注册表替换不换签名人。
- `sdk_client/sol_transfer.rs` 保留受限 prepare/sign/send/confirm。无 journal 的旧 `SdkTradeClient::transfer_sol` 组合入口已移除，旧 repository reserve/finish/get 仅作测试 oracle。

顺序为：鉴权 → 禁播政策 → key → source/owner → target/owner → 目标语法 → 金额 → source 语法 → journal reserve → 仅 Created 获取 runtime → unsigned prepare → snapshot 核对 → sign → 签名/snapshot 核对 → record_signed 成功 → 一次 broadcast → 保存结果/确认 → 读回。

## 保留合同与明确修正

保留原 URL/method、loopback/交易鉴权、JSON 先提取、错误优先级、literal key/owner、目标地址 trim、全局 key、fixed/50%/100% 和 11 字段响应。固定/50% 保留 rent + 10,000 lamports，100% 扣准确网络费；1,890 组金额指纹不变。确认遵守原 RPC commitment，不等于 finalized。没有新增 simulate 参数、Pool/MEV/滑点或通用转账接口。

以下为刻意修正，调用方上线前必须验收：

- 同 key 异钱包/source/目标/owner/金额返回 409，不能沿用原只比 owner 的缺陷。
- 已存在 reservation 只读原结果，绝不 prepare/sign/send；标签改变不影响实际目标身份。
- 原 runtime 公钥、prepared/signed 快照必须绑定接受时的意图。
- 签名、原字节和快照持久化成功才广播。所有存储失败显式返回通用 503：`SOL transfer journal unavailable; do not resubmit or use a new key`。
- 发送错误/超时/回执不一致保存原本地签名并返回 unknown；不对此继续确认，不切节点补单、不重签/换 blockhash/新 key。
- prepare/sign 失败仍保存 failed 并按原合同返回 HTTP 200；runtime 缺失记录签前失败后仍返回 503。
- 匹配意图且具有签名的旧 submitted/confirmed 可读；不能核实的旧不确定/无签名结果 fail closed：`legacy SOL transfer outcome cannot be verified; do not resubmit or use a new key`。

取消后不在请求销毁时自动发起任何恢复副作用。保留原记录，后续相同 key 只读；初始化恢复语义沿用 F2-B。没有新增转账状态查询端点或后台任务。

## 离线证据

新增 22 项：Application 11、runtime adapter 2、真实 Router 9。使用公开公钥 fixture、fake 执行、临时 SQLite，合成签名和无效占位字节；fake broadcast 查询实际 journal 验证已保存相同签名/字节。

- 完整事件顺序、fixed/50/100、11 字段、目标别名/地址等价、全部意图冲突、旧记录重放。
- prepare/sign、signed/result/confirm 写入、最终读回故障；unknown 与错误回执保留原签名。
- 五个取消点：reserve 已提交、prepare 中、signed 已提交未返回、broadcast 中、confirm 中；重试均不重新执行。
- 第一个请求与另 16 个竞争任务仅一次签名/发送；Router 并发、取消、文件重开及未确认 submitted。
- 原注册表替换不改变保留的执行钱包；真实 runtime adapter 不加载/解密钱包。

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet sol_transfer
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

本阶段并发/串行全套各 500 项、HTTP 101 项、sol_transfer 筛选 65 项通过，测试构建无警告。生产 cargo check 通过，剩余两条既有兼容重导出警告（models/persistence），五条未接入账本接口警告消除；没有用 allow 掩盖。

manifest SHA-256 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`，lock `4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`，原 release `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`。没有构建或替换 release，vendor/原迁移不变。

## 发布门槛与下一项

仍需独立窗口验证调用方 unknown/409/503、备份/生产迁移、FULL 写入延迟、真实 SDK 签名/节点和制品恢复。不能以 fake 成功宣称链上验收，回滚也不得恢复旧数据库消除已保存操作。

后续转入 WSOL：先冻结已有请求/响应、报价绑定、全量/部分解包、账本故障与重启行为，再隔离 Application/SDK/持久化边界；不将 native 的状态模型机械套给兑换，不扩大 Live。
