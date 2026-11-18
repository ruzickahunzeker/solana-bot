# DAMM Candidate Simulation Shared Reads

Date: 2026-11-18, Asia/Shanghai. Continues the original NP-03 / TS-07/09 work,
not a new task or denominator. Source implementation; independent Rust acceptance
is pending. Accepted parent: e81dda4 / Actions 44223334225. Its 1874 passing tests
are not evidence for this batch. No SDK upgrade, deployment or production access.

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

Expected 23 new Rust names: shared/oracle 12, actual buy/sell candidate methods 8,
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
of scope. No operational completeness percentage is increased by a pending run.
