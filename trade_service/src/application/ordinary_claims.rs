use crate::{
    application::{
        ports::ordinary_journal::TradeJournalMode, trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::trade::{ExecutionMethod, Pool, TradeAmount, TradeStatus, USDC_MINT, WSOL_MINT},
};

#[cfg(test)]
mod sdk_claim_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub(crate) fn ensure_scope(
    execution: &AdmittedTrade,
    mode: TradeJournalMode,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        execution.reservation().mode() == mode
            && execution.command().aggregator_sol
            && !execution.command().execution.mode.simulate()
            && matches!(execution.command().amount, TradeAmount::BuySol { .. }),
        "ordinary execution claim mode or operation mismatch"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClaimOperation {
    SolBuy,
    SdkTrade,
}

impl ClaimOperation {
    pub(crate) fn storage_name(self) -> &'static str {
        match self {
            Self::SolBuy => "ordinary-sol-buy",
            Self::SdkTrade => "ordinary-sdk-trade",
        }
    }
    pub(crate) fn aggregator_sol(self) -> bool {
        self == Self::SolBuy
    }
}

/// Claim admission only. The legacy quote journal retains its stricter ensure_scope.
pub(crate) fn operation_for_claim(
    execution: &AdmittedTrade,
    mode: TradeJournalMode,
) -> anyhow::Result<ClaimOperation> {
    if ensure_scope(execution, mode).is_ok() {
        return Ok(ClaimOperation::SolBuy);
    }
    let command = execution.command();
    let amount_allowed = match command.amount {
        TradeAmount::SellPercent { percent: 1..=100 } => true,
        TradeAmount::BuySol { lamports } => {
            lamports > 0 && command.route.quote_mint.to_string() == WSOL_MINT
        }
        TradeAmount::BuyQuote { mint, units } => {
            units > 0 && mint == command.route.quote_mint && mint.to_string() == USDC_MINT
        }
        _ => false,
    };
    anyhow::ensure!(
        execution.reservation().mode() == mode
            && mode == TradeJournalMode::Durable
            && !command.aggregator_sol
            && command.execution.method == ExecutionMethod::Sdk
            && !command.execution.mode.simulate()
            && command.route.pool == Pool::MeteoraDammV2
            && command.route.sol_hop_pool.is_none()
            && command.route.sol_hop_pool_id.is_none()
            && amount_allowed,
        "ordinary execution claim mode or operation mismatch"
    );
    Ok(ClaimOperation::SdkTrade)
}

pub(crate) fn ensure_parent(
    execution: &AdmittedTrade,
    parent: &TradeRecordV1,
) -> anyhow::Result<()> {
    let anchor = execution.reservation();
    let command = execution.command();
    anyhow::ensure!(
        parent.request_id == anchor.request_id()
            && parent.wallet_id == anchor.wallet_id()
            && parent.wallet_pubkey == anchor.wallet_pubkey()
            && parent.side == command.amount.side()
            && parent.pool == command.route.pool
            && parent.execution_method == command.execution.method
            && parent.mint.trim() == command.route.mint.to_string()
            && parent.pool_id.as_deref().map(str::trim)
                == command.route.pool_id.map(|id| id.to_string()).as_deref(),
        "ordinary execution parent identity mismatch"
    );
    Ok(())
}

/// Profile scope only; the original SDK owner still needs a current prepared backend.
pub(crate) fn pump_prepared_scope(execution: &AdmittedTrade, mode: TradeJournalMode) -> bool {
    let c = execution.command();
    execution.reservation().mode() == mode
        && mode == TradeJournalMode::Durable
        && c.wallet_id == execution.reservation().wallet_id()
        && c.idempotency_key == execution.reservation().idempotency_key()
        && c.execution.method == ExecutionMethod::Sdk
        && !c.execution.mode.simulate()
        && !c.aggregator_sol
        && c.route.pool == Pool::Pump
        && c.route.pool_id.is_none()
        && c.route.quote_mint.to_string() == WSOL_MINT
        && c.route.sol_hop_pool.is_none()
        && c.route.sol_hop_pool_id.is_none()
        && matches!(
            c.amount,
            TradeAmount::BuySol { lamports: 1.. } | TradeAmount::SellPercent { percent: 1..=99 }
        )
}

pub(crate) fn validate_prepared_claim(
    execution: &AdmittedTrade,
    mode: TradeJournalMode,
    prepared: &dyn super::ports::ordinary_execution::PreparedSdkExecution,
) -> anyhow::Result<ClaimOperation> {
    anyhow::ensure!(
        pump_prepared_scope(execution, mode)
            && matches!(
                prepared.metadata(),
                crate::domain::sdk_prepared_record::SdkPreparedRecord::PumpPrepared(_)
            ),
        "prepared SDK claim requires the original Pump native profile"
    );
    prepared.validate_current(execution)?;
    super::sdk_preparation::validate_origin(execution, prepared.metadata(), prepared.message())?;
    Ok(ClaimOperation::SdkTrade)
}

pub(crate) fn parent_can_start(parent: &TradeRecordV1) -> bool {
    parent.status == TradeStatus::Accepted && parent.signature.is_none()
}
