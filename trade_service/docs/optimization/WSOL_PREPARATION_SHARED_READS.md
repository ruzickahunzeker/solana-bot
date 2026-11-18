# WSOL Preparation Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues original NP-03 / TS-07/09, not a new
task or denominator. Source and independent offline Rust CI accepted, not deployed:
cf5e63b / Actions 44195580476 / artifact 14381545057. Parent ec608c0 evidence is
not reused for this batch. No production use or live acceptance is implied.

## Actual Execution Path

Bootstrap passes the same original SharedReadRuntime Arc into SdkWsolRuntime.
Resolved WsolWallet retains that Arc alongside its original SDK client. The
existing WsolExecutionWallet::prepare selects the same first Default RPC and
passes only its URL/commitment, wallet public key and original quote to
sdk_client/wsol_conversion/preparation.rs. Both unsigned-simulation preparation
and live preparation use this real path, not an optional probe or empty backend.

The prepared wrapper still retains its original execution RpcClient for later
simulation and signed submission, deliberately unchanged in this batch. Do not
claim those capabilities migrated or that the execution wallet is read-only.
No wallet signer is passed to the read scope, but the original partial-unwrap
builder still creates and retains its ephemeral temporary Keypair in the
UnsignedWsolTransaction. It is not discarded/recreated before signing, exported
to the read scope, or presented as a key-free operation or zeroization proof.

## Reused Resources And Fresh Evidence

Reuse the already accepted WsolQuoteReadSession and WsolQuote runtime class for
these identical five public reads. No thirteenth class, HTTP client, semaphore,
cache or background task is added. The existing twelve capacities and intervals
stay byte-identical. Quote estimation and preparation share the single permit
and 100 ms start pacing; busy admission fails unavailable, not a hidden queue,
retry, provider switch, signature or submission. Critical hash capacity remains
independent. A complete operation owns its scope until success/error/cancellation.

Every preparation opens a fresh session: getBalance at original lane commitment,
processed/base64 derived WSOL ATA, rent for 165 bytes without an added commitment,
fresh original-commitment blockhash, then fee for the exact public message with
that hash and original commitment. No quote-time account/hash/fee value is reused.
The existing once-only order, ten-second total/two-second step deadline, registered
endpoint/429 cooldown, no retry/redirect/fallback, declared/streamed 64 KiB bounds
and typed metadata/hash/message/fee validation apply unchanged. Cross-slot and
cross-commitment behavior remains the old preparation semantics, not an atomic
bank snapshot or a new quote authorization.

## Preserved Policy And Ownership

The original RpcClient prepare_unsigned_wsol_transaction remains unchanged under
cfg(test) for its old tests and independent oracle. New preparation differs only
in fresh scope admission and replacing its five RPC reads; preserve order and
original errors for owner, ATA presence/program/mint/native/unpack, current source
balance, source rent refund, temporary rent, fee ceiling and checked SOL budget.
Preserve raw u64 units, saturation for source rent, fee/rent/reserve arithmetic,
and the requirement for actual SOL even when unwrapping WSOL. Null fee fails;
explicit zero is retained and cannot be invented from missing metadata.

The original instruction builder, WsolAuthorization, quote snapshot, canonical
message, signer count/temporary-account ownership, into_signing_parts validation,
unsigned/signed simulation, signer/send implementation and application/journal/
reservation/CAS ordering remain unchanged. A failed or cancelled read returns
no prepared candidate and is not permission to change an idempotency key or
retry an uncertain signed order. Preparation itself never invokes simulation,
signing, sending, quote saving or journal mutation.

## Required Acceptance

Sixteen new names: twelve wsol_preparation_shared_, three real loopback
wsol_preparation_wire_, one wsol_preparation_ownership_. Compare all authorization
fields, public messages and all five RPC request witnesses against the original
real RpcClient path for missing/existing-ATA wrap and full/partial unwrap. Normalize
only base64+zstd request encoding to base64 and the original random partial
temporary public key; never normalize hash, amounts, instructions, fee/rent or
commitments. Validate original and new candidates against original authorization
and confirm the retained temporary signer matches its own message before any
normalization. No wallet key or transaction signing/broadcast is used.

Cover original early-stop/error kinds and read counts, fee/rent/ATA/source changes,
exact reserve/overflow/u64 budgets, null versus explicit zero, fresh post-quote
requests, intent/message/signer tampering, every-stage 429/transport/size/identity
failure and cancellation, unknown endpoint/busy admission, first/last timeout and
late polling, original ownership/drop, and hash progress. Actual HTTP uses the
fixed reqwest transport and shared fixture to test both body limits, redirects,
disconnects, slow headers/body and cancellation without retries or fallback.

Expected complete concurrent/serial each 1799: all 1783 parent names retained,
exactly sixteen source-defined additions. Three focused steps make 81 total.
Compiler gate adds UnsignedWsolTransaction and PreparedWsol: 42 selected concrete
types / 129 scoped field rejections including outside alias; original/restored
checks and raw spans must independently pass. Parent 123-field evidence is not
reused. Check source blobs/modes, identity and manifest/lock/vendor provenance.
No local service Cargo retry after the recorded 2200 MiB failure; CI is source-only,
offline with loopback and no production wallet/configuration/database inputs.

## Accepted Evidence

Source `cf5e63bca191362bf7f234af942b63f73ffc8e16`,
[Actions 44195580476](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44195580476),
first run terminal success in 27m22s; downloaded artifact `14381545057` independently
audited. All 81 steps exit zero. Full concurrent 1799 (263.49 seconds) and serial
1799 (485.64 seconds) have zero failed/ignored/filtered tests and identical raw
name sets; all 1783 parent names remain, exactly the sixteen source-defined names
are added. Shared/oracle 12 / real HTTP 3 / ownership 1 and all prior focused
regressions pass, including original status contract 59. Original/restored compiler
checks and all 129 distinct raw field spans pass on this source: 42 selected
concrete types plus outside alias. Python policy 12 / workflow 7 and SDK provenance
pass; identity and manifest/lock/vendor-manifest hashes match.

Before recording acceptance all 1114 snapshot blobs/modes (13,553,143 bytes) match
the local source. The original root SDK module is byte-identical after removing
only the new module registration and three test-only cfg attributes; this covers
the original prepare oracle/tests and all validation/sign/simulate/send bodies.
The new preparation body matches the original after only the fresh scope and
five typed-read substitutions. Authorization/instructions/old estimator and its
tests, read runtime/budgets, quote session and original scope tests remain exact.
Every authorization field, normalized public message and five-request witness
comparison actually ran; each artifact's own temporary signer binding is checked
before normalization. The known old serial discovery child-stderr split is
associated only with its exact name/pinned fixture/immediate ok; two positive/
seven negative audit-format regressions pass. No raw logs are rewritten or names
omitted. All main/index/frozen-history invariants remain unchanged.

No local service Cargo retry, release build, deployment/restart, production data
access, real RPC or trades. Do not rerun terminal CI or trigger documentation-only
CI; carry acceptance records with the next actual source batch.

## Remaining Original Gates

The subsequent original unsigned/signed simulation entry is independently accepted
in 9d40936 / Actions 44204140210 / artifact 14385056342: concurrent/serial each
1817, parent 1799 retained, exactly eighteen added, all 138 fields/84 steps pass.
See WSOL_SIMULATION_SHARED_READS.md. Both wrappers
use the original shared simulation quota with original validation/request/error
policies. This preparation batch's accepted run was not reused for that source.

WSOL unsigned/signed simulation and submission RPC ownership, standalone Quote
reconciliation status, DAMM/SDK-internal/opaque tasks, provider-wide quotas and
original NP-01/03/04/05/rollout gates remain. SDK stays 6.0.2; no manifest/lock/vendor/
migration/API/amount changes, release build, deployment/restart or production data
access. No real RPC or trade. New PumpAPI integration remains paused.
