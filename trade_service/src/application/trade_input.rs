use crate::domain::trade::{ExecutionMethod, MevProvider, Pool, SettlementAsset, Side};

/// Borrowed, unvalidated trade fields; no wallet/source metadata or execution authority.
#[derive(Clone, Copy)]
pub(crate) struct TradeInput<'a> {
    pub side: Side,
    pub pool: Pool,
    pub mint: &'a str,
    pub pool_id: Option<&'a str>,
    pub quote_mint: Option<&'a str>,
    pub settlement_asset: Option<SettlementAsset>,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<&'a str>,
    pub amount_sol: Option<f64>,
    pub amount_lamports: Option<u64>,
    pub amount_quote: Option<f64>,
    pub amount_usdc: Option<f64>,
    pub amount_quote_units: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_percent: Option<f64>,
    pub slippage_bps: Option<u64>,
    pub tip_sol: Option<f64>,
    pub execution_method: Option<ExecutionMethod>,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
    pub simulate: Option<bool>,
    pub idempotency_key: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) struct WarmupInput<'a> {
    pub pool: Pool,
    pub mint: &'a str,
    pub pool_id: Option<&'a str>,
    pub quote_mint: Option<&'a str>,
}
