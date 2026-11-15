use crate::domain::trade::{Address, Pool};

/// Pool/Mint/Quote hints only; no claim about owner, context slot or token extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolRouteHint {
    pub pool: Pool,
    pub mint: Address,
    pub quote_mint: Address,
    pub pool_id: Option<Address>,
}
