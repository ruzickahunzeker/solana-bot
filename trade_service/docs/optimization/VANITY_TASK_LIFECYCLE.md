# Vanity Task Ownership And Shutdown

Date: 2026-11-18, Asia/Shanghai. Continues the existing NP-03 independent-task
resource item. This is not a new feature, task number, supervisor for every SDK
task, or authorization to generate production wallets. Production wiring and
read-only original-body audit and independent acceptance are complete in 3d72a14 /
Actions 44245588737 / artifact 14403371260 (details below). No deployment.

## Actual Gap

The original WalletOps creates one semaphore and the original per-job runtime
map. Its creation path persists the public job, inserts that runtime and drops
the tokio::spawn handle. The generation function separately awaits spawn_blocking.
main currently closes/drains ordinary TradeTasks and existing observation,
blockhash and shadow resources, not vanity work. HTTP graceful completion alone
does not show that a previously accepted vanity job or its blocking worker ended.

## Intended Wiring

- Add one private lifecycle owner to the existing WalletOps, not another worker,
  key store, database, generation queue, semaphore or global runtime.
- Hold an admission read guard before the original repository await until the
  original job is dispatched. Closing intake rejects later creation before a
  repository write; an already-admitted request still follows its original
  persistence flow and is included in shutdown.
- Keep the same JoinSet in the owner. Reap finished handles on later intake and
  join all original tasks on shutdown. Preserve panic evidence after reaping.
  Timeout/cancelling drain drops only the wait, not the handles; later drain can
  join the same work. Never abort a wrapper and claim its blocking worker stopped.
- On stop, close admission and set the existing cooperative cancellation flag
  for live runtimes. Recheck closure after insertion to cover admission paused in
  the repository before the stop sweep. Keep the original cancellation cadence,
  single-slot queue and generation/import/activation body unchanged.
- Bootstrap hands the same worker Arc to its existing Application service and
  main's shutdown owner. Signal closes both existing ordinary intake and vanity
  intake. main awaits vanity drain alongside the existing cleanup, using the
  existing task-drain timeout; both results are obtained before propagating an
  error. No new configuration field or production value is introduced.

## Evidence Required

Eight owner tests cover closed intake, an admission paused before spawn, dropped
admission, timeout with the same async task, blocking-work join, cancelled drain,
panic retention and reaping completed handles. Four actual adapter tests use
only a fresh in-memory SQLite pool and empty registry: rejection without a row,
queued cancellation behind the original held permit, held-database admission
crossing stop, and dropping a held-database create before dispatch. All actual
worker tests stop before its original generation branch; no keypair is created
or imported. A bootstrap test must prove the original worker Arc handoff and
shared intake closure, without initializing an SDK client or external node.

Expected acceptance is complete concurrent/serial each 1939, retaining all 1926
accepted-parent names and adding exactly thirteen: owner8, adapter4 and bootstrap1.
Two focused steps select owner8/shutdown5 (the latter includes bootstrap). All
101 steps and the unchanged 55-type/168-field gate require independent execution;
do not inherit the accepted parent's result. Local formatting, policy12,
workflow7, SDK provenance and one-positive/nine-negative source audits pass.

Preserve all existing test names, production wallet API/DTO/auth mappings,
repository/registry bytes and the entire original run_vanity_job body. Existing
bootstrap release assertions must still execute after dropping the extra owner,
not be deleted. Preserve main/index/frozen history, SDK 6.0.2, manifests/lock/
vendor/migrations and all trading/PumpAPI implementations. Use only authorized
source-only offline/loopback CI; do not retry local service Cargo.

## Explicit Limits And Remaining Original Queue

Cancellation remains cooperative, not atomic. A match/import already in progress
may finish and must be joined; no rollback, requeue or automatic regeneration is
added. Original ignored repository-update errors, startup cancel_requested gap,
owner count/insert race, queue total and key-memory/privacy remain separate
review items. Task completion alone is not durable fsync or crash recovery proof.
Graceful HTTP shutdown still precedes the drain; this does not establish a hard
whole-process deadline or cancellation of independent wallet/transfer operations.

The read-only SDK inventory confirmed that this service already disables startup
WSOL ATA creation and seed optimization. Do not implement another stop mechanism
for that disabled path. The pinned SDK's start_rent_updater returns no handle;
provider ownership differs (Temporal owns its ping stop/handle). Potential public
TradingInfrastructure sharing needs its own original NP-03 compatibility review;
do not fork an SDK initializer or modify vendor to manufacture handles. Remaining
SDK/independent-operation lifecycle, global quotas, NP-01/04/05 and rollout gates
remain open. No service is deployed/restarted and no real RPC/trade is performed.

## Independent Acceptance

Source 3d72a14ebc5df3150fb552d2399a4170b0a899fe / Actions 44245588737 completed
success on its first run. Downloaded artifact 14403371260 was independently
audited before successor wiring or acceptance records changed the snapshot.
Complete concurrent 1939 (333.66 seconds) and serial 1939 (645.91 seconds) have
identical raw name sets, zero failures/ignored/filtered; all 1926 parent names
remain and exactly the thirteen source-defined tests are added. Focused owner8/
shutdown5, bootstrap5, original service-shutdown8/shadow-shutdown4 and all 101
steps pass. Original/restored compiler checks and all 168 distinct raw field
spans verify the unchanged 55-type policy plus outside alias.

All 1152 source blobs/modes (13,912,894 bytes) match tree
38c601a9cfbc918f2f0e8a1aa608b2574f95e129; identity/dependency hashes and original
generation/import/activation body, same-Arc bootstrap, old release assertions and
original cleanup reconstruct exactly. One-positive/nine-negative source audit,
Python policy12/workflow7/provenance pass. SDK6.0.2, main/index/frozen history and
vendor/lock/manifests/migrations remain. No local service Cargo retry, terminal
rerun, docs-only CI or production action. Earlier expected/pending language is
historical source intent, not a new queue. Next original NP-03 path is native SOL
transfer preparation reads; see SOL_TRANSFER_PREPARATION_SHARED_READS.md. Other
SDK/independent-operation lifecycle, quotas and original rollout gates remain.
