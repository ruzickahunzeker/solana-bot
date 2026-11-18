# SOL Transfer Preparation Shared Reads

Date: 2026-11-18, Asia/Shanghai. Continues the original NP-03 independent-operation
read/ownership queue, not a new task, transfer feature or SDK upgrade. Production
wiring and local source audit are complete; independent Rust acceptance is pending.
The preceding vanity source 3d72a14 / Actions 44245588737 / artifact 14403371260
is independently accepted (1939 each complete suite); do not inherit that evidence.

## Existing Path And Invariants

The actual NativeTransferWallet::prepare calls SdkTradeClient::prepare_sol_transfer.
It checks the original broadcast policy, gets the original first payer's public
key, then tries Default lane RPCs in original order. Each lane has one five-second
total budget for getBalance -> getLatestBlockhash -> getFeeForMessage ->
getMinimumBalanceForRentExemption. Fee uses the original one-lamport system
transfer message and selected RPC commitment; rent uses data length zero, not
the 165-byte SPL token account rent used by WSOL preparation.

Only RPC failures/timeouts permit trying the next original lane. Balance, fee,
rent and blockhash from one successful lane form the input to the original amount
policy; an amount/reserve failure is terminal, not a reason to try a richer node.
Preserve fixed/50%/100%, full integer values, original error ordering and the
selected original RPC Arc carried in PreparedSolTransfer for later broadcast.
No same-bank atomic snapshot, new freshness, balance recheck or quote is implied.

## Draft Read Boundary

A private-constructor, non-Clone SolTransferReadSession borrows the original
SharedReadRuntime, snapshots the original lane URL/commitment/public addresses,
and consumes one ordered four-read scope. It has no signer, execution RPC,
transaction or send method. The fee request is constructed internally from the
same original one-lamport message, not caller-provided instructions. Out-of-order
calls issue no I/O; an attempted valid stage closes on failure and cannot retry
or advance within that scope. Production exposes only the consuming snapshot.

Reuse the existing WsolQuote permit/start budget (100 ms) and global endpoint
cooldown; do not add a thirteenth class, HTTP client, cache or worker. Preserve
one five-second whole-lane deadline and post-I/O/parse checks. A fixed five-second
HTTP override is required so the shared transport's two-second default cannot
silently shorten a valid original preparation; no per-step deadline renewal.
No retry or alternate endpoint is available inside the scope; only the original
outer SDK preparation loop can advance to its next configured Default lane.

Parsing keeps original typed/scalar behavior, including zero fee/rent, context
slot zero and u64 values. Do not import DAMM quote bank/slot policies into native
transfer evidence. Missing/invalid context, hash, nullable fee, bad identity,
oversized body, redirects and 429 fail without turning missing data into zero.
SDK-internal retry scheduling/provider error strings are not claimed byte-exact;
the shared bounded/no-retry/cooldown policy is explicit and errors redact URLs.

## Required Actual Wiring And Evidence

The existing SOL transfer runtime must receive the same bootstrap read runtime;
NativeTransferWallet passes it to preparation only. Root keeps original policy/
payer/lane guards, final message/source/amount checks, signer, signed bytes,
broadcast, confirmation and Application journal flow. Keep the old raw RPC
preparation function and all its existing tests as cfg(test) oracle. Do not merge
the independent transfer journal into ordinary trades or reinterpret receipts.

Twelve scope tests exercise exact configs/fee bytes/order, single-use/failure
closure, registration/quota, scalar/schema faults, cooldown, cancellation/hash
independence, pacing/IDs and the original five-second budget. Six actual SDK
preparation tests delegate to the original MemoryRpc oracle, compare complete
requests/messages/snapshots/errors and selected Arc, cover all four failure stages,
no mixed-lane funding, terminal amount failures and no send on cancellation.
Three real loopback wire cases cover exact four requests/body bounds, bad replies/
redirect/429/no retry, original long HTTP allowance, whole scope expiry and cancel.

Only publicly fixed addresses, synthetic requests and fresh loopback/memory
fixtures are allowed; no keypair generation, SDK initialization, real RPC,
production configuration/wallet/database or broadcast. Existing test names and
original broadcast/confirmation/amount-policy bytes must remain. Independent
compiler policy/source/full-suite evidence is required on the final source;
no local service Cargo check/test/link or docs-only CI. Confirmation/status and
broadcast RPC ownership, SDK opaque internals and remaining original NP-01/03/04/05
and rollout gates remain separate, unfinished work.

## Current Source And Acceptance Queue

Production receives the same bootstrap runtime Arc; only preparation delegates
to the ordered borrowed scope. The original raw preparation oracle is cfg(test),
with its entire function, MemoryRpc and old assertion bodies retained after
explicit module/runtime-argument wiring. Final amount/message/snapshot tail,
Prepared/Signed types, source/sign/send/confirm and Application/journal remain
exact. Test-only HTTP server adds a distinct strict four-method/rent0/64KiB branch;
all old branches, wire cases, 15-second child and body limits reconstruct exactly.

Expected independent acceptance: scope12 / actual SDK oracle6 / real wire3,
complete concurrent/serial each1960 with all1939 accepted-parent names retained
and exactly21 added; all104 steps and56 selected types/171 raw field rejections
including outside alias. The added borrowed scope joins the existing fixed
static-lifetime denial policy. Actual SDK success comparison spans36 combinations
(three commitments, three integer balances including >2^53/u64MAX, four modes),
not just approximate amounts. Full original request arrays are compared, not
filtered to hide extra RPCs. Local format/policy12/workflow7/provenance and source
audit one positive/ten read-only negatives pass; no local service Cargo ran.

## First Candidate Failure And Fixture Correction

Source 67d1d3935bb1d9bbf89bed589bc6decf1f8626cb / Actions 44251289587 is terminal
failure; never rerun it. Artifact14404017903 was downloaded. The first ten
preflight/policy/namespace/DDL steps pass, then type-check exits101 with exactly
three E0061 diagnostics: the old shared HTTP state, native-transfer HTTP fixture
and SDK broadcast-isolation SOL test missed the new read-runtime argument.
No Rust runtime tests, new171-field gate or complete suites ran; production
compilation diagnostics alone are not acceptance. Existing warnings are not
suppressed or fixed as part of this correction.

Only those three test call sites receive the same existing fixture runtime or
a new empty, network-free test runtime. All old HTTP/auth/status/body and SDK
isolation assertions remain byte-exact after that explicit argument wiring;
production preparation/sign/send/confirmation/API/journal/SDK source is unchanged
from67d1d39. Re-scanned every constructor/preparation call. The source verifier
now reconstructs these full old fixture files as well as the earlier oracle/
transport/wire contracts; one positive/ten read-only negative mutations pass.
The corrected source requires a new independent CI run and full acceptance,
not a retry of the failed terminal job or inherited parent evidence.
