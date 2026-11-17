# WSOL Status Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues the original NP-03 / TS-07/09 queue,
not a new task or completion denominator. Source and independent offline Rust CI
accepted, not deployed: ec608c0 / Actions 44188954352 / artifact 14377382655.
Parent c196d1b evidence is not reused for this batch.

## Ownership And Compatibility

SdkWsolQuerySource still parses the signature before resolving the wallet, then
selects only the first original Default RPC. It projects that endpoint into
WsolSignatureReader alongside the same bootstrap SharedReadRuntime Arc and the
bound original Signature. The production reader no longer retains an execution
RpcClient, wallet registry, repository, key, SDK lane or signing/sending port.
No new HTTP client, cache, worker or per-reader semaphore is created.

Reuse the existing Observation class: one active request and one second between
starts, shared with ordinary signature observations. Do not add a thirteenth
class, alter the twelve-class capacities or weaken other intervals. The complete
lookup, including pacing, I/O and decoding, has a two-second deadline; also check
wall time after decoding so late executor polling cannot return valid evidence.
Cancellation drops local I/O and releases the original observation permit. It
does not cancel remote processing. A later explicit status lookup is allowed;
there is no background retry, provider switch or transaction resubmission.

Only getSignatureStatuses is available, for exactly one original signature with
searchTransactionHistory=true. No commitment parameter is invented. Runtime-wide
request IDs, registered endpoints, shared HTTP, 429 cooldown, no redirect/retry
and declared/streamed 64 KiB response bounds are reused. Invalid HTTP/JSON/identity,
result+error, missing result, transport errors or oversized responses fail closed
without echoing raw provider diagnostics or credentials.

## Original Strict Status Policy

Preserve exactly one result and a nonzero context slot, including missing status.
Retain legacy-status versus explicit-err consistency, required explicit
confirmationStatus, and finalized iff confirmations is None. Retain the original
TransactionError Display string, not the ordinary reader's debug/error prefix.
Use the original WsolSignatureObservation::outcome_for validation for the bound
signature and nonzero transaction slot <= context. Processed or confirmed errors
are not terminal failures. Only finalized evidence is terminal.

The original RpcClient lookup implementation remains test-only in
wsol_status/original.rs. The six pre-existing strict parser/request tests retain
their names and assertions. New comparisons execute the original real RpcClient
path, compare every observation field/outcome, and compare the request witness
without normalizing params, signatures or error text. The existing HTTP/journal
eight-case finality fixture now runs through the new shared production reader.
Application metadata and owner validation, finality CAS, signed bytes, journal,
quote/unsigned preparation/simulation/signing/submission/recovery are unchanged.

## Required Acceptance

Fourteen new test names: eleven wsol_status_shared_ cases and three real loopback
wsol_status_wire_ cases. Cover the original valid/invalid matrix and missing
status, exact history/signature request and unique IDs on later polls, original
resource ownership/drop, unknown endpoint, envelope/size/HTTP/transport failure,
429 shared cooldown, no lane widening, existing observation contention, hash
progress, cancellation/permit release, deadline/late polling. Real HTTP covers
exact and oversized declared/streamed limits, bad ID/JSON/RPC errors, redirects,
slow headers/body and disconnects using the actual fixed reqwest transport.

Expected complete concurrent/serial each 1783, retaining all 1769 parent names
and exactly fourteen source-defined additions. Three focused CI steps make 78
total. Compiler gate adds the concrete WsolSignatureReader: 40 selected types /
123 distinct scoped field rejections including outside alias; original/restored
checks must independently pass. Do not reuse the parent's 120-field evidence.
Verify identity, source blobs/modes, locked dependency/provenance hashes, raw test
name sets and diagnostic spans before marking this batch accepted. No local
service Cargo retry after the recorded 2200 MiB failure. CI remains source-only,
offline with loopback, without wallets, runtime databases or real RPC.

## Accepted Evidence

Source `ec608c08db09981eee27c5b15727e6d660aee6a8`,
[Actions 44188954352](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44188954352),
first run terminal success in 25m23s. Downloaded artifact `14377382655` independently
audited: all 78 steps exit zero; full concurrent 1783 (244.35 seconds) and serial
1783 (447.74 seconds), zero failed/ignored/filtered tests and identical raw name
sets. All 1769 parent names remain and exactly the fourteen source-defined names
are added. Shared/oracle 11, real HTTP 3 and complete status contract 59 pass;
all earlier focused regressions also pass. Original/restored compiler checks
and all 123 distinct raw field spans pass on this source, including the new
concrete reader and outside alias. Python policy 12 / workflow 7 and SDK
provenance pass; identity and manifest/lock/vendor-manifest hashes match.

Before recording acceptance, all 1110 snapshot blobs/modes (13,509,839 bytes)
match the local source. The original RpcClient lookup body is byte-identical
except test-only rpc field visibility; six original test bodies are byte-identical.
The production strict parser matches after whitespace and the sole bound-signature
reference substitution. Original quote estimator/instruction blobs remain as
recorded in WSOL_QUOTE_SHARED_READS.md. Original result fields, exact request
witness without normalization and eight-case HTTP/journal finality matrix execute
in the accepted run. The old serial discovery child-stderr split is associated
only with its exact test/pinned fixture/immediate ok; two positive/seven negative
audit-format regressions pass. Raw logs are not rewritten or names omitted.

Main/index/frozen history, SDK/manifests/lock/vendor/migrations remain unchanged.
No local service Cargo retry, release/deployment, production configuration or
wallet/database access, real RPC or trades. Do not rerun terminal CI or start
documentation-only CI; carry acceptance records with the next actual source batch.

## Remaining Original Gates

The subsequent original WSOL preparation-read entry is now implemented, pending
its own independent CI; see WSOL_PREPARATION_SHARED_READS.md. Its actual wallet
path reuses the existing five-step class and retains original authorization and
temporary-signer ownership. This status batch's acceptance does not cover it.

WSOL unsigned preparation/simulation/sign/submission read resources, standalone
Quote reconciliation status, DAMM/SDK-internal or opaque reads/tasks, provider-wide
quotas and the original NP-01/03/04/05 and rollout gates remain. SDK stays 6.0.2;
manifests/lock/vendor/migrations/API and amounts remain unchanged. No release,
deployment/restart, production configuration/wallet/database access or trades.
New PumpAPI integration remains paused.
