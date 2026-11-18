# NP-01 Pump.fun same-bank native destination evidence

Follow-up source after `9bb5330`, not validated by that commit's accepted CI.
All new work stays inside the existing test-gated ordinary SDK adapter. No
production request, SDK version, route, journal, wallet or RPC configuration is
changed. The existing raw account validator is reused without changing the
paused PumpAPI integration.

The curve evidence now retains the validated bounded Global bytes from its
original snapshot. The native funding decoder uses that same object; there is
no second Global lookup or externally replaceable fee snapshot. A fixed-size
Serde/bincode prefix matches the Borsh-compatible primitive/array fields in
the pinned IDL through `buyback_basis_points` (byte 1005 including discriminator).
There are no variable-size vectors or unbounded allocations in that decoder.
Additional fields stay opaque. Tests encode this prefix from the pinned IDL,
independently of the decoder struct.

`Global.initialized` is explicitly marked unused in the IDL. Its boolean
encoding must be valid, but false is not an inactive-trading signal. The older
test-only prefix check is corrected accordingly, retaining owner, discriminator,
length, positive account balance and nonzero primary recipient checks.

For the initial standard, native SOL V1 evidence profile:

- The curve must be non-mayhem and the sharing PDA must have been explicitly
  absent in the same successful snapshot. Present status 0/1/unknown is not
  equivalent to absence; fee-sharing execution is still unapproved.
- The primary protocol fee is frozen from that Global account, not replaced
  by a static SDK fallback. Alternate primary addresses are parsed but do not
  broaden the frozen selection.
- The creator vault must be the canonical PDA of the snapshot's curve creator,
  not an SDK caller/cache hint. The actual SDK builder test demonstrates that
  such a hint can otherwise override derivation.
- The last V1 buyback account must belong to the same Global's eight-member
  buyback set, and the fee/creator/buyback roles must have expected privileges.
- This initial native profile rejects repeated account roles. A destination
  sharing the fixed fee-program key would otherwise promote the compiled
  program account to writable even if its own meta remains readonly. The test
  mutates an actual SDK instruction with a matching bank primary, compiles a
  standard Solana message without signing, and inspects its header/account order
  to witness the promotion before requiring the destination guard to reject it.
- The amount/critical-role inspector runs first. A shorter Global prefix,
  malformed bool, unknown layout, unset buyback address or invalid fee field
  cannot produce destination evidence.

Three new tests are prepared: bank-backed destinations in actual SDK buys and
sells; rotated/static recipients, caller-supplied creator hints and damaged
destination metas; and incomplete Global/sharing/mode/layout/rate failures.
The original prefix test also gains the valid unused-false case. Rust execution
is pending, not inferred from the parent CI or formatting check.

This is destination identity evidence only. It does not establish the active
fee tier, transfer fee/net output, deployed program ABI, current recipient account
owners/existence, creator-sharing semantics, rent/total-spend adequacy, final
compiled-message authorization or original signed-byte journal. Those remain
required before production integration or any signer access.
