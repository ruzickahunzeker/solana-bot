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

No simulation request or post-account proof is connected yet. Provider support,
request ID/packet binding, returned account owners/data and current epoch,
deployed ABI, blockhash validity, actual fees/rent and fee tier remain unproven.
The observation is not a trusted quote or signing permit. Next bind the
response to the fixed shared-runtime request and revalidate returned accounts,
then bind a separately justified positive live minimum into a complete
immutable candidate and the existing original signed-byte journal. None of
those gates is removed by this test-only validator. Non-native settlement,
sharing, Token-2022 output, ATA create/close, extra budget/tip instructions and
production integration remain outside its profile.

A separate [shared-request follow-up](PUMP_SDK_SHARED_PROBE.md) now connects
the prototype to a fixed request/reply binding on the existing read runtime.
Its thirteen tests have not run, it is not in the nine-test correction's CI,
and it does not complete post-account proof or any trusted quote/signing gate.
