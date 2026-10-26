# Trade Persistence Acceptance

Date: 2026-07-14

## Scope

Only trade-record and idempotency persistence was changed. Wallet loading, private-key encryption, SDK/PumpAPI transaction construction, blockhash, RPC, MEV, and confirmation behavior remain outside this change.

## Backup

- Valid archive: `../backups/http_before_trade_persistence_20260714T045709Z_v2.tar.zst`
- SHA-256: `e20f07d171c8aad2198077217e635f38ccfae580a983507783a9a6f1efc12545`
- Archive integrity: passed `zstd -t`
- Entries: 455
- Rust `target` and Python `.pyc` files: excluded
- The first truncated gzip attempt is retained with the suffix `.INVALID_TRUNCATED` and must not be used for restore.

## Automated acceptance

- `cargo fmt --check`: passed
- `cargo check --offline`: passed
- `cargo test --bins --offline`: 12 passed, 0 failed
- Migration execution: exercised by every SQLite persistence test through the same embedded SQL file

Covered cases:

- memory mode does not open or validate the database
- equal durable request returns the original trade
- same key with different normalized trade conflicts
- 100 concurrent equal reservations produce 1 creation and 99 duplicates
- closed durable database rejects reserve and update
- interrupted non-terminal trade becomes `unknown` without replay
- rejected snapshots do not consume an executable idempotency key
- shadow writer preserves insert/update order
- status serialization round-trip

## Deployment state

The live `.env` has no `TRADE_PERSISTENCE_MODE`, so the effective mode remains `memory`. No service restart, live trade, wallet change, or durable-mode activation was performed by this change.

Before enabling durable execution, use this order:

1. Deploy in `memory` mode and run the existing simulated trade smoke test.
2. Switch to `shadow`, observe database health, and reconcile records.
3. Switch to `durable` with `ENABLE_SIMULATE=true` and repeat buy/sell/idempotency/restart tests.
4. Restore the previous simulate/live setting only after explicit approval.
