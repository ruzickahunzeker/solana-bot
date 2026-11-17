# Aggregator Simulation Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues NP-03 / TS-07/09. Source and independent
offline Rust CI accepted, not deployed. Identity: 2f12c97 / Actions 44173368003 /
artifact 14371180749. Parent 10c5b4b evidence is not reused for this source.
No production action or new execution authority.

## Existing Pipeline And Ownership

The same runtime already projected into independent preflight, Quote submission
and ordinary aggregate buy now serves their baseline and unsigned simulation.
The root SDK facade retains first-Default-lane selection and delegates using only
the original runtime, URL and wallet public key. No signer, execution RpcClient,
new HTTP client/cache/registry/journal or detached task enters the read session.

The SDK adapter still checks baseline payer and target Token/Token-2022 data
before simulation. A bad payer/mint/program/owner/state cannot trigger the second
request. Original report calculation, CPI inventory, target mutation checks,
System spend, duplicate token-balance handling and same-slot deltas are retained.
Optional metadata is not fabricated and different-bank baseline differences
remain observational None. Original simulated transaction errors produce a
non-executable report, not a fabricated signature or implicit submission.

The pure helper bodies moved to the SDK adapter with only imports/visibility
changes; rustfmt-normalized comparison against accepted 10c5b4b yields SHA-256
71ed3be589cf5a5375ce4cc83afc047ded1e50ed6d94a3824e236227e1318e9c. The old root
aggregator simulation test module remains byte-for-byte unchanged and keeps
its existing names through compatibility imports. The root report and public
target-instruction helper paths retain compatibility re-exports.

## Fixed Unsigned Read Plan

UnsignedAggregatorSimulation freezes the original canonical packet (maximum
1232 bytes), one required default signature, payer, writable target/index,
static/resolved account prefix, distinct/count bounds and loaded key order.
Only legacy/v0, original 1..32 instructions and at most four lookups are allowed.
It owns no key and cannot sign/broadcast. Source/amount/route/preflight policy and
the final Quote-expiry/sign/journal/sender checks are not replaced by this plan.

Its only two typed HTTP bodies are processed/base64 getMultipleAccounts for the
exact payer/target pair and simulateTransaction with sigVerify=false,
replaceRecentBlockhash=false, exact same pair, innerInstructions=true and the
successful baseline's minContextSlot. There is no arbitrary method/config/body.
Simulation slot must not precede baseline; a replacement blockhash is rejected.
Loaded addresses, when supplied, must match the frozen message. Returned account
metadata, when supplied, must contain the exact two entries and uncompressed
bounded account data. These checks do not prove node honesty or independently
validate missing optional metadata.

## Session And Transport

AggregatorSimulationReadSession borrows the existing runtime and owns one permit
for the complete sequence; it cannot clone, detach or outlive that resource.
Each network stage is attempted at most once. An attempted read is marked closed
before awaiting, so read error/cancellation cannot retry it or skip a failed
baseline. A premature out-of-order call sends nothing and does not advance state.
Dropping the scope cancels in-scope I/O and releases its permit. It cannot retract
work already received by a remote node.

Add the eleventh class, one active sequence and 100 ms between starts; all prior
capacities/intervals remain, including two ordinary-discovery branches. Four
old cancellation fixtures assert the exact eleven-class vector, not nonzero
capacity. The sequence has the original two-five-second upper bound of ten
seconds, with each shared step additionally capped at two seconds including
pacing. Decoding and subsequent stages do not renew total time. The shared
registry/429 cooldown/fixed Reqwest transport applies. Only the original registered
selected lane is used; no retry, redirect or hidden fallback during either stage.

Both bodies have declared/streamed 512 KiB limits. Existing bounded Base64 account
mapping is reused (64 KiB decoded account limit); no compression or copied Token
decoder. JSON-RPC identity/result/error, positive slot/exact count and typed
simulation result are validated. Errors do not echo URLs or provider bodies.

## Required Evidence

Twenty new Rust names: scope/plan ten, SDK pipeline seven, real loopback HTTP three.
Cases include legacy/v0 and both Token programs, frozen unsigned bytes, rejected
signatures/bindings/size, exact pair/slot, stage order and terminal faults, bad
baseline/post data, optional/different-slot semantics, CPI violations, chain
simulation error, shared 429, body/identity/redirect, timeout/late polling and
cancellation while hash progresses. Existing test names and pure helper behavior
must remain, not be replaced by totals or inherited parent evidence.

CI must retain all 1729 parent names and add exactly 20 (expected 1749), run
complete concurrent/serial suites and three new focused steps (72 total). The
type policy adds the plan and borrowed session: 37 selected types / 114 fields
including outside alias. A narrowly validated static_lifetime=true probe flag
emits only the fixed 'static literal for the session; paths remain identifier-only
and arbitrary Rust/lifetime injection is rejected. Twelve Python policy tests
include two new metadata cases. Original/restored checks, raw rejection spans,
source/artifact identity and old name sets require independent audit on this
source. Local fmt/Python/workflow/provenance is not Rust runtime acceptance;
do not retry local Cargo after the recorded 2200 MiB failure.

## Accepted Evidence

Source `2f12c97fad9b35d81c38e4515b03362c410234b9`,
[Actions 44173368003](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44173368003),
first run terminal success; downloaded artifact `14371180749` independently
audited. All 72 steps exit zero. Complete concurrent 1749 (220.73 seconds) and
serial 1749 (402.90 seconds) have zero failed/ignored/filtered tests and identical
raw name sets. All 1729 parent names remain; exactly the 20 source-defined names
are added. Scope 10 / SDK pipeline 7 / real HTTP 3 pass, as do all earlier account,
hop, ordinary discovery, preparation and execution focused regressions. Original/
restored compiler checks and all 114 distinct raw field spans pass, including the
borrowed session's fixed static-lifetime probe. The 12 Python policy tests pass;
no generic Rust/path/lifetime input was enabled to satisfy the compiler gate.

Identity and manifest/lock/vendor-manifest hashes match. Before recording this
acceptance, all 1098 snapshot files / 13,405,912 bytes and executable modes matched
the working source. Pure helpers retain the canonical hash above and the old
test module remains byte-identical. Main/index, frozen history, SDK/manifests/
lock/vendor/migrations remain unchanged. The known old serial discovery child
stderr split is associated only with its exact test/pinned fixture/immediate ok;
two positive/seven negative format regressions pass. No raw log rewrite, omitted
name, local service Cargo retry or production action. Do not rerun terminal CI or
trigger documentation-only CI; carry these records with the next source batch.

## Remaining Work

Independent quote/WSOL, DAMM/SDK-internal reads and opaque workers, provider-wide
quotas and other original NP-01/03/04/05/rollout gates remain. No SDK upgrade
(still 6.0.2), manifests/lock/vendor/migrations/release change, deployment/restart,
production configuration/wallet/database access, real RPC or trade. PumpAPI new
integration remains paused. No new route/backend or permission to submit follows
from these read-only simulation checks.

The next original NP-03 independent-read entry is WSOL quote estimation:
`sdk_client/wsol_conversion/queries.rs` still reads native balance, the processed
ATA, fixed Account::LEN rent, latest hash and fee for its own unsigned message
through the execution RpcClient. `adapters/sdk/wsol_queries.rs` still projects
that first existing RPC. Preserve its original error/Token/native/rent/all/fee
semantics and old oracle while replacing only those reads; do not merge the
independent conversion journal or change sign/submission/status ownership.
