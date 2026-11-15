use crate::domain::{trade::Address, wallet::WalletDescriptor};

#[derive(Debug, Clone)]
pub struct ReadinessWalletMetadata {
    pub wallet: Address,
    pub rpc_lanes: usize,
    pub swqos_lanes: Vec<String>,
    pub active_swqos_lanes: Vec<String>,
    pub mev_protection: bool,
    pub require_mev_submit: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct WalletReadinessObservation {
    pub default_wallet_id: String,
    pub wallet_count: usize,
    pub wallets: Vec<WalletDescriptor>,
    pub wallet_metadata: ReadinessWalletMetadata,
    pub rpc_ok: bool,
}

#[derive(Debug, Clone)]
pub struct ReadinessSnapshot {
    pub default_wallet_id: String,
    pub wallet_count: usize,
    pub wallets: Vec<WalletDescriptor>,
    pub wallet_metadata: ReadinessWalletMetadata,
    pub has_blockhash: bool,
    pub blockhash_age_ms: Option<u128>,
    pub rpc_ok: bool,
    pub paused: bool,
    pub pumpapi_executor: bool,
    pub trade_persistence_mode: String,
    pub trade_database_healthy: bool,
}

#[derive(Debug)]
pub struct ReadinessReport {
    pub ready: bool,
    pub snapshot: ReadinessSnapshot,
}
