/// Only values used by ordinary wire validation; no wallet, RPC or credentials.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TradeValidationSettings {
    pub default_buy_sol: f64,
    pub default_slippage_percent: f64,
    pub default_tip_sol: f64,
    pub via_sol_enabled: bool,
    pub via_sol_max_buy_sol: f64,
}
