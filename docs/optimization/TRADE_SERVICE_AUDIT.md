# Trade Service Submission Audit

Date: 2026-11-11. This is a source audit, not a runtime or chain acceptance
record. Services were not started and no transaction was submitted.

## Existing durable path

`POST /v1/trades` calls `accept_trade_for_wallet` in
`trade_service/src/http.rs`. The batch route calls the same function per
wallet. In durable mode, that function reserves the trade through
`TradePersistence::reserve` before it spawns `executor::execute_trade`.
Duplicate keys return the existing record; conflicting requests are rejected.
The executor's SDK and PumpAPI branches therefore share this reservation
entry point for these two routes. This needs a restart/fault-injection test,
not just source review, before declaring cross-process safety.

## Independent submission routes

`POST /v1/quotes/sol-buy/:quote_id/submit` obtains a bound quote and checks
`aggregator_quotes.get_submission`, then calls `aggregator_quote::submit`
directly. Its successful response is recorded afterward in the quote cache.
Source review did not find a call to `TradePersistence::reserve` on this route.
The quote cache is not a substitute for the durable trade reservation after a
process restart or crash between broadcast and cache update.

WSOL conversion submission and wallet transfer use their own persistence
paths and must be reviewed under their own operation semantics. Do not infer
that the trade reservation automatically covers them.

## Action and release gate

1. Preserve the current Public Trade API and production execution backend.
2. Do not expand aggregator Live usage until its quote-bound submission has a
   durable intent/reservation and conflict semantics, or an equivalent
   independently tested recovery boundary.
3. Test timeout, process crash immediately after broadcast, duplicate same
   key/same request, and same key/different request in a non-production DB.
4. Verify the running `/ready` reports `durable` and healthy, and confirm all
   routes' external effects occur only after the matching durable reservation.
5. Do not automatically reissue an uncertain submission under a new key.
