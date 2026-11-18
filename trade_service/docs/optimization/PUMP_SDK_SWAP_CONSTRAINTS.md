# NP-01 Native Pump Swap And Message Constraints

This is a local successor of the original post-bank prototype, not a new
protocol, public endpoint or execution profile. Post-bank correction `9e1305f`
/ Actions44339297404 / artifact14447005729 was independently accepted with
full2044 each, parent2039 retained plus5,120 steps and64 types/195 fields.
Matching raw artifacts and source reconstruction/ten rejected mutations were
checked before these records. None of that run's results can validate this
source, which was deliberately excluded from the correction. Eight successor
tests still await their own independent CI.

## Amount And Packet Binding

The existing private `NativeBankObservation` is consumed into a private,
non-Clone, non-serializable `NativeSwapConstraints`. It retains the complete
original observation and post bank. The integer minimum is
`net_output * (10000 - slippage_bps) / 10000`, using `u128` intermediates.
Slippage must be below 10000; a zero result is rejected, not replaced with one.
No float or UI amount participates.

The zero-signature swap-only template changes only the frozen instruction's
second amount operand. Exact input, discriminator, trailing flags, accounts,
privileges, payer, blockhash and single-instruction form remain unchanged.
This is not SDK `fixed_output_amount`, which would change the buy semantics.
Construction rechecks native destination identities and the 1232-byte limit.
The packet accessor rechecks original freshness, minimum, native budget and the
complete compiled message, including exactly one default signature. It does
not renew the original three-second window.

The floor-one discovery RPC constructor is unchanged and must reject a template
whose formal minimum is not one. This work does not widen that RPC profile or
pretend it already supports final-candidate simulation.

## Swap-Only Native Debit

The validated same-response post wallet balance and observed net delta/fee
reconstruct the pre wallet balance; the earlier preparation RPC balance is not
used. Buy reserves its exact input plus network fee. Sell reserves its network
fee without crediting expected proceeds or any future ATA close refund.
`checked_add`, `checked_sub`, `i128` and fallible conversion reject overflow or
invalid pre balances. Bank timestamp, keys, side, quote and simulation slot must
remain bound to the original observation, including on packet access.

This is only a swap-only debit constraint. It excludes compute-budget/tip
envelopes, setup/close instructions, extra rent and new fee estimates. It is not
a complete transaction spending authorization or a final simulation result.

## Complete Unsigned Message Template

The same unpublished work pack also freezes a complete unsigned message in a
private `NativeEnvelopeTemplate`. It reuses the existing in-process instruction and
compiler helpers under `damm_instructions`, without changing those helpers or
using any DAMM nominal authorization. A live-shaped tip (when applicable),
compute-unit price1000, limit300000 and exactly the validated swap are frozen
in their original order. The SDK's V0-without-ALT legacy choice, sole wallet
signer, nondefault original hash and1232-byte limit are preserved. Tip route
membership uses the existing provider list; a tip may not alias any frozen
swap account. No provider list, client, initializer or sender is copied. The
locked SDK's existing compute-budget instruction cache is still used by that
helper; this is not proof that all SDK global resources have been isolated.

Native prefunding adds the fixed300-lamport priority ceiling and rounded relay
tip to the swap-only debit, using checked arithmetic. No sell proceeds or
refunds fund that ceiling. Nonfinite/negative/out-of-range tips are rejected
before the existing compatibility rounding. Default provider omits tips just
as the existing helper does. Minimum/amount, envelope and source remain owned;
packet access rechecks the source, budget, tip identity, full instructions,
zero signature and exact canonical compiled message before returning a shared
reference. It neither executes nor simulates that complete message.

The existing helper's fixed fee layout is explicitly pinned by tests, not
assumed to survive an SDK/profile change. The initial probe's observed network
fee plus300 is a construction ceiling under that fixed layout, not proof of
the final provider's fee or blockhash validity. A later final simulation must
verify actual balances/fee/rent/output for the complete message. All remaining
policy/ABI/signing gates below still apply.

## Verification Pending

Four swap tests cover exact input/message roles across both sides and slippage
boundaries; same-response prefunding including a sell payout below its network
fee; integer/zero/invalid-slippage cases; and context, amount, signature,
instruction, budget and expiry damage. Old post-bank/probe/bridge assertions
and all RPC profiles remain unchanged. Four additional envelope tests cover
buy/sell with Default/Temporal/Jito and zero/nonzero tip, independent standard
message, unchanged compiled signer/writable privileges for every original key,
raw compute-budget bytes, exact-balance/one-lamport-short
prefunding, invalid tip/provider/alias routes, ten packet/fee/context damages
and expiry. Complete envelopes are also rejected by the unchanged discovery
RPC profile. These are fixtures, not claims of live provider compatibility.

Expected future evidence only: parent2044 names retained plus exactly8, full
concurrent/serial2052 each, constraints4/envelope4, post-bank5, Pump50, original
read7/wire3/adapter3, HTTP208,122 ordered steps,64 types/195 compiler fields.
No Rust compile, link or test is run locally; only formatting and source/policy
checks. Publication waits for independently accepted post-bank evidence.

## Remaining Gates

Recipient owner/data policy, actual provider support, deployed ABI, current
fee/rent/blockhash validity, complete envelope, final simulation and the
original controlled signer/journal remain unfinished. Existing DAMM nominal
records accept DAMM profiles; they must not be reused as Pump permission.
Any later Pump record needs an audited protocol-specific version under that
same original journal authority, not a parallel journal or public API.

SDK stays6.0.2. No SDK/vendor/manifest/lock/migration/API/persistence or
production behavior changes; no PumpAPI integration, real RPC, wallet/database
access, release/deploy/restart, signing or transaction.
