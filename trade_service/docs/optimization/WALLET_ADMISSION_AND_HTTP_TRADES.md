# TS-04-E2 下单身份端口与真实 HTTP 回归

日期：2026-11-13。状态：新增 12 项离线测试，全套 282 → 294，通过且无编译警告。未升级 SDK/依赖/vendor、构建 release、部署/重启、读取生产钱包/数据库、连接真实 RPC 或发送交易。

## 已实施

- `application/ports/wallet_identity.rs`：`LoadedWalletDirectory` 只解析默认/显式 ID 并返回 `LoadedWalletIdentity`；身份只暴露项目 `Address`。
- `adapters/sdk/wallet_identity.rs`：查询现有已加载 WalletRegistry，不查询全部 wallet repository，不将未加载/已禁用钱包自动认定为可执行身份。
- 真实身份句柄私有持有原 `Arc<SdkTradeClient>`，在 HTTP 接受函数作用域内保留原引用生命周期；不暴露 SDK client、signer、RPC 或通用签名能力。
- 普通/批量接受流程只改默认 ID 与钱包身份来源。身份检查仍先于参数校验，执行策略/command 转换仍先于幂等保留；未移动这些政策或改变失败状态。
- Executor 仍在原执行时点独立解析可执行钱包，不能用接受时查到公钥替代执行前检查。
- AppState/bootstrap 注入真实 directory；共同 HTTP fixture 使用空真实注册表的 directory，原“找不到钱包”的拒绝测试保持，不构造 SDK 钱包。

这一步不修改钱包管理、导入/导出、默认 ID 存储或远程映射，也不将身份 port 当作资金授权。

## 真实 Router 验证

`http_contract_tests/trade_execution_tests.rs` 新增 11 项，通过实际 Axum Router / Tower oneshot 调用原 POST/GET 路由、原 validator/command mapper、原幂等/接受流程、真实 Executor 与原持久化实现。仅身份/资金后端被注入为无 signer、无 RPC 的 fake。

| 场景 | 验证 |
| --- | --- |
| memory/durable × SDK/PumpAPI × 买/卖 | POST `/v1/trades` 原 8 字段成功响应、公钥/ID/method/签名一致；GET 原订单；每笔 1 次执行，空真实钱包注册表；接受身份 lease 在等待执行时存活、请求结束释放 |
| 默认/显式钱包 | alpha/beta 的 ID、公钥、签名与实际 command 一一对应，保留默认解析和字面 ID 语义 |
| 重复与金额冲突 | 两种持久化模式均返回原 request_id/signature，duplicate=true；冲突 409，不保留第二笔订单，不再解析/执行资金后端 |
| 批量 | 实际入口 `/v1/batch/trades`；forced IDs 覆盖 payload wallet_id，各钱包独立执行，重复钱包去重，缺失项保留原错误格式，整体 202 |
| 身份/参数错误顺序 | 缺失身份先报原 400；已有身份的非法 Mint 返回原 rejected TradeResponse 并保存拒绝记录；0 backend call |
| 鉴权/JSON | 无 token、IP 拒绝和 JSON 提取错误均不调用身份或执行源；不增加路由或改变提取规则 |
| 禁播 | 已有身份且参数有效后按原策略返回 403，0 订单保留/执行；lease 释放 |
| durable reserve 失败 | 关闭新建内存 SQLite 后实际 POST 返回原 503，删除临时内存订单，0 executor/backend 调用 |
| 接受后钱包不可执行 | 身份仍存在但执行解析失败，返回原 failed/missing-wallet；不因已有公钥而放行 |
| pending 与重放 | 原 POST 202，无签名；执行被 Notify 闸门暂停期间同 key 重放返回同订单/duplicate，不新执行；释放后原 GET 查询 submitted/signature |
| 并发重放 | 两种模式分别并发两次同 key 请求，只有一项 duplicate，request_id 相同，原订单完成，1 次后端调用 |

SDK 身份 adapter 另有 1 项：空已加载注册表不返回默认/其他钱包身份，默认 ID 与显式空串/空格字面输入不被擅自修正，不初始化钱包/SDK/RPC。

HTTP 中的成功仅指进程内 fake 资金后端成功，不是链上签名合法性、广播、确认或协议交易成功。新 HTTP 矩阵为显式 Live 输入，模拟/缺省模式的原合同继续由先前 validation/command/executor 测试覆盖，没有据此宣称新增完整 HTTP 模拟矩阵。

## 数据与安全边界

- durable 测试只创建新的 `sqlite::memory:`，执行原迁移/保留/查询；关闭操作也只针对该临时 pool。
- 所有身份、地址、签名、token 和 IP 都是合成 fixture；没有 `.env`、私钥或生产连接。
- pending/concurrent 任务均在测试中等待完成，有界 timeout 防止静默遗留；没有新后台 worker 或定时器。
- 身份 trait 的 Box 保留原对象生命周期，避免把原长期 client 引用改成提前释放的裸地址快照；fake lease 的增减计数在正常等待/提前拒绝/错误中验证。
- API 路径、字段、状态、默认值、校验顺序、幂等 hash/旧 JSON/schema 不变；没有修改历史订单或重发失败交易。

## 验证命令

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet http_contract_tests::trade_execution_tests
cargo test --locked --offline --quiet
git diff --check
```

结果：294 passed、0 failed、无编译警告。原 37 项 HTTP 合同加 11 项 = 48 项（含既有序列化基线）；SDK 身份 adapter 测试不计入 HTTP 数量。

Cargo.toml、Cargo.lock、现有 release SHA-256 仍与 `SDK_PARAMETER_FIXTURES.md` 一致；vendor diff 为空，没有替换运行二进制。

## 下一批 TS-02-C1

2026-11-13 续作：普通持久化输入与 V1 codec 的核心隔离已实施，新增 7 项、全套 301 项通过，见 `ORDINARY_PERSISTENCE_INPUT.md`。调用方仍通过旧桥接收 DTO；内部 record、journal port 与接受编排继续下一批，以下保留原执行要求。

1. 普通接受流程仍直接使用 wire TradeRequest、NormalizedTrade 和 AppState；本批身份隔离不等于 Application 已完全脱离 DTO/链类型。
2. 先审查旧 reserve/mirror 的 hash、request_json、normalization 和 rejected-record 规则，捕获必要的字节/错误基线，再定义项目拥有的普通持久化输入。
3. 让 DTO 到旧格式快照/hash 的映射位于边界/adapter，应用和后续 journal port 不依赖 SDK Pubkey 或 HTTP DTO；已有历史 hash/JSON/schema 不重写，不能悄悄切换新幂等算法。
4. 用本批真实 Router 的成功/重放/冲突/批量/503 和已冻结 hash 指纹保护迁移，再逐步提取接受编排；不能只移动文件而保留相同跨层依赖。
5. 普通 SDK/PumpAPI/legacy 聚合发送前签名账本、完整 VerifiedRoute、资产扩展、真实节点/协议/SDK provenance 继续独立推进。本批 HTTP mock 成功不解决发送后持久化失败或恢复风险。
