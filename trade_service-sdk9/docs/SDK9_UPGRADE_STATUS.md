# SDK 9 Upgrade Status

Date: 2026-10-26

## Scope

This directory is an isolated SDK 9.0.1 candidate. The production
`trade_service` directory and systemd services were not replaced or restarted.
No live transaction was submitted.

## Validation

- SDK: `sol-trade-sdk 9.0.1`
- `cargo check --release --locked`: passed
- `cargo test --release --locked`: 43 passed, 0 failed
- `cargo build --release --locked`: passed
- Release binary: `target/release/pump-trade-service`
- SHA-256: `eb119ca7019b6ea8dc8ca9925e8d8fd856706ddc39b8d508322011afe8aeff5d`

## Compatibility work

The upstream SDK does not include the project's Meteora DBC extension. The
candidate therefore carries the existing DBC parameter, instruction builder,
pool discovery module, factory executor, and protocol registration. Existing
Trade API code, persistence, CPMM, DAMM, StonkFun, and WSOL conversion tests
compile against the candidate.

PumpFun is pinned to Legacy in both default and MEV lanes to preserve the
current service behavior. V2/V3 are not enabled through the Trade API yet.

## Canary lock

Set `CANARY_SIMULATION_ONLY=true` when running this candidate. Startup rejects
non-loopback binding, non-memory trade persistence, and live WSOL conversion.
The trade endpoint accepts only explicit `simulate:true`, including batch
trades; `simulate:false` and omitted `simulate` are rejected before reservation.
The lock also rejects SOL transfers, WSOL submission, wallet create/update/
delete/export, and vanity job changes. It cannot be overridden by a request.
Simulated success is reported as `simulated` and simulated failure as `failed`;
neither response exposes the locally generated simulation signature.

`ENABLE_SIMULATE=true` alone is **not** a safety lock: callers can override it
with `simulate:false`. `TRADE_SUBMISSION_ENABLED` is not a recognized setting.

Use an isolated wallet database copy, a separate loopback port, and a distinct
Bearer token. Do not expose the canary to the Telegram bot. SDK logs can contain
RPC URLs with API keys; keep logs local and redact them before sharing.

Example (run in the foreground from this directory after preparing the wallet
database copy and an environment file with a distinct token):

```bash
CANARY_SIMULATION_ONLY=true \
TRADE_HTTP_ADDR=127.0.0.1:28787 \
TRADE_PERSISTENCE_MODE=memory \
CONVERSION_QUOTES_ENABLED=false \
CONVERSION_LIVE_ENABLED=false \
WALLET_DATABASE_URL=sqlite:///tmp/sol-trade-sdk9-canary/wallets.sqlite \
WALLET_DATABASE_PASSPHRASE_FILE=/tmp/sol-trade-sdk9-canary/.wallets.passphrase \
./target/release/pump-trade-service
```

The isolated loopback canary previously passed `/health` and authenticated
`/ready`. Requests with `simulate:false` or omitted `simulate`, wallet export,
and WSOL submission returned HTTP 403. One explicit `simulate:true` request
for a historical PumpFun mint reached RPC simulation but failed with
`InstructionError(3, Custom(6005))`; this is **not** a successful protocol
acceptance test. A read-only signature-status query returned null. The canary
was stopped afterward. No transaction was broadcast.

## Remaining gate before deployment

1. Run `simulate=true` for active PumpFun Legacy, StonkFun, CPMM, DAMM v2, and
   DBC using suitable current pools and wallet balances. Record unavailable
   fixtures as unverified, never as passed.
2. Validate Legacy PumpFun behavior against real RPC; separately design V2/V3
   selection before enabling those versions.
3. Verify MEV provider initialization and fee strategy selection.
4. Start the candidate on a separate port with live submission disabled.
5. Only after the above passes, prepare a systemd canary and rollback to the
   current 6.0.2 binary.

## Backup

The pre-upgrade source/data backup is stored outside the project at:

`/root/http/backups/trade_service_pre_sdk9_20261023_075128`

Secrets and runtime databases are not part of Git changes.
