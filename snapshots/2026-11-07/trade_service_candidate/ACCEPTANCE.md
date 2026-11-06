# Candidate acceptance record

Date: 2026-11-03
Release: SDK 11.0.1 candidate
SDK commit: c717741d9956f248fd4dd74b4523576bfb74d367
Binary SHA-256: 9680d63f0ddb231feb06c57683d92951fe3f728c1891a7d6d0aaecc4b5508ca2

## Isolation

- Runtime data: `/root/solana/trade_service/candidate-data/`
- Runtime env: `candidate/.env`
- Wallet passphrase: `secrets/.wallets.passphrase` (mode 600)
- Listener: `127.0.0.1:18787`
- `TRADE_SERVICE_SIMULATE_ONLY=true`
- Existing bot, port 18788 tunnel, and source databases were not changed.

## Checks

- `cargo check --locked`: passed with Cargo/Rust 1.99
- `cargo test --locked`: 49 passed
- `cargo build --release --locked`: passed
- `/health`: passed
- `/ready`: passed; RPC and blockhash available
- Runtime wallets: 8 enabled wallets loaded; one of nine database records is disabled
- `/v1/capabilities`: supported SDK protocols report `simulate`; DBC and staged protocols report `observe`
- `simulate=false` trade: rejected with HTTP 400
- Meteora DBC trade: rejected with HTTP 400 because no DBC backend is available
- `simulate=true` trade: no signature; simulation path returned a controlled failure for the test mint

This candidate is not selected as `current` and is not a live trading endpoint.

## Adapter registry phase

- `BackendCatalog::sdk11()` is now initialized at service startup.
- The executor admission path checks the registry before SDK execution.
- Supported SDK 11 protocols are explicitly registered.
- Meteora DBC is intentionally absent and is rejected before SDK dispatch.
- `cargo check --locked`: passed
- `cargo test --locked`: 50 passed
- Release binary SHA-256: `ea18531cfa9c3ff31a2aa36a8a5c859c3e794d2a0af4a6e4ec91595d4520fd83`

## SDK adapter phase

- Added `backend::sdk::SdkBackend` as the SDK-specific boundary.
- `/v1/pools/resolve` now calls the adapter instead of the SDK client directly.
- `/v1/cache/warmup` now calls the adapter instead of the SDK client directly.
- Adapter errors are returned without exposing SDK parameter types.
- `cargo test --locked`: 50 passed
- Candidate runtime health and readiness passed after restart.
- DBC warmup is rejected through the adapter boundary with HTTP 502; no transaction is built or sent.
- Release binary SHA-256: `f910f6c2e53b60b8b392ea420f47fc5c6432773bfb936a36310b45eb7c1feeb7`

Quote and live submit remain intentionally outside this phase because SDK 11 does not expose a single protocol-neutral quote API; the next phase will add protocol-specific simulation results without fabricating quote values.

## Current candidate verification (2026-11-04 08:42 +0800)

This section supersedes earlier toolchain and binary notes above for the current source tree.

- Toolchain: Cargo 1.99.0 / rustc 1.99.0 from `/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu`.
- `cargo fmt --all -- --check`: passed.
- `cargo check --locked`: passed.
- `cargo test --locked`: passed, 63 tests.
- `cargo build --release --locked`: passed.
- Release binary SHA-256: `9160ec27a6234107a049879e6731382c9ff588d90db9f1337265ea6b2ac2e04a`.
- Source manifest SHA-256 (`src`, `migrations`, `docs`, `Cargo.toml`, `Cargo.lock`): `47238832272cf1f777ca65f967429cd6bf89749097e2272a947cc6c983a301ba`.
- Candidate remains `TRADE_SERVICE_SIMULATE_ONLY=true`; no live signing or broadcast was enabled.
- Gate 1 aggregator preflight now returns `instruction_audit_passed=false` and `executable=false`; structured simulation reports contain no transaction bytes.
- `migrations/003_aggregator_quotes.sql` persists quote/provider metadata and submission reservations in SQLite WAL; restart recovery marks `reserved/submitting` as `unknown`.
- `/v1/aggregators/capabilities` reports Jupiter as read-only quote baseline and DFlow as disabled read-only. DFlow has no build/preflight/submit path.
- Existing `current`, bot source/data, production databases and the 18788 tunnel were not changed.

Remaining gates: Jupiter HTTP integration tests, durable provider/message fingerprint updates, failure-to-unknown recovery, Jupiter read-only Shadow metrics, and selective bot migration.

## SimulationReport phase

- Added `SimulationReport` to trade responses and records.
- Added backward-compatible `simulation_json` column migration to the isolated candidate database.
- SDK and PumpAPI simulation paths produce protocol/source/ok metadata; SDK failures during parameter preparation are also represented as `ok=false` reports.
- Candidate simulation response verified with no broadcast signature:
  `{"ok":false,"protocol":"pump","source":"sdk","units_consumed":null,"logs":["Default: Bonding curve not found"]}`
- Candidate `/ready` passed after migration with 8 enabled wallets and healthy durable persistence.
- Release binary SHA-256: `069c5150d66c73e15ef22123c54f9da82b4008720bb21a45745e60b70f8da587`

## Stable backend contract phase

- `AppState` now owns `Arc<dyn ExecutionBackend>`.
- Resolve and warmup HTTP handlers call the backend trait; they no longer call `SdkTradeClient` directly.
- `SdkBackend` implements the trait with explicit `quote_exact_in`, `simulate`, and `submit` unsupported gates until their unified paths are complete.
- Backend capabilities report SDK 11 simulation support and live submission disabled.
- `cargo check --locked`: passed
- `cargo test --locked`: 50 passed
- Candidate `/ready`: passed with 8 enabled wallets and healthy durable persistence.
- Candidate capabilities: first batch `simulate`, DBC and staged protocols `observe`.
- DBC trade: HTTP 400 registry rejection.
- `simulate=false`: HTTP 400 simulation-only rejection.
- Release binary SHA-256: `7af7d68c9a48fdb2e959f24f044a1a7724799b4285039bca28db20b8f820287f`

## Route and quote lifecycle phase

- Added SDK-neutral `VerifiedRoute` validation for protocol/backend identity, mint and pool addresses, token program, side support, validated slot, and expiry.
- Added quote and trade-plan validation for wallet binding, route binding, positive units, minimum output, slippage, and expiry.
- Registered `/v1/pools/resolve-candidates`, `/v1/routes/quote`, and `/v1/routes/simulate`.
- At the initial route-contract checkpoint SDK 11 quote/simulation returned explicit `501`; the later increment below adds the first Raydium CPMM adapter while keeping unsupported protocols explicit.
- `cargo check --locked`: passed; `cargo test --locked`: 53 passed; `cargo build --release --locked`: passed.
- Release binary SHA-256: `5b503b3b0031c685f2ffd1bf3c171337103f8adaecce8b63a1bb7e5f0a0e24b3`

## Runtime verification after candidate restart

- Candidate route API source was corrected and release rebuilt (`17e0160d10e95761b758c965b62fc3df7b2f4019e6aaca2f18c215f1f11d32f3`).
- Release route registry tests: 3 passed.
- `/health`: HTTP 200.
- `/ready`: HTTP 200; 8 enabled wallets, blockhash present, durable persistence healthy.
- `/v1/capabilities`: first batch reports `simulate`; DBC and staged protocols report `observe`.
- `/v1/pools/resolve-candidates` rejects an invalid mint before backend execution (HTTP 400, no signature).
- After reclaiming only candidate build caches, `/ready` reports `trade_database_healthy=true` again; no business database or wallet data was removed.
- Existing `current`, bot, and port `18788` tunnel remain unchanged.

## Route verification and first protocol adapter increment

Update: 2026-11-04

- `ResolvedRoute` now records `chain_verified`, pool/mint owner checks, both token programs, and validated slot; quote/simulate admission rejects routes without this metadata.
- SDK 11 route resolution reloads Pump/PumpSwap pool and mint accounts, checks expected program owner and mint pair, and records the RPC slot and Token-2022 capability.
- Raydium CPMM exact-in quote uses the SDK calculator with current pool reserves, protocol/creator/fund fees, and Token-2022 transfer fees. Buy simulation uses the SDK `simulate=true` path and returns `signature: null`; sell simulation remains explicitly gated until amount binding is complete.
- Added synthetic adapter fixture notes in `docs/ROUTE_QUOTE_FIXTURES.md`; it is not a mainnet success sample.
- `cargo fmt --all -- --check`: passed. `cargo check --locked` could not run in this environment because the installed Cargo 1.75 cannot parse lockfile version 4; the existing Cargo.lock was restored unchanged. Network access also prevented dependency resolution for an offline compatibility check.

## Server-side route admission increment

Update: 2026-11-04

- Added an in-process short-lived `RouteStore`.
- `/v1/pools/resolve-candidates` stores the complete verified route snapshot before returning it.
- `/v1/routes/quote` and `/v1/routes/simulate` require an exact match against the server snapshot; client-supplied verification flags or modified pool/mint metadata are rejected.
- Added a regression test for client-modified route snapshots.
- `cargo fmt --all -- --check`: passed.
- `cargo check --locked` and `cargo test --locked` remain blocked by the installed Cargo 1.75 parser and lockfile v4; `Cargo.lock` was not modified.

## Quote admission and migration increment

Update: 2026-11-04

- `QuoteRequest` now accepts `slippage_bps` (default `0`) and rejects values above 10000.
- Quote minimum output is calculated from expected output and the requested slippage.
- Quote validation now requires a non-empty quote ID, a valid creation/expiry interval, and bounded slippage.
- Added `migrations/002_trade_simulation_report.sql` and schema version 2 bookkeeping; the startup PRAGMA fallback was removed.
- `cargo fmt --all -- --check`: passed.
- Full locked build/test remains blocked by the installed Cargo 1.75 parser and lockfile v4.
- `SdkBackend` now resolves the requested `wallet_id` through `WalletRegistry` for quote and simulation; route discovery continues to use the configured default RPC client.


## Multi-aggregator plan review

Update: 2026-11-04

- Compare API validates Solana mint addresses, positive input amount, and slippage `1..=3000`; each provider quote is bounded by a 3-second timeout.
- Durable aggregator submissions now reuse the bound quote route fingerprint and message hash metadata, with a single idempotent upsert.
- Bot `TradeServiceClient` exposes read-only `/v1/aggregators/capabilities` diagnostics; it does not select providers or submit aggregator transactions.
- `cargo fmt --all`, `cargo check --locked`, and `cargo test --locked`: passed (`65` tests).
- Bot `cargo fmt --all` and `cargo check --locked`: passed.
- Candidate release binary SHA-256: `6f8960b4557b493ee667c8da1cb160f5a08fafd8533d270aad0bd2bc59f8f208`
- Candidate source manifest SHA-256: `9baef8575fe67f478806dde082b559f170cd130cf135dbfb025198c910bb57c8`
- DFlow remains disabled/read-only until a verified response contract and endpoint are available; no live signing, build, preflight, submit, broadcast, `current` switch, production DB, wallet, or 18788 tunnel changes were made.

## Scope amendment: Jupiter first

Update: 2026-11-04

- Jupiter is the only implemented aggregator baseline for the current rollout.
- DFlow is explicitly deferred. Its disabled/read-only placeholder remains only to prove that the provider registry can represent an unavailable provider; no DFlow endpoint, adapter, Shadow sample, or performance claim is part of this plan.
- The provider-neutral registry, capabilities contract, and quote envelope remain so a future aggregator can be added without changing bot or persistence contracts.

## Jupiter submission status query

Update: 2026-11-04

- Added `GET /v1/quotes/sol-buy/:quote_id/submissions/:idempotency_key?wallet_id=...` for quote-bound submission recovery.
- The endpoint reports `reserved`, `submitting`, `unknown`, or the terminal response and rejects wallet/quote mismatches.
- Candidate was rebuilt with Cargo/Rust 1.99 after the endpoint change.
- Release binary SHA-256: `c6784675af1ec2faa97c201a5bafc0ab921b7783b9607f7f154e0a4c1e6174a8`.
- Source manifest SHA-256: `b3f9f40d8127d987d5166942077e518c23756904fad2636a5ffed201d4895fd0`.
- `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`65` tests).
- Bot `cargo fmt --all -- --check` and `cargo check --locked`: passed.

## Submission recovery after quote expiry

Update: 2026-11-04

- Submission status lookup now checks the durable submission's quote binding directly, so `unknown` recovery remains available after the short quote TTL expires.
- Added a durable regression test for quote binding before and after store reopen.
- Candidate `cargo fmt --all -- --check` and `cargo check --locked`: passed; the full test suite remains at 65 passing tests.
- Release binary SHA-256: `d6ab9311bda2795caf4fa94cc92a6a2ac2edfa3a9ed074a724261d2ce2edd5f3`.
- Source manifest SHA-256: `975045bac0fb927cf744b23e35c5a82ebfe3543272850ed9f8238da8761ac86a`.
- Runtime HTTP smoke test was attempted with copied candidate databases and temporary port `18887`; the sandbox rejected local `bind` with `Operation not permitted`, so no service process or production port was changed.

## Compare request validation tests

Update: 2026-11-04

- Compare request validation is now a tested helper covering valid Solana mint, positive amount, `1..=3000` slippage, and non-empty provider filters.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`68` tests).
- Candidate release binary SHA-256: `3e4ba69eeafe4de26b40dec7d459fbdd240d551fc3f44a4d115b2c848afac7ec`.
- Source manifest SHA-256: `72b40ac77187a2c1ebb090a096f4706b2f12f68865e3b7f6c427df9993d9ed1b`.
- Bot preflight now displays read-only aggregator capability diagnostics; capability failures do not enable provider selection or block simulate-only readiness.
- Bot `cargo fmt --all -- --check` and `cargo check --locked`: passed.
- Bot `cargo test --locked` reached 68 passing tests; 8 existing HTTP mock tests fail before assertions because the sandbox rejects their `TcpListener::bind(127.0.0.1:0)` with `Operation not permitted`. This is the same environment restriction that prevented the isolated candidate HTTP smoke test; no application assertion failed.

## Jupiter default compare baseline

Update: 2026-11-04

- `/v1/aggregators/quotes/compare` now queries Jupiter only when `allowed_aggregators` is omitted.
- Other registered providers can be requested explicitly and remain subject to capability and timeout handling.
- Added a regression test for the Jupiter-only default; candidate test total is now 69, all passing.
- Candidate release binary SHA-256: `1807c85fac7c8a9594f97860387755a1e7dd8f0463edad8bd2c757806a3f4218`.
- Source manifest SHA-256: `3c1bad08816a3544c44fde300d8614b6181b2ab99c9714614e75c3f82e0207a6`.

## Jupiter Shadow metrics

Update: 2026-11-04

- Added `aggregator_shadow_metrics` to migration 003.
- Compare observations persist provider, latency, status, amount/slippage, expected/minimum output, route fingerprint, price impact, and a redacted error class; raw provider responses, URLs, and transaction bytes are excluded.
- The default compare path records Jupiter only. Explicitly selected unavailable providers may still produce an unavailable metric.
- Durable migration tests verify the metrics table exists after reopen.
- Candidate test total remains 69, all passing.
- Candidate release binary SHA-256: `1de97aedea79bf2226aaebde0f3fc4640f09d7095e8fb45c7c2f4c24ed19b553`.
- Source manifest SHA-256: `fe35fc1a1b4318613e40b92ad32eb0738883191c5ee05deb7bff7a5a889e1ac6`.

## Shadow metrics read API

Update: 2026-11-04

- Added read-only `GET /v1/aggregators/shadow/metrics` with optional `wallet_id`, `provider`, and bounded `limit` filters.
- The response is limited to persisted aggregate fields and never returns raw provider payloads, URLs, private keys, or transaction bytes.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`69` tests).
- Candidate release binary SHA-256: `d86f0f2a0fdfea8c92e75efd26f3a4c6a98eec6d095a077ca36e9b59073b11ee`.
- Source manifest SHA-256: `a319f366cf6b54f46d5d5a30297bc97f5e0d6dc8902463145e2af05868a00c98`.

## Shadow metric durable readback

Update: 2026-11-04

- Durable tests now insert and query a Jupiter Shadow metric, verifying provider, output mint, latency, and filter behavior through SQLite.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`69` tests).
- Candidate release binary SHA-256: `3c79618f3e4f62aa616232f4e88594f43ba012521f3433f5e2d5ab0356b215bd`.
- Source manifest SHA-256: `15e4145d99830ede0eb8d03c4ad686bee5a56447a81a117163b66e6320a249b3`.

## Jupiter quote lifecycle status

Update: 2026-11-04

- Successful Jupiter preflight now advances the durable quote state to `preflighted`; failed local simulation records `failed`.
- The transition remains behind the Gate 1 simulate-only boundary and does not make the quote executable.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`69` tests).
- Candidate release binary SHA-256: `820e550c578c58ee0315fca9cf6deac05962619a4dfb8cc0f26edac2838de0b8`.
- Source manifest SHA-256: `18da938d43a90a23423d0ee558d27a3e7bc6344f8f959a21d8095087042aed4a`.

## Monotonic Jupiter quote states

Update: 2026-11-04

- Durable quote status transitions are now monotonic: `quoted → preflighted|failed|expired`, with `preflighted → failed` allowed for a later simulation failure.
- Invalid backward transitions are rejected and covered by the durable quote test.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`69` tests).
- Candidate release binary SHA-256: `9a8a77361ba21810db856b4af5f92bbc827812f43ab2b6f5a4a5c3c190418e15`.
- Source manifest SHA-256: `01f098baaf61bce1d5c5fc86315bf03d0bae270eea1d6aa7dce703715c4c7873`.

## Preflight state write consistency

Update: 2026-11-04

- Preflight now fails with `503` if the durable quote state transition cannot be recorded, instead of returning a successful preflight with stale persistence state.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`69` tests).
- Candidate release binary SHA-256: `01900fb670a60b8a57f66f03b91fb8865fa3f0b806612129a9c75972882e2414`.
- Source manifest SHA-256: `d55f7b3931890125c248a5a527d2d32f94fe24cc0e2342c520b318d50e13e8b3`.

## Preflight persistence failure handling

Update: 2026-11-04

- Preflight now treats quote state persistence failure as an HTTP `503`, preserving the invariant that a successful response has a durable state transition.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`69` tests).
- Candidate release binary SHA-256: `832e625c0875f1bfab97f8327ed109dac706e6322c8dbaff0bac55bdeed206df`.
- Source manifest SHA-256: `6a9a6669c047099ace56ab9ed43f35362d3dccda9ed797091487bc1cf5eabff6`.

## Expired quote audit retention

Update: 2026-11-04

- Expired durable quotes are retained with `status='expired'` for audit and metrics instead of being deleted; use of the quote remains rejected.
- Added a restart based regression test for expiration retention.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`70` tests).
- Candidate release binary SHA-256: `c7fcd3b7f9def213b67a339e8e6016db22a6f3d336d9d2943c755e8d56dd564b`.
- Source manifest SHA-256: `9b2bf44b458b53d1e48a65080eb985b4fec9d7a84304b179c038a2b451e4f5d0`.

## Unified executor quote state update

Update: 2026-11-04

- The legacy `/v1/trades` aggregator path now records the same quote state transition after local preflight as the dedicated quote endpoint.
- If that transition cannot be persisted, the path marks the reservation unknown and returns the persistence error; no new idempotency key is created.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`70` tests).
- Candidate release binary SHA-256: `fb635cb4a4567f651459908552da4f362030e9381949a30e0e7ae77401d73b81`.
- Source manifest SHA-256: `83bd18df9ce43161d85ecc0db8532a52adb03d6a6b8e169f04b2561850038642`.

## Failed aggregator submission quote state

Update: 2026-11-04

- Dedicated and legacy aggregator submission paths now mark the bound quote `failed` when provider submission returns an error, while preserving the submission `unknown` state for explicit recovery.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`70` tests).
- Candidate release binary SHA-256: `85faa757425b9c6d697b3990ecec37884caf29f5be481763b1c4b3bbbb5c0b88`.
- Source manifest SHA-256: `697583b5a67e3116bb343ded20907d2843b6baec818e1a072fb6bca3c5a2281d`.

## Quote status visible after restart

Update: 2026-11-04

- Durable quote reopen tests now verify that the public bound quote returns its persisted `failed` status after restart.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`70` tests).
- Candidate release binary SHA-256: `256bc53c175e6a44713618d6b6ecd09d673730093e791e7a9c06e16b9dc1b3d8`.
- Source manifest SHA-256: `36e440506778f2a00b086e3bc8460bcbbb3562bc6b1e5c06360ce95ad9bcee62`.

## Quote cache and durable status ordering

Update: 2026-11-04

- Quote status updates now validate the cached status before SQLite mutation, then update the in-memory cache only after durable validation succeeds.
- This prevents a rejected durable transition from leaving memory ahead of the persisted state.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`70` tests).
- Candidate release binary SHA-256: `50d8e6208638046eee87461124a9a17795fe94587230f55de423d224886f8fdc`.
- Source manifest SHA-256: `a7b1d36014c117c3739a7283bc34fbe669a57aa321997f53c7615ef48e952290`.

## Terminal quote execution guard

Update: 2026-11-04

- Dedicated preflight and submit endpoints now reject a quote whose persisted status is `failed`; callers must request a new quote.
- Read-only quote retrieval still exposes the terminal status for audit and recovery visibility.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, and `cargo test --locked`: passed (`70` tests).
- Candidate release binary SHA-256: `2a5ce86db4089e22dcf4cc0c4dad6d0cf254137b9f880ce687c2e58b57d7b341`.
- Source manifest SHA-256: `d8184a1d307608acb3cf0e23e1f6d6eacbd0ee28f75d65c63ce78bf5b51c8d1c`.

## Bot Shadow diagnostics client

Update: 2026-11-04

- `bot_trading` now exposes a read-only `aggregator_shadow_metrics()` client method matching the candidate metrics endpoint.
- The method is diagnostic only; it does not affect route choice, submission policy, wallet state, or simulate-only readiness.
- Bot `cargo fmt --all -- --check` and `cargo check --locked`: passed.

## Durable submission cache ordering and missing-quote guard

Update: 2026-11-04

- Durable aggregator submission responses now update the in-memory terminal cache only after the SQLite write succeeds.
- Memory mode now rejects quote status updates for unknown quote IDs, matching durable behavior.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, `cargo test --locked`: passed (`70` tests).
- Candidate release binary SHA-256: `af8479d81a608f5d5e3fc9de255294cf8ce6b6361983cc9ec574cfd345824d93`.
- Source manifest SHA-256: `45abf186ad2c04c29c1ba18913bc1a7c1c4e7d275546fc31068be7bbca0fa50b`.

## Persisted aggregator preflight and audit evidence

Update: 2026-11-04

- Added idempotent schema version 4 columns `preflight_json` and `audit_json` for aggregator quotes.
- HTTP preflight and the legacy `/v1/trades` aggregator executor now persist the same preflight/audit evidence before advancing quote state.
- If that evidence write fails after a provider response, the legacy executor marks the reservation `unknown` for manual recovery.
- Reopen tests verify schema version 4, both persisted columns, and parsed preflight/audit evidence after restart.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, `cargo test --locked`: passed (70 tests).
- Candidate release binary SHA-256: `f0e87ad8f5f69031d56a78d773b7c1a3419bd713229fc8c624c1c74c1f2812aa`.
- Source manifest SHA-256: `246dd45e2703cd2a0f2eb4e8ed70958e4f06c16e43dc52383d2a0e99ac6c0e56`.

## Public quote evidence contract

Update: 2026-11-04

- Public Trade API documentation now specifies the additive read-only `preflight` and `audit` quote fields and their restart behavior.
- These fields remain diagnostic evidence only; they do not enable provider selection, signing, or broadcast.

## Submission binding guard

Update: 2026-11-04

- Durable and memory submission recording now rejects wallet or quote IDs that do not match the original reservation, and refuses overwriting terminal reservations.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, `cargo test --locked`: passed (71 tests).
- Candidate release binary SHA-256: `6fc0680083531dea1352fa233f24d78e3063eff1ccdeec7b873904dd3e0299f1`.
- Source manifest SHA-256: `71af9d8f5a5b8d9960356230200021624960f8c691872124ade7c1e570f63ab6`.

## Shadow latency summary

Update: 2026-11-04

- Shadow metrics now include read-only p50/p95/p99 latency, success rate, expiry rate, and status counts.
- The bot client accepts the additive summary model without using it for provider selection or execution.
- Candidate tests: 71 passed; release build passed.
- Candidate release binary SHA-256: `dae40a65319bef04d17cfe3882aad64823eaa0887fff000dda69762e4c1771ad`.
- Source manifest SHA-256: `4ca7d93f78fedb611612efe50d970a971b9ce062f92c29f2b625a13e687944e0`.

## Bot Shadow summary compatibility

Update: 2026-11-04

- `bot_trading` now has a serde contract test proving the additive Shadow `summary` is optional for older Trade Service responses and parsed for the current candidate response.
- The bot remains read-only for aggregator diagnostics and does not use summary data for provider selection or submission.
- Bot `cargo fmt --all -- --check`, targeted test, and `cargo check --locked`: passed.

## Shadow summary regression coverage

Update: 2026-11-04

- Added deterministic SQLite coverage for p50/p95/p99, success rate, expiry rate, and quoted/failed/unavailable counts across mixed Shadow statuses.
- Candidate `cargo fmt --all -- --check`, `cargo check --locked`, `cargo test --locked`: passed (72 tests).
- Candidate release binary SHA-256: `1df6ae54439f0667c4327486f844607fb52c67827d33f1c2ba61a66788d4bafb`.
- Source manifest SHA-256: `a787a637ab03fc32407e6e0f05b140e8c95fdfbe527a9218bc391acd3aecec58`.

## Compare provider binding guard

Update: 2026-11-04

- Compare results now revalidate provider, wallet, mint, amount, slippage, expiry, request hash, route fingerprint, and expected/minimum output before exposing a quoted result or recording Shadow metrics.
- Added mismatch regression coverage; candidate tests: 73 passed.
- Candidate release binary SHA-256: `41508c635864fe528287c6fc11c21defd1687940ce5369c7b3dbe5d86d399802`.
- Source manifest SHA-256: `0467b5b1754f9dce52f4a2b132c8104ee3567c32af61e795ebd98f990d896c35`.

## Candidate systemd static verification

Update: 2026-11-04

- Added staging-only `trade-service-candidate.service` with the candidate binary path and explicit `TRADE_SERVICE_SIMULATE_ONLY=true`.
- `systemd-analyze verify` passed for the candidate unit; the legacy `trade-service-db.service` remains a disabled reference to the old `/root/http/sol-bot` path and was not installed or started.
- No service process was started and no current symlink, wallet database, or 18788 tunnel was changed.

## Candidate audit manifest

Update: 2026-11-04

- Added [AUDIT_MANIFEST.json](AUDIT_MANIFEST.json) with candidate binary, Cargo files, toolchain, resolved current/previous paths, non-sensitive environment key names, and simulate-only/runtime guard assertions.
- Secret values, wallet private keys, database contents, and RPC responses are not included.
- Manifest SHA-256: `f8105050212233dba223332e581aa40f21fa317216dd7e9252f65e576ae3b61e`.

## Source manifest includes candidate unit

Update: 2026-11-04

- Source manifest scope now includes `scripts/` and `trade-service-candidate.service` alongside source, migrations, docs, and Cargo files.
- Candidate unit SHA-256: `b87729cd59e5f27df8b5f1d6a60e0c8e0f30f372a54c20d1d3b0c573110ac2d2`.
- Updated `BUILD_INFO` source manifest fingerprint: `6a889bb3773b1b6c18833007524320c4ce67135f06826e64a0a7f8fa4cd8189a`.
- Refreshed AUDIT_MANIFEST.json SHA-256 after adding the reproducible manifest script: `49de02fada8918e997a62b3575b132ae256ee71150fe8fad4ed3d1f014449fdc`.

## Reproducible manifest gate

Update: 2026-11-04

- `bash -n scripts/source_manifest_sha256.sh` passed.
- Script output matches `BUILD_INFO.source_manifest_sha256`.
- Candidate `fmt/check` and bot `fmt/check` passed.

## Unified aggregator simulation evidence

Update: 2026-11-04

- Legacy `/v1/trades` aggregator simulation reports now carry optional `request_id`, `route_fingerprint`, and `message_hash` fields alongside provider, quote, output, fee, slot, and Gate 1 fields.
- Fields are additive and remain diagnostic; absent message hashes stay `null` rather than being fabricated.
- Candidate tests: 73 passed; release build passed.
- Candidate release binary SHA-256: `cf1fcdd94c9e62f3ee0256592409b84148d0966deec8dc71b1ca89aaff56c4f8`.
- Source manifest SHA-256: `a32e188024a99c0f44eee1cf55e5094b7a00c090e8a550cd4c58c1fd6ab003ca`.

## Legacy aggregator simulate-only guard

Update: 2026-11-04

- The legacy `/v1/trades` aggregator executor now rejects `simulate=false` before quote/provider work whenever candidate `simulate_only` is enabled.
- Added a pure guard regression test; dedicated quote submit already enforces the same rule.
- Candidate tests: 74 passed; release build passed.
- Candidate release binary SHA-256: `4468e7ae9b53b14dfd3de3ce0def59c5b5c99538e2cb79aee16a5d530a87f929`.
- Source manifest SHA-256: `d91c9ba629cb2be40afe31b48e4073bd946d1f1b9aa4c9d8deddeeeec740379a`.

## Roadmap status synchronization

Update: 2026-11-04

- Reviewed roadmap now distinguishes completed Jupiter durable quote/preflight/audit, Shadow summaries, provider binding, and legacy simulate-only guard from remaining runtime HTTP integration, 24-hour Shadow, balance snapshots, and live/canary work.
- Source manifest SHA-256 after documentation sync: `2e42507f7098955165023011386ada67aad8ff3d65e0176e5764ebfcbea18975`.

## Dedicated submit state synchronization

Update: 2026-11-04

- Dedicated aggregator submit now persists preflight/audit evidence and advances quote status to `preflighted` or `failed`, matching the legacy executor path.
- Persistence failures mark the reservation `unknown` before returning an error.
- Candidate tests: 74 passed; release build passed.
- Candidate release binary SHA-256: `ce2ede49ca41409488bc1989d7a7fa6936307417502b74077431d3d62c3c5424`.
- Source manifest SHA-256: `41ee8f0b564a0764d47fe4c50e0131ba4904ac684875c382eb779aed68b050fe`.

## Memory/durable terminal submission parity

Update: 2026-11-04

- Memory-mode `record_submission` now rejects overwriting an existing terminal response, matching durable SQLite behavior.
- Candidate tests: 74 passed; release build passed.
- Candidate release binary SHA-256: `08d0e9bef89dc0b497e29ae246c91f4f8f87a3c3a02db161416eea5bdbce887b`.
- Source manifest SHA-256: `af34144a654feb90e4d5f866879dc0b3e7d271c13a606f71a693786ff9f8dd63`.

## Simulation evidence API documentation

Update: 2026-11-04

- Public API docs now describe optional aggregator `request_id`, `route_fingerprint`, and `message_hash` fields on legacy `/v1/trades` simulation responses.
- The docs explicitly state these fields are audit-only and cannot authorize signing or broadcast.
- Source manifest SHA-256: `ca4d80f848f9bbd3c0359890c5870746c47b4c5a05b06f662f604c1e82d49a5e`.

## Bot full test evidence

Update: 2026-11-04

- bot `cargo fmt --all -- --check` passed.
- bot full test run: 69 passed; 8 tests could not initialize local `TcpListener` because the sandbox returned `Operation not permitted`. No business assertion failed; the read-only aggregator Shadow summary contract test passed.

## Latest bot migration verification

Update: 2026-11-04

- With Cargo 1.99 selected from `/root/.cargo/bin`, bot `cargo fmt --all -- --check` passed.
- Bot test run: 71 passed; 8 tests were stopped before their assertions because the sandbox
  rejected local `TcpListener` creation with `Operation not permitted`.
- The read-only aggregator Shadow summary contract test passed. The bot still only reads
  capabilities and Shadow diagnostics; it does not select an aggregator or submit through the
  aggregator endpoints.

## Aggregator submit call-site audit

Update: 2026-11-04

- Current source has two direct `aggregator_quote::submit` call sites: dedicated HTTP submit and the legacy executor.
- Both are behind explicit candidate simulate-only guards; no third internal call path exists.

## Completion audit status

Update: 2026-11-04

- Candidate P0/P1 static, persistence, binding, simulate-only, and build evidence is present.
- The objective is intentionally not marked complete: runtime HTTP smoke tests remain sandbox-limited, DFlow remains deferred by plan, and 24-hour Shadow/live-canary evidence does not exist.

## Unified submission finalization

Update: 2026-11-04

- Added one `finalize_submission` state transition shared by dedicated quote submit and legacy `/v1/trades`.
- The legacy executor now persists successful submission responses for durable idempotency recovery; previously it only updated quote/preflight state.
- Any preflight, quote-status, or submission persistence failure marks the reservation `unknown`.
- Candidate tests: 74 passed; release build passed.
- Candidate release binary SHA-256: `c258bf73d163a7509726c38f4f02215007640126250b24b3c8e64cca39efe7c2`.
- Source manifest SHA-256: `9f1d9054f8d73ac39d7b48c673b3e12f828a404cf9ea32773e54c753708076e5`.

## Legacy submission persistence regression

Update: 2026-11-04

- Added durable `finalize_submission` regression coverage proving a successful legacy-style submission survives store reopen and remains quote-bound.
- Candidate tests: 75 passed; release build passed.
- Candidate release binary SHA-256: `beb2aaffc49088624e2c9c96ffe70f56addaa072feef0347d6cfa1463d3bff86`.
- Source manifest SHA-256: `bbdfc816ce1624b18360d7a9c6c4bc843d78b2d28aa3155b9b316375a0049a1c`.

## Finalize binding before evidence write

Update: 2026-11-04

- `finalize_submission` now verifies the original idempotency reservation is bound to the response quote before writing preflight/audit evidence.
- Wrong-key regression confirms no quote evidence is mutated on a binding failure.
- Candidate tests: 75 passed; release build passed.
- Candidate release binary SHA-256: `3664657de3289ff08d8c6e9712d7b663d6235ed7628a75133af1e1e78a7dbf6c`.
- Source manifest SHA-256: `9e9a761fa90d9dbcaaaaed7d187fc23200526385b4cff10aef0c08e9e04618b3`.

## Finalize open-reservation guard

Update: 2026-11-04

- `finalize_submission` now requires the reservation to be `reserved` or `submitting` before any preflight/audit write. Unknown and terminal reservations are rejected without quote mutation.
- Candidate tests: 75 passed; release build passed.
- Candidate release binary SHA-256: `3b77cf78e43878233777cb22f34eeb5844c5ffb851b31f489a855508108bbc84`.
- Source manifest SHA-256: `b055e33dfb36d9f03518b78ab8cbd1397a2902c159965ae9995a2f7801e1a217`.

## Memory preflight parity and final verification

Update: 2026-11-04

- Memory-mode `record_preflight` now stores preflight and audit evidence on the in-memory quote, matching durable behavior; the regression test verifies the evidence can be read back.
- Candidate `cargo fmt --all -- --check` passed.
- Candidate `cargo check --locked` passed.
- Candidate tests: 75 passed; release build passed.
- Candidate release binary SHA-256: `2699693eebd4d853d073f6735374e911b9ae5a30cb29a0c4b5f06ead6bed3fd7`.
- Source manifest SHA-256: `23903babc0151edfe5e799b25364097f70babb8c6f3ffa6a119e5db29dfd93b9`.

## Jupiter Gate 2 regression guard

Update: 2026-11-04

- Added a regression test asserting Jupiter remains quote-only while Gate 2 is closed: build, preflight, and submit all reject, and capabilities remain read-only.
- Targeted Jupiter Gate 2 test passed; format check, locked check, and release build passed.
- Candidate release binary SHA-256: `c221886d32672333ba375403a74e691ec271842fbd2e50ffdd5cc523ef5a6904`.
- Source manifest SHA-256: `74dd12ce159df5a69be2235fee6cfafc100d3a62939124ee43ae37f46867d410`.

## Bot compatibility-toolchain retest

Update: 2026-11-04

- With `/root/.cargo/bin` (Cargo 1.99.0), bot `cargo fmt --all -- --check` passed and `cargo test --locked` reached 69 passing tests.
- Eight bot HTTP tests remain sandbox-blocked before assertions because local `TcpListener::bind` returns `Operation not permitted`; no business assertion failed.
- The host default Cargo cannot parse lockfile v4, so bot verification must use the recorded compatibility toolchain.

## Bot read-only provider diagnostics

Update: 2026-11-04

- Bot preflight now reports Jupiter quote capability, DFlow disabled/read-only state, and the service `simulate_only` flag without selecting a provider or submitting a trade.
- Bot full test run with Cargo 1.99.0: 70 passed; 8 HTTP tests remain blocked before assertions by sandbox `TcpListener::bind` permission errors.
- The previously observed strategy test passed in isolation and under serialized execution; it was an intermittent test-run timing failure, not a reproduced business regression.

## Jupiter adapter boundary validation

Update: 2026-11-04

- Jupiter provider-neutral adapter now rejects empty wallet IDs, invalid wallet or output mint public keys, zero input amounts, and out-of-range slippage before calling the provider.
- Added regression coverage proving malformed requests fail before network access.
- Targeted Jupiter tests: 2 passed; `cargo fmt --check`, `cargo check --locked`, and release build passed.
- Candidate release binary SHA-256: `1fc98f600ffb062dd6bd03fa5a4ffb239e404cd78d22289b1f70316766ec160d`.
- Source manifest SHA-256: `6552b5a441f0b257c350d744890215d0d0cee0c33efee2e9c22d7bf2aa8dcfe0`.

## Release artifact reconciliation

Update: 2026-11-04

- Rebuilt the release binary after the adapter validation change and reconciled all artifact records.
- Current binary SHA-256: `009bb08d635b85b88fc8fc6417007a444e890906e8f4c89851e44d34821d6898`.
- `BUILD_INFO` and `AUDIT_MANIFEST.json` now match the binary; the source manifest is `2ebee9258e0ed2ee394d345008a8182a917214951730c606ac94bed93c2f2c3c`.

## Registry capability-gated quote dispatch

Update: 2026-11-04

- Quote dispatch now goes through `AggregatorRegistry::quote`, which enforces provider registration and `supports_quote` before invoking an adapter.
- Compare requests use this registry path; disabled DFlow is returned as unavailable without calling an endpoint.
- Candidate tests: 78 passed; release build passed.

## Public provider-boundary documentation

Update: 2026-11-04

- `PUBLIC_TRADE_API.md` now documents server-side registry dispatch, Jupiter-only default compare behavior, DFlow unavailable Shadow behavior, and the bot read-only boundary.
- Source manifest SHA-256: `5355d5e461f4133a80712b58e0135aa779fa598df6aaafd6f7a7fe04f30757cc`.

## Candidate gate revalidation

Update: 2026-11-04

- Candidate `fmt --check`, `check --locked`, full `test --locked` (78 passed), and `build --release --locked` all passed.
- Bot `cargo fmt --all -- --check` passed with the Cargo 1.99.0 compatibility toolchain.
- `current` remains `/root/solana/trade_service/releases/sdk-6.0.2-legacy`; candidate systemd unit is not active.
- Runtime HTTP smoke and 24-hour Shadow evidence remain intentionally absent; no live or broadcast path was enabled.

## Provider route continuity validation

Update: 2026-11-04

- Compare quote validation now checks that route legs start at the requested input mint, connect contiguously, and end at the requested output mint, independently of the route fingerprint.
- Added a discontinuous-route regression test.
- Candidate tests: 79 passed; format, locked check, and release build passed.
- Source manifest SHA-256: `f95b973586e1f5f9071dad2b7415fd161fd819550b080d9eace336411c7a45ee`.

## Reproducible candidate gate runner

Update: 2026-11-04

- Added `scripts/verify_candidate_gates.sh`, which runs format, locked check, full tests, release build, simulate-only guards, artifact hash checks, and current-path protection without starting a service or binding a port.
- The gate runner reached the full 79-test pass stage; artifact checks are recorded against the current candidate build.
- Source manifest SHA-256: `bbfb53e364c9ee9a097edff62a7eacb443fad6e21c22b8a65301610484d0d9e1`.

## Candidate guard-source verification

Update: 2026-11-04

- The gate runner now validates simulate-only values from the candidate `.env` and candidate systemd unit directly; caller environment defaults cannot mask a bad staging configuration.
- The runner completed all seven gates, including 79 tests, release build, artifact hashes, and unchanged `current` path.

## Candidate inactive-unit guard

Update: 2026-11-04

- The gate runner now also rejects an active `trade-service-candidate.service`; verification remains staging-only and cannot be mistaken for deployment.
- Source manifest SHA-256: `c3d4f1db1c08ceb143702a12b32ecad2dcbd73f06c8cc2004b2e9c60b07c0d81`.

## DFlow configured-endpoint safety regression

Update: 2026-11-04

- Added regression coverage proving that even a configured DFlow endpoint remains read-only and rejects quote requests without entering a network adapter path.
- DFlow tests: 2 passed; locked check and release build passed.
- Source manifest SHA-256: `73323aa83ea5316a7efec336c2073b8c9763b870027ebe601350c38caeb88824`.

## Final candidate test revalidation

Update: 2026-11-04

- Full candidate test suite: **80 passed, 0 failed**.
- Source manifest remains `73323aa83ea5316a7efec336c2073b8c9763b870027ebe601350c38caeb88824`.
- Current release binary SHA-256: `4b424d749806c58b27cf4c5ac86b1665a72e8afb13bf9e929c1f67d3b02bc261`.

## Router SDK classification

Update: 2026-11-04

- Documented `sol-trade-router-sdk` as a separate optional `router_backend`, not an aggregator provider.
- It remains outside `AggregatorRegistry`, quote compare, provider fallback, build, submit, and live capability.
- Any future integration must begin with route replay/observe/simulate and reuse the existing wallet, quote, preflight, audit, and simulate-only guards.
- Source manifest SHA-256: `eed82f255361eb6c4e5392a8d5768fc1eae2798dda60e4b937c7f87005450fa0`.

## Router SDK evaluation record

Update: 2026-11-04

- Added `docs/ROUTER_SDK_EVALUATION.md` with the router-versus-aggregator classification, integration prerequisites, and explicit candidate prohibitions.
- The SDK remains unevaluated for execution and is not part of the current aggregator registry.
- Source manifest SHA-256: `0bb6691ba05305170b2a1af5df0a380eacf7e29b956c340a6d78633f8ffdafaa`.

## Router SDK workspace evidence

Update: 2026-11-04

- Verified the upstream workspace contains both `programs/sol-trade-router` (`cdylib`/`lib` Pinocchio on-chain program) and `crates/sdk` (default client package).
- This confirms the Router SDK requires a matching self-deployed program for complete Route execution and remains separate from aggregator providers.
- Source manifest SHA-256: `0b67c0e695023bcb886afc2d71c6f3832c3e386351e8602c98179c2fd3cb9ee2`.

## Production tunnel preservation guard

Update: 2026-11-04

- The candidate gate runner now checks that no listener is bound to `127.0.0.1:18788`; it never binds or modifies that port.
- Source manifest SHA-256: `2321b00a4d346f36b8527875aa1d0c182f0de47a5ce8662fcc276e6adcc20031`.

## Port-check sandbox limitation

Update: 2026-11-04

- The gate runner now distinguishes a successful `ss` check from unavailable netlink access; in this sandbox the 18788 listener check reports `unavailable` rather than claiming that the port is free.
- No listener is started or modified by the runner.
- Source manifest SHA-256: `bf71bc250e0da606057b126e46a88009f2e95e9c57fa357f993efae429d64f15`.

Latest gate rerun: `ss` access was available and confirmed no listener on `18788`; the script retains the explicit `unavailable` path for restricted runs.

## Staging runtime unblock plan

Update: 2026-11-04

- Added an explicit staging runtime prerequisite to the reviewed plan and remaining roadmap.
- The prerequisite requires an isolated candidate port/database, local bind permission, HTTP smoke,
  restart readback, and DFlow unavailable verification before starting the 24-hour Jupiter Shadow.
- The current sandbox remains static/portable-gate only; no runtime completion is claimed.
- Added `scripts/staging_http_smoke.sh` as the repeatable read-only runtime check. It defaults to
  isolated port `18887`, validates health/ready/capabilities, performs one DFlow read-only compare
  to verify unavailable Shadow persistence, and sends no build, sign, or submit request.

## Latest candidate gate rerun

Update: 2026-11-04

- `scripts/staging_http_smoke.sh` passed `bash -n` validation.
- Candidate gate runner passed: 80 tests, locked check, release build, simulate-only guards,
  unchanged `current`, inactive candidate unit, and no listener on `18788`.
- Current release binary SHA-256: `4b424d749806c58b27cf4c5ac86b1665a72e8afb13bf9e929c1f67d3b02bc261`.
- Current source manifest SHA-256: `c694e6d640f327ac5abad465565737d0bbf2e21f3fcaab569387ee522adec70d`.
- Runtime HTTP smoke and 24-hour Shadow remain unclaimed until an isolated staging instance is
  available.

## Candidate gate rerun after DFlow regression coverage

Update: 2026-11-04

- The gate runner selected Cargo 1.99 from `/root/.cargo/bin` and completed the full sequence.
- Candidate tests: 80 passed; locked check and release build passed.
- Runtime guards passed: simulate-only configuration, inactive candidate unit, unchanged
  `current`, and no listener on `18788`.
- Candidate release binary SHA-256: `01cbd6223c608ccb27671239af235e650eac11551bc5650f78e5660253f0413e`.
- Source manifest SHA-256 before this documentation entry: `62690464de5c30dd9089057882358ff31ed236d51a5787d9e5d01ee942b1b1c3`.

## DFlow staging smoke extension

Update: 2026-11-04

- `scripts/staging_http_smoke.sh` now performs one DFlow-only compare request, requires
  `shadow_only=true` and `status=unavailable`, then verifies the persisted DFlow
  `unavailable_count` through the read-only metrics endpoint.
- No build, signing, submit, production port, or current release path is used by the script.
- Candidate gate rerun passed: 80 tests, release build, runtime guards, and artifact hashes.
- Current source manifest SHA-256: `17456720d346b3965448a3353cade82205986feb11508f1c165e0123385bcd5b`.

## Staging persistence readback helper

Update: 2026-11-04

- Added `scripts/verify_staging_persistence.py` for isolated post-restart checks.
- It opens the supplied SQLite file read-only, verifies the quote/submission/Shadow schema and
  binding columns, and rejects leftover `reserved/submitting` submissions.
- It performs no migration, update, or service restart.

- The existing `candidate-data/trades.sqlite` was inspected read-only and correctly rejected as
  an old database without aggregator tables; it was not modified.
- A fresh temporary SQLite fixture with migrations 001, 003, and 004 was checked successfully by
  the helper, then deleted.

## Latest candidate gate after persistence helper

Update: 2026-11-04

- Python persistence helper passed `py_compile`; staging smoke passed shell syntax validation.
- Candidate gate passed again: 80 tests, release build, runtime guards, unchanged `current`, and
  no listener on `18788`.
- Current source manifest SHA-256: `20aba9ec27107453d75fda6f1282efd562f09586f9bb5f2e8070f00859adbcc1`.

## Isolated staging launch attempt

Update: 2026-11-04

- Candidate was launched for 15 seconds with copied candidate-data SQLite files, port `18887`,
  and explicit simulate-only overrides.
- Database initialization and provider setup completed without touching the source databases.
- The process exited when the sandbox rejected `TcpListener::bind(127.0.0.1:18887)` with
  `Operation not permitted`; no HTTP smoke or Shadow sample was claimed.
- Temporary database copies and logs were removed after the attempt.

## Staging smoke safety guard

Update: 2026-11-04

- Gate runner now rejects staging smoke scripts containing build, signing, or submit paths and
  requires the DFlow compare route to remain present.
- Candidate gate passed after the guard change: 80 tests, release build, runtime guards, and
  artifact hashes.
- Current source manifest SHA-256: `49cedd24cc52ca9cd834d01217638cc307ebd475559ee2450724a84e3e0f6723`.

## Validation helper syntax gate

Update: 2026-11-04

- Gate runner now compiles `verify_staging_persistence.py` and runs `bash -n` on
  `staging_http_smoke.sh` before artifact verification.
- Candidate gate passed: 80 tests, release build, runtime guards, and matching artifact hashes.
- Current source manifest SHA-256: `ed175586c140312d884ea2649c352e54aa07a0e137d8e1b0733572f23b8f0d6a`.

## Isolated staging runbook

Update: 2026-11-04

- Added [`docs/STAGING_RUNBOOK.md`](docs/STAGING_RUNBOOK.md) with the exact isolated startup,
  DFlow smoke, restart readback, cleanup, and 24-hour Shadow entry sequence.
- Candidate gate passed after the runbook and guard updates: 80 tests, release build, runtime
  guards, and artifact hashes.
- Current source manifest SHA-256: `c73794ce319b5df3b826f868c5dda415e5ddf141703768bb6b5df8ba1d0587f7`.

## RPC error log redaction

Update: 2026-11-04

- Sanitized RPC and provider error values before logging in wallet initialization, blockhash
  refresh, persistence, HTTP trade handling, executor status handling, and SDK lane failures.
- URLs are replaced with `<redacted-url>` before they reach tracing fields; error status and lane
  context remain available for diagnosis.
- Candidate gate passed: 80 tests, release build, runtime guards, and artifact hashes.
- Current binary SHA-256: `d2d17323c06cf4eeade5bd45780385719374da924020186c9e37415b47dd3201`.
- Current source manifest SHA-256: `da2bfe6c84cf943e254e9631cb41f4a0e8a509b1f65fed0f4fda9b2e239dfadb`.

## Complete RPC log redaction audit

Update: 2026-11-04

- Static audit found and sanitized remaining join, persistence, rejected-trade, and lane-result
  tracing fields that could carry provider URLs.
- Added two `redact_urls` regression tests covering query-string keys and punctuation boundaries.
- Candidate test count: 82 passed; release build and full gate passed.
- Current binary SHA-256: `9e00b6eb76ce95da26055ab8974f9bfd725166ed376c391517a39bb246ff79f4`.
- Current source manifest SHA-256: `eae20fc28c204bf87de44b4a57e92fb7aeed70972e2275af950c6da6e72dad2c`.

## WebSocket endpoint redaction

Update: 2026-11-04

- Extended error redaction to `ws://` and `wss://` endpoints used by QUIC/MEV fallback paths.
- Added a regression test proving WebSocket query tokens are removed.
- Candidate test count: 83 passed; release build and full gate passed.
- Current binary SHA-256: `f0e463688df966e2b0c9997f02cdc0351b27d91658e33529892b3f49250961e6`.
- Current source manifest SHA-256: `66a64443b0fd05c7d4ea64aa113dbe0d4280e9618183d98c55d3f0f8441ad341`.

## DFlow unavailable Shadow regression

Update: 2026-11-04

- Added a durable `QuoteStore` regression proving a DFlow `unavailable` observation is retained
  under the DFlow provider slice, has zero quoted samples, and does not affect Jupiter success
  metrics.
- `cargo fmt --all -- --check` passed after the change.
- Full test rerun is pending a Cargo toolchain that supports lockfile v4; the active shell is
  Cargo 1.75.0 and rejects this candidate lockfile before compilation.

## Bot preflight Shadow display

Update: 2026-11-04

- Preflight now queries the read-only Shadow metrics endpoint and displays sample count, quoted
  count, unavailable count, p95 latency, and the read-only flag.
- Shadow query failures remain diagnostic text and do not create a new Live blocking reason.
- Targeted preflight tests: 4 passed.
- Latest full bot test run: 72 passed; 8 local-listener tests were denied by the sandbox before
  their assertions.

## Candidate gate after Shadow error redaction hardening

Update: 2026-11-04

- Rebuilt the candidate after durable Shadow `error_class` URL redaction.
- Full candidate gate passed: format, locked check, 83 tests, release build, runtime guards,
  staging-script safety checks, unchanged `current`, and no listener on `18788`.
- Binary SHA-256: `7186b6765d72b837e70a2fe27e8ce6efc772abadb7665d1009b3689db09b1cb6`.
- Source manifest SHA-256: `71867e993d0f894621a5d5ddca9080fceca06222e4461294fe412fe9cd940949`.
- HTTP staging smoke and 24-hour Shadow remain pending because this sandbox denies local
  `TcpListener::bind`; no runtime completion is claimed.

## Executor result error redaction

Update: 2026-11-05

- Closed a P1 logging/state boundary: `executor::map_status` now redacts HTTP(S), WS(S),
  and query credential material when an `ExecuteResult` carries an error in either the
  success or failure branch.
- Added regression coverage for success and failure results containing RPC/provider URLs.
- Targeted test passed; the subsequent full candidate gate passed with 84 tests.
- Binary SHA-256: `8567a89e44a5c804c9ce574e2a375b9a7a0f5ce8bec20cf87e53b4ee6c0c2221`.
- The source manifest is refreshed after this documentation update; HTTP staging and 24-hour
  Shadow remain pending because local listener permission is unavailable in this sandbox.

## HTTP error response redaction

Update: 2026-11-05

- Unified the candidate HTTP `error()` response path through `redact_urls`, preventing RPC,
  provider, WebSocket, and query credential material from being returned to API clients.
- Added an in-process HTTP regression test for a provider URL with a query API key.
- Full candidate gate passed: 85 tests, locked check, release build, runtime guards, unchanged
  `current`, and no listener on `18788`.
- Binary SHA-256: `ce162631e2b112c786b1a10f9dadfaa78c4e471e43be52fae97cd226e601353d`.
- Source manifest SHA-256: `60ceca0cfcd7612d732625bd40dae2386ec4879868ecb2b0e1db8b29d18663cc`.

## Staging listener recheck

Update: 2026-11-05

- Rechecked isolated port `127.0.0.1:18887` before attempting runtime smoke.
- The sandbox rejects socket creation with `Operation not permitted`; no candidate process was
  started and no HTTP smoke or 24-hour Shadow evidence was claimed.
- The existing runbook remains the required next step on an allowed isolated staging/VM.

## Error redaction before persistence

Update: 2026-11-05

- Redacted provider/RPC URL material before writing SOL transfer failures, wallet-balance
  failures, and rejected-trade errors into SQLite or returning them in response records.
- `cargo check --locked` passed and the complete candidate gate passed with 85 tests.
- Binary SHA-256: `21e10555d3b76b4a818372068e1b14c819434dc2036975e3867ee6567d047aa5`.
- Source manifest SHA-256: `cbcfbfbdb516a71973f6258e659125d1e02ee2ba911fb259a0dbcd4c85ff12ef`.

## Unknown submission error redaction

Update: 2026-11-05

- `QuoteStore::mark_unknown` now redacts provider/RPC URLs before persisting unknown submission
  response JSON, including restart-recovery records.
- Added durable SQLite regression coverage proving query credentials are absent from stored
  unknown errors.
- Full candidate gate passed with 86 tests.
- Binary SHA-256: `d2ab4f686d959290afd7291c53dda43dd8a780b7337279a0f97a89bd68674dee`.
- Source manifest SHA-256: `61e601eaeade1a67468668ed861413808026e658248b33d18cb44c24bee9e175`.

## SDK RPC fallback error redaction

Update: 2026-11-05

- Redacted RPC lane errors before they enter wallet balance, wallet-balance collection, and SOL
  transfer fallback messages.
- Bonk parameter diagnostics and non-default lane fallback notes now preserve only redacted error
  text as well.
- Full candidate gate passed with 86 tests.
- Binary SHA-256: `32ba21cc3b2cc3f9c931e185951ac4d4cc405c1f2558993a4d29ae7e34a1b611`.
- Source manifest SHA-256: `3427915e81b01be7c4eadc7eaf20b57af529649d5e17f79406538f8349fc66f3`.

## Wallet/PumpAPI error redaction

Update: 2026-11-05

- Vanity wallet worker failures and PumpAPI simulation/confirmation errors are now redacted
  before persistence or response conversion.
- Full candidate gate passed with 86 tests.
- Binary SHA-256: `6fadcc9400956f1e6210abc8d76049b21441ed366b1ff7cb82fe536796d9536e`.
- Source manifest SHA-256: `60a77b9f9c4b45268daba49a3b8b46b445ba2cf72d85919c30dc059d27449887`.

## Aggregator submit unknown response redaction

Update: 2026-11-05

- Aggregator submit now redacts provider/RPC URLs from the `unknown` response error before the
  caller can persist or return it.
- Full candidate gate passed with 86 tests.
- Binary SHA-256: `8aeb8f836f4ba2684d029d5025d975363241862a921b6614b9ef6b33b772b647`.
- Source manifest SHA-256: `3680ea02ed4fada50b385488f3391f8438361d239a74e9a95102937648eb4f5b`.

## Audit manifest and reproducible source fingerprint gate

Update: 2026-11-05

- Refreshed `AUDIT_MANIFEST.json` to the current candidate binary and source fingerprint.
- Added `scripts/verify_audit_manifest.py`; the candidate gate now checks binary, Cargo files,
  candidate unit, current path, runtime guard values, and source manifest against the audit record.
- Excluded Python `__pycache__` and `*.pyc` outputs from the source fingerprint so syntax checks
  cannot change the reproducible hash.
- Candidate gate passed with the existing 86 tests; audit manifest verification and pycache
  stability checks passed.
- Binary SHA-256: `8aeb8f836f4ba2684d029d5025d975363241862a921b6614b9ef6b33b772b647`.
- Source manifest SHA-256: `4b10871295dcad5de26130bd31011f809fa787596e5450977a3a78aa1d4323b0`.

## Staging persistence sanitization gate

Update: 2026-11-05

- `verify_staging_persistence.py` now scans persisted aggregator submission and Shadow error
  fields for unsanitized HTTP(S)/WS(S) URLs and query credential markers.
- A temporary SQLite fixture containing already-redacted values passed the read-only check; the
  fixture was removed afterward.
- Full candidate gate and audit-manifest verification passed.
- Binary SHA-256 remains `8aeb8f836f4ba2684d029d5025d975363241862a921b6614b9ef6b33b772b647`.
- Source manifest SHA-256: `99b039a4c299a92b911e553622390b4dcf77cd7df2daecdca375943849878aa2`.

## Token-account RPC fallback redaction

Update: 2026-11-05

- Token-account balance fallback errors now convert to redacted `anyhow` text before returning
  through SDK diagnostics.
- Full candidate gate and audit-manifest verification passed with 86 tests.
- Binary SHA-256: `c8e44c425eddff21ede1c3d203809ee3a05b4f64d01729343ba1ecbd8b071204`.
- Source manifest SHA-256: `b24de9b28ab8eb084d8ce5dcf34322cc6d0ee9305bfb28c0efa34a4540d9fcdd`.

## Staging sanitization negative fixture

Update: 2026-11-05

- Ran a temporary SQLite fixture containing an unsanitized provider URL and query API key.
- `verify_staging_persistence.py` rejected the fixture with the expected security failure.
- The fixture and captured output were removed; no production or candidate database was changed.

## Roadmap and Bot readiness documentation sync

Update: 2026-11-05

- Updated the remaining-roadmap status to distinguish completed Bot read-only diagnostics and
  simulate-only readiness blocking from pending runtime HTTP/24-hour Shadow evidence.
- Candidate gate and audit-manifest verification passed; binary is unchanged.
- Binary SHA-256: `c8e44c425eddff21ede1c3d203809ee3a05b4f64d01729343ba1ecbd8b071204`.
- Source manifest SHA-256: `b58d6cbdae117974ca57b536d757cc71fdb93496185ff125561ed8761e4e65d8`.

## Cross-project Bot simulate-only gate

Update: 2026-11-05

- Candidate gate now invokes `/root/solana/bot_trading/scripts/verify_simulate_only_config.py`
  when the Bot workspace is present.
- The combined gate verifies the Bot remains on the `18788` tunnel with submission disabled and
  dry-run default enabled; it does not start the Bot or make network requests.
- Full candidate gate and audit-manifest verification passed.
- Binary SHA-256 remains `c8e44c425eddff21ede1c3d203809ee3a05b4f64d01729343ba1ecbd8b071204`.
- Source manifest SHA-256: `1d5ff3a7323ea8995628cad90fad5c8c43f6dc663f9b524c2283a208a8fea23a`.

## Bot unit simulate-only static gate

Update: 2026-11-05

- Candidate gate now verifies that the Bot systemd unit contains the simulate-only
  `ExecStartPre` guard in addition to checking the Bot `.env` values.
- Full candidate gate and audit-manifest verification passed; binary unchanged.
- Binary SHA-256: `c8e44c425eddff21ede1c3d203809ee3a05b4f64d01729343ba1ecbd8b071204`.
- Source manifest SHA-256: `1b406fa98f66fda017eaee4e6b70a7b3f96e097601ecadcc4d71b355c78fa34a`.

## Staging runbook Bot preflight sync

Update: 2026-11-05

- The isolated staging runbook now checks Bot simulate-only configuration and the systemd
  `ExecStartPre` guard before any candidate smoke; it explicitly keeps Bot stopped and `18788`
  untouched.
- Candidate gate and audit-manifest verification passed; binary unchanged.
- Source manifest SHA-256: `3196b51f62bd7caca96f13bc642ff59ce5d864aee743cf384a3b3e613cd9da35`.

## Candidate gate refresh

Update: 2026-11-05 09:59 +0800

- Refreshed `BUILD_INFO` and `AUDIT_MANIFEST.json` after the staging unit and
  Shadow sampler became part of the source fingerprint.
- `scripts/verify_candidate_gates.sh` passed all seven gates: format, locked
  check, 86 tests, release build, runtime guards, artifact hashes, and the
  unchanged `current`/`18788` boundary.
- Binary SHA-256:
  `83dea132ddb3012f0a4297c561933747593bd168f7510d242cfb49786e522690`.
- Source manifest SHA-256:
  `f10b870aa72c150f652e7f2bdd42fbde3e73f6569a9c77ceadffac1169c4343c`.
- Runtime HTTP smoke and the 24-hour Jupiter Shadow remain separate host-level
  evidence; this gate does not claim either one is complete.
