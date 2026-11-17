# Ordinary Buy Account Presence Read

Date: 2026-11-17. Continues the existing NP-03 / TS-07/09 RPC and SDK-facade
boundary, without a new task, execution profile, dependency or production change.
Source is implemented; independent Rust runtime acceptance is still required.

## Actual Call Path

`LanePreparation::mint_account_exists` now calls the same injected
`SharedReadRuntime::preparation_account_exists` instead of the root SDK client's
unbudgeted `get_token_accounts_by_owner`. The removed root helper had exactly one
production caller. The SDK parameter loader, DAMM evidence/unsigned preview and
other internal reads remain separate unfinished scope; this is not a claim that
all SDK/RPC resources are isolated.

The request still uses the selected lane URL and the original wallet/mint:
`getTokenAccountsByOwner`, Mint filter, jsonParsed, processed, null dataSlice and
minContextSlot. No supply, raw-account, ATA fallback, signature query or submission
is performed by this method. It never searches another key/endpoint internally.
The existing lane policy may still choose another lane on a typed Unavailable
preparation error before signing; Invalid evidence is terminal. No post-sign retry
or new owner is introduced.

## Preserved Meaning And New Bounds

- Preserve the original `Vec<RpcKeyedAccount>` deserialization and boolean: any
  JSON parsed program label `spl-token` or `spl-token-2022` is true; empty,
  unparsed-only or unrelated labels are false. Mixed parsed/unparsed, zero
  holdings and non-ATA entries retain V1 semantics. This is only a construction
  hint, not verified identity/state, a balance, ATA evidence or signing permission.
- Reuse preparation's existing capacity-one permit and 250 ms start pacing;
  no seventh semaphore, second HTTP client, cache or cooldown table. Presence and
  balance reads cannot overlap within this runtime. Hash capacity stays separate.
- Two-second total deadline includes pacing, request, body and parsing, with
  explicit checks before HTTP, after response and after typed parsing. Cancellation
  releases the original permit; an actual request's pacing charge is not refunded.
- Reuse the existing fixed TokenAccounts HTTP request: no redirect/retry, 512 KiB
  declared/streamed body cap. Add a 1000-account count limit and keep preparation's
  JSON-RPC ID/version/nonzero-context checks. These deliberately reject oversized
  or invalid metadata previously unbounded by the root helper.
- 429 shares the original endpoint cooldown across migrated profiles. URL aliases
  share a slot; distinct credentials stay distinct. Busy, timeout, HTTP/RPC failure,
  invalid metadata or unknown URL returns an error, never a known absence. Error
  details do not expose URLs or provider bodies.

API fields, request/hash/JSON, amounts, route/claim eligibility, original owner,
signed journal/CAS, sender, migration files and wallet storage are unchanged.
Legacy behavior in other protocols is not upgraded to full account authorization
by moving this hint; later protocol evidence remains mandatory where implemented.

## Evidence Required

Fourteen new Rust contracts are present: nine deterministic runtime tests, four
isolated real HTTP/1.1 tests, and one real original-buy runtime fault/cancel test
covering both payment assets. HTTP parity uses the original SDK RPC method and
predicate without constructing a TradingClient or signer. Tests cover the original
wire parameters/boolean, parsing/count/body bounds, current-lane-only behavior,
cross-profile cooldown and capacity, pacing/deadline/cancel, no redirect/hidden
retry, and failure before quote/probe/sign/send with the original owner retained.

The existing original-buy success test now expects one exact HTTP presence read;
the SDK mock explicitly panics if that read bypasses SharedReadRuntime. Original
sell traces/assertions are unchanged. A new `preparation-presence` CI step selects
all thirteen runtime/wire tests; the original-buy runtime step covers the new
fourteenth test. Full concurrent/serial suites must retain all 1638 parent names
and add exactly these fourteen; 99 field rejections remain required.

Local final bounded test metadata check passed (11 seconds / RSS 1,342,232 KiB),
production metadata check passed (10 seconds / RSS 772,052 KiB; original seven
warnings). Python policy 10, workflow 7 and SDK provenance checks pass. An initial
test-only temporary-borrow error was fixed before publication. Metadata checks do
not execute Rust tests; isolated CI must supply its own identity and raw logs.

SDK stays 6.0.2. No local service test link, release build, deployment/restart,
production configuration/wallet/database access, real RPC call or transaction.
PumpAPI new integration remains paused. Other read paths/global provider quotas,
other protocol/mode authorization and real environment acceptance stay in the
existing plan; no duplicate completion denominator is created.
