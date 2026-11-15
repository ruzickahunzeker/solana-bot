# V1.1 Optimization Backlog

This tracks implementation against the V1.1 plan. A design or an untested
source edit is not a deployed capability. All new Live pathways remain closed
until the plan's P0 acceptance criteria are met.

| Priority | Work item | Evidence / problem | Boundary and acceptance |
| --- | --- | --- | --- |
| P0 | Verify recoverability and deployment identity | `BASELINE.md`; both release binaries built and services started on 2026-11-12, but database restore is still unverified | Record hashes and tests; restore a current encrypted backup off production and retain a durable rollback binary before calling this complete. |
| P0 | Gate orphaned auto-resubmission | `copy_trading/src/order/mod.rs` previously retried without checking persistence | Require `/ready` to report ready, durable, and healthy. Missing fields or errors fail closed to `unknown`; preserve original request and idempotency key. Unit tests cover the gate. |
| P0 | Confirm live idempotency across all execution paths | `trade_service/src/persistence.rs`, `src/executor.rs`, `src/http.rs` | Verify reservation precedes external effects for SDK, PumpAPI, aggregator, and batch; test same-key replay and conflicting payloads with a non-production DB. |
| P0 | Audit and isolate API privileges | `trade_service/src/http.rs` exposes wallet export under protected `/v1` routes | Confirm actual firewall/tunnel; separate trade from wallet-management/export authorization before enlarging Live access. No port or token changes without a deployment window. |
| P0 | Normalize delivery recovery | `pumpapi_zmq_bridge/pumpapi_zmq_bridge.py` has PUB/SUB and observation spool only | Version the event contract, preserve v1 consumers, then add normalized sequence/gap detection and replay with bounded disk usage. |
| P0 | Close fast-follow audit gap | `copy_trading/src/executor/mod.rs` submitted remotely before some local audit writes | Local event/signal/order intent now precedes submit, duplicate keys skip submission, and unknown outcomes stay unknown. A controlled process-crash test remains before deployment. |
| P0 | Reject conflicting Copy Trading duplicate keys and unsafe retries | `copy_trading/src/db/mod.rs` previously returned an old order for the same key without comparing its payload; ordinary and fast Live submit retried without a durable readiness gate | Source checks business identity and embedded trade request; both retry paths now check durable readiness. A local fake server verifies pre-submit write failure blocks HTTP, remote accept/result-write failure recovers with the same key, and memory mode does not auto-retry on the ordinary path. 59 offline tests pass. Fast-path-specific injection remains; running Bot binary predates these changes. |
| P0 | Protect independent aggregator submit | `trade_service/src/http.rs` has a direct SOL-buy submit path outside `accept_trade_for_wallet` | Do not expand this Live path until it has durable reservation and replay/conflict tests or is routed through an equivalent protected workflow. |
| P0 | Separate HTTP submit from chain finality | Existing order states include `submitted` and `unknown` | Add read-only signature tracking and distinguish confirmed/finalized without automatic new orders. |
| P0 | DBC and capability evidence | `trade_service/src/protocol_capabilities.rs` reports only SPL Token while DBC docs discuss Token-2022 no-hook | Gather fixtures and simulation first; reconcile claims only after evidence. Unsupported hook pools stay rejected. |
| P1 | Add one protocol at a time | CLMM, DLMM, Whirlpool, LaunchLab, DAMM V1 are staged/observe | For each: verified pool, quote, buy/sell simulation, canary, rollback; never promote based only on an SDK builder. |
| P2 | Evaluate Router SDK backend | Existing probe pins a revision | Keep production SDK unchanged; compare one allowlisted pool in read-only/simulation mode first. |

## Current change: orphan recovery gate

- Scope: Copy Trading readiness DTO, existing OrderReconciler, and its
  reconcilable-order query. No Trade Service API or database migration.
- Behavior: orphaned orders are not automatically resubmitted unless Trade
  Service reports `ready=true`, `trade_persistence_mode=durable`, and
  `trade_database_healthy=true`. Otherwise they are marked `unknown` with a
  `recovery_blocked` attempt for manual inspection. Orders that already have a
  request ID remain eligible for read-only status reconciliation, including
  `unknown` orders. Unchanged `unknown` responses get a five-minute cooldown
  and a refreshed check timestamp so old records cannot monopolize the queue.
- Validation: run `cargo fmt --check` and the targeted readiness tests, then
  the Copy Trading suite in a build environment with enough disk space. Test a
  controlled durable restart before deploying.
- Rollback: restore the previous Copy Trading binary/source version; do not
  reset or delete order records. Any `unknown` order needs manual status
  inspection by its original idempotency key before further action.

## Fast-follow intent ordering

- Scope: `copy_trading/src/executor/mod.rs` only. No Trade Service API,
  migration, wallet, or strategy setting changes.
- Before an outbound call, persist the source event, signal, original request,
  idempotency key, and order. Record each submit attempt before that attempt.
  A duplicate order reservation returns without a second outbound call.
- A timeout or transport failure does not prove the trade failed. After all
  attempts, retain `unknown` and the original request for manual review;
  successful responses persist their request ID and signature as before.
- Acceptance still pending: compile and run Copy Trading tests, inject local
  write failure and remote timeout against a non-production Trade Service,
  then verify restart behavior under both durable and memory modes. The
  additional local writes may increase fast-follow latency; measure before
  production rollout.
- Rollback: revert only this executor change, not the database. Do not replay
  or delete any intent already stored by this version.
