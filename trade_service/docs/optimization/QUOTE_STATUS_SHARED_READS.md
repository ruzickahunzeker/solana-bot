# Standalone Quote Status Reads

Date: 2026-11-18, Asia/Shanghai. Continues original NP-03 / TS-07/09, without a
new task or denominator. Source candidate; independent Rust CI still required.
Accepted parent d1d9038 / Actions 44210742181 is not acceptance for this source.
No deployment, production wallet/database access, real RPC or transaction.

## Actual Scope

Bootstrap gives the existing shared-read Arc to SdkQuoteReconciliationRuntime.
Its resolved wallet retains the same already-loaded SDK wallet and that Arc.
At the original reader-creation point, QuoteStatusReader::from_selected projects
only the first two original RPC URLs, in order and without deduplicating their
positions. RpcQuoteStatusReader retains those strings and the same runtime, not
an execution RpcClient or signer. No subsequent wallet/lane resolution, cache,
HTTP client, worker, configuration or signing capability is added.

The original RpcSignatureStatusReader, its complete file and tests remain exact.
It is the independent RpcClient oracle and still serves other existing SDK paths;
this batch does not claim those opaque/internal observation paths migrated.
Application reconciliation, original three-second outer deadline, persistence
view, original signature/CAS and HTTP response/status rules remain unchanged.
Neither lookup nor a failure may submit, re-sign or recover submission ownership.

## Original Result Policy

Parse the persisted signature before quota/network access, retaining the original
invalid-signature and empty-lane errors. Query no more than the original two
positions with exactly getSignatureStatuses([signature], searchTransactionHistory=true).
Return the first parsed non-null status through the unchanged original
signature_status::observation function. Its semantic error is terminal, not a
reason to consult a second node. A valid null on either node still wins over a
later transport/invalid-count failure and returns None. With no valid null or
status, retain the last attempted-node error. Count and timeout diagnostics and
the on-chain error Debug prefix remain original.

Do not substitute ordinary/WSOL stricter parsers. Context/transaction slot zero,
context smaller than transaction slot, finalized with non-null confirmations,
and the pinned serde optional-err behavior remain original, not invented finality
evidence. Explicit confirmationStatus and consistent status/err are still required
by the original observation function. Typed decoding errors remain eligible for
fallback, unlike that function's terminal semantic rejection.

JSON-RPC v2/ID/exclusive outcome, typed response shape and 64 KiB body limits are
now checked by the shared transport/decoder. Wrong identity cannot masquerade as
a missing signature. New quota/admission/HTTP/protocol errors use bounded public
diagnostics (HTTP or JSON-RPC code only), not the SDK transport's raw provider
message, URL or arbitrary response body. This is a deliberate error-detail
sanitization change, not a promise that arbitrary provider error strings match.

## Pacing And Resource Ownership

Reuse the one existing Observation permit for the entire at-most-two-node lookup,
shared with ordinary and WSOL observation. Shared starts remain at least one
second apart; there is no thirteenth class, capacity/interval change, extra queue
or HTTP retry. A busy class fails promptly without consulting another profile.
Hash capacity remains separate. Existing registered-endpoint normalization and
429 cooldown (default 30 seconds, bounded 1..300) remain shared across profiles;
different credentials stay distinct, and no globally configured third node is
silently used when the selected two fail.

Split the existing private read implementation into start_read and finish_read.
Its original admission, pacing, transport, cooldown and post-response bodies are
retained; the finish step additionally checks deadline immediately before I/O.
Existing callers still pass one absolute deadline through both helpers. Only
this new Quote profile starts each node's one-second I/O/decode budget after
shared pacing, bounded by a three-second operation deadline and the original
Application timeout. Otherwise a fast primary null would use almost the entire
backup budget waiting for the shared one-second interval. No backup is sent
after expiry, no late positive response is accepted, and cancellation releases
the permit without refunding an already-used start or undoing remote work.

## Required Acceptance

Nineteen new Rust names: shared/oracle 14, real-loopback wire 3 and actual adapter
2. Full concurrent/serial each 1850 must retain all 1831 accepted-parent names
and add exactly those nineteen. Three focused steps make 89 total. Compiler gate
adds RpcQuoteStatusReader and the concrete QuoteStatusReader wrapper: 49 selected
types / 150 scoped field rejections including outside alias. Independent original
and restored checks and raw diagnostic spans are required on this exact source.

Compare exact original request params and observations across all explicit
finalities, on-chain errors, zero/raw-u64 slots and original permissive fields.
Cover terminal parser errors vs transport fallback, missing/error priority,
two-position/alias/unknown-node selection, class contention and critical hash
progress, fresh backup budget after a fast null or primary timeout, late polling,
unpolled/I/O/pacing cancellation, shared cooldown and all original capacities.
Actual adapter must snapshot the original two URLs, release all execution RPCs,
make zero original-client requests and retain/release the original runtime Arc.
Real HTTP covers exact params, declared/streamed limits, malformed/redirect/429
fallback without retry/third node, one-second timeouts and both cooldown directions.
Correlation IDs belong to the runtime, not each old client; they are validated,
not claimed equal to the old per-client counter.

Use only authorized source-only offline/loopback CI. No local service Cargo retry
after the recorded 2200 MiB failure. Independently audit complete source blobs,
identity/dependency hashes, raw names/spans and preserved original code before
acceptance. No docs-only CI or rerunning terminal runs.

## Remaining Original Gates

Other SDK-internal/DAMM reads/tasks, original SDK/Quote send observers, provider-wide
quotas and NP-01/03/04/05/rollout gates remain. SDK stays 6.0.2; manifests/lock/
vendor/migrations/API/amounts/wallet identity and production state are unchanged.
New PumpAPI integration stays paused. This is not all-protocol or live acceptance.
