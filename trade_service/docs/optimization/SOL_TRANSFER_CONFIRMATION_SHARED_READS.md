# SOL Transfer Confirmation Shared Reads

Continues the existing NP-03 independent-operation read boundary. Production
wiring and source-only acceptance are complete, see below. This is not a
change to the public API, journal or submission policy.

## Preserve The Actual Contract

The pinned client's `confirm_transaction` makes one `getSignatureStatuses`
request with `[[signature]]`, without `searchTransactionHistory`. It evaluates
the first status with the original selected RPC's commitment and the legacy
`status.is_ok()` field. Reuse the pinned `satisfies_commitment` implementation,
including absent `confirmationStatus`, rooted/legacy confirmations and original
context semantics. Do not substitute WSOL's stricter slot and error policy or
force finalized commitment. Reject malformed envelopes, empty/multiple entries
and invalid signatures without panicking or reporting confirmation.

The reader borrows the same `SharedReadRuntime`, not a wallet, signer, SDK
client or send handle. Bind to the artifact's original signature and the exact
RPC selected by preparation; no alternate endpoint, retry or poll. Reuse the
Observation class permit, one-second start pacing and endpoint cooldown. Keep
the existing 15-second confirmation allowance, including pacing and decoding,
instead of shortening it to the other status profiles' two seconds. The bounded
HTTP response and post-decode deadline checks cannot grant a late success.

Quota contention, cooldown or malformed/late replies remain unconfirmed at
the existing Application boundary. The status stays `Submitted`; this result
does not assert non-inclusion or trigger another send. Sharing the observation
slot can affect which concurrent read is admitted, particularly with a slow
node; it is not a claim of equivalent timing or an isolated provider quota.
The existing `transfer_http_confirmation_absence_keeps_submitted_signature_without_claiming_confirmation`
contract remains the HTTP-level witness: original signature, no error or
confirmed write, and one broadcast. It complements the new real-reader tests
rather than substituting for them.

## Wiring And Acceptance

Pass the existing `NativeTransferWallet` runtime to the confirmation helper.
Keep the raw RPC confirmation as a test-only oracle and leave the original
prepare, sign, send, receipt comparison and persistence logic unchanged. Add
only a fixed request variant, HTTP budget and the borrowed reader to the
dependency gate. Do not give the Application or Protocol layers a raw RPC.

Coverage: 12 scoped tests include exact parameters, 768 typed legacy status /
commitment cases against the pinned `RpcClient`, malformed replies, pacing,
quota sharing, independent blockhash refresh and cancellation. Three SDK tests
compare actual artifact-bound outcomes with the old oracle, including selected
backup endpoint, preserved bytes, mismatch and timeout. Three loopback cases
cover real HTTP bounds, no retry or redirect, a reply beyond two seconds and
permit release after cancel. The full 15-second boundary is exercised by the
in-process fake, not by relaxing the wire child's existing limit. All fixtures
are public/synthetic; no real node, production wallet, database or broadcast.

Keep all 1,960 accepted parent test names and add exactly 18: complete suites
of 1,978 each, 107 steps and 174 rejection spans for 57 types plus the outside
alias. New CI
evidence is required for the focused suites, full concurrent/serial suites and
the updated type gate. No local service check/link, SDK upgrade, deployment,
restart or production operation. Broadcast RPC ownership is a separate
remaining item, not completed by the confirmation reader.

## Independent Acceptance

Snapshot `c9394e7310d9f428bf262a284a6f51d3b29a035c`, Actions `44257386302`,
artifact `14408765651`: all 107 steps passed. Concurrent 1,978 (360.21 s) and
serial 1,978 (697.06 s) retained all 1,960 parent tests and added exactly 18.
Focused groups passed 12 read, 3 adapter and 3 loopback tests. Original and
restored compiler checks and all 174 raw field rejections passed for 57 types
plus the outside alias. Verified the complete 1,166-file snapshot (14,016,617
bytes) before the successor, and reproduced source checks from the exact
commit after local successor wiring. No local service Cargo test, deployment,
production data access, or real RPC/broadcast. The next work is
[broadcast ownership](SOL_TRANSFER_BROADCAST_OWNERSHIP.md).
