# Sol-Bot V1.1 Execution Plan

Date: 2026-11-12 (Asia/Shanghai). Scope: `trade_service`,
`copy_trading`, and `pumpapi_zmq_bridge`. This plan is based on the local
working tree and the attached V1.1 optimization plan, not on the older ZIP
alone. A checked source change is not necessarily the deployed binary.

## Operating rules

1. Keep the three services independent. Trade Service remains the only
   signer/broadcaster; Bridge is the PumpAPI ingress; Copy Trading owns
   strategies, Telegram UI, and order orchestration. Preserve existing HTTP
   contracts while adding tests and internal safeguards.
2. Do not enable a new Live route, protocol, strategy, or automatic replay
   merely because a unit test passes. Never retry an uncertain order with a
   new idempotency key. Do not submit a mainnet order as a diagnostic probe.
3. Do not alter wallet keys, production databases, service bind addresses,
   tokens, or firewall rules without a recovery copy and a coordinated
   deployment window. `trade_service-sdk9/` is an unrelated untracked tree.
4. Every work item closes with tests, evidence, deployed-version identity,
   and rollback steps. Update `OPTIMIZATION_BACKLOG.md` and `BASELINE.md` as
   evidence changes; mark `source`, `tested`, and `deployed` separately.

## Gate A: Recoverability and exposure (P0, 2-4 working days)

| Item | Current evidence | Next action | Completion proof |
| --- | --- | --- | --- |
| Binary and schema identity | Both Rust binaries built; hashes and migrations recorded | Keep versioned off-host rollback artifacts | Hashes, source commit, and restored executable match |
| Three production databases | Trade, wallet, and Copy Trading SQLite are distinct; Copy Trading is about 17 GiB | Make a current encrypted online backup to storage with enough free space; restore an isolated copy | Integrity check, row counts, wallet public-key map, and restore log |
| API exposure | Trade API binds `0.0.0.0:18787`; UFW inactive; no host port allowlist found | Verify cloud security group, tunnel/TLS, and caller paths; design separate trade/admin credentials | Authorized/denied probes from actual network paths; no wallet export via trade-only credential |
| Runtime readiness | Authenticated `/ready` returned durable and healthy once; RPC 429 also observed | Record repeat readiness timings, blockhash age, and failure behavior | Stable readiness evidence without submitting orders |

No SQLite `VACUUM`, deletion, or production schema change is allowed until
the off-host restore is verified. This gate currently blocks expanding Live.

## Gate B: Submission and reconciliation (P0, 4-7 working days)

1. Verify the ordinary `/v1/trades` and batch reservation precede every SDK
   and PumpAPI external effect. The file-backed Trade Service restart test
   already proves same-key replay returns the original `unknown` record and
   changed payload conflicts; it does not prove every HTTP route is covered.
2. Add isolated fault-injection tests for Copy Trading: DB write failure before
   submit, HTTP timeout after remote accept, crash after remote response but
   before local write, and restart with an orphaned intent. The local test
   now covers pre-submit attempt-write failure, remote accept plus result-write
   failure, and reconciler recovery with the same key. A real process-crash
   timing test remains. Assert no
   second outbound request under a new key and no silent loss of `unknown`.
3. Keep the independent SOL aggregator submit route closed to expanded Live
   use until it has a durable reservation with replay/conflict semantics and
   an after-broadcast crash test. Do not infer protection from the ordinary
   trade reservation.
4. Add read-only signature status tracking. Distinguish accepted/submitted,
   chain confirmed, and finalized; never treat HTTP submission as finality.
5. Deploy the Copy Trading local duplicate-request conflict guard only after
   fault tests and a rollback build. Its source and unit test currently pass,
   but the running release binary does not include this follow-up change.

Completion proof: one request per business action across the four failure
points, stable idempotency across restart, explicit `unknown` handling, and
separate finality states. No live canary before Gate A and B pass.

## Gate C: PumpAPI delivery (P0, 4-7 working days)

1. Audit raw, observation, and normalized schemas; add fixtures for multiple
   events in one signature and prove `event_id` uniqueness.
2. Define a versioned envelope while keeping the current v1 consumer working.
   Include source event time, receive time, event index, protocol, pool, raw
   hash, and completeness status.
3. Implement a bounded normalized delivery log, consumer cursor, gap alarm,
   and idempotent replay. Simulate slow subscriber, Bridge restart, bot
   restart, and full disk in a non-production directory.
4. Document any PumpAPI outage window that cannot be replayed from upstream;
   restrict affected Live follow rules instead of claiming lossless delivery.

Completion proof: fixture tests and a 24-hour shadow soak with measured gaps,
disk growth, and recovery time. No direct production spool replacement until
the replay format is compatible with existing subscribers.

## Modularization work queue (V1.1 sections 5.2, 5.5, and Phase 4)

The directory tree in V1.1 section 5.2 is the target for Trade Service, not
an optional replacement architecture. Migrate one tested boundary at a time;
do not create empty protocol implementations or combine an SDK upgrade with
this refactor. These tasks are planned, not completed or deployed.

| ID | Depends on | Task and target | Acceptance |
| --- | --- | --- | --- |
| M01 | Existing test baseline | Add behavior tests for public SDK entry points and API request/response fixtures | Current DTO fields, status codes, amount units, and supported protocol behavior captured |
| M02 | M01 | Define stable route/trade/quote types in `domain/`; preserve public DTO serialization | No SDK-specific type leaks into new public contracts; API fixtures unchanged |
| M03 | M02, capability audit | Extract parsing and capabilities into `protocol/registry.rs` | Alias and capability regression tests; unsupported protocols remain closed |
| M04 | M02-M03 | Extract pool resolution to `application/pool_resolver.rs` and SDK/RPC adapters | Verified owner/mints, fallback, cache, and timeout behavior unchanged |
| M05 | M04 | Extract quote and amount conversion to `application/quote_service.rs` | Exact raw units, minimum output, quote asset, and rounding tests pass |
| M06 | M05, submission safety tests | Separate build/simulate/submit inside `adapters/sdk/sol_trade_sdk.rs` | Reservation still precedes broadcast; simulation cannot broadcast; existing backend remains default |
| M07 | M06 | Wrap existing SDK and PumpAPI implementations behind a capability-aware backend interface | Equivalent existing behavior; no forced common quote semantics; Router backend stays experimental/off |
| M08 | M07 | Move orchestration to `application/` and handlers to `api/handlers/`; register routes in `api/routes.rs` | HTTP methods, paths, auth, fields, and error behavior pass contract tests |
| M09 | M08, security design | Extract middleware and cache/storage/metrics into `api/middleware/` and `infrastructure/` | Auth checks cannot be bypassed; persistence ownership is unambiguous; no simultaneous credential migration |
| M10 | M01-M09 | Remove replaced branches only after protocol regression | Per-protocol regression evidence, build identity, rollback artifact; no empty directories counted as support |

`adapters/persistence/` holds domain-facing repository implementations;
`infrastructure/persistence.rs` owns SQLite connection/migration wiring. Do
not duplicate durable reservation ownership between them. Actual source
moves must follow the responsibilities proven during extraction.

M01 and capability audits can proceed offline while backup or cloud checks
wait for external prerequisites. Production rollout of these changes still
requires the P0 gates. Full-protocol expansion follows capability calibration
and the tested extraction work, not directory creation alone.

## Gate D: Protocol evidence (P0/P1, per protocol 3-7 working days)

1. Freeze the current Live capability matrix from actual API behavior, SDK
   version, verified pool owners, token programs, quote assets, and fixtures.
2. Run DBC ExactIn buy/sell regression first; keep migrated, stale, hook, and
   unsupported pools fail-closed. Reconcile Token-2022 no-hook documentation
   with capability responses before widening support.
3. Stage CLMM, DLMM, Whirlpool, LaunchLab, and DAMM V1 one at a time:
   verified pool -> deterministic quote -> no-broadcast buy/sell simulation ->
   guarded canary -> rollback record. SDK builder availability is not an
   acceptance test.
4. Keep Router SDK in the isolated probe until a single allowlisted route
   shows measurable benefit without changing the production SDK contract.

## Gate E: Release and observation (P0/P1, 2-4 working days per release)

Build and test each service separately; preserve the previous binary and
database snapshot. Roll out one service at a time, check `/health`, `/ready`,
Bridge connection, Telegram response, read-only order lookup, disk usage,
and restart count. Use a small Live canary only after all P0 gates pass and
the user explicitly authorizes that transaction. Roll back binaries without
deleting `unknown` intents or replaying them under new keys.

## Immediate queue

1. **In progress:** local Copy Trading duplicate-key conflict check, a
   file-backed restart test, unavailable-readiness, pre-submit DB failure,
   and remote-accept/result-write-failure tests. The reconciler recovers the
   original order with the same key. 59 offline tests pass twice. A real
   process-crash test and fast-follow-specific retry injection remain. A
   pre-existing asynchronous test race
   was fixed in test code. Running binary predates these follow-up changes.
2. **Next:** use `BACKUP_RESTORE_CHECKLIST.md` to perform an isolated
   restore on storage with sufficient space. This host has about 4.4 GiB free,
   so it cannot safely hold a second 17 GiB Copy Trading database.
3. **Next:** finish route-by-route authorization and aggregator reservation
   audits. Verify cloud-side access controls before any network change.
4. **Then:** use `BRIDGE_AUDIT.md` to build a compatible v2 envelope and
   replay prototype, followed by DBC fixture work. Maintain protocol-specific
   acceptance records.
5. **Then:** execute M01-M10 in dependency order before broadening protocol
   support. Start M01/API fixtures during external backup/security waits.

## Continuation rules

- Next runnable item: fast-follow-specific retry fault injection, followed
  by a controlled process-crash/recovery test using fake HTTP and temporary
  SQLite only. Then complete the independent aggregator reservation design
  and tests, plus read-only signature reconciliation.
- After each item, record affected files, exact test command/result, pending
  acceptance, and whether it is deployed. Continue to the next unblocked
  item without asking for confirmation at every small step.
- If off-host capacity or cloud-control evidence is unavailable, mark that
  item waiting and continue offline tests, API fixtures, and capability
  audits. Do not bypass its release gate or indefinitely add unrelated tests.
- Stop for explicit authorization before a real transaction, secret export
  or rotation, production database replacement/compaction, network-access
  change, or an unapproved service deployment. A prior service-start request
  is not blanket approval for future production changes.
- A unit-test pass or source edit is not a release. Do not describe this
  queue as an unattended background job unless such a job has actually been
  started; the document is the durable handoff for subsequent work sessions.
