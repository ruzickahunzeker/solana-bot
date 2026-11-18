//! Wallet-independent SDK configuration only, not initialization or resource ownership.
use sol_trade_sdk::{common::TradeConfig, swqos::SwqosConfig};
use solana_commitment_config::CommitmentConfig;

pub(crate) fn lane_config(
    rpc_url: String,
    provider: SwqosConfig,
    commitment: CommitmentConfig,
    log_enabled: bool,
    mev_protection: bool,
) -> TradeConfig {
    TradeConfig::builder(rpc_url, vec![provider], commitment)
        .create_wsol_ata_on_startup(false)
        .use_seed_optimize(false)
        .log_enabled(log_enabled)
        .mev_protection(mev_protection)
        .build()
}

#[cfg(test)]
mod tests;
