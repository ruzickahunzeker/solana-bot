# DAMM Quote Evidence Shared Reads

Date: 2026-11-18, Asia/Shanghai. Continues the original NP-03 step, not a new
task or denominator. Production source wiring and original-body audit are
complete; independent Rust acceptance is pending. It is not in the preceding accepted
candidate snapshot 26e4b8b / Actions 44234932246 / artifact 14397918749 (complete
concurrent/serial each 1897). Do not inherit that result or publish docs-only CI.

## Actual Original Paths

- Sell: LanePreparation::damm_sell_output -> SdkTradeClient::simulate_damm_v2_sell_output
  -> inspect_prepared_damm_probe_at_slot -> evidence_damm_probe_output.
- Buy: LanePreparation::damm_buy_output -> DammBuyQuote::read -> quote_current
  -> quote_inner, capturing the same per-invocation output exactly once.
- Final candidate simulations/previews are a separate preceding batch. Do not
  rerun or duplicate it when wiring this evidence chain.

## Preserved Evidence

Read exactly the original eight sell/nine buy accounts at processed/base64,
same order, original optional minimum slot and count checks. Keep pool/mints/
vaults/Clock/ATA/wallet role checks and canonical token-program validation.
The unsigned legacy probe keeps original bytes, original minimum account bank,
sigVerify=false, replaceRecentBlockhash=false, requested accounts and inner CPI.
Keep full typed simulation response: accounts, inner instructions, fee, pre/post
native/token balances and loaded context. Do not replace these with a price,
an estimated scalar, min_out=1 or a delta against an earlier RPC bank.

Keep output validators, transfer-fee/decimal calculations, fingerprinting, net
positive output, same-bank funding/rent/fee and original integer minimum output.
Do not renew original Instant timestamps. Original buy whole-quote timeout is
five seconds; original sell account evidence timeout is independently five
seconds. The selected SDK RpcClient uses its default 30-second HTTP timeout for
sell probe I/O, without a separate caller wrapper there. A valid quote still has
to pass original three-second freshness; this does not justify silently replacing
those operations with the shared transport's two-second default.

## Intended Read Boundary

A sealed DammQuoteRpc trait keeps original SDK evidence method bodies generic
over read-only input. Production DammQuoteReader borrows the same service runtime
and snapshots only the original selected lane URL. Test-only RpcClient delegates
to the original simulation oracle and original account config. Evidence no longer
retains a production raw execution RPC. No new HTTP client, worker or endpoint.

Private-constructor, non-Clone query/packet types validate bounded eight/nine
distinct accounts and nonzero provided slot before I/O. Probe additionally
requires original zero signature/single payer/hash/packet bound and inner evidence.
The fixed shared requests are getMultipleAccounts and simulateTransaction only.
Request/response counts and typed contexts remain bound; explicit nullable err
is required, rejecting malformed omitted outcomes as in the preceding batches.

Use existing Parameters permit/start pacing for account snapshots and existing
AggregatorSimulation permit/start pacing for probes, registered endpoint and
cross-profile cooldown. No thirteenth class, retry, alternate credential or
success cache. Fixed per-request HTTP allowances must preserve the original
five-second account and 30-second probe limits; buy's original whole timeout
cancels earlier. Keep post-I/O/parse deadline checks and original freshness.
Shared 429 no-retry and bounded transport are explicit policies, not claims of
byte-identical arbitrary provider errors or old internal SDK retry scheduling.

Keep semantic response slot/count/role checks in their original SDK validators,
not only in the transport. In particular, sell account-slot regression originally
produces a typed terminal balance rejection: moving that decision into a generic
RPC error could incorrectly permit lane fallback. The shared profile validates
fixed request/response schema, identity and transport bounds; full typed data
continues unchanged to the same business validators. Negative tests must compare
the original rejection category, not merely assert that both calls return errors.

## Wiring And Acceptance Queue

Following the preceding candidate acceptance, adapter/runtime files and fixed
HTTP variants are registered. Both production call chains borrow the same runtime
and original URL from LanePreparation. The three concrete reader/query/packet
types join the three-layer denial policy; three focused CI steps are added.
Expected 55 selected types / 168 raw field rejections, 99 steps and complete
concurrent/serial each 1926: retain all 1897 parent names and add exactly 29.
These are acceptance requirements, not passing evidence. SDK 6.0.2, vendor/lock/
manifests/migrations, API, journal, sign/broadcast and PumpAPI remain untouched.

Compare full original request arrays and quote metadata across native/USDC,
buy/sell, reversed pools, Token-2022 fees, ATA initialization/prefunding and same-
bank balances. Fault tests must preserve no-capture/no-renewal/no-sign behavior,
original quote/account/holding slot order and terminal balance classification.
Exercise malformed account counts/data/schema, missing err/CPI/native/token
evidence, context mutation, 429/cooldown, quota contention, pacing and cancellation.
Real loopback HTTP must prove long fixed allowances without weakening unrelated
profiles or child deadlines. Use source-only offline CI; retain every preceding
test name and independently audit source identity, raw names and compiler spans.
No local service Cargo retry, deployment or real node/wallet/database access.

## Original Runtime Fixture Follow-Through

The original-runtime fixture now dispatches only eight/nine-account quote
snapshots and account-returning unsigned probes through a separate opt-in oracle
to the unchanged MemoryRpc assertions; the preceding preview wrapper stays.
Parameter/presence/balance fixtures are unchanged. Combined oracle trace remains
account/probe/preview/send, with original message/signature, funding, write-ahead
journal, failure/no-retry and exact-one-send expectations. Exact HTTP counts grow
by two requests: sell 5->7, buy 4->6. Added ID/config/packet assertions cover both
new requests; this path still needs its own successful CI on the new source.

Cancellation moves from an in-process RpcSender await to client HTTP. Client
future cancellation does not prove a remote RPC stopped processing. The fixture
observes disconnect while its oracle is blocked; both original cancel tests now
await at most one second for oracle drop before retaining their original drop,
no-sign/no-send, exact packet and claim assertions. This test-only handshake is
not a larger production deadline, renewed freshness or remote cancellation claim.

Implemented sources include the fixed read adapter/runtime, its fourteen root
request/transport tests, eleven original SDK evidence tests and four real HTTP cases.
Sell compares 96 direction/fee/ATA/encoding combinations; buy compares the
existing 72-case matrix, exact requests and funding/minimum evidence. These are
source counts only, not passing tests. Original raw oracle, SDK validator/funding/
CPI/freshness/error bodies and actual capture/sign/journal paths reconstruct
exactly after explicit generic/import/read wiring. Old fixture/wire assertions
reconstruct after the opt-in oracle, exact new witnesses and bounded drop wait.
Read-only audit has one positive/ten negative mutations. Format, policy12,
workflow7 and SDK provenance pass locally; independent source-only CI remains
before acceptance. No local service Cargo check/test/link was run.
