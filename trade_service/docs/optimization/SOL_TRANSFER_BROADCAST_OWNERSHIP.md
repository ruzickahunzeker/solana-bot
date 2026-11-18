# SOL Transfer Broadcast Ownership

Continues the existing NP-03 independent-operation ownership work. The new
module and tests are wired; independent Rust acceptance is pending. The
confirmation baseline `c9394e7` passed acceptance in Actions `44257386302`.

## Invariants

- Carry the exact existing RPC chosen by successful transfer preparation into
  the signed artifact, through a private `SolTransferBroadcastLane`. Do not
  choose another RPC at sign or send time. Confirmation receives only URL
  and commitment from the lane, not its general-purpose RPC handle.
- Retain the SDK `send_transaction` call, selected commitment and original
  12-second timeout and context messages. Policy, payer, snapshot, message,
  signature and byte checks remain before that call. Only the artifact's
  bound lane may send; the private helper takes no separate RPC argument.
- One SDK invocation, no application retry, fallback or receipt rewrite.
  This does not change the pinned transport's internal behavior or the
  provider's own rebroadcast policy. A timeout does not prove non-inclusion.
- Keep the Application's write-ahead record/check/submit/observe order and
  public response unchanged. A receipt error remains `Unknown`; no confirmation
  request or retry on that path. Do not change transfer amount rules, signing,
  the public API, storage format or startup configuration.

## Verification

Three new SDK-side witnesses use synthetic invalid signatures and the existing
in-memory sender, with no socket transport: actual preparation falls back to
the original backup, only that backup sees the exact signed bytes, and its
complete request matches the pinned SDK oracle for all three commitments.
Wrong wallet or altered signed bytes cause no send to either endpoint. SDK
error, deadline and mismatched receipt retain original artifact bytes and
signature, invoke the selected sender once and do not read confirmation.

The old prepared-snapshot, tamper, timeout, signature and HTTP journal tests
must remain. Only switch their test pointer assertion to a private lane identity
check; retain the original raw confirmation oracle under `cfg(test)`. The
new lane's visibility is limited to the SOL transfer module, so it is not a
new boundary-policy entry. Require focused and full concurrent/serial tests in
the authorized source-only CI after wiring: three new names, 1,981 per suite,
108 steps and the same 174-field boundary check, retaining the 1,978-test
confirmation parent. Local formatting, source comparison,
workflow/policy and SDK provenance checks pass. Do not build locally, modify SDK
dependencies, deploy, restart or use production wallets, databases or RPCs.
