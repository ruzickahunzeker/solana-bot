# Sol-Bot Optimization Baseline

Date: 2026-11-11 (Asia/Shanghai)

This is a local, read-only baseline for the V1.1 optimization plan. It is not a
production acceptance record. No service was started, no trade was submitted,
and no wallet or database was modified while collecting it.

## Source and runtime

- Repository HEAD: `00c9b7a47f08e2f66da32926744f09d1aeb36141`.
- The working tree has pre-existing uncommitted Alpha shutdown changes and an
  untracked `trade_service-sdk9/` directory. Do not treat HEAD as the deployed
  binary's source revision.
- Trade Service, Copy Trading, and PumpAPI bridge were inactive when inspected.
- The Trade Service unit points to
  `trade_service/target/release/pump-trade-service`, which is currently absent.
  The Copy Trading release binary exists, but it predates the current source
  edits. Neither should be started as a deployment test without a controlled
  build and review.
- `trade_service/.env` specifies `TRADE_PERSISTENCE_MODE=durable`. Because the
  service is stopped, `/ready` has not verified its actual running mode or
  `trade_database_healthy` state. Configuration is not runtime evidence.
- The filesystem had about 2.1 GiB free. A fresh Rust build and a SQLite
  compaction are not safe to assume on this host.

## Dependency and schema identity

- `trade_service/Cargo.lock` SHA-256:
  `f0f0421ca67903d630027b91e959b23a4bac8dcbe2da43e645cc51e5ecaeb535`.
- `copy_trading/Cargo.lock` SHA-256:
  `112819846b3f4358fb9ca8aa0c84f322597266cc62b14b3ab634674c241508dd`.
- `router_sdk_probe/Cargo.lock` SHA-256:
  `a393fc67516e2fa489df327f946c416aaf9756acffc465194b97a224ca375227`.
- Vendored `sol-trade-sdk` manifest reports version `6.0.2`; the Router SDK
  probe pins commit `de21397aae7836d11124b08b047e390cba3fd46e`.
- Trade Service has migration `001_trade_persistence.sql`; Copy Trading has
  migrations `001` through `007`. The inspected databases had 22 Copy Trading,
  5 wallet, and 2 trade tables; SQLite `user_version` was 0 for each, so it is
  not a useful migration marker here.
- The three database files are distinct. Trade persistence mode does not
  describe wallet storage or Copy Trading business-data durability.

## Contracts and known gaps

- Existing HTTP routes are declared in `trade_service/src/http.rs`. The
  protected `/ready` response includes persistence mode and database health.
- `trade_service/src/config.rs` defaults persistence to `memory`; a configured
  durable mode must still be confirmed at runtime before automatic recovery.
- `copy_trading/src/order/mod.rs` already has an `OrderReconciler`; this change
  adds a fail-closed gate for orphaned automatic resubmission. It does not add
  signature finalization or an event delivery log.
- The Bridge currently publishes raw messages and optional observation and
  normalized messages. A PUB/SUB subscriber can miss events. Observation JSONL
  is not a durable normalized queue.
- `protocol_capabilities.rs` lists several protocols as `observe`. A builder
  in the SDK is not evidence that the Trade API can execute that protocol.
- The Trade API has a route for private-key export under `/v1/wallets/...`.
  It requires the general protected authorization, so route-level privilege
  separation remains an unverified P0 security item.

## Verification update (2026-11-12)

- After clearing regenerable disk caches, `copy_trading` passed
  `cargo test --locked --offline` (53 tests). Its existing release binary was
  not rebuilt or deployed.
- `trade_service` passed `cargo test --locked --offline` (46 tests) and
  `cargo build --release --locked --offline -j 2`. The resulting release binary
  has SHA-256
  `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`.
  It has not been started; its existence is not runtime acceptance.
- `cargo fmt --check` passed for both Rust projects. No service was started,
  no database was modified, and no transaction was submitted.
- Available disk space after both builds was about 5.4 GiB. This is still not
  enough to assume an in-place compaction of the 17 GiB SQLite database is
  safe without a verified current backup and a separate capacity plan.

## Remaining baseline checks

1. Build in an environment with adequate free space; record binary hashes and
   the exact source revision used for each service.
2. Verify `/ready` on the intended deployment after an approved start, without
   submitting an order. Require `durable` and a healthy trade database.
3. Confirm firewall, TLS/tunnel, and proxy path rules from the actual host.
4. Restore a current, encrypted database backup in an isolated environment and
   verify wallet public keys and order counts before any compaction or rollout.
5. Run the full Rust and Python suites, then controlled simulation. Do not
   infer live readiness from source review alone.

## Runtime verification (2026-11-12, Asia/Shanghai)

- After an explicit start request, the Trade Service, PumpAPI bridge, and Copy
  Trading systemd units were started in that order. All three reported
  `active`; the Bridge logged a PumpAPI WebSocket connection. No order or
  simulation was submitted during this check.
- The deployed Trade Service release binary SHA-256 was
  `b606c815480a3538fe97eba423a4e3e8211e764f8cf068673a3aa98ef92ce47f`.
  The newly built Copy Trading release binary SHA-256 was
  `9bad1b0ce1a1087f91df24f47dd087bb7107c2c12e1c48e72213b77cf8c794cc`.
  The previous Copy Trading binary was retained at
  `backups/rollbacks/copy-trading-pre-optimization-20261112` (SHA-256
  `472359a7ef2f08d0e2ee868738ff108f6d8b04df17007a43e78ecae53bb7b89d`).
  This local rollback copy is not a substitute for an off-host backup.
- Trade Service took about two and a half minutes to begin listening while
  initializing SDK channels. `/health` returned `ok=true`. The authenticated
  `/ready` check returned `ready=true`, `trade_persistence_mode=durable`, and
  `trade_database_healthy=true`, with eight loaded wallets. One readiness
  request exceeded 10 seconds; a subsequent request completed in about 13
  seconds. RPC lane 0 also logged HTTP 429 during blockhash refresh.
- Copy Trading had zero enabled Live strategies and zero pending Live limit
  orders before start, but one user's default mode was Live. Starting the bot
  does not disable manual Live trades. Alpha remained disabled by local config.
- Trade Service now has a file-backed, restart/reopen idempotency test. Its 47
  offline Rust tests and formatting check passed. Copy Trading's 53 offline
  Rust tests and formatting check passed before its release build. The new
  Trade Service test is source-only and did not change the running release
  binary.
- Remaining acceptance gaps: current encrypted database restore, failure
  injection across remote submission and local audit writes, independent
  aggregator reservation, firewall/tunnel and wallet-export authorization,
  plus protocol-specific simulation. The running services must not be treated
  as approval to expand Live routes.
- Follow-up source work added a Copy Trading duplicate-key payload conflict
  check, a file-backed restart test, and an unavailable-readiness recovery
  test. An asynchronous test race in the strategy suite was corrected without
  changing production logic. Ordinary and fast Live submit now require durable
  readiness before retrying after an uncertain HTTP error and record
  `unknown` otherwise. A loopback fake Trade Service test injects remote
  accept followed by local result-write failure, then verifies reconciler
  recovery uses the same key. Another test verifies memory mode does not
  retry. A pre-submit attempt-write failure test confirms no HTTP call occurs.
  The 59 offline Copy Trading tests passed twice. These follow-up changes
  are not in the running release binary; a real process-crash test,
  fast-path-specific fault injection, and a controlled rebuild remain before
  deployment.
- Local socket inspection found Trade Service listening on `0.0.0.0:18787`
  while the Bridge PUB socket is bound to `127.0.0.1:8939`. UFW was inactive;
  the inspected host iptables rules showed no port-18787 allowlist. The host's
  visible primary address was private (`10.9.0.15`), but cloud security-group,
  NAT, and tunnel reachability were not verified. Keep route-level wallet
  export authorization and external exposure as open P0 security findings;
  do not change the bind or firewall without a coordinated Bot connectivity
  check and rollback window.
