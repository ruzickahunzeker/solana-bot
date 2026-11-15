use crate::application::ports::ordinary_journal::TradeJournalMode;
use crate::application::trade_admission::AdmittedTrade;
use crate::application::{
    ordinary_claims::{operation_for_claim, ClaimOperation},
    trade_command::TradeCommand,
};
use std::future::Future;

#[cfg(test)]
mod tests;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExecutionClaim {
    Acquired(ExecutionOwner),
    AlreadyHeld,
}

impl ExecutionClaim {
    #[cfg(test)]
    pub(crate) fn is_acquired(&self) -> bool {
        matches!(self, Self::Acquired(_))
    }
}

pub(crate) struct ExecutionOwner {
    request_id: String,
    request_hash: String,
    mode: TradeJournalMode,
    nonce: String,
    operation: ClaimOperation,
    wallet_id: String,
    wallet_pubkey: String,
    idempotency_key: String,
    command: TradeCommand,
}

impl std::fmt::Debug for ExecutionOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExecutionOwner(<redacted>)")
    }
}

impl ExecutionOwner {
    pub(crate) fn new(execution: &AdmittedTrade, nonce: String) -> anyhow::Result<Self> {
        anyhow::ensure!(
            nonce.len() == 32 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid ordinary execution owner"
        );
        let anchor = execution.reservation();
        let command = execution.command();
        anyhow::ensure!(
            command.wallet_id == anchor.wallet_id()
                && command.idempotency_key == anchor.idempotency_key(),
            "ordinary execution command identity mismatch"
        );
        let operation = operation_for_claim(execution, anchor.mode())?;
        Ok(Self {
            request_id: execution.reservation().request_id().into(),
            request_hash: execution.reservation().request_hash().into(),
            mode: execution.reservation().mode(),
            nonce,
            operation,
            wallet_id: anchor.wallet_id().into(),
            wallet_pubkey: anchor.wallet_pubkey().into(),
            idempotency_key: anchor.idempotency_key().into(),
            command: command.clone(),
        })
    }
    pub(crate) fn ensure_binding(&self, execution: &AdmittedTrade) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.request_id == execution.reservation().request_id()
                && self.request_hash == execution.reservation().request_hash()
                && self.mode == execution.reservation().mode(),
            "ordinary execution owner mismatch"
        );
        anyhow::ensure!(
            self.operation == operation_for_claim(execution, self.mode)?
                && self.wallet_id == execution.reservation().wallet_id()
                && self.wallet_pubkey == execution.reservation().wallet_pubkey()
                && self.idempotency_key == execution.reservation().idempotency_key()
                && same_command(&self.command, execution.command()),
            "ordinary execution owner command or wallet identity mismatch"
        );
        Ok(())
    }
    pub(crate) fn nonce(&self) -> &str {
        &self.nonce
    }
    pub(crate) fn mode(&self) -> TradeJournalMode {
        self.mode
    }
    pub(crate) fn operation(&self) -> ClaimOperation {
        self.operation
    }
    #[cfg(test)]
    pub(crate) fn fixture(execution: &AdmittedTrade) -> Self {
        // Fake executors also exercise invalid commands and legacy error ordering.
        // Production claims must use the validated constructor above.
        let anchor = execution.reservation();
        Self {
            request_id: anchor.request_id().into(),
            request_hash: anchor.request_hash().into(),
            mode: anchor.mode(),
            nonce: uuid::Uuid::new_v4().simple().to_string(),
            operation: if execution.command().aggregator_sol {
                ClaimOperation::SolBuy
            } else {
                ClaimOperation::SdkTrade
            },
            wallet_id: anchor.wallet_id().into(),
            wallet_pubkey: anchor.wallet_pubkey().into(),
            idempotency_key: anchor.idempotency_key().into(),
            command: execution.command().clone(),
        }
    }
}

fn same_command(a: &TradeCommand, b: &TradeCommand) -> bool {
    (
        &a.wallet_id,
        &a.idempotency_key,
        a.route.pool,
        a.route.mint,
        a.route.quote_mint,
        a.route.pool_id,
        a.route.sol_hop_pool,
        a.route.sol_hop_pool_id,
    ) == (
        &b.wallet_id,
        &b.idempotency_key,
        b.route.pool,
        b.route.mint,
        b.route.quote_mint,
        b.route.pool_id,
        b.route.sol_hop_pool,
        b.route.sol_hop_pool_id,
    ) && (
        a.amount,
        a.execution.method,
        a.execution.slippage_bps,
        a.execution.tip_sol.to_bits(),
        a.execution.use_mev,
        a.execution.mev_provider,
        a.execution.mode,
        a.aggregator_sol,
        a.pumpapi_priority_fee_sol.to_bits(),
    ) == (
        b.amount,
        b.execution.method,
        b.execution.slippage_bps,
        b.execution.tip_sol.to_bits(),
        b.execution.use_mev,
        b.execution.mev_provider,
        b.execution.mode,
        b.aggregator_sol,
        b.pumpapi_priority_fee_sol.to_bits(),
    )
}

impl PartialEq for ExecutionOwner {
    fn eq(&self, other: &Self) -> bool {
        self.request_id == other.request_id
            && self.request_hash == other.request_hash
            && self.mode == other.mode
            && self.nonce == other.nonce
            && self.operation == other.operation
            && self.wallet_id == other.wallet_id
            && self.wallet_pubkey == other.wallet_pubkey
            && self.idempotency_key == other.idempotency_key
            && same_command(&self.command, &other.command)
    }
}
impl Eq for ExecutionOwner {}

pub(crate) trait OrdinaryExecutionClaimer: Sync {
    fn claim(
        &self,
        execution: &AdmittedTrade,
    ) -> impl Future<Output = anyhow::Result<ExecutionClaim>> + Send;
}
