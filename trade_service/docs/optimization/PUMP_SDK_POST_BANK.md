# NP-01 Native Pump simulated post bank

Follow-up after independently accepted shared-request snapshot `03299ad` /
Actions `44327646677` / artifact `14440796497`: full2039 each and thirteen new
shared/HTTP/SDK tests. That snapshot does not include or validate this source.
Five new Rust tests passed independent CI below, never executed locally. This remains the
original test-gated ordinary SDK work, not a new protocol/API or rollout task.

## One Existing Decoder

The fixed shared reply requests eleven positions. The stricter SDK prototype
now deserializes them and runs the original eight-role `NativeFundingPlan`
decoder over curve/mint/Global/sharing/curve ATA/user ATA/wallet/Clock. Owners,
binary layouts, canonical keys, mint/vault authority/extensions and same-bank
Clock slot/epoch retain their existing checks. No extra RPC, second parser or
earlier-RPC balance subtraction is added. Mint precision/supply and the frozen
instruction's fee/creator/buyback roles must still match that simulated bank.

The original decoder's default remains Preparation and rejects completed
curves. An explicit private AfterSimulation phase permits a completed buy only
with zero real-token reserves. A sell cannot newly declare completion, and a
completed curve retaining real tokens is rejected. All other decoder checks
are shared. This distinction does not authorize a new buy on a completed curve.

The probe retains its validated instruction and requires its canonical compiled
message to equal the frozen packet. Post lamports for every requested message
account must match the same response's indexed native balances; decoded ATA
units must match the same response's two token rows. Existing positive net
output/input/fee/conservation checks still run. Post decoding explicitly keeps
the original preparation timestamp, including across a simulated epoch change,
and rechecks freshness after validation instead of renewing the window.

## Recipient Metadata Is Not Authorization

The three requested recipient positions must exist, have valid bounded binary
metadata, be non-executable/positive and match their indexed post lamports.
Their public keys, advertised owners and data lengths are captured. **An
advertised owner is not proof that the protocol permits that owner.** Fixtures
intentionally use an arbitrary synthetic owner to demonstrate capture only;
they do not certify System/Pump ownership, fee-tier eligibility or rent.
Recipient owner/data policy, deployed ABI and actual provider support still
need independent evidence. This result is not a trusted quote, complete spend
authorization, positive live min-out, signing permit or signed journal record.

## Independent Offline Acceptance

The original three SDK bridge tests keep their names, exact packet/result and
old failure/no-I/O assertions. Positive replies now contain actual generated
account bytes instead of null positions, and add a decoded bank-slot assertion.
Five new tests cover shared decoding/epoch/original age, owner/ATA/Clock/fee
identity errors, indexed/native/token/metadata contradictions, terminal-buy
versus preparation rules, missing fields, stale origin and instruction drift.
Old unbound-model tests remain unchanged.

Correction `9e1305f` / Actions `44339297404` completed successfully. Artifact
`14447005729` was downloaded and independently checked: concurrent2044
(389.05s)/serial2044 (743.91s), all2039 parent names retained plus exactly5;
post-bank5, Pump42, HTTP208,120 ordered zero-exit steps and64 types/195 raw
compiler fields. Original shared read7/wire3/adapter3 remain. Source/locked
dependency identity, original decoder/probe/bridge assertions reconstruction
and ten rejected source mutations passed. This accepts the post-bank prototype
only; the separate eight-test [swap/message successor](PUMP_SDK_SWAP_CONSTRAINTS.md)
was excluded and still needs its own independent CI. No terminal rerun.
No local service Rust compile/link/test, SDK upgrade, release/deploy/restart,
real RPC, production wallet/database access, transaction or PumpAPI integration.

## First Run Fixture Correction

Snapshot `9029053` / Actions `44335341139` ended in failure; artifact
`14440804968` is retained. The metadata size negative expected the encoded-size
guard for 8193 decoded bytes. Both 8192 and 8193 bytes occupy 10924 base64
characters, so the original decoder correctly rejected the latter at its
decoded-length guard instead. No invalid account was accepted.

The isolated correction expects that decoded-length rejection and adds a
separate 8195-byte case for the encoded-size rejection. All five test names and
original guards remain; the metadata matrix now has nine cases. No production,
decoder, RPC, fee or assertion policy is relaxed. The subsequent local swap
constraints prototype is held out. Do not rerun the failed snapshot or inherit
its results; the correction has its own independent complete acceptance above.
