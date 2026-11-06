# Route / quote adapter fixtures

更新时间：2026-11-04

本增量把 route 的 RPC 验证结果写入 `ResolvedRoute`，并接入一个可审阅的
SDK 11 quote/simulation adapter：Raydium CPMM exact-in buy/sell quote，以及
buy simulation。报价使用 SDK 自带的 `compute_swap_amount_for_pool`，包含当前
交易费、creator/fund/protocol fee 和 Token-2022 transfer fee；服务仍固定
`TRADE_SERVICE_SIMULATE_ONLY=true`，不会广播。

## 合成 fixture

fixture 使用固定的 pool/base/quote 地址和 reserve，只验证单位、方向和费用计算，
不替代主网账户读取：

```json
{
  "protocol": "raydium-cpmm",
  "pool_id": "11111111111111111111111111111111",
  "base_mint": "So11111111111111111111111111111111111111112",
  "quote_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
  "base_reserve_units": 1000000000,
  "quote_reserve_units": 2000000000,
  "input_amount_units": 1000000,
  "expected": "SDK compute_swap_amount_for_pool"
}
```

真实请求仍会重新读取 pool、vault、mint owner、token program、当前 slot 和
Token-2022 fee；pool/mint/quote_mint 任一绑定不匹配会在 quote 前拒绝。其它
协议的 quote/simulation 继续返回明确的未实现错误，不产生伪造数值。
