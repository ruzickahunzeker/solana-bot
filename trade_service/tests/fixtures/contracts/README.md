# Contract Fixtures

These fixtures contain synthetic identities, not wallets or chain evidence.
They freeze the existing DTO wire shape before modularization and SDK upgrades.

Run from `trade_service/`:

```bash
cargo test --locked --offline contract_tests
```

The tests are compiled through `src/contract_tests.rs`, registered under
`cfg(test)` in `src/main.rs`. They do not load `.env`, initialize runtime
wallets, access RPC, create production databases, or submit transactions.

Passing these tests does not verify HTTP routing/status/auth behavior,
full configuration-based validation, protocol support, or chain simulation.
Review intentional contract changes before changing expected JSON.

`sol_buy_v1.json` freezes synthetic bound quote and all 32 preflight fields.
Its original fingerprint was captured with the old API types before the internal
DTO cutover. Values above 2^53 and u64::MAX are intentional serialization cases,
not valid trade sizes or chain evidence. Do not recompute the expected hash merely
to accept a journal snapshot change. Temporary-journal tests also verify that
old-format records resume without calling a quote builder, signer, or broadcaster.
