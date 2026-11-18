# NP-01/03 Native Pump shared probe

Follow-up after independently accepted `45d18c0` / Actions `44319842306` /
artifact `14437675802`: full2026 each, nine original packet/response-consistency
tests. That parent does not include or validate this new shared-request source.
No new task, rollout profile, SDK version or public API is introduced.

## Request ownership

`PumpNativeProbePacket` freezes the exact serialized zero-signature legacy
transaction, eleven requested account roles, original minimum slot and remaining
evidence deadline. It permits only one native exact-input Pump swap with floor
one for discovery, a nondefault hash, one zero-signature payer and a packet at
most1232 bytes. It is not an executable candidate or an encoded-packet API.

The service-owned `SharedReadRuntime` consumes it with the existing aggregator
simulation semaphore, pacing and shared endpoint cooldown. It uses the original
bounded HTTP transport and512KiB response limit. There is no new HTTP client,
semaphore, node retry, background task or sender. The request stays on its
selected registered endpoint;429 cools that endpoint for the other existing
read profiles too. The deadline is the lesser of two seconds and the remaining
three-second snapshot window. Admission, pacing, I/O, parsing and reply access
all consume that original window.

The JSON-RPC envelope must have the matching numeric ID, version2.0, one result
and no error, an adequate slot, explicit `err:null` and exactly eleven returned
account positions. `PumpNativeProbeReply` retains the original serialized bytes,
role order, slot and deadline; it cannot be reused for a changed transaction,
account list or minimum slot. Neither packet nor reply is Clone/serializable.
Both are added to the existing three-layer forbidden-type policy; the expected
gate becomes64 types/195 raw field rejections, not merely an import-text check.

## Existing SDK Prototype

The original test-gated native probe derives eight snapshot roles plus the
actual instruction's three recipients, sends that frozen request via the same
runtime and consumes only its bound reply. Before and after I/O it rechecks the
original three-second evidence age. Stale or Token-2022 input is refused before
I/O. The earlier same-response integer output/conservation validator is reused
without changing its checks or promoting its output to a trusted quote.

This does **not** prove returned-account ownership/data, same-simulation epoch,
recipient existence/fee tier/rent, deployed ABI or real provider support. The
eleven returned positions are currently shape-checked only; loopback fixtures
deliberately do not pretend null account data is ownership evidence. These
requirements, final positive live min-out, complete message authorization and
the original signed journal remain gates before production integration.

## Verification Status

Thirteen tests cover seven packet/shared-runtime tests, three real
loopback HTTP tests and three actual-SDK-prototype bridge tests. They cover
exact request/response binding, signed/invalid packets, ID/version/result/error
conflicts, missing outcome/slot/account positions,512KiB limits, redirects,
cooldown without fallback, deadline/cancellation and same-response output.
Independent `03299ad` / Actions `44327646677` completed successfully. Artifact
`14440796497` was downloaded and audited: full concurrent2039 (359.38s),
serial2039 (699.35s), all2026 parent names retained plus exactly13; focused
read7/wire3/adapter3, Pump37, HTTP208,119 ordered zero-exit steps and64 types/195
raw compiler field rejections. Source identity/locked dependencies, original
runtime/HTTP/fixture/observation reconstruction and ten rejected source
mutations were verified. No local service Rust execution or terminal rerun.

The shared profile is not in the parent's CI. Do not inherit its result,
rerun terminal tasks, change API/validation/persistence, enable PumpAPI or use a
production wallet/database/RPC. SDK remains6.0.2; no release/deploy/restart or
transaction has been performed.

The [post-bank follow-up](PUMP_SDK_POST_BANK.md) tightens the test-gated
SDK bridge using the existing decoder and actual returned account bytes, with
five more tests. It is not in this shared-profile CI and has no Rust acceptance
yet. Recipient policy and final authorization remain distinct unfinished gates.
