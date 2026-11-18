# WSOL Broadcast Ownership

Date: 2026-11-18, Asia/Shanghai. Continues original NP-03 / TS-07/09, without a
new task or denominator. Source and independent offline Rust CI accepted:
d1d9038 / Actions 44210742181 / artifact 14386593797. The accepted parent is
9d40936 / Actions 44204140210; its evidence was not reused for this batch.
No deployment or production access.

Follow-up standalone Quote status reads are independently accepted in 259b2fe /
Actions 44217059338 / artifact 14390716531, complete suites each 1850 and all
150 fields/89 steps. See QUOTE_STATUS_SHARED_READS.md. The scope/remaining list
below is historical for this broadcast batch, not a task to repeat. Other SDK
observers/opaque resources, global quotas and rollout gates still remain.

## Actual Boundary

PreparedWsol and SignedWsol now own WsolBroadcastLane instead of a raw RpcClient.
Preparation still chooses the first original Default RPC in the original order.
The lane moves that exact Arc through preparation and signing: no new client,
wallet lookup, endpoint selection or worker. Existing shared preparation and
both simulation paths project only its original URL and commitment as before.
The lane exposes no generic request, transaction construction or signing API.

WsolWallet::broadcast now calls SdkTradeClient::broadcast_signed_wsol. It first
runs ensure_broadcast_allowed, before wallet/snapshot/RPC access, then the exact
original validate_snapshot against the currently loaded wallet. Only that
private path can construct WsolBroadcastPacket for production. The packet is
an immutable borrow of the exact transaction in SignedWsolTransaction, not a
redecoded, rebuilt or re-signed transaction. It has no public constructor or
fields, no Clone and no arbitrary bytes/transaction input API. The private lane
send consumes the packet and calls the original RpcClient::send_transaction.

The old root send_signed_wsol body and old policy test remain unchanged under
cfg(test). Sign_prepared_wsol, validate_snapshot, authorization/instructions,
Application/journal/state/timeout sequencing and all existing test bodies remain
unchanged, apart from the existing wrapper test's required move into the lane.
The SDK still signs the prepared transaction; this is not a new signer authority.

## Deliberately Unchanged Policy

The pinned client's send_transaction sets original preflight commitment and
default send config: skipPreflight=false, Base64, maxRetries=null and
minContextSlot=null. It parses the response signature and rejects a different
signature. Do not reuse DAMM's skip-preflight/retries/three-second configuration.
Application retains its twelve-second caller timeout and uncertain-outcome
reconciliation. No new adapter timeout, retry, fallback, re-sign or resend.

This lane retains the original RpcClient's transport behavior, including any
internal HTTP/429 retry policy. Consuming the packet is not an absolute
at-most-once network/chain guarantee. A later explicit call can repeat the old
validation; only the existing Application journal controls submission authority.
Cancellation stops the local future, not a provider's remote processing. Error
strings remain original and must still pass through existing outward sanitizers;
the lane never logs packet bytes or credentials.

No shared read slot is used for broadcasting, no thirteenth read class is added,
and read quotas/capacities/intervals are unchanged. Provider-wide submission
quotas and opaque SDK resources remain original gates, not silently completed
by wrapping the client. The signed-simulation provider trust assumption remains.

## Required Evidence

Fourteen new Rust names: twelve wsol_broadcast_lane_ (nine in-memory checks,
two real-loopback parent tests and one isolated child entry) and two actual
wsol_broadcast_adapter_ checks. Complete concurrent/serial suites must each
contain 1831, retaining all 1817 accepted-parent names, adding exactly fourteen.
Two focused steps make 86 total. Policy adds the concrete sender and borrowed
packet: 47 selected types / 144 scoped field rejections including outside alias.
The existing fixed-static-lifetime gate also applies to the packet; no arbitrary
Rust syntax is accepted in policy metadata.

Compare exact original and lane send params and receipts across all four WSOL
shapes, one/two signers and processed/confirmed/finalized commitment. Real HTTP
envelopes compare without normalization, including IDs/default config fields.
Cover malformed/mismatching receipts, typed preflight error equivalence, all
original snapshot/wallet negatives before a packet, original Arc ownership/drop,
unpolled drop, cancellation and caller timeout with unchanged journal bytes.
The actual wallet broadcast must reject Disabled before accessing empty SDK
lanes or network. Actual wrappers must retain/release the selected RPC owner.

Fixtures use dummy signatures and loopback only, with proxy-free isolated child
processes. They do not load production wallet keys, initialize an SDK signer or
prove cryptographic signature validity/chain execution. Success transport tests
exercise the concrete lane; the complete loaded-wallet signing-to-network path
is not claimed as a real-node acceptance test.

Before acceptance independently audit artifacts, raw test-name sets/compiler
spans, complete source blobs/modes and manifest/lock/vendor hashes. No local
service Cargo retry after the recorded 2200 MiB failure. Use only authorized
source-only offline CI. Do not rerun terminal CI or start documentation-only CI.

## Remaining Original Work

Standalone Quote reconciliation reads, DAMM/opaque SDK reads/tasks, provider-wide
quotas and NP-01/03/04/05/rollout gates remain. SDK stays 6.0.2. No changes to
API/amounts/wallet identity, dependency manifests/lock/vendor, migrations or
production configuration/data/binaries/services. New PumpAPI work stays paused.

## Accepted Evidence

Source `d1d9038fec184cb62519741572d70e81fab4fc57`,
[Actions 44210742181](https://github.com/ruzickahunzeker/solana-bot/actions/runs/44210742181),
its first run terminal success (job 21m37s). Downloaded artifact `14386593797`
independently audited: all 86 steps exit zero. Full concurrent 1831 (233.10
seconds) and serial 1831 (447.86 seconds), zero failed/ignored/filtered tests,
identical raw name sets. All 1817 accepted-parent names retained and exactly
the fourteen source-defined names added. Lane 12 / actual adapter 2 and every
earlier focused regression pass, including both shared simulation forms and
the complete 59-name WSOL status contract.

Original/restored compiler checks and all 144 distinct raw field spans pass:
47 selected types plus outside alias, including the borrowed packet. Python
policy 12 / workflow 7 and SDK provenance pass. Identity and manifest/lock/
vendor-manifest hashes match. Before recording acceptance all 1123 snapshot
blobs/modes (13,648,153 bytes) match the exact source. The root SDK module,
including all original tests/authorization/sign/snapshot/send-oracle bodies,
is identical after removing only module registration and cfg/import wiring.
Existing wrapper assertions and the complete original HTTP fixture behavior
are identical after removing only the explicitly added variant/dispatch/checks.
Application/journal/state sequencing, preparation/simulation code and runtime
capacities/intervals remain byte-identical. Actual production guard/sender wiring
and no-normalization original request/result witnesses are independently checked.

The old serial discovery child-stderr split is associated only with its exact
name/pinned fixture/immediate ok; two positive/seven negative format regressions
pass. No raw logs or name sets are rewritten. Main/index/frozen history remain
unchanged. No local service Cargo retry, release build, deployment/restart,
production data access, real RPC or trade. Do not rerun terminal CI or trigger
documentation-only CI; carry this acceptance record with the next source batch.
