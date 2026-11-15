# 普通交易旧指纹的输入资产歧义

日期：2026-11-13。状态：TS-S1-E 兼容修复已实现，177 项离线测试通过；未部署、未操作生产库。本项为独立行为修复，不混同 DTO 文件迁移。

## 证据

`src/trade_contract_tests.rs` 的 `legacy_hash_omits_sol_aggregator_vs_direct_quote_input_kind` 使用合成 Mint 和内存配置证明：

```json
{"side":"buy","pool":"pump-amm","quote_mint":"<USDC>","amount_sol":0.001}
{"side":"buy","pool":"pump-amm","quote_mint":"<USDC>","amount_usdc":1.0}
```

其他参与 hash 的参数相同时：第一笔规范化为 SOL 聚合器输入 1,000,000 lamports，第二笔为直接 Quote 输入 1,000,000 USDC 原始单位；旧 hash 不包含 `aggregator_sol`/输入资产种类，因此两者 hash 相同。

这不是新命令转换造成的；旧 `NormalizedTrade` 已有不同标志，内部金额也正确区分 SOL/Quote，但 durable reservation 的历史 hash 未纳入该区别。同钱包、同 key 下可能错误返回先前不同路径的订单为 duplicate。没有证据表明这会自动额外广播；不能据此生成新 key 补单。

## 兼容修复

1. 不直接更换 hash 算法，不删除幂等记录，不重发历史交易；保留已存订单、签名和状态。
2. durable reservation 仍先做原唯一键插入。命中旧行后先核对旧 hash，再解析 `request_json`，核对钱包/key/方向/Pool/Mint 记录身份，并比较聚合器执行分支；不同分支返回 Conflict，HTTP 继续使用原 `409` 错误合同。
3. 旧请求 JSON 缺失、损坏、身份不一致、金额字段互斥不成立或 Quote 无法解析时 fail closed。HTTP 返回既有持久化不可用 `503`；原行保持，GET 记录仍可查询，不释放键、不创建替代记录。
4. Replay 只复用确定性的 Quote 别名与分支选择，不重新应用当前配置默认值、协议白名单或 feature flag。旧可选字段缺省允许读取，历史 JSON 不回填或覆写。
5. 正常 SOL/lamports 别名、USDC/Quote/units 别名和非交易 source metadata 仍命中原订单；原 hash 算法和所有冻结指纹保持不变。
6. Memory/shadow 现在原子保留 `(wallet_id, literal key)`、规范化 hash 和执行分支；相同参数/分支返回原 ID，不同参数/分支返回相同的 `409`。两个命名空间字段分开保存，避免拼接冒号造成意外重合。仍不提供跨重启保障，shadow 不因镜像写入而成为 durable。

本轮保留的 legacy hash 复现测试只冻结历史证据，不是继续把两种交易判成同一笔。修复测试已新增正确的 reservation 冲突断言，同时保留旧 hash 兼容验证。

## 验证

新增 11 项测试，全部使用内存结构或临时 SQLite，无 signer、RPC 或广播：

- SOL/Quote 相同旧 hash 的两个先后顺序均 Conflict，原请求仍 Duplicate；不插入第二行。
- 买入 SOL/lamports、USDC/Quote/units 别名及 metadata 兼容。
- 稀疏旧 JSON、缺省 wallet_id、原 hash 和已存签名保留。
- 损坏 JSON、身份错误、非法 Quote 和互斥金额 fail closed；修复测试行后只返回原订单，不生成新订单。
- 文件库重开后原 Submitting 变 Unknown，原签名保留；跨路径仍拒绝，不自动执行。
- 100 个并发 SQLite 请求含两种付款资产，结果为 1 Created、49 Duplicate、50 Conflict。
- PumpSwap、Raydium CPMM、StonkFun × USDC/USD1/custom Quote 的 9 组输入，不局限单个 Mint。
- 不同钱包作用域独立；memory 的同参/跨路径/参数冲突、命名空间分隔及 100 请求并发覆盖。

```bash
cd /root/http/sol-bot/trade_service
cargo fmt --check
cargo test --locked --offline --quiet
git diff --check
```

结果：177 passed，0 failed，无编译警告。未改变数据库 schema、hash 算法或 SDK，未构建/替换 release。

## 发布边界

- memory/shadow 对不同参数的重用 key 以前可能返回旧订单，现在会拒绝；调用方须按既有 409 合同处理，不能自动生成新 key 重试。
- 全流程 HTTP 成功钱包/故障注入仍需 TS-08 的 ports/fakes；本项只证明 reservation、恢复与现有 Handler 映射源码，未做真实交易验收。
- 这是针对已证实输入分支歧义的兼容检查，不是对全部历史 JSON 的防篡改认证，也不修复其他资金路径的独立幂等问题。
