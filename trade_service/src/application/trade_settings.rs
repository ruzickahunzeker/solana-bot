/// Only values used by ordinary wire validation; no wallet, RPC or credentials.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TradeValidationSettings {
    pub default_buy_sol: f64,
    pub default_slippage_percent: f64,
    pub default_tip_sol: f64,
    pub via_sol_enabled: bool,
    pub via_sol_max_buy_sol: f64,
}

/// Only defaults needed after validation to construct the internal command.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TradeCommandDefaults {
    pub default_simulate: bool,
    pub pumpapi_priority_fee_sol: f64,
}

/// The four immutable values used while executing an admitted ordinary trade.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OrdinaryExecutionSettings {
    pub broadcast: crate::domain::execution_policy::BroadcastPolicy,
    pub blockhash_max_age_ms: u64,
    pub wait_tx_confirmed: bool,
    pub mev_submit_verify_ms: u64,
}
