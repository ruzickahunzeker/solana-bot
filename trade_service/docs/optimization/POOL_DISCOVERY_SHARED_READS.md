# Ordinary Pool Discovery Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues NP-03 / TS-07/09, not a new task or
protocol implementation. Current source has independent offline Rust acceptance:
cde04bd / Actions 44146511097 / artifact 14359025763. Parent remains 1792c51 /
Actions 44133854054 (full suites each 1675), not reused as this batch's proof.

## Actual Production Wiring

Ordinary resolve no longer runs its SDK algorithm inside SdkTradeClient. The
root method only selects its original Default lanes and projects endpoint,
commitment and Helius classification. The SDK adapter owns protocol selection
and receives only these read descriptors plus the original SharedReadRuntime.
It does not retain a wallet, signer or TradingClient. PoolReadBackend passes the
same runtime Arc already used by preparation and warmup. No second HTTP client,
cache or persistence authority is created.

Preserve the original first-two-Helius random ordering, serial node fallback,
three-second per-node outer deadline and Pump/DBC join. A live Pump curve takes
priority, then an active DBC pool, then the existing PumpSwap fallback. Completed
curves and migrated DBC do not gain new permission. The original SDK decoders,
WSOL/Quote projection and diagnostic categories remain. Existing root test
names delegate to the moved pure order/classification functions.

DiscoveryReadSession uses the already accepted scoped sender/query engine, not
raw execution RPC. Every protocol branch has its own sticky failure, cancellation
and retained-reader lifecycle. A failed DBC probe cannot poison already-valid
Pump evidence; conversely, an SDK-erased error cannot make that failed branch
successful. A hint is not trade permission or a fresh economic quote.

## Budgets And Requests

- Original runtime gains a discovery profile: two active protocol branches and
  100 ms between HTTP starts across its endpoints. This replaces previously
  unbudgeted parallel discovery; it does not reduce the accepted parameter
  profile's 250 ms spacing or single-scope capacity. Node lanes remain serial.
- Every branch has at most 32 SDK admissions, a caller deadline clamped to three
  seconds, and at most two seconds per HTTP read including pacing. All three
  branches share the same original per-node deadline; fallback never renews it.
- Concurrent wakeups recheck the next start under the existing budget mutex.
  Waiting/cancelled calls do not reserve future starts. This prevents the new
  second branch from bypassing pacing. Existing single-capacity classes keep
  their intervals, deadlines, cooldown and cancellation behavior.
- Reuse original registered endpoints, streaming HTTP, no redirects/retries,
  cross-class 429 cooldown, bounded account decoding and exact envelope identity.
  Parameter and blockhash capacities stay separate. Do not interpret this as a
  provider-account/global quota covering other processes or SDK internal reads.
- Only account, multiple-account and epoch methods are available. PumpSwap scans
  retain their exact discriminator/mint/size/43-or-75 profiles. DBC discovery alone
  adds the current 424-byte layout, discriminator and mint offset 136. Ordinary
  DBC parameter loading does not gain GPA; Pump discovery cannot use DBC/Swap GPA.
  Scan bodies remain 1 MiB/1000 accounts, other bodies 512 KiB, decoded data 64 KiB.
- PumpSwap reuses the accepted same-bank six-to-four snapshot and two scoped raw
  config values, avoiding the pinned SDK's 180 ms optional-config timeout. No
  vendor changes, decoder copy, static fee replacement or cross-session cache.

## Acceptance

Source adds eighteen Rust test names: eight scoped-session, eight loader entries
(one isolated helper) and two actual HTTP cases. Verify real SDK DBC filters,
purpose/mint/size/flags and money-method rejection; simultaneous branch pacing,
separate parameter/hash capacity, 429, cancellation, sticky failure and deadline
clamping. Actual protocol fixtures cover live Pump priority, independent DBC
503/invalid replies, completed/missing curves, DBC migration, cold PumpSwap six
scans and truly empty discovery. Separate child processes prevent SDK config
caches from hiding the cold route. Node failover checks endpoint seriality.

Application PoolQueryService witnesses record exactly one successful hint and
none on RPC failure/timeout, without a real wallet, database, signing or send.
Real loopback HTTP covers account/GPA serialization, body limits, bad envelopes,
429 and redirect/no hidden fallback. Both complete suites passed: concurrent
1693 (182.21 seconds), serial 1693 (340.51 seconds), no failures/ignored/filtered.
All 1675 parent names remain and exactly 18 were added. Scope 8 / loader 8 / real
HTTP 2 and all 63 steps passed. The same type policy includes this concrete
session (34 types / 105 scoped fields including outside alias): raw spans and
original/restored checks were independently verified. Identity and manifest/
lock/vendor-manifest hashes match; the whole 1079-file, 13,241,022-byte tree
matches the accepted source snapshot. Real main/index, frozen history, SDK and
migrations remain unchanged. Do not rerun the terminal run or trigger docs-only
CI; carry acceptance records with the next source batch.

One serial test prefix is interrupted by a pinned SDK child's stderr diagnostic,
then an immediate standalone `ok`. The audit associates only that exact name,
fixed fixture mint/diagnostic and immediate completion; raw logs are not changed.
Two positive and seven negative local format cases reject a failed completion,
wrong fixture/name/diagnostic, intervening test, duplicate or concurrent split.
This does not omit a name or accept arbitrary split output. Both complete name
sets and all focused sets match after this strict association.

Local formatting, source policy and workflow checks are separate evidence only.
After the prior 2200 MiB resource failure, do not repeat local Cargo checks or
test linking; use the authorized source-only, network-isolated GitHub Actions.
The final source-only run supplies runtime evidence; local checks are not a
substitute for it. No production rollout or live-node proof is claimed.

First snapshot 7495079 / Actions 44142801934 / artifact 14356861217 passed its
type check, 105-field compiler gate and all focused steps, including the 18 new
names. Full concurrent execution was 1690 passed / 3 failed; serial did not run.
The three old readiness/native/quote cancellation fixtures assumed every runtime
class had exactly one slot. Discovery explicitly adds two. The correction checks
the exact eight-class vector [1,1,1,1,1,1,1,2], retaining every old release/start/
drop assertion. Production budgets and cancellation code are unchanged. This is
not a relaxation to nonzero permits. Do not rerun that terminal failure; the
corrected cde04bd snapshot independently passed complete concurrent/serial
acceptance as recorded above.

## Boundaries And Remaining Work

No API/DTO, amount units, wallet selection, request/hash/JSON, claim, journal,
signature or sender change. SDK stays 6.0.2; manifests, lockfiles, vendor and
migrations stay at the parent. No release build, deployment/restart, production
configuration/wallet/database access, real RPC or trade. PumpAPI new integration
remains paused; ordinary SDK Pump/DBC/Swap resolution is not that backend.

The original positive two-hop cache/scan path remains untouched and unbudgeted by
this batch, as do simulations, independent quote/WSOL and SDK background reads.
PoolReadBackend still selects the original default wallet before taking its read
configuration; this is not all wallet-resource ownership completion. Cancelling
local HTTP does not prove that a remote RPC server stopped its own processing.
Other NP-01/03/04/05, real-node latency/capabilities, recovery and rollout gates
remain in the original queue. Do not merge main or repeat terminal CI runs.
