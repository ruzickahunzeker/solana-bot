# TS-S1-H1 PumpAPI 请求与格式边界

日期：2026-11-13。接续 `PUMPAPI_SIGNING_SAFETY_WORKPACK.md` 的 H0。仅 Trade Service 源码、离线测试和文档；不升级 SDK/依赖、不构建 release、不部署重启、不访问生产钱包/数据库、不调用真实 HTTP/RPC、不签名或广播。

## 先冻结与复现

原实现先通过三项请求/格式基线，再由第四项测试复现：合法 Legacy 交易的最后一个指令数据字节为 ASCII 空白时，trim 会删除交易数据，导致 unexpected end of file。该失败发生在纯解码，不需要 TradingClient、Keypair 或真实节点。

- 请求矩阵 80 组：WSOL/USDC/USD1/合成自定义 Quote、有无 Pool、买卖、五种整数金额（包括超过 2^53 和 u64::MAX）。冻结 JSON SHA-256 为 `79eae7218b84f98ccf08c048282855ff952b057ff1701c2525dfd74b4606cd0c`，并单独冻结完整买入示例与错误优先级。
- Legacy 格式样本 204 字节，SHA-256 为 `a61bd3bfbefcc156288c7ea818676bfd03affcc04e5d1a56481f1a968fadb5a8`。
- v0 格式样本 206 字节，SHA-256 为 `72002e8a37d668eaa11dddd40f8057581a1cb9867f25a0c9e50fb1db5c3a4867`。
- 样本含零填充签名槽及合成账户/指令，只证明编码合同，不是真实可交易池或消息授权证据。黄金断言位于 `src/pumpapi_client/tests.rs`，不是本轮修改后重新生成期望值。

## 实际边界

| 模块 | 实际职责 | 不具备的能力 |
| --- | --- | --- |
| adapters/pumpapi/request | 接项目 Address、Side、原整数/百分比、滑点和旧 priority fee，生成原 JSON | 无 SDK client、Keypair、RPC、URL/配置或签名能力 |
| adapters/pumpapi/response | 有界 JSON/base64 容器解码及错误 URL 脱敏 | 不解释 Solana 指令，不声称交易获授权 |
| adapters/pumpapi/transport | 原 Local Transaction POST/timeout、流式有界读取及 HTTP 错误处理 | 只取 PumpApiConfig/JSON，不接受 signer/交易客户端，不签名或广播 |
| adapters/sdk/pumpapi_trade | 原 command -> SDK 调用参数及 SDK 公钥 -> 项目公开请求输入 | 不决定路由、授权、幂等或新能力 |
| adapters/sdk/pumpapi_transaction | 有界、严格的 Legacy/v0 bincode 解码与结构 sanitize | 无 HTTP/RPC/signer；不验证池/资产/支出/费用或 ALT 内容 |
| pumpapi_client | 仍负责旧组合执行及 simulate/send | 仍持有 TradingClient；H1 没有完成其运行时能力隔离 |

旧 adapters/pumpapi::pumpapi_trade 路径继续重导出 SDK mapper，既有调用与测试不换语义。兼容 re-export 不意味着整个历史模块已完成依赖方向验收。

## 有意收紧的格式行为

1. 先对**未经裁剪的原始 bytes**尝试严格解码；只有失败后才解释文本容器。合法 binary 的末尾空白值全部保留，JSON string/单元素 array 的外部空白仍兼容。
2. 单次 HTTP 响应最多 16 KiB：先检查可用的 Content-Length，再按实际 chunk 累积检查；缺失/低报长度不能绕过上限。拒绝超限 chunk 时不部分追加。成功和 HTTP 错误响应都受限制。
3. base64 字符串最多 1,644 字节，解码后及 raw transaction 最多 1,232 字节；不是用 JSON 长度代替链上 packet 长度。
4. 使用原 bincode fixint 格式，增加解码 limit、reject_trailing_bytes 和 Solana sanitize。合法样本字节保持；截断、额外交易/附加空白、签名向量与 header 不匹配、越界索引被拒绝。
5. 当前只接受 Legacy/v0；SDK 类型中虽存在 v1，本阶段显式拒绝，不能随依赖类型增加而默默开放。未知版本/离线消息同样失败。
6. JSON object 仍是错误而非交易；bundle 必须恰好一笔。JSON object 与 HTTP 错误里的 URL 会脱敏；请求/响应传输错误不保留含 URL 的原 reqwest error chain。

以上是单独记录的格式安全修正，不声称畸形数据/未支持版本仍与旧实现完全兼容。原普通 API 字段、状态结构、校验顺序、金额含义、持久化 key/hash/schema 和默认后端未改。

## 保持的金额语义

USDC/USD1 仍按 6 位、其他 Quote 仍按旧 9 位生成浮点 wire amount；sell 仍仅接受 100%，denominatedInQuote 仍为字符串，滑点仍 bps/100。超过 2^53 的旧 f64 舍入也被黄金矩阵冻结，不能把本次拆分当作精度修复。

矩阵中的合成自定义 Quote 只用于 mapper 合同，不授权绕过现有 validator/registry，也不表示任意 SPL Quote 真实可交易。H2 必须按原整数意图审计真实指令支出，而不能信任第三方回包或其浮点数量。

## 测试及剩余风险

H1 新增 16 项测试：请求/映射 3、Legacy/v0/容器/截断/限额/结构 10、传输累积/无 socket Response/HTTP 错误 3。所有交易由公开固定字节和零签名槽构造；单独使用非零固定 signature bytes 的样本只证明结构解码不等于验签或授权，没有调用 signer。

覆盖两版本全部截断前缀、六种末尾字节、raw/text 附加数据、两种版本恰好 1,232 与 1,233 字节、JSON 恰好 16 KiB 与超限、base64 上限、六种结构损坏、未知/v1、v0 lookup 以及 HTTP 200/400/429/502。另已纠正测试对 Rust ASCII whitespace 的假设：0x0b 是非法二进制/容器输入，不属于空响应；没有放宽生产解码来迎合该错误期望。

最终验证全部通过：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 711 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 711 passed
cargo test --locked --offline --quiet http_contract_tests  # 152 passed
cargo test --locked --offline --quiet pumpapi_h1           # 16 passed
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 仅剩 models/persistence 两条既有兼容重导出警告。Cargo.toml、Cargo.lock、既有 release SHA-256 仍为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原 migration 无 diff。执行队列同步至本阶段，不把本轮当作外部 PumpAPI 供应商全量兼容、真实节点或链上验收。

**H1 时点的 H2/H3 风险**：当时组合执行在判断 simulate 前仍签外来 Message；没有完整 payer/额外 signer、指令、池/Quote/Token Program、方向、整数预算、min_out、费用和目的账户授权，也没有普通交易发送前 signed evidence。sanitize/零签名槽/成功解码不构成这些证据；v0 lookup 未解析验证。

后续 H2-A 已将 unsigned 模拟移到 Live 入口之前，并核对模拟钱包/单零签名槽；见 `PUMPAPI_UNSIGNED_SIMULATION.md`。本文件保留 H1 历史证据，当前剩余是完整 Live 授权和普通 journal，不把后续局部修复当成整个 H2/H3 完成。不开放任意签名接口、不自动重发历史失败单。

再续 H2-B1：PumpAPI 调用参数改为完整项目 command 的自有只读快照，请求直接使用项目 Address；原 SDK mapper 文件因已无 SDK 转换责任而移除，当前实现在 `adapters/pumpapi/execution.rs`。H1 表格是当时目录证据，不要求为兼容说明保留空模块；原工厂/类型调用路径与请求黄金字节保持，详见 `PUMPAPI_OWNED_EXECUTION_CONTEXT.md`。
