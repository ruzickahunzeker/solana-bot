# SOL Hop Discovery Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues NP-03 / TS-07/09. This batch is source
wired and pending independent Rust CI. Parent cde04bd / Actions 44146511097 has
complete suites each 1693; that evidence is not reused for these changes.

## Ownership And Compatibility

The root SDK method now only projects its original Default lanes and passes its
original sol_hop_cache by reference to the SDK read adapter. The cache remains
the same per-wallet Mutex<HashMap>, not a new shared/global cache. Keep its lock
through discovery, positive-only 30-second TTL, hit-before-WSOL-validation order,
expired-row retention on failure and serialization across repeated panel opens.
Changing a default wallet still selects that original wallet's own cache.

The original algorithm still scans quote/WSOL at offsets 168/200, then 200/168,
refetches each candidate, checks pair membership, calls the original CPMM params
loader and checks both vault balances. Keep token programs, fees, epoch, transfer
fee and active-hook policy in the existing SDK; no decoder copy or metadata
replacement. Zero vault units skip that candidate. Only two complete empty lane
scans produce the existing definitive absence error; a partial scan, RPC error,
budget limit or cancellation cannot count as empty. Keep the legacy single-empty
and no-lane diagnostic even though it does not identify those cases separately.

Public HTTP/DTO, amounts, wallet identity, request/hash/JSON, claim, journal,
signatures and sender remain unchanged. A cached hop ID remains only the original
hint, not an economic quote, full route authorization or permission to broadcast.

## Fixed Read Scope

SolHopReadSession uses the accepted scoped sender/query engine and the same
SharedReadRuntime/HTTP/cooldowns, not a raw execution RpcClient. Add a ninth class:
one active hop scope, 100 ms between starts, original selected registered endpoint
only; other eight classes retain exact capacities/intervals. Existing release
fixtures now assert all nine capacities [1,1,1,1,1,1,1,2,1], never a nonzero check.

Clamp each node scope to the original five-second total deadline, two seconds per
HTTP step including pacing and 32 SDK admissions including queued calls. Opposite
orientation/candidate reads cannot renew the node deadline. Drop/cancellation
closes the sender and releases its permit only after active reads stop. Failure
remains sticky even if SDK helpers erase it. No hidden node retry/redirect; the
adapter retains original serial Default-lane fallback and two-empty threshold.

Only this quote-bound scope gains CPMM GPA with exactly the current discriminator
and both mints in the two audited orientations. No unfiltered scan, extra filter,
slice/min-slot widening or unrelated protocol. Keep 1 MiB / 1000 scan-account
bounds, 512 KiB other bodies, 100 batch keys and 64 KiB decoded accounts. Also
allow this scope's SDK getTokenAccountBalance with fixed commitment shape,
nonzero context, typed UiTokenAmount and checked u64 raw units; display numbers
are not reserve evidence. Existing parameter and ordinary discovery scopes do not
gain that method or the CPMM scan. Signing, simulation and other methods remain
denied before HTTP. There is no new SDK dependency, HTTP client or signing path.

## Required Acceptance

Twenty new test names: eight scope/query contracts, ten real SDK/cache witnesses
and two isolated real HTTP cases. Cover exact pair filters, purpose isolation,
u64 bounds, config/context/metadata errors, one-scope admission, other capacities,
429/cancel, five-second clamp, queued cap and swallowed-error rejection. SDK
fixtures compare both orientations and original read multisets, TTL and expiry,
concurrent cache hits, two-empty versus partial failures, serial node fallback,
zero/malformed balances, fee underflow/active hook, candidate cap, cancellation
and slow multi-step deadline. Real HTTP covers both GPA and balance body limits,
bad IDs/RPC errors, redirects, rate limits and cancellation while hash progresses.

CI must retain all 1693 parent names and add exactly 20 (expected 1713), execute
complete concurrent/serial suites plus three new focused steps (66 total), and
independently verify raw compiler spans for the added concrete session: 35 types /
108 fields including outside alias. Original/restored checks must pass. Check
artifact/source identity and retained named parent tests, not only totals.

After the earlier local 2200 MiB resource failure, no local Cargo check/test link
is retried. Formatting/Python/workflow/provenance checks are distinct evidence,
not substitutes for the authorized source-only, loopback-only GitHub Actions.

## Remaining Gates

No SDK upgrade (still 6.0.2), manifest/lock/vendor/migration changes, release build,
deployment/restart, production configuration/wallet/database read or real trade.
PumpAPI new integration remains paused. Simulations, independent quote/WSOL, SDK
background reads, global provider quotas and remaining resource/lifecycle and
NP-01/03/04/05 gates continue in the original queue. Local cancellation does not
prove a remote node stopped processing; no real latency/node/provider or rollout
acceptance is claimed. Do not repeat terminal CI or create a parallel authority.
