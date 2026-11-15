use crate::application::validated_trade::ValidatedTrade;
use crate::domain::trade::{
    Address, ExecutionMethod, ExecutionMode, MevProvider, Pool, SettlementAsset, Side, TradeAmount,
    WSOL_MINT,
};
use anyhow::Context;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub(crate) struct CommandContext<'a> {
    pub wallet_id: String,
    pub idempotency_key: &'a str,
    pub side: Side,
    pub pool: Pool,
    pub default_simulate: bool,
    pub pumpapi_priority_fee_sol: f64,
}

#[derive(Debug, Clone)]
/// Syntactically validated route hints, not proof of on-chain pool verification.
pub(crate) struct TradeRoute {
    pub pool: Pool,
    pub mint: Address,
    pub quote_mint: Address,
    pub pool_id: Option<Address>,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<Address>,
}

#[derive(Debug, Clone)]
pub(crate) struct TradeExecution {
    pub method: ExecutionMethod,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
    pub mode: ExecutionMode,
}

/// Resolved wallet identity and validated amounts, not an API or journal snapshot.
#[derive(Debug, Clone)]
pub(crate) struct TradeCommand {
    pub wallet_id: String,
    pub idempotency_key: String,
    pub route: TradeRoute,
    pub amount: TradeAmount,
    pub execution: TradeExecution,
    pub aggregator_sol: bool,
    pub pumpapi_priority_fee_sol: f64,
}

impl TradeCommand {
    pub(crate) fn from_validated(
        context: CommandContext<'_>,
        value: &ValidatedTrade,
    ) -> anyhow::Result<Self> {
        let quote_mint = value.quote_mint;
        let amount = match context.side {
            Side::Buy => {
                let units = value
                    .amount_lamports
                    .context("validated buy amount is missing")?;
                if value.aggregator_sol
                    || value.settlement_asset == SettlementAsset::Sol
                    || value.quote_mint.to_string() == WSOL_MINT
                {
                    TradeAmount::BuySol { lamports: units }
                } else {
                    TradeAmount::BuyQuote {
                        mint: quote_mint,
                        units,
                    }
                }
            }
            Side::Sell => TradeAmount::SellPercent {
                percent: value
                    .sell_percent
                    .context("validated sell percent is missing")?,
            },
        };
        Ok(Self {
            wallet_id: context.wallet_id,
            idempotency_key: context.idempotency_key.into(),
            route: TradeRoute {
                pool: context.pool,
                mint: value.mint,
                quote_mint,
                pool_id: value.pool_id,
                sol_hop_pool: value.sol_hop_pool,
                sol_hop_pool_id: value.sol_hop_pool_id,
            },
            amount,
            execution: TradeExecution {
                method: value.execution_method,
                slippage_bps: value.slippage_bps,
                tip_sol: value.tip_sol,
                use_mev: value.use_mev,
                mev_provider: value.mev_provider,
                mode: ExecutionMode::resolve(value.simulate, context.default_simulate),
            },
            aggregator_sol: value.aggregator_sol,
            pumpapi_priority_fee_sol: context.pumpapi_priority_fee_sol,
        })
    }

    pub fn quote_mint(&self) -> Address {
        match self.amount {
            TradeAmount::BuyQuote { mint, .. } => mint,
            _ => self.route.quote_mint,
        }
    }
}
