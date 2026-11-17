# Aggregator Account Read Boundary

Date: 2026-11-18, Asia/Shanghai. Continues existing NP-03 / TS-07/09; not a new
protocol, router backend or execution authority. Source and independent offline
Rust CI accepted, not deployed. Accepted identity is 10c5b4b / Actions 44164727015 /
artifact 14366377330; the 17bc3be parent evidence is not reused for these changes.

## Ownership And Compatibility

The existing bootstrap SharedReadRuntime Arc is projected into all three real
call chains: independent Quote preflight, Quote submission, and ordinary SOL
aggregate buy (including legacy and durable/observed submission). Resolved
wallet adapters keep that original Arc. No new client, cache, registry, journal,
signer or runtime is created per wallet/request. Empty-registry constructor
contracts check the original resource identities without initializing wallets.

The root SDK facade still selects the first original Default lane and projects
only its URL and commitment. Its mint/ALT methods delegate to a keyless SDK
adapter. Mint owner checks remain the original Token/Token-2022 policy; ATA
derivation still uses the original wallet/mint/program and SDK helper. This is
not a new initialized-mint, balance or ownership proof. ALT owner, SDK decoder,
deactivation sentinel and address order remain unchanged. No copied SDK decoder
or new lookup cache. Aggregate transaction lookup counts, index/duplicate/writable
checks, route/minimum/fees, simulation, signed message authorization and sender
are untouched. The two root read bodies no longer use execution RpcClient reads.

All public HTTP routes/DTOs, amounts, wallet identity, request/hash/JSON, admission,
claim/journal and sign/send guards stay unchanged. Account read failures remain
preflight failures, not default ATA/lookup success or permission to submit.

## Fixed Transport And Budget

Add the tenth runtime class, one active account read and 100 ms between starts;
the other nine capacities/intervals remain exactly [1,1,1,1,1,1,1,2,1]. The three
existing cancellation fixtures assert all ten capacities, preserving old drop
assertions. Critical hash and other classes keep their own permits. The same
endpoint registry, URL canonicalization, shared 429 cooldown and fixed Reqwest
transport apply. One registered selected lane only, no retry/redirect or automatic
fallback. A failed primary does not query a backup account or mix bank evidence.

The only new request variant is typed getAccountInfo with a bound address and the
original lane commitment, canonical base64 encoding and no arbitrary method or
JSON configuration. A single read has a five-second upper bound, further clamped
to the existing two-second shared HTTP step including pacing. No loop can renew
it. Admission is nonblocking, cancellation drops the active future and releases
its permit, without refunding an admitted start or claiming remote cancellation.

Declared/streamed 64 KiB body bounds precede serde/base64 decoding; no compressed
or parsed account encoding is accepted. JSON-RPC version/ID, mutually exclusive
result/error, nonzero context, account schema/owner and base64 decoding are
checked. Raw u64 lamports and bytes are retained. Provider messages/URLs do not
enter read errors. Bad/missing data, oversized replies, late responses and busy/
cooled/unknown endpoints do not become successful empty evidence.

## Acceptance Required

Sixteen new names: runtime eight, SDK/ATA/ALT five, isolated real HTTP three.
Runtime tests cover exact address/commitment, full u64/raw data, bad metadata,
body/transport errors, zero-send admission, pacing, shared 429/no fallback,
busy/cancel/timeout and delayed polling. SDK cases cover both token programs,
original unsupported-owner errors, empty/256-address ALT order and original
owner/decoder/deactivation rejection. HTTP cases cover actual fixed transport,
exact body limit, malformed/oversized responses, redirect/429, both slow-header/
body deadlines, and cancellation while hash progresses. Existing constructor
and signer-disabled cases retain their assertions and gain the shared resource.

CI must retain all 1713 parent names, add exactly 16 (expected 1729), run complete
concurrent/serial suites and three focused steps (69 total). The unchanged
35-type/108-field gate, original/restored checks and raw spans must pass on this
source, not be inherited from its parent. Verify artifact/source identity and
raw name sets independently. Local fmt/Python/provenance checks are not Rust
runtime proof; do not retry local Cargo after the recorded 2200 MiB failure.

First source 43a17e0 / Actions 44163177413 / artifact 14365660021 is terminal
failure: ten preliminary checks pass, type check rejects UiAccount::decode
because the pinned version has no such method. No Rust tests/boundary gate ran.
The correction uses the established Base64/owner/size checks and maps the original
Account fields, with extra space/owner-length negatives; no SDK/dependency change
or relaxed policy. The corrected source is independently accepted below; do not
restart this terminal failure or borrow parent evidence.

## Accepted Evidence

Source `10c5b4b0f8cd8ba04c03d088850e0806e2c35f18`,
[Actions 44164727015](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44164727015),
first run for this corrected source terminal success; downloaded artifact
`14366377330` independently audited. All 69 steps exit zero. Complete concurrent
1729 (212.62 seconds) and serial 1729 (387.94 seconds) have zero failed/ignored/
filtered tests and identical raw name sets. All 1713 parent names remain, exactly
the 16 source-defined tests above are added. Runtime 8 / SDK policy 5 / real HTTP
3 pass; earlier parameter session 28, ordinary discovery 8/8/2, hop 8/10/2,
presence and execution regressions also pass. Original/restored compiler checks
and all 108 distinct raw diagnostic field spans pass on this source, not just
the accepted parent.

Identity and manifest/lock/vendor-manifest hashes match. Before updating these
acceptance records, all 1090 snapshot files / 13,341,064 bytes and executable
modes matched the working source. Real main/index, frozen history, SDK/vendor,
manifests/lock and migrations remain unchanged. The exact old serial discovery
child-stderr split is associated with its pinned test/fixture/immediate ok only;
two positive/seven negative parser cases pass. Raw logs are not rewritten and
no test name is omitted. No local service Cargo retry or production action.
Do not rerun either terminal CI or trigger documentation-only CI; carry these
records with the next source batch.

## Remaining Gates

Simulation/baseline reads, independent quote/WSOL, execution/SDK opaque workers,
global provider quotas and remaining NP-01/03/04/05/rollout work stay in the
original queue. No SDK upgrade (still 6.0.2), manifest/lock/vendor/migration edit,
release build, deployment/restart, production configuration/wallet/database read
or real transaction. New PumpAPI integration remains paused. No end-to-end
aggregate availability, real-node timing or new Live permission is claimed.
