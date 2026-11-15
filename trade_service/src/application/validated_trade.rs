use crate::domain::trade::{Address, ExecutionMethod, MevProvider, Pool, SettlementAsset};

/// Syntactically normalized values, not an on-chain verified route or signing authority.
pub struct ValidatedTrade {
    pub mint: Address,
    pub quote_mint: Address,
    pub pool_id: Option<Address>,
    pub settlement_asset: SettlementAsset,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<Address>,
    // Legacy name: these are Quote units unless settlement/branch selects SOL.
    pub amount_lamports: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub execution_method: ExecutionMethod,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
    pub simulate: Option<bool>,
    pub aggregator_sol: bool,
}

/// Warmup input policy only; not proof of an existing or executable on-chain pool.
pub struct ValidatedWarmup {
    pub mint: Address,
    pub quote_mint: Address,
    pub pool_id: Option<Address>,
}
