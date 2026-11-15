use crate::{domain::readiness::ReadinessReport, models::WalletInfo};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub wallet: String,
    pub default_wallet_id: String,
    pub wallet_count: usize,
    pub wallets: Vec<WalletInfo>,
    pub has_blockhash: bool,
    pub blockhash_age_ms: Option<u128>,
    pub rpc_ok: bool,
    pub paused: bool,
    pub rpc_lanes: usize,
    pub swqos_lanes: Vec<String>,
    pub active_swqos_lanes: Vec<String>,
    pub mev_protection: bool,
    pub require_mev_submit: bool,
    pub pumpapi_executor: bool,
    pub trade_persistence_mode: String,
    pub trade_database_healthy: bool,
}

impl From<ReadinessReport> for ReadyResponse {
    fn from(report: ReadinessReport) -> Self {
        let s = report.snapshot;
        Self {
            ready: report.ready,
            wallet: s.wallet_metadata.wallet.to_string(),
            default_wallet_id: s.default_wallet_id,
            wallet_count: s.wallet_count,
            wallets: s
                .wallets
                .into_iter()
                .map(|w| WalletInfo {
                    wallet_id: w.wallet_id,
                    pubkey: w.pubkey.to_string(),
                })
                .collect(),
            has_blockhash: s.has_blockhash,
            blockhash_age_ms: s.blockhash_age_ms,
            rpc_ok: s.rpc_ok,
            paused: s.paused,
            rpc_lanes: s.wallet_metadata.rpc_lanes,
            swqos_lanes: s.wallet_metadata.swqos_lanes,
            active_swqos_lanes: s.wallet_metadata.active_swqos_lanes,
            mev_protection: s.wallet_metadata.mev_protection,
            require_mev_submit: s.wallet_metadata.require_mev_submit,
            pumpapi_executor: s.pumpapi_executor,
            trade_persistence_mode: s.trade_persistence_mode,
            trade_database_healthy: s.trade_database_healthy,
        }
    }
}
