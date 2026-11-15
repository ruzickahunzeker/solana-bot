//! Frozen ordinary-trade row/read model, not a new domain aggregate or authorization.
use crate::domain::trade::{ExecutionMethod, Pool, Side, TradeStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TradeRecordV1 {
    pub request_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub side: Side,
    pub pool: Pool,
    pub mint: String,
    pub pool_id: Option<String>,
    pub execution_method: ExecutionMethod,
    pub status: TradeStatus,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}
