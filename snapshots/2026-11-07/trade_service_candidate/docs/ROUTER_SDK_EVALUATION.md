# sol-trade-router-sdk 评估

更新时间：2026-11-04

当前结论（2026-11-05）：`trade_service/candidate` 继续使用 `sol-trade-sdk 11.0.1`。
先补齐其已提供但服务尚未接入的多跳 builder；Router SDK 仅用于原生 builder 确认
无法构建的已解析跨 DEX 路线的隔离对照。两者的延迟、费用和成交质量需同条件测量，
不能把 Router 的“低延迟”描述直接当作本项目的实测优势。

参考仓库：[0xfnzero/sol-trade-router-sdk](https://github.com/0xfnzero/sol-trade-router-sdk)
（2026-11-05 核对 `69eaa2c03eb832c5cda15d885a4483197e65b0d1`，版本 `0.5.0`）。

Router SDK 的 Cargo workspace 将 `sol-trade-sdk` 精确固定为 `11.0.1`，客户端
也重新导出其交易参数和 SWQoS 等接口。因此两者不是二选一的互斥 SDK：Router 在
Trade SDK 基础上增加自部署 Router Program 的 CPI 执行路径。

workspace 清单同时包含 `programs/sol-trade-router` 和 `crates/sdk`；前者的 crate 类型为
`cdylib`/`lib`，描述为 Pinocchio on-chain multi-hop swap router，后者是默认发布的客户端
SDK。这支持“SDK 不能脱离匹配的链上 Router Program 独立提供完整 Route 执行”的判断。

## 定位

该项目是自部署 Solana Router Program 的 client SDK。它使用 Pinocchio 程序把调用方已经
解析的 DEX market/route 组织成多跳 CPI，并提供交易构建、nonce、SWQoS 和账户策略等执行能力。

`sol-trade-sdk 11.0.1` 自身也有 Pump 官方两/三跳、PumpSwap 和 StonkFun 的显式 quote
路线；Router 的优势需限定在这些原生 builder 无法满足、而匹配链上 Router 已部署且
审计通过的跨 DEX 路线。当前 Trade Service 尚未集成全部原生路线，因此应先补能力盘点，
不能用服务端未接入来证明原 SDK 不支持。两种路径的速度和费用仍需同条件实测。

它不是 Jupiter/DFlow 类型的报价聚合器：没有统一的外部 quote 服务，也不负责在全市场主动
比较 provider 报价。它更接近“原生链上路由执行后端”。

## 与当前聚合器路线的边界

| 项目 | Jupiter/DFlow provider | sol-trade-router-sdk |
|---|---|---|
| 主要职责 | 外部报价/路线 provider | 自部署 Router CPI 执行 |
| 路线来源 | provider quote response | 事件、缓存池状态或调用方 market |
| 是否自动比较全市场报价 | Jupiter 可返回聚合路线；DFlow 当前禁用 | 不负责 |
| 是否需要自部署程序 | 否 | 是，且必须初始化配置 |
| 当前 candidate 状态 | Jupiter read-only；DFlow disabled | 未接入 |

## 接入前置条件

1. 固定并审计匹配的 Router Program source、program ID 和 initialize 配置。
2. 用公开 market fixture 验证每个 DEX leg、SOL/WSOL/quote mint、Token-2022、ATA、fee 和
   多跳余额约束。
3. 把 SDK 的 route builder 包在 Trade Service 的 provider-neutral `router_backend` 中；
   SDK 不能直接管理 Trade Service 钱包或绕过幂等状态机。
4. 先实现 replay/observe/simulate；完成独立 preflight/audit 后才评估 build capability。

## 当前禁止事项

- 不加入 `AggregatorRegistry`，不参与 Jupiter/DFlow compare 或 fallback。
- 不部署 Router Program，不使用 live key，不签名，不广播。
- 不因为 SDK 支持多个 DEX 就把它标记为报价聚合器。
- 不在 candidate simulate-only 之外开放任何 Router capability。
