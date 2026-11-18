# SDK And Quote Submission Signature Reads

Date: 2026-11-18, Asia/Shanghai. Continues original NP-03 / TS-07/09, without
a new task or denominator. Source candidate; independent offline Rust CI required.
Accepted parent 259b2fe / Actions 44217059338 is not evidence for this source.
No SDK upgrade, release build, deployment/restart, production data or real trade.

## Actual Paths

LaneExecution::observe keeps its original Default/wait-confirmed/simulate/zero-
budget skip conditions. Only after that guard it snapshots the same selected
lane's RPC URL and passes the already-held shared runtime to the keyless
observe_sdk_signature_shared. No raw execution RpcClient is retained by that
observer, no endpoint selection/fallback, provider change or signer access.

SdkQuoteSubmission::lookup_signature now delegates to status::lookup_selected.
That helper projects only the original first two RPC URLs, preserving positions,
then drops every captured RPC Arc before shared I/O. It uses the same borrowed
runtime already used by this adapter's preflight. The accepted Quote status
profile supplies unchanged missing/error/terminal-parser policy, per-node budgets,
strict transport bounds and shared cooldown. Wallet/default selection, prepare,
sign, broadcast, message verification, Application/journal and API do not change.

The original observe_sdk_signature body and all seven old observer test bodies
remain exact under cfg(test). RpcSignatureStatusReader's complete original loop,
parser and tests also remain exact after cfg/import wiring: the raw loop is now
test-only. Production still uses its original observation parser, not stricter
WSOL rules. Existing read runtime/profile/transport bodies are unchanged; the
old Quote fixture only exposes its existing capacity assertion to new tests.

## SDK Budget Policy

Zero budget returns None before signature/endpoint/quota/I/O work. Keep the
original captured single node, original signature/history-only request, 100 ms
local missing-result polling delay, explicit commitment/error mapping and SDK-
specific zero-transaction-slot rejection. Non-null on-chain errors are observations,
not transport failures or permission to submit again. No observation means only
not visible; there is no new finality or claim that the transaction failed.

The original total budget still includes all I/O and waiting; timeout returns
None. Each shared query remains at most one second of node I/O after pacing and
is also bounded by the existing shared operation limit. A shorter caller budget
cancels it first. An exhausted node budget while the caller still has time is
an error, as before. Check elapsed wall-clock before each poll and after the
await: a synchronously late ready response cannot be accepted past this caller's
budget. Duration comparison avoids adding an Instant overflow or a new maximum
configuration value; a huge finite budget still accepts an immediate result.

The shared Observation class has always required one-second request starts.
Consequently the old 100 ms network polling frequency is deliberately not
preserved: a 900 ms window may now return None after one null where the old
isolated client could query twice. Do not extend MEV_SUBMIT_VERIFY_MS, invent a
new class, bypass pacing or alter production configuration to hide this change.
Quota/admission/protocol errors retain the existing uncertain-result route.
The same original signature must remain attached; no new lane/key/re-sign/resend.

Quote submission observation intentionally retains the generic Quote parser's
zero-slot/optional-field policy; SDK's extra zero-slot check does not leak into
it. Original caller reconciliation/validation still decides whether an observation
is sufficient. Transport error detail sanitization is the accepted shared-profile
policy, not exact equivalence to arbitrary raw provider error text.

## Shared Ownership And Type Boundary

Both paths reuse the same runtime/HTTP, Observation permit, one-second starts,
64 KiB response bounds, original endpoint registry and cross-profile 429 cooldown.
No thirteenth class, new HTTP client, worker, retry or redirect. The concrete
SharedReadRuntime itself joins the three-layer disallowed-types list, closing a
direct concrete-network-resource field omission without changing runtime behavior.
Expected 50 selected types / 153 scoped field rejections including outside alias.
This selected-type/compiler gate is not proof that all opaque capabilities or
all provider/SDK-internal resource use are governed.

## Required Acceptance

Twenty-four new Rust names: SDK shared/oracle 12, Quote submission status 6,
actual lane-controller contracts with fake invoke 3, real loopback wire 3. Full
concurrent/serial each 1874 must retain all 1850 parent names and add exactly
these twenty-four. Four focused steps make 93 total. Original/restored compiler
checks and all 153 raw field spans must pass independently on this source.

Compare exact original observation fields and request params across finalities,
on-chain errors and semantic failures. Cover repeated null, short/zero/huge
budgets, node vs caller expiry, late ready success/error, I/O/pacing cancellation,
shared quota and no alternate key. Actual controller uses fake invoke only but
real readonly parsing/timeout: missing, fault, chain error and cancellation must
retain Unknown/original signature and never prepare/invoke the second lane.
The valid receipt remains Submitted, not a new finality claim.

Quote adapter tests cover original two-node/oracle policy, dropping execution
RPCs before I/O, terminal semantic error, the actual disabled-policy/empty-wallet
trait path without a signer, shared class and the generic parser's distinct policy.
Loopback uses the fixed production HTTP client and exact params/body limits,
bad identity/status/schema/redirect/429, caller budgets/cancel and hash progress.
All fixtures are public synthetic data; no SDK signer initializer, wallet key,
real-node access, transaction signing or broadcasting.

Use only authorized source-only offline/loopback CI. No local service Cargo retry
after the recorded 2200 MiB failure. Audit raw names/spans, source blobs/modes,
identity/dependency hashes and original bodies before acceptance; no terminal
rerun or documentation-only CI.

## Remaining Original Work

DAMM and other opaque SDK reads/tasks/confirmers, provider-wide quotas and
NP-01/03/04/05/rollout gates remain. Manifests/lock/vendor/migrations, API/amounts/
wallet identity and production state stay unchanged. PumpAPI new work stays paused.
