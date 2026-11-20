# NP-01 Pump.fun unsigned candidate acceptance

This records the follow-up to [the account snapshot](PUMP_SDK_ACCOUNT_EVIDENCE.md)
within the existing NP-01 work. The source findings and test-only native probe
below are not an enabled production capability or a new rollout task. PumpAPI,
the locked SDK and current routes are unchanged.

## Locked source findings

| Case | SDK 6.0.2 behavior | Evidence required before controlled execution |
| --- | --- | --- |
| Native SOL on a SOL-paired curve | `should_use_v2_layout` uses V1 for native settlement. A canonical WSOL curve quote does not imply a WSOL transfer. | Bind the native settlement mode, not just the normalized quote mint. Establish the exact spend, rent and fee ceilings and validate the base mint and accounts. |
| Explicit WSOL or non-native quote | V2 adds the quote accounts and uses legacy SPL Token for the quote side. Service's existing Pump quote policy is WSOL/USDC; a structurally decoded custom mint does not widen it. | Establish mint owner, precision, extensions and curve/user vaults from bank evidence. Validate each ATA creation, funding, sync or close. Require the actual settlement to match the intent. |
| Exact-input buy | `ordinary_buy_params` sets `use_exact_sol_amount=true`. The builder computes `min_tokens_out` via locked helper estimates. `fixed_output_amount=Some(n)` switches it to exact-output buy, not an exact-input minimum. | Freeze exact spend and a separately justified positive minimum in `buy_exact_sol_in` or `buy_exact_quote_in_v2`. Do not use a fixed-output request as a shortcut. |
| Sell | The first integer is token input; the second is minimum quote output. The existing ordinary projection can close the base ATA when selling 100%. | Bind token input to the verified balance and percentage, enforce a positive minimum, and verify a close destination/authority and rent refund or exclude the close profile. |
| Fee destinations | The curve's mode affects standard/mayhem fee selection. The SDK can pick a fee recipient from a static fallback, picks a separate recipient from a static set for the final fee meta, and existing sell can prefer a supplied creator vault. | Verify the program-specific fee accounts/configuration, active sharing versus verified absence, and creator/vault authority. Freeze the actual authorized recipient and final bytes; do not permit arbitrary changes or give prefix data authority. |

The SDK's legacy exact-input buy encoder emits 25 bytes (two `u64`s and
`track_volume`), and its V2 encoder emits 24 bytes (two `u64`s). The bundled
Pump IDL also lists a trailing `partial_fill` `OptionBool` for both exact-input
buys. The corresponding discriminators agree. These are two distinct sources
of ABI evidence; the field list alone does not establish whether a deployed
program accepts the shorter forms or their implicit behavior. Preserve the
current SDK projection in compatibility comparisons, and require evidence for
the target program/layout before authorizing a write. Do not add a trailing
byte or enable partial fills solely to match IDL shape.

## Next evidence

1. The offline native-SOL funding reader now binds the eight account roles and
   Clock/epoch in one bounded shared read. Next verify balances, rent and fees
   for the initial canonical-ATA V1 profile.
   Leave non-native settlement, hooks and fee sharing execution outside that
   proof until independently checked.
2. Compare unsigned SDK instruction data and positional privileges against a
   project-owned exact-input/minimum projection. Check bad owner, wrong token
   program (including the SDK mint-suffix override), mutated recipient, wrong
   variant, inserted instructions, unsupported extension, stale slot, and
   zero/smaller minimum. An SDK builder match only proves compatibility, not
   trusted output or permission to sign.
3. Require zero-signature simulation or equivalent trusted net-output evidence
   for the exact profile, then re-check the complete compiled message, quote
   freshness and existing single journal authority. Only later can the
   existing controlled signer/sender path be considered. Do not change public
   validation, fall back after acquiring a signing permit, or rebuild after
   signing. Production verification remains a separate authorization gate.

Source references: vendored `instruction/pumpfun.rs`, `pumpfun_ix_data.rs`,
`instruction/utils/pumpfun.rs`, `utils/calc/pumpfun.rs`, and `idl/pump.json`;
service `adapters/sdk/ordinary_params.rs`, `preparation_input.rs` and
`protocol/registry.rs`.

## Local unsigned quote probe

Follow-up after the independently accepted destination correction `69e1230` /
Actions `44310987087` / artifact `14432369441` (full2017 each). That snapshot
does not include or validate this probe. Nine new Rust tests were independently
accepted as `45d18c0` / Actions `44319842306` / artifact `14437675802`:
four packet/account-age contracts and five response-consistency contracts.

Matching artifact/source audits verified full concurrent2026 (374.16s),
serial2026 (721.32s), all2017 parent names retained plus exactly nine; Pump34,
HTTP208,116 ordered zero-exit steps and62 types/189 raw compiler field
rejections. Original reader/decoder and old tests were reconstructed unchanged;
twelve weakened-source mutations were rejected. No local service Rust build or
test execution occurred. This accepts only the test-gated unbound model below.

First snapshot `9808224` / Actions `44318458423` failed at test type checking:
two `E0624` diagnostics share one cause, the sibling probe could not call the
private destination inspector. The correction only makes that existing method
`pub(super)` inside the original test-gated native module and records this
failure. No test assertion or production scope changes; no Rust tests ran.
The terminal snapshot is not rerun. Later shared-RPC work is held locally and
is not included in the correction.

The existing test-gated native funding snapshot owns the original bank evidence
and a monotonic timestamp captured before its shared parameter request. Waiting
for RPC admission, HTTP and decoding consumes the same three-second freshness
allowance; decoding does not restart it. Pure fixture decoding records its own
start. A fake transport test checks the exact eight-account request and that the
timestamp precedes transport receipt. No second HTTP client or RPC budget is
introduced.

The private, non-Clone, non-serializable probe consumes that snapshot and one
instruction from the locked SDK. It requires the existing canonical user ATA,
verified native destinations/roles and exact input matching an explicit nonzero
intent. Buy input must fit native balance; sell input must fit token balance.
It accepts only a nondefault blockhash and compiles exactly that swap with the
same wallet as sole payer/signer. The unsigned legacy packet must fit 1232 bytes.
The packet accessor returns only a shared reference and rechecks freshness.

Only the probe's output floor is set to one unit, after amount/layout checks.
This is a zero-signature quote-discovery packet, never an authorized minimum
for a live trade. Tests compare the entire compiled message and prove that
later mutations of the caller's original instruction do not change it. They
also reject input mismatch, missing ATA, insufficient input balance, default
hash, malformed wire, unexpected signer, unverified fee address, stale/future
snapshot and expired packet access.

The private observation validator consumes the probe and retains it alongside
integer output, nonzero network fee and simulation slot. Its input is still an
**unbound caller-supplied response**, not a result bound by the shared transport.
It requires explicit JSON `err: null`, a slot at least as new as the snapshot,
the unchanged legacy/hash context, complete native pre/post arrays in compiled
message order and exactly the two expected classic-SPL token rows. Mint, owner,
program, precision and unique indexes must match the frozen roles. Integer
amounts, not float/UI display fields, determine output. It verifies native
conservation including the network fee, unchanged unrelated native accounts,
nondecreasing pre-existing fee recipients, exact buy spend/sell token input,
classic-token conservation and positive output. Sell output includes the fee
even when the payout is smaller than the fee; native totals use `u128`.

This validator never subtracts an earlier RPC balance: tests intentionally make
the same-response pre balances differ from that earlier snapshot. Missing
fields, absent/duplicate/wrong token roles, malformed integers, zero output,
unfunded fees, unexpected funds movement, stale evidence and Token-2022 are
explicit rejection cases. No native System CPI transfer is universally required
as a payout witness. This only checks the supplied response's consistency.

The original nine-test snapshot did not connect a simulation request or
post-account proof. Subsequent separate acceptance below supplies fixed
request binding and simulated account/epoch cross-checks, not real provider
support, deployed ABI, blockhash validity, actual fees/rent or fee-tier policy.
The observation is still not a trusted quote or signing permit. A justified
positive minimum, complete message authorization, final simulation and original
signed-byte journal must be bound without reopening any execution owner.
Non-native settlement, sharing, Token-2022 output, ATA create/close and
production integration remain outside this original profile.

A separate [shared-request follow-up](PUMP_SDK_SHARED_PROBE.md) now connects
the prototype to a fixed request/reply binding on the existing read runtime.
Its thirteen tests were independently accepted in03299ad / Actions44327646677,
not in the original nine-test correction's CI. The [post-bank successor](PUMP_SDK_POST_BANK.md)
was independently accepted in9e1305f / Actions44339297404 / artifact14447005729,
full2044 each. Neither grants trusted quote/signing authority. The current
[swap/message constraints](PUMP_SDK_SWAP_CONSTRAINTS.md) adds positive minima
and complete unsigned templates only; eight tests were independently accepted
ina2a9f27 / Actions44348156801 / artifact14450265978, full2052 each. The
[final-simulation follow-up](PUMP_SDK_FINAL_SIMULATION.md) retains thirteen
independently accepted tests inf519207 / Actions44355235997 /
artifact14454946722, full2065 each. It does not grant quote/signing permission.

## Original Journal Integration Review

Read-only caller review after publishing f519207 confirmed the remaining
boundary is not merely a new Pool enum arm. `SdkPreparedRecord` currently
accepts only strict DAMM buy/sell profiles, and `prepared_candidate` dispatches
semantic message reconstruction by those types. `claim_operation` selects the
original SDK owner only for eligible direct DAMM operations. The original
controller then authorizes before preparation, after independent readback,
before signing and before consuming/broadcasting the persisted original bytes.
Unknown outcomes remain original-signature reconciliation, never a new owner.

A Pump extension must retain that same authority, owner/CAS ordering, row/key/
hash and readback checks, and add its own strict profile/reconstruction and
admitted-intent binding. Existing JSON profiles must encode identically and
remain readable. New profile data needs explicit compatibility/rollback
evidence even if it fits the existing nullable JSON column without new DDL.
Restoring historical metadata must not grant freshness, business authorization,
signing permission or a recovered execution owner. No such extension is
implemented or enabled by this review.

Do not broaden the Pool-based owner selector yet: the current prototype is
classic-token/native/existing-ATA and excludes rent/setup/close effects. A new
buy commonly needs ATA/volume setup; unsupported profiles cannot silently
become a regression or fall back to another execution after owner/signing
acquisition. Preparation, supported-profile selection and fail-closed policy
must be reviewed together before changing actual execution dispatch. This
remains the existing NP-01 queue, not a parallel journal or a new task.

## Original NP-01 Bank-Only Instruction Draft

The fixed SDK's actual native instruction builder still takes SwapParams with
an Arc<Keypair>, even when callers only want an unsigned instruction. Its amount
encoders are pub(crate) inside the SDK, not publicly callable service helpers.
Do not change vendor visibility or copy its whole initializer/executor to evade
the project's presigning boundary. The same test-gated native prototype now has
a private BankNativeSwap owning the existing public-account funding snapshot,
positive integer input/minimum, bank-selected buyback and exact native V1 swap.
It does not carry Keypair, TradingClient, SDK SwapParams, RPC or sender capability.

The pure construction uses the original bank roles, creator/PDA and selected
Global destinations, exact native buy/sell discriminator and two little-endian
u64 fields, plus the SDK's current legacy buy volume byte. It reuses the original
role/destination/amount validators, checks native/classic-SPL/incomplete/existing-
ATA profile, input funding and original age before/after building. The accessor
reconstructs and compares the full instruction and returns only a shared view.
No fee estimate is obtained from the SDK's globals and no snapshot age restarts.

Four Rust contracts were independently accepted as1bced80 / Actions44441322988 /
artifact14493071699: actual fixed SDK
buy/sell x cashback comparisons of full instruction and compiled message;
integer1/2^53+1/u64MAX and every account key/writable privilege plus changed wire;
twelve unsupported/damaged/expired/unfunded bank cases per direction; and exact
zero-signature handoff into the old discovery probe with its original floor1,
hash and age rejection. The SDK comparison uses the helper's own positive
minimum for ABI comparison, not as a trusted quote. Only its random buyback meta
is pinned to a member of the same bank set; the old SDK tests remain untouched.

This is still an offline native/classic/existing-ATA prototype. It does not solve
ATA/volume setup rent, quote provider truth, deployed shorter-wire/partial-fill
ABI behavior, fee schedules, close effects, non-native settlement/Token-2022,
original Pump nominal journal/intent/signing or rollout. It does not change
Pool-based owner selection, production API/validation, SDK/vendor, persistence
or any transaction permission. The preceding wallet coordination CI excludes
these files and cannot validate them. This batch's own logs/source were checked:
full concurrent2098 (355.74s),serial2098 (692.38s),all2094 parent names retained
plus exactly four; bank4/Pump60/wallet9/HTTP208,132 ordered zero-exit steps and
75 types/228 raw compiler fields. Original reader/validator/probe reconstruction
and ten actual changed-source negatives pass. All1233 snapshot files/14548221
bytes/modes were audited at publication; successor restoration was separately
checked before recording acceptance. No deployed ABI, real provider or live
permission is inferred. Never rerun this terminal batch or publish docs-only CI.

## Same NP-01 Bank-To-Final Readonly Follow-Up

After independent acceptance of parent1bced80 / Actions44441322988, the successor
connects the existing bank-only builder to the original consuming readonly chain:
funding snapshot -> native swap -> unsigned discovery probe -> bound bank quote ->
slippage constraints -> complete envelope -> final bound simulation. The discovery
floor remains one only in the zero-signature probe. The final swap uses the
original quote-derived positive minimum; provider/tip, blockhash, original age
and the same runtime/selected URL flow through without a retry or backup lane.

The resulting observation exposes only its existing shared packet accessor and
remains confined to the original cfg(test) native-funding subtree. The builder,
simulator and all old assertions are unchanged apart from subtree visibility.
The old final-response fixture is factored to accept an existing probe and an
explicit side; reconstructing the old function retains its body byte-for-byte.
No new store, parser, signer, owner, budget, production dispatcher or API is added.

Three Rust contracts were independently accepted asf84c283 / Actions44445933880 /
artifact14495271521: buy/sell with
Default/Temporal compare exact discovery/final request bodies, original IDs and
primary URL, complete frozen bytes/zero signature, original age, bank slots and
derived minimum; seven failure cases stop at the original phase with no backup
or retry; actual task abort/join at both transport boundaries releases the
original semaphore and cannot continue after the transport gate is released.
The SDK fixture is only a comparison oracle, not a production payer or quote.

Independent artifact/source checks verify full concurrent2101 (381.24s),serial
2101 (733.97s),all2098 parent names retained plus exactly three,133 ordered
zero-exit steps,Pump63/flow3/bank4/wallet9/HTTP208 and75 types/228 raw field rejects.
Original builder/simulator/fixture/tests reconstruct,994 other source/dependency
blobs are unchanged and eight actual changed-source negatives reject. All1235
files/14566203 bytes/modes were audited at publication; successor restoration
was separately checked before recording acceptance. Never inherit results or
rerun this terminal batch; records travel with actual intent-binding source.
All original profile, deployed ABI,
provider/rent, intent/journal/signing and production boundaries above remain.

## Same NP-01 Original Admission Binding

Following independent acceptance off84c283 / Actions44445933880, this successor
follow-up consumes the original final observation into a private non-Clone,
non-serializable holder borrowing the actual AdmittedTrade and the same original
reservation. Every shared packet access rechecks reservation pointer identity,
the original wallet ID/public key, literal key, direct Durable Live SDK/Pump
intent, mint/WSOL/optional canonical curve hint, no aggregator or hop, exact
positive input, slippage, tip bits and the unchanged requested-provider policy.
No new reservation, hash, store, claim selector, profile decoder, signer, SDK
initializer, RPC or broadcast capability is introduced.

Native buy binds the admitted SOL integer. Partial sell uses the original
preparation balance with the existing checked-u64 multiplication/integer division
semantics, not a new quote-bank holding or float. The prototype has no close
effects; the existing ordinary sell projection closes on100%, so that profile
is explicitly excluded here instead of pretending a swap-only message matches
it. This exclusion is test-only and changes no current production behavior.
Missing balance, overflow, zero rounding and mismatched amount are rejected.
Original timestamp and all full-message checks remain in the old packet path.

Four Rust tests were independently accepted in correctedaed9838: actual temporary-SQLite
admission buy/partial-sell with Default/Temporal retains full frozen zero-signed
bytes, writes no claim and the original Pump claim still rejects; eighteen
command mutations per direction/provider and a wrong public key reject while
compatible absent hints/Auto retain original semantics; percentage boundaries/missing/overflowing source
balance and close-on-full-sell reject; Memory/Shadow, switched reservation,
nonzero signature and expired source reject. The old fake-transport fixture is
factored with an optional sell-balance fixture; its original None path and all
old assertions reconstruct byte-for-byte. These fixtures prove no real account
or provider behavior.

Local format/policy and original source reconstruction pass, including997 other
original source/dependency blobs and fourteen actual changed-source negatives.
Own corrected CI full2105 each,all2101 parent names plus exactly four,134 steps,
75 types/228 raw field rejects,Pump67/intent4/flow3/bank4/HTTP208 are accepted below.
Parentf84 has independently passed; its result cannot validate this source.
Protocol-specific nominal persistence/reconstruction,
intent-to-original-journal handoff, setup/rent/close and real ABI/provider evidence
are still the existing NP-01 work, not a second plan or permission to sign.

### Original Admission Fixture Correction

First snapshot49b9b8e / Actions44450571354 completed failure, matching artifact
14496470144 independently checked on2026-11-20. Type check, linking,75 types/
228 raw field rejects and the first91 steps passed. Pump group passed the63
previous tests but all four new intent tests failed at the same original
admission helper: `pool_id is not used for this pool`. Full suites and the
separate intent step were not run. This is not acceptance of2105 tests.

The new fixture supplied a curve hint in the public SDK/Pump request, but the
unchanged registry/validator forbids that field. The correction removes only
that field from the fixture request. It does not relax admission, change the
binding implementation, remove assertions or alter four names. An absent hint
already exercises the intended public path; mismatched internal hint rejection
is retained. All production, SDK, metadata, claim and journal source stays
byte-identical. Publish a minimal correction and validate its own complete
2105/2105,134 ordered steps and original source identity; never rerun the failed
terminal batch or use its partial results as complete evidence.

Correctedaed9838 / Actions44657960234 completed success,artifact14596276409
independently checked: concurrent2105 (360.84s),serial2105 (700.06s),all2101 parent
names retained plus exactly four; all134 ordered steps,intent4/Pump67/flow3/
bank4/wallet9/HTTP208 and75 types/228 raw fields pass. Source/locked identity,
one-field-only correction and1012 other original source/script/dependency blobs
are unchanged, preserving original intent logic and14 source negatives. All1237
files/14589862 bytes/modes were checked before publication. Only the test request
field changed; no original validator/registry/assertion/production behavior did.
Do not rerun49b or aed; acceptance records travel with actual record successor.

## Same NP-01 Nominal Native Record Draft

Following independent acceptance of correctedaed9838 / Actions44657960234.
The same admitted-evidence holder now projects a project-owned strict nominal
record: original request/wallet/literal key/hash, account roles and selected
destinations, base program/decimals, side and original percentage balance,
quoted/minimum/final integer output, source/quote/final slots, fee/tip/native
prefunding arithmetic, actual relay and requested MEV, blockhash and frozen
message SHA256. Both the new Domain module and original SDK prototype remain
cfg(test)-gated. No production profile/decoder/owner/claim, DB write or new store
is enabled. The original SdkPreparedRecord decoder explicitly rejects this tag.

The codec bounds JSON/message sizes, uses strict unknown/duplicate rejection,
positive integer checked percentage/minimum/budget arithmetic and the original
tip conversion. It contains no SDK/RPC/key/Instant/owner or signing capability.
Historical decode only proves nominal consistency, not historical semantic
message reconstruction, chain truth, freshness or execution authority. A valid
self-consistent changed address/identity can still be decoded; rebinding to the
same live holder requires exact original record and full message comparison.
This distinction is deliberate, not permission to insert an unchecked row.

Four independently accepted Rust contracts cover buy/partial sell with Default/Temporal and
literal identity/zero signature roundtrip;21 malformed schema/amount/context/
fee cases plus duplicate/trailing/oversized/mutated-message rejection;20
nominally valid changes rejected by original bound-evidence comparison; and
historical decode after expiry cannot regenerate a fresh record/packet or enter
the old signing-record decoder. Only the original test admission helper gains
subtree visibility; its body and all old assertions are unchanged.

Own902f5e9 / Actions44662344662 success,artifact14598021039 independently checked:
concurrent2109 (384.13s),serial2109 (740.25s),all2105 parent names retained plus
exactly four; all135 ordered steps,Pump71/record4/intent4/flow3/bank4/wallet9/
HTTP208 and75 types/228 raw field rejects. Source/locked identity and original
projection/intent/fixture reconstruct,998 other original source/dependency blobs
and12 actual changed-source negatives pass. All1240 files/14616978 bytes/modes
were checked at publication; successor encoder/projection reconstruction was
separately checked before acceptance records. Never rerun terminal batches or
emit docs-only CI; records travel with actual reconstruction source.
Next original work still includes semantic historical message
reconstruction and compatible original-journal/readback integration before any
production profile/claim/signing dispatch, alongside setup/rent/close/ABI/provider
evidence. A codec is not a new journal authority or an execution permit.

## Same NP-01 Historical Message Reconstruction Draft

After independent acceptance of parent902f5e9 / Actions44662344662. The original
native instruction encoder is extracted once into a pure roles/amount helper.
Inlining that helper reconstructs the exact old builder; original before/after
freshness, bank-derived destinations/roles, input funding and access checks stay
in place. The historical path never fabricates a new funding snapshot or age.

The historical validator first validates the bounded nominal record, derives
the canonical curve/sharing/ATA roles with the original project helpers, checks
classic SPL, creator vault, fixed priority budget, exact locked SDK provider and
original requested-MEV policy. It uses the same native encoder, rejects aliased
swap roles/tip, then builds the original tip/compute/swap envelope and compiles
the complete zero-signature message with the original blockhash. All serialized
message bytes must match, not only the supplied hash, account subset or amounts.
The existing holder calls this semantic check when projecting or comparing a
record. Historical reconstruction still creates no live permit, original claim,
SDK client, key, RPC, timestamp or journal write; production decoder is unchanged.

Four independently accepted Rust contracts cover buy/partial sell x Default/Temporal exact full
historical message/zero signature; every account and each instruction's data/
program plus six header/order/shape changes rejected even after recomputing
the record hash;16 role/PDA/program/fee/provider/tip/alias changes rejected; and
a deliberately coherent altered fee record/message can satisfy historical
consistency but is rejected by the original bound bank evidence and old signing
record decoder. This last distinction prevents historical math from being
misrepresented as bank membership/chain truth or signing authority.

Own3ad246b / Actions44667188021 success,artifact14600965320 independently checked:
concurrent2113 (383.63s),serial2113 (739.22s),all2109 parent names retained plus
exactly four; all136 ordered steps,Pump75/reconstruction4/record4/intent4/flow3/
bank4/wallet9/HTTP208 and75 types/228 raw field rejects pass. Source/locked identity,
exact original encoder/guards/age/projection reconstruction,1002 other original
source/dependency blobs and12 actual changed-source negatives pass. All1243
files/14635611 bytes/modes were checked before publication; passive successor
code/body reconstruction was separately checked before acceptance records.
No local service Rust compile/test/link occurred. Never rerun terminal batches
or emit docs-only CI; records travel with actual passive-history source. Same original-journal
integration, setup/rent/close/ABI/provider and other original safety gaps remain.

## Same NP-01 Passive Original Codec And SQLite Draft

Following independent acceptance of parent3ad246b / Actions44667188021. This is the first
passive compatibility wiring into the original SdkPreparedRecord and original
SQLite row validator, not another store or execution authority. The pure domain
schema and exact SDK semantic reconstruction/encoder move to normal modules;
the original test paths bridge to the same functions, retaining all old test
names and the previous encoding/guards. No SDK initializer, RPC or signer moves.

The original strict decoder gains only pump-native-intent-v1. All previous DAMM
JSON bytes/branches stay unchanged, with no enum envelope, DDL or new columns.
Original row validation reconstructs this profile and checks canonical JSON/
digest through its existing code. Original claim admission/origin/writing/CAS/
senders do not change. Pump still cannot acquire the original operation owner.
Signed-candidate validation explicitly rejects this historical-only profile,
and historical execution recovery rejects it too; reading metadata is not
admission or signing. Older releases still reject the new tag. There is no
production writer or deployment in this batch; actual writing/rollback support
needs its later separate original-journal compatibility evidence.

Four Rust contracts use only original temporary SQLite/public fixtures:
strict codec recognition plus original Pump claim/replay rejection; actual raw
row readback cannot replace an existing DAMM owner or advance prepare/signing,
including rejection before SQL on a closed pool; six malformed tag/hash/rehash-
wire cases fail original row validation without mutation; file reopen preserves
original columns/nonce/message with no signature/owner recovery or observation.
The four older unknown-tag assertions intentionally become stronger recognized-
history checks with explicit signing rejection, and retain their other body/
assertions. Existing old DAMM codec and file/recovery regressions are not replaced.

Local source audit reconstructs exact old DAMM/codec/semantic bodies and old
tests except those documented tag checks;996 other original source/dependency
blobs remain unchanged and10 actual changed-source negatives reject. No local
service Rust compilation occurs. Prospective own CI requires full2117 each,
all2113 parent names plus exactly four,137 steps,native-history4 and unchanged
Pump75/old groups/75 types228 raw rejects. Accept exact parent evidence before
publishing; no inherited acceptance, terminal rerun or docs-only CI. Live bank
evidence-to-original-journal writing/signing, startup/rent/close/provider/ABI and
all other original gates remain. This draft does not resume PumpAPI integration.

Passive history and its gate correction were independently accepted in3e3ee2f /
Actions44673319012 / artifact14602911023: concurrent2117 (404.04s),serial2117
(763.58s),all2113 accepted3ad names plus exactly four;137 ordered zero-exit steps,
75 types228 raw compiler field rejections and locked source identity pass. The
one-declaration correction preserves1241 other blobs/modes; the complete1246
file/14658660-byte snapshot was checked at publication. The original8a source
reconstruction/996 other source blobs/10 negatives were separately checked,
not rerun against intentional child edits. History recognition does not grant
freshness, an owner, signing or execution recovery. No terminal or docs-only CI
rerun; evidence travels with the actual original-reservation successor.

### Original Module-Gate Correction

Own8a1d170 / Actions44671917774 failed, artifact14599604835 independently audited:
ten successful steps, then all-target type-check E0432. The normal historical
reconstructor imported the pure pump_curve PDA/account-key/query-description
module while its declaration was still cfg(test). Linked tests, full2117 and
the compiler boundary negatives did not run. No inherited Rust acceptance.

The correction removes only that declaration's cfg(test). The module contents,
all original tests, original claim selector, signing/recovery rejection, journal
writes, API and SDK remain unchanged. Query descriptions acquire no RPC client
or sender and the module contains no signer. The separate historical-intent
draft is held out of this minimal fix. Publish a new correction, never rerun
the terminal failed batch; require the same137 steps/full2117 each and exact
four added tests against accepted3ad before acceptance. No production actions.

## Same NP-01 Original Reservation Comparison

The module-gate correction3e3ee2f / Actions44673319012 was independently accepted
above. This successor is not in that correction and cannot inherit its result.
The existing application candidate-intent entry compares native Pump history
against the original admitted reservation, not a reconstructed execution owner.
The original bank-bound projection reuses the same pure comparison after its
unchanged nominal and semantic checks, before its final freshness recheck.

Binding includes exact request/wallet IDs, wallet public key, literal idempotency
key and request hash, durable/direct/live SDK/native SOL route, mint/curve,
integer input, original partial-sell percentage, slippage, bit-exact requested
tip and original optional MEV fields. Recalculated matching inputs cannot hide
a different user percentage. No trimming, new fingerprint, new reservation or
fresh timestamp is introduced. Historical balances still cannot prove current
bank truth. Origin first requires the unchanged original operation selector;
Pump claims/signing/recovery remain closed and original journal writes/CAS
stay unchanged. DAMM paths and old JSON/message bytes stay unchanged.

Five independently accepted Rust tests cover buy/partial-sell with Default/Temporal,19 altered
command fields,8 nominally valid metadata changes, a coherent alternative
percentage with the same wire input, and identical requests admitted under new
durable/shadow/memory reservations. They verify no original claim/signing rights
are granted. They use public fixtures/temporary SQLite, no keys/RPC or signing.
Source reconstruction preserves the old DAMM/origin/producer bodies,1008 other
source/dependency blobs and rejects10 changed-source negatives. Format/policy
checks are not Rust acceptance. Own e10037a / Actions44677923187 attempt2 /
artifact14604564014 independently passes concurrent2122 (373.94s),serial2122
(738.30s),all2117 parent names plus exactly five,138 ordered zero-exit steps,
75 types228 actual raw compiler field rejections and locked source identity.
All1248 files/14678418 bytes/modes were checked at publication. Attempt1 never
started due platform billing; only that zero-step attempt was retried after
the repository became public and the user resumed the active goal. No billing,
visibility, workflow or source changes accompanied the retry. Do not rerun
accepted batches or create docs-only CI; carry these records with real successor
code. The bank producer remains test-gated, original controller/journal live
authority and setup/rent/close/ABI/provider gaps remain in the same original work.
No new task, owner/store, production permission or PumpAPI integration.

## Same NP-01 Read-Only Bank Producer

Following independently accepted e100, the original evidence chain is promoted
out of its parent test gate. Test modules/oracles remain explicitly test-only,
all original test names and assertion bodies are retained, and narrow reexports
expose the same uncloneable bound evidence rather than a copied history object.
No SDK initializer/key/client, journal owner, configuration or API is added.

The read-only entry accepts the original AdmittedTrade, existing shared runtime,
one fixed URL and a bounded value context. It rejects wrong scope, mode, key,
provider, missing hash/slot or expired deadline before any read. The four-account
read chooses only the base program; the subsequent canonical eight-account read
must independently validate mint/program/curve/ATA/wallet/Clock. Native buy input
comes from original lamports; partial sell uses the original verified balance
and checked integer percentage. It supports only the existing native/classic/
existing-ATA/no-close prototype. No new setup, rent, Token-2022 or full-sell claim.

The old quote probe and final complete-message simulation use the same runtime,
URL, bank origin/freshness, positive minimum, fee envelope and message. A caller
deadline caps the whole sequence, not each stage independently. The resulting
evidence borrows the original reservation, retains the original private live
bank and rechecks freshness on packet/record access. Nominal reconstruction
cannot replace it. Context/provider receivers remain evidence inputs, not new
sender authorization; provider/deployed ABI/real bank trust and original journal
controlled signing remain separate existing gates. Original Pump claims/signing/
recovery stay closed, and no production caller or sender is connected here.

Five independently accepted Rust contracts cover exact four request bodies and original unsigned
bytes for buy/partial-sell with Default/Temporal;17 pre-read scope/context rejects;
each failed stage plus absent ATA; four-stage cancellation and whole-deadline
permit release; coherent history substitution and actual age expiry. They reuse
public account fixtures, never production wallets/RPC or signing. A fixture-only
raw-account extraction preserves the old decoder helper byte-for-byte after
reconstruction; the extra fixture is appended without changing old tests.

The two new capability-bearing types enter the original application/domain/
protocol compiler gate through public internal aliases:77 types/234 raw field
rejections are required. Pure history data remains allowed. Source audit restores
all original readers/decoders/builders/math/freshness/binding/test bodies,993 other
source/dependency blobs and rejects12 changed-source negatives. Format/policy
checks are not Rust acceptance. Own6e4c173 / Actions44686314535 /
artifact14608760931 independently passes concurrent2127 (380.14s),serial2127
(755.19s),all2122 parent names plus exactly five,139 ordered zero-exit steps,
77 types234 raw compiler field rejections and locked source identity. Original
functions/old assertions and993 other source/dependency blobs/12 negatives pass;
all1250 files14712170 bytes/modes/main/index were checked at publication and
after artifact download. Pump80 includes the five new entry tests.

Normal bin type-check reports100 warnings versus30 in accepted e100, test bin2
versus1; most new diagnostics are the intentionally not-yet-called normal bank
chain and its facade exports. No allow attributes, cargo fix or production
activation were used to hide them. They remain an integration cleanup issue,
not proof of execution authority or performance. Current CI is terminal success,
never rerun it or create docs-only CI; carry acceptance with real successor code.
No new task/store/owner, SDK change,
deployment or PumpAPI integration. Continue original controller/journal handoff
only after independently accepting this source, without Pool-only permission.

## Same NP-01 Original Journal Handoff (Offline Accepted)

The local successor retains the original bank object through a project-only
prepared-execution port, not a recovered nominal DTO. Native classic-SPL buy
with existing ATA and partial sell (1..99) are prepared before the original
Accepted-parent claim. Other modes/profiles keep the original path: multiple
selected lanes, V1, seed optimization, middleware and an existing SDK risk gate
are classified unsupported before RPC. A malformed bank, RPC failure or expired
evidence is an error, never an excuse to enter legacy after preparation.

The same original owner/SQLite claims table, operation, insert/CAS/signature
journal and controller remain authoritative. The qualified claim validates the
current backend before the write transaction, during owner construction and
before commit. AlreadyHeld, uncertain commit and later failure never re-claim,
fallback or sign again. Preparation failure without an owner uses the original
parent CAS only if Accepted/unsigned/unclaimed, with identity and readback checks;
it cannot mirror Unknown or Failed over another owner's result.

`pump-native-prepared-v1` is distinct from historical `pump-native-intent-v1`.
The latter stays unable to claim, sign or recover. The new tag alone also grants
no key: only the original controller's completed signing CAS plus the retained
fresh bank authorize the exact message. Key acquisition, signature validation,
signed readback and broadcast repeat these bindings. Recovery only observes the
already-persisted original signature; no prepared object/owner is restored.
Old DAMM codecs, API/hash/amount semantics, migrations and dependency bytes stay
unchanged. Older binaries cannot understand this new tag: no rolling rollback
or deployment is authorized by this source batch, especially with pending rows.

Added contracts use public unfunded fixture identities, temporary SQLite and
in-memory or loopback transports only. They cover strict tags, explicit discovery
and positive bank anchoring, atomic failure/claim competition, expiry at commit,
runtime cancellation/no fallback, actual original controller signing, uncertain
receipt and file-reopen signature-only finality. Real factory early-return paths
are tested without SDK initialization/workers or reads. Own aa8e993 /
Actions44703868863 attempt1/job136178498686/artifact14615947617 independently
passes concurrent2150 (388.51s), serial2150 (767.54s), all2127 parent names retained
plus exactly23 new,143 ordered zero-exit steps and80 types/243 actual raw compiler
field rejections. Identity/locked manifests,1216 unchanged blobs, original claim
SQL/selectors/DAMM validators/reader/sender/lane methods and old test bodies match.
All1258 files14813527 bytes/modes/main/index/history were checked before acceptance
record edits. Normal bin warnings17 versus parent100; test1 versus2, no suppression.
Six independent temporary-SQLite checks of the exact committed CAS also passed;
those alone are not Rust or SQLx acceptance. No terminal rerun or docs-only CI.
Actual factory successful HTTP-to-original-sender integration, other profiles,
ABI/provider/bank trust, setup/rent/close and real environment gates remain in
the original NP-01/03/04/05 scope. No production RPC/wallet/transaction, SDK update,
deployment or PumpAPI integration is authorized or performed.

## Same NP-01 Factory/Runtime Package (Unverified)

The unpublished package exercises the actual original Pump factory through four
strict loopback bank/probe/final reads, original claims/controller and signed-byte
journal, and the fixed original sender with a memory-only RpcSender. Its callback
checks the original broadcasting stage, signature and persisted packet before
returning a receipt. SDK/provider workers and legacy fallback panic in the fixture.
The public unfunded fixture's buyback is selected before compiling any message or
response account indices; default existing fixture behavior remains unchanged.

Eight contract groups cover buy/partial sell, bad bank/quote/final, wrong receipt
or failed receipt write, cancellation, separate step/whole preparation deadlines,
closed/missing/mismatched parents, policy rejection of only an unsigned Accepted
parent, and unavailable original storage without index overwrite. For cancellation,
the server survives task abortion and its held oracle must observe disconnection;
a signed pending send may only be observed, never prepared or sent a second time.
Status rejection is checked with and without signatures and with broadcasting both
enabled and disabled. Only the existing Pump prepared scope now reads its original
parent before broadcast-policy rejection; no new owner/store or claim SQL exists.

Formatting, source/policy and provenance evidence are not Rust acceptance. This
package does not inherit aa8e993's test result, enable another profile/protocol,
change SDK/API/dependencies, or authorize deployment or production RPC/transactions.
Run one coherent source-only CI milestone, not separate jobs for individual edits.
