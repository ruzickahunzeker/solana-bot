# Telegram SOL / WSOL read-only rollout (10.9.0.7)

Scope: add quote and non-broadcast simulation to the running Bot at
`10.9.0.7:/root/solana/bot_trading`. Do not add a submit button, enable live
conversion, send a transaction, migrate wallets, or modify `/v1/trades`.
Treat the Bot server's source as authoritative. The local draft in
`/root/http/sol-bot/copy_trading` is a reference only: it has not passed a
complete compile because the trade server is nearly out of disk space.

## 1. Establish the baseline

On 10.9.0.7, record `git status --short`, the current commit, the Bot service
unit and `TRADE_SERVICE_URL` (do not print the token). Read the wallet detail
keyboard, callback handler, Trade API client, wallet ID mapping and existing
tests. Preserve all local changes. Confirm that the Bot reaches the trade
server through its loopback SSH tunnel at `http://127.0.0.1:18788`; the
conversion endpoints are loopback-only on the trade server and cannot be
called directly across the LAN. Test `/health` first.

Do not copy the local `copy_trading` files wholesale. Port only the conversion
UI/client additions into the Bot server's current structure. Keep the Bot's
current database, `MASTER_KEY_HEX`, Telegram token and running service intact.

## 2. Trade API contract

All requests use the existing `TRADE_SERVICE_TOKEN` as a Bearer token and the
tunnel base URL. The trade server must first deploy a version containing the
conversion endpoints and set `CONVERSION_QUOTES_ENABLED=true` while keeping
`CONVERSION_LIVE_ENABLED=false`. A healthy `/health` alone does not prove that
quotes are enabled. Until the quote endpoint succeeds, hide or disable the
conversion entry and report that the trade server is not ready.

- `POST /internal/conversions/wsol/quotes`:
  `{"wallet_id":"<Trade API ID>","owner_type":"copy_trading","owner_id":"<app_users.id>","direction":"wrap","amount_lamports":1000000,"all":false}`.
  For a fixed unwrap use `direction:"unwrap"` and an amount. For 100% unwrap,
  omit `amount_lamports` and use `"all":true`. Never set both.
- The response contains `quote.quote_id`, `direction`, `amount_lamports`,
  `source_balance_lamports`, `estimated_network_fee_lamports`,
  `temporary_rent_lamports`, `rent_refund_lamports`, `expires_at_ms`, and
  `executable`. A quote lasts about 60 seconds. Rent refund is separate from
  conversion proceeds. Do not interpret `executable` as permission to submit
  from this read-only Bot phase.
- `POST /internal/conversions/wsol/simulate`:
  `{"quote_id":"<fresh quote ID>","wallet_id":"<same Trade API ID>","owner_type":"copy_trading","owner_id":"<same app_users.id>","simulate":true,"idempotency_key":"<new random key>"}`.
  Expect `status:"simulated"`, the same `quote_id`, fee and optional CU count.
  This endpoint signs for RPC simulation but does not broadcast or reserve a
  live submission. Never call `/submit` from the Bot in this phase.

For system wallets already exposed by the Bot, use the *registered* owner
`system/global` only if the existing Bot authorization policy allows that
wallet. Do not guess the owner from the Telegram user ID. Resolve the local
wallet's `external_wallet_id` against `/v1/wallets/manage`, and verify its
pubkey, enabled state and owner before requesting a quote. Never send a local
SQLite wallet ID as the Trade API wallet ID.

## 3. Bot UI and state

Add `SOL / WSOL` to eligible wallet details. Use edit-message navigation:

1. Selection panel: `包装 0.001 SOL`, `包装 0.01 SOL`, `解包 0.001 WSOL`,
   `全部解包`, `返回钱包`.
2. After a quote, show direction, source wallet, amount and units, source
   balance, estimated fee, temporary rent and separate ATA rent refund.
   Buttons: `模拟本笔`, `返回兑换`. State clearly that this is a preview and cannot
   submit a live transaction.
3. `模拟本笔` uses only the quote for that Telegram user, local wallet and panel
   message. Reject an expired/missing quote or changed wallet mapping. Show
   success/failure without exposing RPC URLs or credentials; return to the
   selection panel for a new quote. No transaction confirmation or submit
   callback is present.

Store the short-lived quote binding in memory or another existing session
mechanism. Do not put wallet secrets or the Bearer token in callback data.
Validate fixed options server-side; a forged callback must not select an
arbitrary amount or wallet. Keep amount math in lamports, not floating point.
Handle unavailable/disabled quote endpoints as a normal UI state, not a
silent callback failure. Ensure the HTTP timeout accommodates RPC simulation.

## 4. Verification and rollout

Run the Bot project's formatter and focused tests, then its full test suite
from `/root/solana/bot_trading`. Add tests for wallet ID/owner mapping,
callback amount validation, stale quote rejection and response parsing.
Review the diff for an accidental `/submit` call or changes to existing trade
and transfer paths. Preserve the dirty worktree; commit only this logical
change if committing is requested.

With the trade server's quote flag enabled and live flag still off, use a
funded test wallet to verify wrap, partial unwrap and full unwrap quotes and
simulations. Confirm the same quote ID returns from simulation. Query the
trade server's separate conversion database: simulations must leave
`conversion_submissions` at zero. Verify the disabled live gate by inspecting
the deployed configuration and code, plus an isolated test environment; do
not exercise `/submit` against a funded production wallet merely to test
rejection. Then deploy and restart only the Bot service, check logs without
printing tokens/RPC URLs, and click through one wallet in Telegram.

Do not enable `CONVERSION_LIVE_ENABLED`, add a submit button, or run a small
live transaction as part of this rollout. Those need separate explicit
authorization and acceptance after the read-only UI is verified.

## Current trade-server evidence

On 2026-10-08 the trade server deployed the release binary containing the
conversion routes and restarted `trade-service-db.service`. The running binary
hash matched the newly built release. `/health` returned `{"ok":true}`.
The trade-server `.env` has `CONVERSION_QUOTES_ENABLED=true` and
`CONVERSION_LIVE_ENABLED=false`; the process has no same-name environment
overrides. Existing Trade API, wallets and credentials were retained.

Production test wallet ID: `2TE` (`system/global`). Non-broadcast RPC
simulation succeeded for 0.001 SOL wrap (351 CU), partial WSOL unwrap (609
CU), and full unwrap (118 CU). Each returned its own original quote ID. The
full-unwrap quote listed 1,488,440 lamports of ATA rent refund separately.
The production conversion database held three quotes and zero submissions.

An isolated test instance previously returned HTTP 400 for `/submit` while
live conversion was disabled. A production `/submit` rejection test was **not
run**: the execution environment blocked it because a configuration or gate
failure could cause a real broadcast. Static inspection confirmed the live
flag is checked before signing or persistence. Do not describe production
submit rejection as empirically verified. The Bot tunnel and Bot UI still need
their own verification on 10.9.0.7.
