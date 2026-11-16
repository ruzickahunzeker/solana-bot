# SDK Buy History And Original Intent

Date: 2026-11-16. Continues the existing NP-01/main plan section 3.3, after the
accepted common-record boundary `2fe436d`. It does not open a buy execution claim.

## Actual Boundary Changes

- `adapters/sdk/prepared_candidate.rs` selects the existing sell reconstruction
  or the existing buy reconstruction by the project-owned record variant. It then
  validates the bounded canonical signed packet, exact original message, single
  signer/payer, wallet signature and reported signature. The former sell evidence
  implementation is shared, not duplicated or replaced by hash-only validation.
- The seller-specific signed entry still rejects a buy record. The original
  journal's raw row/signed evidence checks now use the shared validator. A valid
  nominal buy row may reconstruct, but cannot bind to a sell parent or owner.
- `application/sdk_preparation.rs::validate_candidate_intent` binds a buy to its
  original admitted wallet, direct route, integer input asset/units, Live SDK mode,
  slippage and MEV intent. Native SOL/WSOL and existing USDC are nominally distinct.
  `validate_origin` still requires the existing operation classifier first; that
  classifier remains **sell only**. Pure intent binding cannot grant an owner or
  manufacture a fresh quote/linear signing permit.
- Historical request replay dispatches by record. Buy replay uses the original
  V1 amount parser for explicit fields. Its default is supplied lazily: normal
  validation still converts the same current default at the same point; historical
  omitted WSOL amounts use the original raw integer, without Config or a float
  round trip. USDC still requires an explicit amount. The original fee defaults,
  percent precedence, aliases, request/hash format and payment-branch checks remain.
- The existing ownerless observation capture now validates a signed buy history
  against the original parent, literal key, request JSON and V1 hash. Finality
  uses the same original context/fingerprint/CAS. It neither obtains an execution
  owner nor returns signing/sending capability. No second table or authority is
  introduced. The existing automatic scan still excludes buy rows.

## Test Evidence To Obtain

Sixteen new tests cover both assets' real message reconstruction and temporary-key
signature evidence, malformed/zero/wrong-wallet/trailing packets, matching-digest
instruction tampering, direction isolation and the retained sell matrix; admitted
buy intent mutations and continued claim denial; V1 explicit fields/defaults/raw
integer precision; and isolated SQLite history in all four signed stages, original
identity corruption, finalized CAS, immutable packet/nonce and context replacement.
The history tests import synthetic public-test rows and sign only with temporary
fixture keys. They are not evidence that the production controlled buy signer ran.
The original scan is exercised to prove it still does not enumerate these buys.

The prior corrupt-buy-in-sell-row test retains all parent/readback/owner rejection
assertions. Its raw row assertion now accepts nominal buy reconstruction, because
that is the deliberate new capability; original-intent binding must still reject
the seller association. Parent fixture names are retained for regression auditing.

Independent results belong in `EXECUTION_STATUS.md`; an earlier 1605-test success
does not validate this batch. No local service test binary is linked on the small VM.

## Remaining Work, Not New Tasks

The controlled buy candidate/signer, same original-owner prepared/signing CAS and
signed-byte readback, fixed Buy sender, live runtime selection, coherent claim
admission and buy scan are still to connect. Existing runtime buy workers remain
legacy. Neither nominal reconstruction, a valid historical signature nor finality
reconciliation can refresh evidence or authorize a new trade. Unknown results must
continue tracking the original signature, without route/key/backend retry.

Old SellV1 payload/codec, production operation classifier, sender, scanner SQL,
migrations, HTTP/request/hash contracts, manifest/lock/vendor and frozen execution
history remain unchanged. PumpAPI new integration stays paused. No SDK upgrade,
release build, deployment/restart, production wallet/database access or transaction
submission is included. Other protocols/modes, global RPC budgets and real-node/
production recovery gates remain in the original plan.
