# Aggregator SOL-buy rollout

Implementation note (2026-11-12): this is a historical gate checklist, not a deployment status report. The existing code already contains a submit endpoint and can report successful preflight flags. The independent submit endpoint now has an offline-tested durable journal, persisted signatures/bytes and replay-only restart handling; see [acceptance and remaining gates](optimization/QUOTE_SUBMISSION_JOURNAL.md). It has not been deployed in this change. This does not certify full instruction auditing, chain reconciliation or controlled Live acceptance.

Scope: add an optional SOL-denominated buy path for custom-quote tokens. Existing `POST /v1/trades`, SDK routes, wallet storage, Telegram follow/limit execution, and vendor SDK remain unchanged until separately reviewed. Keep the new path disabled for Live until every gate below is complete.

## Gate 1: Read-only quote and preflight

- Quote must bind wallet ID/public key, input/output mints, exact input, slippage, route and expiry. Re-quote before constructing an unsigned transaction.
- Validate Solana message structure, single empty payer signature, ALT owner/data/indexes, all instruction account indexes, direct System Program transfers, Compute Budget instructions and independent unsigned simulation.
- Initial read-only stage: responses kept `instruction_audit_passed:false` and `executable:false`. Current preflight can return true after its checks; this is not evidence that every gate below is complete. No transaction bytes are returned to clients.
- Acceptance: unit tests for malformed messages, ALT index failures, duplicate accounts, unsupported System/Compute Budget instructions, fee overflow and wallet mismatch; read-only integration probe with real RPC. An unsigned simulation does not certify signed execution.

## Gate 2: Complete instruction and balance audit

- Enumerate and allowlist every top-level and inner program in a representative route (including aggregator, AMM, token, associated-token and WSOL handling). Reject any unrecognized program/version, delegate, authority change or arbitrary wallet-owned token transfer.
- Derive and verify the destination token account from the bound wallet and target mint. Verify input WSOL source and every writable wallet-owned account; prove that account creation/closure cannot redirect funds.
- Cap direct and CPI SOL movements, platform/affiliate/referral fees, rent, priority fee and base signature fee against a total wallet-debit budget. Validate post-simulation token/SOL balance deltas and minimum output in raw units; distinguish ATA rent from spent trade input.
- Reconcile aggregator fee metadata against decoded transaction instructions and actual simulation results. Avoid treating route labels or successful simulation as authorization.
- Acceptance: adversarial fixtures (recipient substitution, hidden tip, extra signer, malicious CPI, stale ALT, excess debit, output below threshold) all fail closed; one verified multi-hop fixture passes. Only then may `instruction_audit_passed` become true.

## Gate 3: Isolated execution endpoint

- Add a separate, feature-gated submit endpoint; do not change existing trade request semantics. Re-quote and re-audit immediately before signing. Bind the exact message hash to wallet, quote, amount, slippage, expiry and an idempotency key.
- Persist order intent before signing; reserve/reject duplicate keys across restarts. Recheck current blockhash/ALT state and wallet balance. Sign only the audited bytes with the selected wallet; submit once through the approved transport and reconcile signature/confirmation/failure into persistent state.
- Never expose unsigned or signed transaction bytes or private keys to Telegram or the aggregator. All errors must redact RPC URLs and credentials.
- Acceptance: restart/retry tests, duplicate concurrent requests, stale quote/blockhash, simulated failure and dropped RPC response; no duplicate live order.

## Gate 4: Controlled rollout

- Keep Live off by default. Enable one wallet, one mint class and a small per-order/daily SOL cap. Use explicit operator approval, balance monitoring, structured audit logs and a kill switch.
- Run dry-run and tiny-value canary transactions before exposing a Telegram SOL amount button. Keep old SDK trading paths available as fallback, without silent route switching.
- Acceptance: reconcile on-chain input/output, fees and status against persisted intent; test rollback by disabling the feature flag without touching existing services.

Do not advance a gate because compilation passed or an aggregator simulation returned no error. Each gate needs focused tests and an operator-reviewed record of the actual transaction semantics.
