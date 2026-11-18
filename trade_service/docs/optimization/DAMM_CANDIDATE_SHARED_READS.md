# DAMM Candidate Simulation Shared Reads

Date: 2026-11-18, Asia/Shanghai. Continues the original NP-03 / TS-07/09 work,
not a new task or denominator. Source and independent offline Rust acceptance:
26e4b8b / Actions 44234932246 / artifact 14397918749. Accepted parent e81dda4 /
Actions 44223334225 was not used as evidence for this batch. No SDK upgrade,
release build, deployment or production access.

## Scope And Ownership

Move final unsigned candidate simulation and presigning previews, buy and sell,
from both the legacy lane and original/Durable path to the existing shared read
runtime. Snapshot only the original selected execution RPC URL. The borrowed,
sealed DammSimulationReader retains neither a signer nor an execution RpcClient.
It constructs one non-Clone DammSimulationPacket after validating the original
single-payer, zero-signature legacy transaction, nonzero payer/hash, size <=1232
and nonzero minimum quote slot. Runtime consumes the packet. No arbitrary encoded
packet, method, endpoint fallback, resigning or broadcast capability is added.

Original candidate/profile methods preserve their guard and await/recheck order;
only the simulation argument and helper import change. Original RpcClient helper
and old tests remain exact under cfg(test), serving as an independent oracle.
No Application, admission, journal, sign, broadcast or API changes. An unsigned
successful simulation is not a signature, a live send or a finality claim.

## Request And Response Contract

Keep exact serialized candidate bytes and the original config: simulateTransaction,
base64, processed, sigVerify=false, replaceRecentBlockhash=false, accounts=null,
minContextSlot=original quote slot, innerInstructions=true. Reject slot zero,
response slot before quote, replacementBlockhash and nonempty loadedAddresses.
Keep original typed program-error formatting and before/after evidence freshness.
The three-second balance/quote freshness limits are never renewed by a response.

Reuse existing AggregatorSimulation capacity (one), 100 ms request starts, shared
HTTP client, registered endpoint, 429 cooldown and bounded 512 KiB response.
There is no thirteenth class, new client, task, worker, cache or retry. Keep the
original four-second total deadline, including admission/pacing/I/O/decoding.
Only this fixed DAMM request overrides the shared client's default two-second
HTTP timeout to four seconds. Other profiles keep their old timeout. A final
elapsed check rejects a response becoming ready after delayed executor polling.

Response identity/version/outcome and explicit nullable err are required. Missing
err previously could deserialize to success; rejecting it is an intentional
malformed-response tightening, as in the accepted WSOL simulation batch. Provider
and transport details are sanitized, not advertised as identical raw error text.

## Acceptance Gates

Accepted 23 new Rust names: shared/oracle 12, actual buy/sell candidate methods 8,
real loopback wire 3. Complete concurrent/serial suites must each retain all 1874
parent names and add exactly those 23 (1897 total). Three new focused steps make
96 total. Add concrete borrowed reader and packet to the three-layer denial
policy: 52 selected types / 159 distinct field rejections including outside alias.

Compare original params/bytes/program outcomes; cover invalid candidate before
I/O, old bank, mutated context, bad identity/body/status, missing err, 429,
existing quota contention, cancellation, unique IDs/pacing and hash progress.
Exercise actual buy/sell simulation and live preview methods with fake SDK/submit
closures. Failed/expired evidence must not release submit. Real HTTP must accept
a valid response after two seconds but before four, and cancel slow headers/body
at the original deadline. Existing child case deadlines remain unchanged.

Use authorized source-only offline/loopback GitHub Actions only. No local service
Cargo check/test/link retry after the recorded resource limit. Independently
audit raw names, compiler spans, snapshot blobs/modes and dependency identities.
Keep main/index/frozen history; no terminal rerun or documentation-only CI.

## Deferred Original Work

DAMM output-quote probes and their account evidence reads are not migrated here.
Buy quote has a five-second whole quote limit; sell evidence has its own context
and deadlines. Their simulation/account/CPI/fee/same-bank checks and original
timestamps stay unchanged. Do not count this candidate batch as that chain's
completion. SDK-internal confirmation/tasks, provider-wide quotas, remaining
NP-01/03/04/05 and rollout gates remain. PumpAPI new integration stays paused;
Copy Trading/Bridge, manifests/lock/vendor/migrations and production data are out
of scope. This source-only acceptance is not operational completeness.

## Accepted Evidence

Source `26e4b8b97e424b0808a248c88fb5a5a30359e034`,
[Actions 44234932246](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44234932246)
first run terminal success, job 28m24s. Downloaded artifact `14397918749`
independently checked: all 96 steps exit zero. Complete concurrent 1897 (292.52
seconds), serial 1897 (561.63 seconds), zero failed/ignored/filtered, identical
raw name sets. All 1874 accepted-parent names retained, exactly the 23 names
defined in this source added. Shared/oracle 12 / actual buy/sell methods 8 /
real HTTP 3 and all preceding focused suites pass. Actual original runtime 5
and original buy runtime 6 retain sign/send/journal and exact new HTTP witnesses.

Original/restored compiler checks and all 159 distinct raw field spans pass:
52 selected types plus outside alias, including the borrowed reader and packet.
Policy 12 / workflow 7, provenance and locked-source checks pass. Before recording
acceptance all 1141 snapshot blobs/modes (13,804,808 bytes), identity and dependency
hashes match. Original simulation oracle/old tests and SDK method bodies reconstruct
exactly after explicit cfg/import/type wiring; actual lane and original sender,
tip/authorize/sign/journal/control reconstruct after shared read delegation only.
Old transport defaults/budgets/wire cases stay exact after this fixed request's
4s allowance and explicit fixture additions. Original MemoryRpc body and old
runtime assertions stay exact after the opt-in HTTP bridge and exact-one-added-
request/ID/packet witnesses. Deferred quote evidence chain remains untouched.

Read-only source audit has one positive/eight negative mutations; serial child
stderr is associated strictly with the original pinned discovery diagnostic and
immediate ok, with two positive/seven negative format regressions. No log/name
rewriting. Main/index/frozen history remain unchanged. No local service Cargo
retry, release build, deployment/restart, real RPC, production data or trade.
Do not rerun terminal CI or publish documentation-only CI; carry these acceptance
records with the next actual source batch. Original DAMM quote evidence, opaque
SDK tasks/confirmers, provider-wide quotas and NP-01/03/04/05/rollout gates remain.

## Terminal Candidate History

First candidate 582564f / Actions 44229785442 is terminal failure during test
type checking: the old Quote balance fake's exhaustive enum match lacked the
new request variant. Add an explicit panic-only DAMM arm; no old fixture behavior
or assertion is relaxed. No Rust test suite ran. Preserve that run as evidence,
do not rerun it, and require independent validation of the corrected source.

Second candidate c71e57b / Actions 44230719819 passed type checking, all 159
boundary rejections and prior focused steps, then failed one new shared test:
the fixture compared the canonical root URL (trailing slash) with noncanonical
text. Fix the synthetic backup constant to its canonical URL, retaining the
exact endpoint assertion. Production code is unchanged; shared tests were 11/12,
not accepted. Preserve this terminal run; require a new-source full validation.

Third candidate ed92781 / Actions 44232289823 passed all 23 new focused tests
(shared 12, adapter 8, wire 3), type/boundary and preceding focused steps. It
failed old original-runtime contracts because their loopback fixture only knew
the earlier parameter/status requests, while the migrated preview now correctly
arrives over shared HTTP. Add an opt-in preview-only fixture wrapper delegating
to the exact original MemoryRpc simulation oracle, keeping its message/config,
trace, program-error and signature/journal assertions. Expected HTTP count grows
by exactly one (sell 4->5, buy 3->4); assert its exact ID and packet against the
original simulated message. Original raw RPC implementation remains exact. No
production fix, assertion removal or timeout increase; full acceptance remains
pending on the corrected source. Retain the terminal failure, never rerun it.
