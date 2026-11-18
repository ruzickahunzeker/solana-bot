# NP-01 Native Pump Final Simulation Follow-Up

Local unpublished continuation of the original ordinary-SDK safety work.
The swap/envelope parent `a2a9f27` / Actions44348156801 / artifact14450265978
was independently accepted with full2052 each, parent2044 plus8,122 steps and
64 types/195 raw fields. Source identity/reconstruction/27 bad-source mutations
and raw artifacts were checked before this record. None of those results
includes or validates this source. This is not a new
protocol/API, enabled trade capability, SDK update or rollout authorization.

## Fixed Read Boundary

`PumpNativeFinalPacket` accepts a typed versioned transaction, never encoded
caller bytes. It permits only the locked no-ALT legacy form, one zero payer
signature, a nondefault blockhash and at most1232 bytes. The complete message
contains optional payer System transfer, compute-unit price1000, limit300000,
and exactly one native Pump exact-input buy/sell with positive input/minimum.
System transfer decoding uses the existing bounded typed bincode decoder.
The first eleven returned roles are fixed by the SDK consumer; an optional
twelfth role binds the distinct relay-tip recipient. This transport type
checks wire shape, not Global/owner/role policy or the justification for min-out.

The opaque reply is tied to the exact serialized packet, ordered roles,
minimum slot and supplied bounded expiry. It cannot be rebound to a modified
template, roles or slot. The eventual SDK consumer must supply its original
preparation expiry and simulation-slot floor; the transport cannot infer the
origin of an arbitrary Instant. The original floor-one discovery packet and
its constructor remain unchanged and are not widened to accept this envelope.

The original SharedReadRuntime, one-slot AggregatorSimulation admission,
100ms start pacing, endpoint cooldown and bounded HTTP transport are reused.
No new client, quota class, pool, worker, node fallback or retry is added.
One request is bounded by min(two seconds, remaining supplied expiry), with
512KiB responses. Envelope ID/version, one result/no error, adequate slot,
explicit success and exact returned count are required. Field presence does
not prove actual provider support. New packet/reply types are forbidden in the
existing three Application/Domain/Protocol compiler gates: expected66 types/
201 original field rejections, pending CI rather than accepted evidence.

## Unexecuted Read Tests

Four read tests cover exact byte/config ownership for buy/sell with/without
tip; thirty malformed/signature/budget/amount/role/context cases and rejection
of a bare discovery packet; eight invalid RPC envelopes; original one-slot
admission and unregistered endpoints before I/O. Three real-loopback HTTP
tests cover exact512KiB boundaries, packet/role/slot/expiry rebinding, bad
HTTP/redirect/disconnect/overflow, shared429 cooldown without backup, two-second
timeout, cancellation, shorter original expiry and sequential100ms pacing.
These seven Rust tests passed independent CI below, never run locally. Only
formatting and source/policy checks are run on this VM.

## Complete Message Effects Bridge

The same unpublished work pack now connects the private complete unsigned
template, using its original preparation deadline and the quote observation's
slot floor. Only its exact frozen packet can consume the bound reply. The
original native/token effect checker is factored once, with explicit compiled
message/swap and optional tip context. The old observation still passes its
original single swap and None tip; its original math/guards are reconstructed
by a source audit, not replaced with a new oracle. The post-bank decoder keeps
the original eleven-role entry and gains a private explicit-message/count
entry for this complete-envelope context. The original decoder/policy stays.

Final checks require unchanged legacy/hash context, classic SPL Token, exact
fee equal to the quote's signature fee plus the fixed300-lamport priority fee,
positive output at least the frozen minimum, complete native/token rows and
conservation in the actual final message's indices. Native debit includes
the explicit tip without crediting sell proceeds. A tip recipient must be
pre-existing and gain exactly the frozen transfer, with System-owned empty
post data matching the indexed native balance. All other unrelated native
accounts must remain unchanged; ATA/accumulator creation/close or rent effects
outside this profile are rejected, not silently budgeted. The same decoder
checks returned token/account data, identities/Clock/epoch and original age.
This is a strict initial profile, not all native Pump transaction layouts.

`NativeFinalObservation` owns the unchanged template, post bank, output, fee
and slot. Its read-only packet accessor rechecks original freshness and the
retained fee/output/bank context. There is no raw packet mutation, signer,
sending capability, journal or trusted live quote constructor. Advertised
protocol-recipient owner metadata remains capture only, not policy proof.

Six additional SDK bridge tests use actual template bytes and loopback HTTP:
buy/sell with Default/Temporal, epoch21/slot177/original timestamp, final70M
native baseline rather than quote50M/preparation20M; sixteen malformed
fee/minimum/tip/token/account/context cases; small native sell payout below
fee and no output-credit prefunding; minimum-1/minimum/minimum+1 with matching
token account bytes; expired/altered/unsupported no-I/O cases; cancellation/
RPC errors without fallback. These six plus the seven reads were independently
accepted as the complete thirteen-test batch below, not in earlier snapshots.

The final typed request must remain separate from discovery, but validation
must not create a second parser, client, cache or journal. Missing fields,
expired evidence, inconsistent fee/data/balances and changed message/context
must be terminal rejections without signing, retries or automatic resubmission.
The combined source `f519207` / Actions44355235997 completed successfully.
Artifact14454946722 was downloaded and independently checked: concurrent2065
(394.35s), serial2065 (755.19s), parent2052 retained plus exactly13;
read4/wire3/SDK6, Pump56, HTTP208,125 ordered zero-exit steps and66 types/201
raw compiler fields. Matching source/locked hashes and original decoder/math/
discovery/HTTP/runtime/fixture/template reconstruction with22 rejected source
mutations passed. No terminal rerun or local service Rust compile/link/test.
The separate five-test config/surface successor was excluded; records are
carried with that real source, not a docs-only CI. This accepts the test-gated
final simulation only, not a trusted live quote or signer/journal integration.

Recipient-policy/deployed-ABI/provider evidence, exact final fee/rent policy
and a protocol-specific nominal record under the original signer/journal
authority remain separate gates. Never reuse DAMM records as Pump permission.
PumpAPI remains paused; no real RPC, production wallet/database access, release,
deployment, restart, signing or transaction is authorized or performed here.
