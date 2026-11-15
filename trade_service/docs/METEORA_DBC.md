# Meteora DBC trading

`trade_service` supports active Meteora Dynamic Bonding Curve pools through the
existing Trade API. No separate transaction API is introduced.

## Protocol identifiers

- Trade API pool: `meteora-dbc`
- Program: `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`
- Pool authority: `FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM`
- Instruction: `swap2`, ExactIn mode

The official Meteora Rust quote implementation is pinned in
`vendor/sol-trade-sdk/Cargo.toml`. See `vendor/sol-trade-sdk/METEORA_DBC_NOTICE.md`
for the source revision.

## Resolution flow

`POST /v1/pools/resolve` queries Pump and DBC concurrently. DBC lookup is
restricted by the current VirtualPool account size, discriminator, and base mint
offset. A successful result includes the real pool address and quote mint:

```json
{
  "mint": "3R7orBRzXtELryYoPD6aveozcnuFAzRUzbYuGSMq8HvW",
  "pool": "meteora-dbc",
  "pool_id": "2a4h2QxQNYuSAk918uUTbNS3D4N2Fb63yxJy2aQqfuRv",
  "quote_mint": "So11111111111111111111111111111111111111112"
}
```

Pool loading rejects migrated pools. A migrated token must be resolved through
its DAMM destination instead of submitting another DBC swap.

## Execution flow

For every buy or sell, the SDK:

1. Fetches the VirtualPool, PoolConfig, and token mint owners.
2. Rejects a migrated pool or a pool whose requested mint/quote does not match.
3. Uses the official exact-in quote math with the current slot/time.
4. Applies `slippage_bps` to produce `minimum_amount_out`.
5. Creates or wraps token accounts and builds `swap2`.
6. Includes the instructions sysvar required by rate-limited configurations.
7. Uses the existing RPC/MEV submission and confirmation path.

## Simulated request

```json
{
  "wallet_id": "main",
  "side": "buy",
  "pool": "meteora-dbc",
  "mint": "3R7orBRzXtELryYoPD6aveozcnuFAzRUzbYuGSMq8HvW",
  "pool_id": "2a4h2QxQNYuSAk918uUTbNS3D4N2Fb63yxJy2aQqfuRv",
  "quote_mint": "So11111111111111111111111111111111111111112",
  "amount_sol": 0.001,
  "slippage_bps": 1500,
  "execution_method": "sdk",
  "use_mev": false,
  "simulate": true,
  "idempotency_key": "dbc-sim-3r7-001"
}
```

## Initial scope

- Supported: active VirtualPool accounts, SPL Token, Token-2022 without transfer
  hooks, WSOL/USDC quote, ExactIn buy/sell.
- Not yet supported: TransferHookPool, referral accounts, ExactOut, PartialFill.
- PumpAPI must emit an explicit `meteora-dbc`, `dynamic-bonding-curve`, or `dbc`
  label for the bridge fast path. Generic `meteora` remains DAMM v2.

## Verification commands

Run these commands from `/root/http/sol-bot/trade_service` after reviewing the diff:

```bash
cargo fmt --all -- --check
cargo check
cargo test
```

Then rebuild the service and call the pool resolver before sending the simulated
request. Do not use `simulate=false` until both buy and sell simulations pass.
