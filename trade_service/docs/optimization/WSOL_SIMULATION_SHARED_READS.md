# WSOL Simulation Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues original NP-03 / TS-07/09; no new task
or denominator. Corrected source and independent offline Rust CI accepted, not
deployed: 9d40936 / Actions 44204140210 / artifact 14385056342. Accepted parent
cf5e63b evidence is not reused for this batch. No production use is implied.

## Actual Paths And Original Policy

Both WsolExecutionWallet simulation methods now delegate to the original
PreparedWsol/SignedWsol wrapper, which projects only its selected original RPC
URL/commitment and artifact into the shared simulation path. It uses the same
bootstrap runtime Arc already injected into the WsolWallet. The prepared/signed
wrappers still retain that original execution RPC for later broadcast; its
ownership and the signing/send implementation are not claimed migrated here.

The root SDK module's original unsigned/signed simulate methods and unsigned
config remain unchanged under cfg(test), alongside all original tests, as an
independent real RpcClient oracle. Preserve unsigned validate(owner, quote,
current_time) before any admission/network action, then Transaction::new_unsigned
with one/two zero signatures. Preserve signed validate_snapshot before admission,
and the exact already-signed transaction, original signature and journal bytes.
No signer is invoked, no signature is changed and no transaction is rebuilt from
quote or RPC response data. Original authorization, preparation, instruction
builder, sign/send bodies, Application ordering, persistence and finality CAS
remain unchanged.

Unsigned program errors retain the exact WsolFailure::Rejected Display text.
Signed program errors remain a returned result: the original execution wrapper
maps err=None to true and err=Some to false. Transport/protocol errors remain
errors, never false success or a fabricated program failure. The existing
Application's signed-persistence-before-simulation, uncertain outcomes and no
automatic resubmission policy remain in force.

## Sealed Packet And Fixed Request

WsolSimulationPacket has private fields and constructors in the existing SDK
module. Construct only after the original authorization/snapshot checks. It owns
only a Base64 packet and original commitment, no key/client/journal. Reject
non-one/two signer counts, mismatched signature count and serialized size >1232
before allocating the encoded packet. The type is not Clone and is consumed by
one runtime request. Reconstructing a later explicit query requires repeating
the original checks; there is no retry or detached worker in this path.

The only request is simulateTransaction. Match the locked client's exact params:
sigVerify=false, replaceRecentBlockhash=false, original commitment, encoding=base64,
accounts=null, minContextSlot=null, innerInstructions=false. No getVersion/hash/
account pre-read, commitment substitution, new bank requirement or signature
verification is invented. Return all original typed simulation-result fields,
including optional units, fees, logs, return data and balance metadata. A missing
units value stays None, a real zero stays zero, raw u64 is not rounded. Metadata
such as replacementBlockhash is returned as before but never applied to the
captured transaction; this is not a new simulation-output or bank-evidence policy.

Signed simulation necessarily transmits a replayable signed packet to the same
configured RPC as the original implementation. The runtime has no sendTransaction
method, but this is not a claim that an untrusted simulation provider cannot
rebroadcast a received signed packet. No new endpoint or signing authority is
introduced. Never log the packet/credentials. Offline signed fixtures use dummy
signatures, not real wallet keys, transaction signing or real-node access. Neither
local snapshot validation nor sigVerify=false is cryptographic signature proof.

## Shared Resource And Failure Bounds

Reuse existing AggregatorSimulation quota: one active simulation and 100 ms starts,
shared across both WSOL forms and aggregator baseline/simulation. No thirteenth
class, HTTP client, semaphore, cache, worker or interval/capacity change. Admission
is try-only; busy is an unavailable error, not a new queue. Critical hash capacity
is separate. The same endpoint registry, HTTP client, no retry/redirect/fallback,
shared 429 cooldown and pacing apply. The whole request, pacing/I/O/typed decoding,
has a two-second deadline and an explicit post-decode wall-clock check. Drop
cancels local I/O/releases its permit, not remote processing.

Declared and streamed response bodies are capped at 512 KiB. Require typed
JSON-RPC v2/request ID/exclusive result with no error, a present result and nonzero
context slot. Require the value object to contain explicit err (null is valid),
rather than treating an omitted error outcome as success through optional-field
deserialization. This deliberately rejects malformed provider replies tolerated
by the old client; conforming response/error semantics remain unchanged.
Bad status/identity/schema/context/JSON/body or transport/deadline
failure returns an error without echoing raw provider messages, URLs or packets.

## Required Acceptance

Eighteen new Rust names: fourteen wsol_simulation_shared_, three real loopback
wsol_simulation_wire_, one wsol_simulation_adapter_. Exact no-normalization params
and all typed result fields must match the original real RpcClient path using
the same unsigned or dummy-signed artifact across processed/confirmed/finalized
and one/two-signature cases. Cover unsigned wrap new/existing ATA/full/partial
unwrap, unchanged signed bytes, original error mapping and validation-before-I/O.
The actual PreparedWsol/SignedWsol helpers must use the selected shared transport
while the original execution RpcClient records zero requests.

Cover malformed/tampered/expired inputs, packet size/count, rich optional/raw-u64
result preservation, missing units, bad envelope/schema/body/context/status,
unknown endpoint/busy quota, shared 429, original commitment and pacing, cancel/
timeouts/late polling on both forms, critical hash progress and exact twelve-class
capacity release. Real HTTP tests use the fixed reqwest client for declared/
streamed exact and oversized limits, redirects/disconnect/slow headers/body,
both signature forms and cancellation without hidden retry or alternate lanes.
The quote-balance fake adds an explicit panic for the new request variant; it
does not silently allow simulation in a balance-only fixture.

Expected full concurrent/serial each 1817, all 1799 parent names retained and
exactly eighteen source-defined additions. Three focused steps make 84 total.
Compiler gate adds WsolSimulationPacket, SignedWsolTransaction and SignedWsol:
45 selected types / 138 scoped field rejections including outside alias, with
independent original/restored checks and raw spans. Verify source blobs/modes,
identity, manifest/lock/vendor hashes and original oracle/body invariants before
acceptance. Do not reuse parent's 129-field run. No local service Cargo retry
after the recorded 2200 MiB failure: source-only offline/loopback-only CI only.

## Accepted Evidence

Corrected source `9d4093617be00634c2e05a453daf9ff6e0bb1bb9`,
[Actions 44204140210](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44204140210),
its first run terminal success in 28m35s; downloaded artifact `14385056342`
independently audited. All 84 steps exit zero. Full concurrent 1817 (270.55
seconds) and serial 1817 (504.81 seconds), zero failed/ignored/filtered tests,
identical raw name sets. All 1799 accepted-parent names remain and exactly the
eighteen source-defined names are added. Shared/oracle 14 / real HTTP 3 / actual
adapter 1 and all earlier focused regressions pass, including status contract 59.
Original/restored compiler checks and all 138 distinct raw field spans pass on
this source: 45 selected concrete types plus outside alias. Python policy 12 /
workflow 7 and SDK provenance pass; identity and manifest/lock/vendor-manifest
hashes match.

Before recording acceptance all 1119 snapshot blobs/modes (13,609,187 bytes)
match the source. The entire original root SDK module, including unsigned/signed
simulate and config oracles, original tests, validation/sign/send bodies, matches
after removing only the new module registration and test-only cfg/import wiring.
Authorization, preparation/oracles, instruction builder, Application journal/state
flow and original runtime capacities/intervals remain exact. Both real wrappers
actually use shared transport while original execution-RPC counters remain zero.
Exact no-normalization request and full-result witnesses execute, signed bytes
stay unchanged, and explicit err/missing-units/raw-u64 and original permissive
missing-err behavior are independently exercised. Missing err fails closed in both
forms without changing conforming response semantics or old program-error mapping.

The known old serial discovery child-stderr split is associated only with its
exact test/pinned fixture/immediate ok; two positive/seven negative audit-format
regressions pass. No raw logs are rewritten or test names omitted. Main/index/
frozen history remain unchanged. No local service Cargo retry, release build,
deployment/restart, production data access, real RPC or trade. Do not rerun terminal
CI or trigger documentation-only CI; carry records with the next actual source batch.

## Candidate History

First candidate 2a1cc27 / Actions 44202635009 was cancelled during validation after
review found missing err could deserialize to None and imply success. The corrected
source requires explicit err and tests both forms plus the old client's permissive
behavior. Do not restart that candidate or treat partial logs as acceptance. Only
the corrected source's independently audited full run can accept this batch.

Corrected-outcome candidate 33cece1 / Actions 44203291583 failed test type-check:
the SDK fixture attempted to construct RPC-private HttpReply fields (E0451).
The ten preceding checks passed; no Rust test/negative field gate ran. The fixture
now uses the existing cfg(test) HttpReply::for_test API, retaining private fields
and production code. The core fake uses the existing default Retry-After policy;
the real wire fixture still verifies an explicit header. Do not rerun that
terminal candidate or count its partial checks as this batch's acceptance.

## Remaining Original Gates

Actual WSOL broadcast RPC ownership/SDK internals, standalone Quote reconciliation
status, DAMM/opaque reads/tasks, provider-wide quotas and NP-01/03/04/05/rollout
gates remain. SDK stays 6.0.2; manifests/lock/vendor/migrations/API/amounts and
production configuration, wallets/databases, binaries and services are untouched.
No release build, deployment/restart, real RPC or trade. New PumpAPI integration
remains paused.
