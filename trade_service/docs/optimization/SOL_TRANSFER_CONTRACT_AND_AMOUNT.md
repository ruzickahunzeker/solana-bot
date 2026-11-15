# TS-S1-F1 SOL 转账合同与金额政策

状态：源码/离线通过；不是签名账本修复完成。未部署、未加载生产钱包、未操作生产数据库、未连接真实 RPC、未签名或发送交易。

## 已实施

- `api/dto/sol_transfers.rs` 保存原 V1 请求、金额模式和 11 字段响应。models 保留兼容重导出，HTTP 路径和提取器不变。
- `domain/sol_transfer.rs` 表达固定 lamports、50%、100% 三种内部选择，不携带 serde、SDK、RPC 或私钥。
- `application/sol_transfer_policy.rs` 接管 key 校验、互斥金额选择和原 fee/rent 计算；DTO 仅映射输入，不提前校验。
- Handler 仍在原位置调用 key/金额政策；SDK transfer 原位置调用移出的计算函数。钱包、目标、reserve、SDK 调用、finish 和响应顺序未改。
- `WalletRepository::empty_for_transfer_tests` 仅在 cfg(test) 下创建内存 SQLite 和原表结构，passphrase=None。fixture 只写公钥和无效 secret 占位符，未构造可签名钱包，空运行注册表也不初始化 SDK。

## 不变的语义

```text
JSON 提取 → internal 鉴权 → 禁播政策 → key 校验
→ source/owner → destination/owner → 地址语法 → 金额互斥
→ reserve → 原 runtime lookup/转账 → 原 finish/读回
```

- owner 和 key 保留 literal 值；trim 仅检查 key 非空，128 限制仍按字节。目标 raw address 保持原 trim。
- 固定金额必须为正且没有 percent；百分比仅允许 50/100 且不同时传固定金额。
- 固定金额及 50% 预留 rent_exempt_minimum + 10,000 lamports，再扣 fee；50% 对 spendable 整数向下取半。
- 100% 只扣本次网络费，转出余额，其预留为 0，不改成固定租金保底。
- saturating/checked 运算、错误优先级、u64 全精度与旧 SDK raw 参数错误保持。
- 请求原来没有 simulate 字段；未知字段仍按原 serde 行为忽略，不能把传 simulate=true 当成转账模拟。硬禁止广播仍由既有政策执行。

## 原实现证据

先对未提取的原算法执行 1,890 组 balance/fee/rent/fixed/percent 组合，包含 u64::MAX、超过 2^53 的整数、零金额、非法组合和饱和边界；冻结结果及错误文本的 SHA-256：

```text
18bdd66c466fbf01c6b72ca4f1c1189c3126fbccc58aed53ae5fcaff7162660b
```

提取后同一测试从 SDK 兼容引用实际调用 Application 函数，指纹不变。冻结不是复制一份旧算法让新旧代码各跑各的。

7 项真实 Router/临时库基线覆盖鉴权/提取器、key/owner/目标/金额错误顺序、固定/50/100/system/u64 输入、未加载 runtime、完整重复响应、全局 key、8 并发 reserve、写入 trigger 故障、literal 身份与整数提取器。

缺陷证据已明确命名 legacy/defect：

1. 同 owner 同 key 即使更换 source、destination 或金额，旧 Handler 仍返回原记录；不是正确幂等合同。
2. finish 更新失败被忽略，processing 行可能残留；更新不存在的 transfer_id 也返回成功。
3. 当前没有发送前签名字节账本，send 超时不能证明未上链。
4. 旧 INTEGER 写入用 u64 as i64，u64::MAX 存为 -1 后读回再转 u64；当前测试只冻结旧格式，不把此写法用于新签名账本。

这些缺陷测试不是未来行为规范。安全修复时应明确改变 Handler 的缺陷预期，并保留旧记录兼容样本，不能为了旧测试通过而继续错误行为。

## 验证

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet sol_transfer
cargo test --locked --offline --quiet http_contract_tests
cargo test --locked --offline --quiet
cargo test --locked --offline --quiet -- --test-threads=1
git diff --check
```

2026-11-13：原实现 9 项基线先通过，再新增政策 3 项、DTO 2 项。提取后针对性筛选 15 项通过（含原有隔离测试），HTTP 92 项通过；默认并发/串行全套各 450 项通过，0 failed，无测试构建警告。

manifest、lock、现有 release SHA-256 与 F3 一致；vendor/原迁移 diff 为空。钱包 schema 未改，生产配置、钱包、passphrase 和数据库未读写。

后续 F2-A SDK 阶段分离、F2-B 账本及 F2-C 原 API/Application 接入已完成源码/离线验证，见 `SOL_TRANSFER_SDK_STAGES.md`、`SOL_TRANSFER_JOURNAL.md`、`SOL_TRANSFER_API_EXECUTION.md`。本文件旧缺陷为历史基线，不是当前生产源码合同；不得将离线接入等同生产/链上验收。
