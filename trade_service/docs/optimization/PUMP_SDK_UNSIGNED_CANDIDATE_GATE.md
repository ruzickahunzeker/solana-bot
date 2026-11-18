# NP-01 Pump.fun unsigned candidate acceptance

This records the follow-up to [the account snapshot](PUMP_SDK_ACCOUNT_EVIDENCE.md)
within the existing NP-01 work. It is a source audit, not an implemented
capability or a new rollout task. PumpAPI, the locked SDK and current routes
are unchanged.

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
