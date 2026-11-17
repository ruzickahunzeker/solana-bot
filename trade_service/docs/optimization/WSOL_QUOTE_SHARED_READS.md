# WSOL Quote Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues original NP-03 / TS-07/09. Source and
independent offline Rust CI accepted, not deployed. Identity: c196d1b / Actions
44182843252 / artifact 14374676338. Parent evidence is not reused. This is quote
estimation only, not conversion submission.

## Resource And Compatibility

Bootstrap passes its existing SharedReadRuntime Arc into SdkWsolQuerySource.
The quote reader projects the original first Default RPC's URL and commitment;
it no longer retains that execution RpcClient. Registry/repository/runtime Arcs
remain original instances; constructor and empty-registry tests retain existing
assertions. Application metadata/owner/input validation, quote saving, expiry,
failure stages/codes, conversion journal and prepare/sign/submit/status/recovery
remain unchanged. The status reader still has its old ownership/path; do not
claim it migrated with quote estimation.

The estimator lives in the SDK adapter. The complete original
`sdk_client/wsol_conversion/queries.rs` remains byte-identical under cfg(test),
blob 353bf0d6bf5df678ab7331ae8a1d77c6afc0bbc9, for existing tests and independent
oracle comparisons. Original instruction builder bytes remain unchanged, blob
e439a4202f02b181fd79661e75c06e55c765ed96. A narrow compatibility helper returns only
its public Message and temporary-rent integer. For partial unwrap the original
builder still generates its ephemeral temporary account; the key does not enter
the new reader/session. This is not a claim that the entire old quote builder
never creates key material, memory zeroization or production key lifecycle proof.

Preserve new/existing ATA wrap, full/all/partial unwrap, native Token checks,
missing-amount/error order, checked SOL fee/rent/reserve arithmetic and saturating
source-rent refund. Keep u64 units and typed Integrity/Rejected failures. Null fee
fails; a real explicit zero is not invented from missing metadata. Independent
submission still rechecks actual state and message before signing; the quote
and its fee message are not an execution authorization.

## Fixed Five-Step Session

WsolQuoteReadSession borrows the existing runtime and owns one permit for the
complete sequence. It fixes owner/derived classic WSOL ATA and selected endpoint.
Only five typed requests exist, once each in this order:

1. getBalance for the owner with original lane commitment.
2. getAccountInfo for the derived ATA, processed/base64, original null slice/slot.
3. getMinimumBalanceForRentExemption with classic Account::LEN (165), no extra
   commitment argument, matching the locked client's actual implementation.
4. getLatestBlockhash with original lane commitment, not an unrelated cache hit.
5. getFeeForMessage for the original public unsigned legacy Message and lane
   commitment; payer/ATA/one-or-two required signers/previous hash/1232-byte size
   are checked before serialization/send. No transaction signature or signer is
   accepted, returned or invoked by the read scope.

An attempted read closes its stage before awaiting; failure/cancellation cannot
retry or proceed to another stage. Premature out-of-order calls send nothing and
do not advance. Drop cancels local I/O and releases the permit, not remote work.
Add the twelfth class, one active quote and 100 ms start pacing. All previous
capacities/intervals stay unchanged, including two discovery branches; four old
release fixtures assert the exact twelve-class vector. Total scope is bounded to
ten seconds, each HTTP step/pacing to two seconds, no renewal across stages.
Use the same registered endpoint, HTTP client, 429 cooldown and response limits;
no hidden retry/redirect/fallback or parallel key query.

Each response has declared/streamed 64 KiB bounds, JSON-RPC identity/exclusive
result/error and typed scalar/context/account/hash validation. Existing bounded
uncompressed account mapping is reused; absence is distinct from corrupt data.
No raw provider body/URL is echoed. Cross-commitment/slot reads retain original
estimation semantics; a shared endpoint is not an atomic bank snapshot.

## Required Acceptance

Twenty new Rust names: scope ten, SDK oracle/policy seven, real loopback HTTP
three. Compare all report fields and all five request bodies with the old real
RpcClient path. Normalize only base64+zstd request encoding to base64 and the
original randomly generated partial-unwrap public key in fee-message witnesses;
do not normalize amounts, commitments, hash, signer count or fee/rent data.
Cover every stage's 429/cancel, bad metadata/fee/hash, first/last timeout/late
polling, order/admission/deadline, u64 limits, missing versus explicit zero,
native/owner errors and original early-stop behavior. Real HTTP covers four
quote paths, exact body bounds, malformed/oversized replies/redirects and critical
hash progress during cancellation. Existing Application/HTTP/journal/sign tests
must remain and pass independently on this source.

Expected full concurrent/serial each 1769, retaining all 1749 parent names and
adding exactly twenty. Three focused steps make 75 total. The dependency gate
adds Message and borrowed WsolQuoteReadSession: 39 selected types / 120 scoped
fields including outside alias. Reuse only the already fixed static-lifetime
probe metadata, not arbitrary Rust paths. Verify raw spans, original/restored
checks, source identity and test-name sets; do not reuse the parent's 114-field
evidence. No local service Cargo check/test retry after the recorded 2200 MiB
failure; use authorized source-only/loopback-only GitHub Actions.

## Accepted Evidence

Source `c196d1b6f1ec3124e141114441e855c18dc70f4b`,
[Actions 44182843252](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44182843252),
first run terminal success; downloaded artifact `14374676338` independently
audited. All 75 steps exit zero. Complete concurrent 1769 (200.75 seconds) and
serial 1769 (379.92 seconds) have zero failed/ignored/filtered tests and identical
raw name sets. All 1749 parent names remain; exactly the twenty source-defined
names are added. Scope 10 / SDK oracle and policy 7 / real HTTP 3 and all earlier
focused regressions pass. The old estimator/instruction blobs above are unchanged.
All report fields and normalized request witnesses execute in the new SDK cases,
including original partial-unwrap randomness, explicit zero and missing fee.

Original/restored compiler checks and all 120 distinct raw field spans pass on
this source: 39 selected types plus outside alias, including Message and the
borrowed session's fixed static-lifetime probe. Python policy 12 / workflow 7 and
SDK provenance pass. Identity and manifest/lock/vendor-manifest hashes match.
Before recording acceptance, all 1105 source blobs / 13,473,071 bytes and executable
modes matched the snapshot. Main/index, frozen history, SDK/manifests/lock/vendor/
migrations remain unchanged. The known old serial discovery child stderr split
is associated only with its exact test/pinned fixture/immediate ok; two positive/
seven negative format cases pass. Raw logs are not rewritten and no test names
omitted. No local service Cargo retry or production action. Do not rerun terminal
CI or trigger documentation-only CI; carry records with the next source batch.

## Remaining Gates

WSOL unsigned preparation/simulation/sign/submission/status, other independent
reads and SDK opaque tasks, provider-wide quotas and remaining NP-01/03/04/05/
rollout gates continue in the original queue. No new task or denominator. SDK
stays 6.0.2; manifests/lock/vendor/migrations/release, production configuration,
wallets/databases and running services are untouched. No real RPC/trade and
no new Live permission. New PumpAPI integration stays paused.

The subsequent original NP-03 status entry is now independently accepted in
ec608c0 / Actions 44188954352 / artifact 14377382655: complete concurrent/serial
each 1783, parent 1769 retained, exactly fourteen added, all 123 fields/78 steps
pass. See WSOL_STATUS_SHARED_READS.md. Its shared reader removes
execution-RPC ownership while preserving the single original signature/history
and strict finality/error/slot/outcome policy; ordinary parsing is not an
interchangeable replacement. Status journal/finality CAS is deliberately unchanged.
This quote batch's accepted run was not reused as evidence for the status source.
