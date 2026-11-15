# TS-S1-H2-B1 PumpAPI 完整本地意图

日期：2026-11-13。接续 `PUMPAPI_UNSIGNED_SIMULATION.md`。只针对 Trade Service 源码、离线测试和记录；不升级 SDK/依赖、不构建 release、不部署重启、不访问生产钱包/数据库、不调用真实 HTTP/RPC、不签名或广播。

## 本次解决的缺口

旧 PumpApiTrade 只剩 HTTP 所需的 side、SDK Pubkey、可空 amount/sell_percent、滑点、priority fee 和模式等字段。原 TradeCommand 中的钱包 ID、幂等键、期望协议、金额种类、两跳提示、SDK fee/MEV 等在映射时丢失；后续无法可靠区分本地意图和远端给出的路由。

现在实际 ordinary adapter 仍调用同一个 pumpapi_trade 工厂，但返回对象持有**完整 TradeCommand 的自有只读快照**，不再复制一组可独立修改的请求字段。外部只能读取 command/config/mode/wait，不能取得快照的可变引用。

```text
Application TradeCommand
  -> adapters/pumpapi/execution::PumpApiTrade
       private command: owned TradeCommand clone
       private config: original PumpApiConfig
       private wait_tx_confirmed
  -> pumpapi_request(public Address, &snapshot)
  -> original JSON / Local Transaction preparation
  -> unsigned preview OR existing Live path
```

快照贯穿原异步准备和执行调用，保留原 wallet/key 字面值、完整候选 route、BuySol/BuyQuote/SellPercent、execution method/mode、独立 SDK tip/PumpAPI priority fee、MEV、aggregator 标志及 hop 上下文。新字段若增加到项目 command，会随已有 Clone 一并保留，不要求先改一份有损 SDK 参数映射才能传到后续审计点。

## 责任边界

- 新 `adapters/pumpapi/execution.rs` 只用项目类型和原 PumpApiConfig，不引入 SDK、Keypair、RPC、API DTO 或序列化依赖。
- `pumpapi_request` 直接从只读项目快照投影，不做 Address -> SDK Pubkey -> Address 的往返转换。
- 原 `adapters/sdk/pumpapi_trade.rs` 已没有实际 SDK 转换责任，因此移除；工厂和请求投影归 PumpAPI adapter。原 adapters/pumpapi::pumpapi_trade 和 pumpapi_client::PumpApiTrade 调用/类型路径保留，后者只是 opaque 类型重导出，不留空转发函数。
- SDK 分发的 simulate/Live、禁播及 Default lane 选择保持原顺序；模拟仍不拿 signer、不返回签名。Live 本轮仍调用原签名/submit，尚未增加协议授权或 signed journal。
- cfg(test) 的 command fixture 仅构造项目地址和整数类型，无配置加载/SDK/密钥/节点，不进入生产构建。

这里的“完整意图”指现有内部 command 完整保留，不表示已经拥有可信 Quote、整数费用上限、验证过的池或消息授权摘要。实际公开钱包地址仍由原已加载 Default lane 提供；仅持有 wallet_id 字符串不能证明钱包归属，仍须沿既有钱包解析及后续绑定流程核对。

## 保持的合同

1. 请求的 publicKey、side、Mint/Quote/Pool ID、原整数 -> 浮点换算、denominatedInQuote 字符串、sell=100%、priorityFee 和字段省略行为不变。
2. quote_mint 仍调用原 command.quote_mint()：BuyQuote 的有效 Quote 优先，原 route hint 同时保留；不静默改写提示或以此许可矛盾资产。
3. SDK tip、PumpAPI fee 和 Config 默认值仍独立，显式零/负零和浮点位模式不被重新计算。测试包含非有限合成值是冻结 clone/mapper 行为，不绕过现有参数校验或授权实盘。
4. H1 的 80 组黄金 JSON SHA 保持 `79eae7218b84f98ccf08c048282855ff952b057ff1701c2525dfd74b4606cd0c`；新增另一组从实际 command -> snapshot -> request 的 80 样本验证同一旧指纹，不以重算期望替代旧基线。
5. API、validator、旧 hash、SQL/schema、幂等作用域和默认后端未改；ordinary 的 aggregator-sol 优先分支不变。不增加本地 metadata 到对外 PumpAPI JSON，也不新增交易能力。

旧原始 request helper 仍通过 H1 测试检查 missing amount/sell_percent 的错误优先级。新的 command amount 是枚举，不再能表示 Buy + amount=None；H2-A 的内部入口测试相应使用不支持的部分卖出和零买入金额证明 mode/config/请求错误顺序。公开缺失参数仍由原 validator 拒绝，既有 API 合同与错误黄金样本保留，不删掉这些断言或修改错误期望。

## 离线验证

新增 7 项纯接口/异步快照测试：

| 证据 | 覆盖 |
| --- | --- |
| 完整自有快照 | 原 command 的每类字段修改后，已构造快照保持原值；无 mutable getter |
| 真实投影 80 样本 | WSOL/USDC/USD1/合成 Quote、有无 pool、买卖、五种 u64 数量，和 H1 原 SHA 相同 |
| 协议/身份/选项矩阵 | 九类既有 Pool × 两种模式、字面 wallet/key、hop/MEV/SDK fee/aggregator 保留但不额外发给供应商 |
| 金额种类与 Quote | BuySol/BuyQuote/SellPercent 原整数、有效 Quote 和原 route hint 分别保留 |
| fee 位模式 | 零/负零/普通值/NaN payload 的 copy 与两种 fee/Config 默认值不混用 |
| 投影隔离 | 编辑临时 wire 输入不改本地快照，重投影恢复原 JSON |
| 异步持有 | 原 source drop、跨 task/yield 后仍保留 wallet/key/协议/池及确认选项 |

同时机械调整原 normalized -> 两类 adapter 合同为从快照读 request；原成功/错误样本、金额/MEV/舍入、请求及交易编码指纹未重生成。合成矩阵不是九类协议的交易成功、链上身份或供应商兼容验收。

最终验证全部通过，`EXECUTION_STATUS.md` 已同步：

```bash
cd /root/http/sol-bot/trade_service
cargo test --locked --offline --quiet                      # 728 passed
cargo test --locked --offline --quiet -- --test-threads=1  # 728 passed
cargo test --locked --offline --quiet http_contract_tests  # 153 passed
cargo test --locked --offline --quiet pumpapi_h            # 33 passed, includes B1's 7
cargo check --locked --offline --bin pump-trade-service
cargo fmt --check
git diff --check
```

生产 check 有两类兼容重导出警告：models 的既有兼容导出中 Side 随本次 PumpAPI 从 wire 路径迁出后也不再由生产代码使用，另为 persistence::request_hash 的既有导出；未加 allow 或删除兼容路径来隐藏。Cargo.toml、Cargo.lock、既有 release SHA-256 仍为 `67e9b12563a4540e5055224aebdb7dc52f16170d838353c5cfa90367fc6f3a89`、`4f0bebf3947336878f5d16ad491074748cf882da806986a4e5b623deb67520d5`、`b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`；vendor/原 migration 无 diff。磁盘剩余约 2.2 GB，未为了本轮测试清理其他项目或生成 release。

## 后续不能省略

H2-B2 先复用已有只读接口/实际 loader 的适用部分，核实池 owner、两侧 Mint、Token Program、slot/时效、ALT 与账户权限。现有 warmup/dex_params 有的仅检查 Quote 或部分成员，并不是统一 VerifiedRoute；不能直接把“SDK loader 成功”当完整授权。

后续 H2-B2-A 已先完成实际消息账户/ALT 视图并接入模拟和 Live 签名前，含 owner/state/slot/库 lookup 前缀、地址顺序/去重及原消息绑定；见 `PUMPAPI_MESSAGE_ACCOUNT_VIEW.md`。这只是 B2 的通用基础，真实池/Token 与扩展/角色证据由 B2-B 继续，不能把 ALT 通过冒充整条路线已核验。

继而 H2-B3/B4 按协议解析完整外来指令、方向、整数支出/min_out、费用与目的账户及 ATA/WSOL 生命周期；把消息与这份本地意图绑定后才进入受控签名。H3 再接唯一 reservation/hash 与发送前原签名字节 journal、取消/恢复/不确定结果观测，H4 继续 SDK/legacy 组合路径。当前快照和 unsigned 通过不填补这些缺口，不自动部署、不自动重签或补发失败订单。
