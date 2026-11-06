# SOL buy aggregator probe (2026-09-30)

## Read-only service probe (2026-10-02)

After release build and restart, `/ready` returned `ready:true`. A 0.001 SOL quote for the target mint returned a three-hop route through USDC and the custom quote token. The current route labels were Kipseli, Hadron and Raydium CP; these labels are not stable and do not authorize execution.

The authenticated preflight returned `local_simulation_ok:true`, `prioritization_fee_lamports:100000`, `simulated_fee_lamports:105000`, `simulated_target_token_post_units:14707627169`, and four inner-instruction groups. It returned `simulated_payer_debit_lamports:null` and `simulated_target_token_delta_units:null`, because the RPC simulation did not provide usable before/after balance records. The post-balance is not the amount bought. `instruction_audit_passed:false` and `executable:false` remained unchanged. No transaction was signed or sent.

Next audit work: inspect inner instructions and writable-account ownership, then establish a reliable pre-state baseline for payer SOL and target ATA at the simulation slot. Until both actual debit and token delta can be bounded, preflight must remain non-executable.

A subsequent read-only observation path requests payer/ATA accounts before simulation from the same RPC and passes that response slot as the simulation's `min_context_slot`. It exposes same-slot balance differences only as `observed_*` diagnostics. Same slot does not guarantee identical account state within that slot, so these diagnostics are never used for safety checks or to mark a quote executable.

On the rebuilt service, a first read-only preflight was rejected because the re-quote changed route or minimum output. A fresh quote succeeded: baseline and simulation both reported slot `452495753`; `observed_same_slot_target_token_delta_units` and the simulated ATA post-balance were both `13236862870`. The observed payer debit was null, as were the simulation-native payer debit and target-token delta. This does not prove net spend or actual bought amount. The local unsigned simulation succeeded, while `instruction_audit_passed:false` and `executable:false` remained in force. No transaction was signed or sent.

The next read-only audit step inventories inner instructions and checks their outer instruction numbers, account indexes and program IDs against the resolved transaction keys. It reports only counts and unique program IDs; this is not an allowlist or a CPI fee audit. If the RPC omits inner instructions, these inventory fields remain null.

The read-only preflight also counts recognized payer-funded System Program CPI transfers. If any inner System instruction cannot be decoded, that spend field is null rather than an invented zero. When known, direct and inner System transfers share the same input-plus-0.02-SOL cap. This does not bound token transfers or other program-specific fees.

Target ATA baseline and simulated post-state now use the SPL Token and Token-2022 structured account decoders, including initialization and extension-layout checks, instead of reading fixed offsets alone. The amount still remains a post-state observation unless a trustworthy pre/post delta is available.

Top-level and simulated inner SPL Token/Token-2022 instructions now reject operations that use the target ATA as their first account, except account initialization. A transfer into the target ATA is not rejected by this rule. This protects one destination account only; it is not an audit of all intermediate token accounts or aggregator CPI behavior.

Scope: read-only quote and unsigned transaction construction. No transaction was signed or sent.

- Target mint: `2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL`
- Input: 0.001 and 0.01 SOL, ExactIn, 1500 bps slippage
- Public Jupiter Lite Quote API returned a route for both sizes:
  `WSOL -> USDC (Byreal) -> Xsf9mBktVB9BSU5kf4nHxPq5hCBJ2j2ui3ecFGxPRGc (Raydium CLMM) -> target mint (Raydium CP)`
- The 0.001 SOL quote returned `outAmount=19441607746`, `otherAmountThreshold=16525366585`, and `priceImpactPct=0.0373635515999455963955049207`. These are transient raw token units, not a standing price or execution guarantee.
- The `/swap/v1/swap` endpoint returned an unsigned transaction. A public wallet with sufficient SOL produced `simulationError=null`; a low-balance public wallet produced a simulation error. No private key was shared with the aggregator.

## Boundary before Live execution

The new `/v1/quotes/sol-buy` endpoint is read-only and returns `executable:false`. It binds a preview to wallet ID/public key and a short durable expiry; it does not reserve an idempotency key, sign, submit, or enable live execution. Before enabling a Live button, the service must re-quote, inspect every transaction instruction and writable account, cap platform and priority fees, verify the signer and output account, simulate with the selected wallet, and persist execution and duplicate-replay state. A returned unsigned transaction alone is not sufficient evidence that these checks pass.

The existing `POST /v1/trades` and SDK execution paths are unchanged. Telegram, limit orders, and copy trading must not treat the read-only quote as an executable route.

The read-only preflight endpoint re-quotes, requests an unsigned transaction, checks payer/signature count and instruction count, resolves up to four address lookup tables from the wallet's default RPC, validates owners, active metadata and all account indices, and independently simulates the unsigned transaction with signature verification disabled. It decodes Compute Budget instructions, caps CU limit at 1.4 million and calculated priority fee at 100,000 lamports; aggregator fee metadata is cross-checked when present, while missing metadata is not treated as zero. It also rejects duplicate resolved accounts and more than 64 writable accounts; direct System Program instructions must be payer-funded transfers with a cumulative cap of input amount plus 0.02 SOL. The target mint's on-chain token-program owner determines the expected wallet ATA; that ATA must be present and writable. The response reports `local_simulation_ok`, `target_ata`, top-level program IDs, account counts and direct System spend. If the RPC provides them, simulation fee, compute consumption, payer debit, inner instruction group count, target-token delta and target ATA post-balance are reported. The ATA post-balance is decoded from the returned initialized token account and checked for program, mint and wallet owner; it is not a buy delta without a before-state baseline. Target-token delta requires unique simulation-native before/after records matching ATA index and mint; a missing delta or output below quote minimum is rejected. Same-slot observations remain diagnostics only. It never returns transaction bytes and still reports `instruction_audit_passed:false` and `executable:false`. An unsigned simulation is not proof of a safe signed transaction. These checks do not fully bound CPI/program-specific fees. Reviewing swap semantics and writable destinations, bounding all fees, and persisting execution/idempotency remain blockers for Live use.

Jupiter returned a 99,999-lamport priority-fee estimate for a read-only probe. The reported fee may differ by one lamport from integer-rounded instruction cost; that single-lamport difference is tolerated, while the decoded cost remains the enforced cap. Larger differences and any CU-limit mismatch are rejected.
