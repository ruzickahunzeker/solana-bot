# Protocol Parameter Read Scopes

Date: 2026-11-18, Asia/Shanghai. Continues NP-03 / TS-07/09. Source is connected
to ordinary SDK preparation and warmup; this batch has independent offline Rust
acceptance in 1792c51 / Actions 44133854054 / artifact 14351589608.
Parent acceptance remains 27bd996 / Actions 44115951258 (full suites each 1652).

## Actual Boundary

`protocol_params::load_for_trade` and `warmup_scoped` borrow a keyless parameter
scope, then call the unchanged original SDK loaders/decoders. Root single-caller
`dex_params` delegate is removed. Warmup receives the same shared runtime Arc via
PoolReadBackend/bootstrap. The SDK execution/signing/sending client is unchanged.
Each scope's RpcClient uses a private fixed-method RpcSender, **not** a second
HTTP transport, shared cache, cooldown, wallet or persistent authority. Public HTTP,
amounts, request/hash/JSON, claim/journal/sender and migration files are unchanged.
The changed warmup Rust method is internal to this binary, not a public route.

Add one class to the original runtime: one whole parameter scope, starts at least
250 ms apart, selected registered endpoint only and shared cross-class 429
cooldown. SDK join branches serialize in the same scope; hash/balance capacities
stay separate. Eight seconds total, two seconds per request, 32 SDK admissions
including queued calls. Admission starts on the first actual RPC, so existing
missing-pool guards retain their original pre-I/O errors.

Read/schema/capability errors are sticky even if an optional SDK reader returns
None instead of its transport error. Only a valid RPC null account retains its
ordinary AccountNotFound/optional-absence meaning. Finish rechecks the same scope.
Cancellation faults that scope; drop/finish closes retained readers. The original
class permit is released after its last active read drops, not just caller return.
Invalid failures stop both original SDK dispatchers; Unavailable retains only
the original pre-sign lane fallback policy. No post-sign retry or new owner.

## Fixed Requests

- Allow getAccountInfo, getMultipleAccounts and getEpochInfo, with original keys,
  commitment/order and SDK decoding except the explicit PumpSwap snapshot extension
  below. Normalize base64+zstd requests to base64;
  compressed replies are rejected without decompression expansion. No signing,
  simulation, balance, blockhash or submission capability in this scoped client.
- At most 100 batch keys; no data slicing/min-slot widening; 512 KiB HTTP body,
  64 KiB decoded account, valid owner/space, nonzero context and exact batch count.
- PumpSwap GPA only for PumpAmm without explicit pool ID, original mint/program,
  discriminator and base/quote offset 43/75, pinned sizes 252/261/270/271/300/643.
  No arbitrary/unfiltered program scan, true withContext or new sort mode.
  Scan body at most 1 MiB / 1000 well-formed binary accounts. These are schema/
  resource bounds, not fresh-route, economic quote or full protocol permission.
- Keep original HTTP no-retry/no-redirect and URL alias/cooldown behavior. Future
  unrecognized SDK requests fail closed for explicit upgrade review, not fallback
  to a raw execution RpcClient.

The pinned PumpSwap SDK wraps its two optional config calls in 180 ms timers.
Serial 250 ms pacing would deterministically cancel cold reads. Instead of
loosening pacing or vendor timers, append exactly FEE_CONFIG and GLOBAL_ACCOUNT
to its four-account pool snapshot on the same endpoint/commitment. Validate all
six, return the original first four unchanged, and keep only the two raw config
values/context in that same eight-second scope. Exact-address/config SDK reads
reuse those raw values and the unchanged SDK decoders; null remains null. This
is not a cross-session cache or an alternative fee source. Other queries,
commitments, protocols and explicit wrong pool IDs cannot use this projection.
There is no extra HTTP request or per-step budget bypass; the two SDK calls still
count toward 32. The scope closes and loses its snapshot when finished/dropped.

## Acceptance

Source has 23 new Rust tests: 22 behavioral contracts and one isolated child entry.
Twelve scope/query tests, six loader entries (including child), four real HTTP
cases and one actual original-buy fault case cover the changes. Nine existing
protocol variants compare key accounts/programs/reserves/fees/mode witnesses to
the unchanged loader. Also exercise six actual SDK filtered scans, explicit
StonkFun CPMM hop, cold optional-config error erasure and original warmup guards.
The isolated optional-config parent now runs a separate cold-success child before
the fault child, ensuring process-global SDK caches cannot hide the 180 ms issue.
Query assertions also verify exact six-to-four projection, null preservation and
wrong-commitment/protocol/pool isolation.
The execution RPC mock now rejects parameter reads; old quote/sign/journal/packet
checks remain. Presence cancellation waits for its actual method, not the first
HTTP request, since parameter reads now precede it.

The final run retained all 1652 parent names and added exactly these 23. Full
concurrent 1675 (156.47 seconds) and serial 1675 (294.26 seconds) have identical
raw name sets, with no failures/ignored/filtered cases. All 60 steps exit zero;
scope 12 / loader 6 / real HTTP 4 / original-buy runtime 6 passed. The concrete
session belongs to the same policy: 33 types / 102 fields including outside
alias; all distinct raw compiler spans plus original/restored builds were checked.
Identity and manifest/lock/vendor-manifest hashes match; the whole 1074-file,
13,188,757-byte source tree matches the accepted snapshot. Real main/index,
frozen history, SDK and migrations remain unchanged. This is source-only evidence,
not live-node or rollout acceptance. Do not rerun the terminal run or create a
docs-only CI; carry these acceptance notes with the next source batch.

Local first check found an exhaustive fixture match omission, now fixed. The next
bounded metadata check was killed at 2200 MiB after 50 seconds (RSS 2,273,960 KiB).
Its import/visibility warnings were fixed; that is not a final type-check pass.
Do not retry local Cargo or raise resources after this failure. Use the authorized
source-only, network-isolated GitHub Actions. Formatting/Python checks alone do
not prove Rust execution. No local service test linking or release build.

First cloud snapshot 255ef09 / Actions 44130661475 / artifact 14348964516
failed test type-check after ten preflight steps passed (221 seconds). The shared
loader fixture lacked accounts/RpcRequest imports and crossed a private RPC body
method. The fix uses explicit test imports and cfg(test)-only request/reply
helpers; production visibility, loaders and assertions are unchanged. No Rust
tests or compiler boundary rejections ran in that snapshot. The failed run is
terminal, not retried; corrected source was independently accepted in the final
1792c51 run above.

Successor 7a63477 / Actions 44132188285 was superseded and cancelled after source
review found that cold-cache timeout conflict. Its artifact is 14351441124; it
is not full acceptance and is not rerun. The batched-snapshot correction and cold
positive witness were accepted in the final 1792c51 run with the same 23 added names.

## Remaining Scope

SDK stays 6.0.2; no dependency/vendor changes, deployment/restart, production
configuration/wallet/database access, real RPC or transaction. PumpAPI new
integration stays paused. Pool resolution, simulations, independent/SDK internal
reads, global provider quotas and lifecycle remain incomplete, as do other
NP-01/03/04/05 and real-node/caller/rollout/crash-recovery gates. No new execution
profile or broadcast permission is opened and no duplicate task is created.
