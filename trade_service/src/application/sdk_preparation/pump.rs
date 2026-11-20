use crate::{
    application::{ports::ordinary_journal::TradeJournalMode, trade_admission::AdmittedTrade},
    domain::{
        pump_native_candidate::PumpNativeCandidateRecord,
        trade::{Address, ExecutionMethod, ExecutionMode, Pool, Side, TradeAmount, WSOL_MINT},
    },
};
use anyhow::{ensure, Context, Result};

/// Historical intent comparison only; a matching record cannot restore bank or claim authority.
pub(crate) fn validate_intent(
    execution: &AdmittedTrade,
    record: &PumpNativeCandidateRecord,
) -> Result<()> {
    let d = record.details();
    let c = execution.command();
    let anchor = execution.reservation();
    anchor.ensure_wallet(&c.wallet_id, Address::from_bytes(d.wallet))?;
    ensure!(
        anchor.mode() == TradeJournalMode::Durable
            && d.request_id == anchor.request_id()
            && d.wallet_id == anchor.wallet_id()
            && d.idempotency_key == anchor.idempotency_key()
            && c.idempotency_key == anchor.idempotency_key()
            && d.request_hash == anchor.request_hash(),
        "native Pump historical identity differs from the original reservation"
    );
    ensure!(
        c.route.pool == Pool::Pump
            && c.execution.method == ExecutionMethod::Sdk
            && c.execution.mode == ExecutionMode::Live
            && !c.aggregator_sol
            && c.route.sol_hop_pool.is_none()
            && c.route.sol_hop_pool_id.is_none()
            && c.route.mint.to_bytes() == d.mint
            && c.route.quote_mint.to_string() == WSOL_MINT
            && c.route
                .pool_id
                .is_none_or(|pool| pool.to_bytes() == d.curve),
        "native Pump historical route differs from the original direct durable intent"
    );
    let input = match c.amount {
        TradeAmount::BuySol { lamports } if d.side == Side::Buy => lamports,
        TradeAmount::SellPercent { percent }
            if d.side == Side::Sell && (1..=99).contains(&percent) =>
        {
            ensure!(
                d.sell_percent == Some(percent),
                "native Pump historical percentage differs from the original intent"
            );
            d.source_token_units
                .and_then(|balance| balance.checked_mul(percent))
                .map(|units| units / 100)
                .context("native Pump historical percentage balance is missing or overflows")?
        }
        _ => anyhow::bail!(
            "native Pump historical intent requires native buy or partial sell without closure"
        ),
    };
    ensure!(
        input > 0
            && input == d.input_units
            && d.slippage_bps == c.execution.slippage_bps
            && d.tip_sol_bits == c.execution.tip_sol.to_bits()
            && d.use_mev == c.execution.use_mev
            && d.requested_mev_provider == c.execution.mev_provider,
        "native Pump historical amount, fees or MEV intent changed"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
