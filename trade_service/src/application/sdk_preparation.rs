use crate::{
    application::{
        ordinary_claims::{operation_for_claim, ClaimOperation},
        ports::ordinary_journal::TradeJournalMode,
        trade_admission::AdmittedTrade,
    },
    domain::{
        sdk_buy_candidate::BuyFunding,
        sdk_prepared_record::SdkPreparedRecord,
        trade::{Address, ExecutionMethod, Pool, TradeAmount, USDC_MINT, WSOL_MINT},
    },
};
use anyhow::{ensure, Result};

/// Intent binding only. Persisted nominal metadata never grants signing authority.
pub(crate) fn validate_origin(
    execution: &AdmittedTrade,
    metadata: &SdkPreparedRecord,
    message: &[u8],
) -> Result<()> {
    ensure!(
        operation_for_claim(execution, TradeJournalMode::Durable)? == ClaimOperation::SdkTrade,
        "SDK preparation requires its original durable operation"
    );
    validate_candidate_intent(execution, metadata, message)
}

/// Pure original-intent binding. This does not grant a claim, permit or fresh quote.
pub(crate) fn validate_candidate_intent(
    execution: &AdmittedTrade,
    metadata: &SdkPreparedRecord,
    message: &[u8],
) -> Result<()> {
    metadata.validate(message)?;
    let c = execution.command();
    ensure!(
        c.route.pool == Pool::MeteoraDammV2
            && c.execution.method == ExecutionMethod::Sdk
            && !c.execution.mode.simulate()
            && !c.aggregator_sol
            && c.route.sol_hop_pool.is_none()
            && c.route.sol_hop_pool_id.is_none(),
        "SDK candidate requires its original direct live DAMM intent"
    );
    if !matches!(c.amount, TradeAmount::SellPercent { .. }) {
        return validate_buy_intent(execution, metadata);
    }
    let d = metadata.sell()?.details();
    let TradeAmount::SellPercent { percent } = c.amount else {
        anyhow::bail!("SDK sell amount missing")
    };
    ensure!(
        Address::from_bytes(d.wallet).to_string() == execution.reservation().wallet_pubkey()
            && d.mint == c.route.mint.to_bytes()
            && d.quote_mint == c.route.quote_mint.to_bytes()
            && d.pool_hint == c.route.pool_id.map(|v| v.to_bytes())
            && d.sell_percent == percent
            && d.slippage_bps == c.execution.slippage_bps
            && d.tip_sol_bits == c.execution.tip_sol.to_bits()
            && !d.simulate
            && d.use_mev == c.execution.use_mev
            && d.requested_mev_provider == c.execution.mev_provider,
        "SDK candidate differs from the original wallet or execution intent"
    );
    Ok(())
}

fn validate_buy_intent(execution: &AdmittedTrade, metadata: &SdkPreparedRecord) -> Result<()> {
    let d = metadata.buy()?.details();
    let c = execution.command();
    let units = match c.amount {
        TradeAmount::BuySol { lamports }
            if d.funding == BuyFunding::NativeSol
                && c.route.quote_mint.to_string() == WSOL_MINT =>
        {
            lamports
        }
        TradeAmount::BuyQuote { mint, units }
            if d.funding == BuyFunding::ExistingQuote
                && mint == c.route.quote_mint
                && mint.to_string() == USDC_MINT =>
        {
            units
        }
        _ => anyhow::bail!("SDK buy input asset or funding differs from the original intent"),
    };
    ensure!(
        Address::from_bytes(d.wallet).to_string() == execution.reservation().wallet_pubkey()
            && d.mint == c.route.mint.to_bytes()
            && d.quote_mint == c.route.quote_mint.to_bytes()
            && d.pool_hint == c.route.pool_id.map(|v| v.to_bytes())
            && d.input_units == units
            && units > 0
            && d.slippage_bps == c.execution.slippage_bps
            && d.tip_sol_bits == c.execution.tip_sol.to_bits()
            && !d.simulate
            && d.use_mev == c.execution.use_mev
            && d.requested_mev_provider == c.execution.mev_provider,
        "SDK candidate differs from the original wallet or execution intent"
    );
    Ok(())
}

pub(crate) struct StoredSdkCandidate {
    pub(crate) metadata: SdkPreparedRecord,
    pub(crate) message: Vec<u8>,
}

#[cfg(test)]
mod tests;
