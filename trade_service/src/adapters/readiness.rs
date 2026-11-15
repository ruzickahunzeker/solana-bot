use std::{future::Future, sync::Arc};

use crate::{
    application::{
        ports::readiness::{ReadinessSource, ReadinessWalletObserver},
        readiness::ReadinessSettings,
    },
    blockhash_cache::BlockhashCache,
    domain::{execution_policy::BroadcastPolicy, readiness::ReadinessSnapshot},
    persistence::TradePersistence,
};

pub(crate) struct RuntimeReadinessSource<W> {
    settings: ReadinessSettings,
    wallets: W,
    blockhash_cache: Arc<BlockhashCache>,
    persistence: Arc<TradePersistence>,
}

impl<W> RuntimeReadinessSource<W> {
    pub fn new(
        settings: ReadinessSettings,
        wallets: W,
        blockhash_cache: Arc<BlockhashCache>,
        persistence: Arc<TradePersistence>,
    ) -> Self {
        Self {
            settings,
            wallets,
            blockhash_cache,
            persistence,
        }
    }
}

impl<W: ReadinessWalletObserver> ReadinessSource for RuntimeReadinessSource<W> {
    async fn read(&self) -> ReadinessSnapshot {
        let (has_blockhash, blockhash_age_ms, wallet_observation, trade_database_healthy) =
            observe_health(
                &self.blockhash_cache,
                self.settings.blockhash_max_age_ms,
                self.wallets.observe_wallets(),
                self.persistence.healthy(),
            )
            .await;
        let paused = self.settings.broadcast == BroadcastPolicy::Disabled;
        let pumpapi_executor = self.settings.pumpapi_enabled;
        let trade_persistence_mode = format!("{:?}", self.persistence.mode()).to_ascii_lowercase();
        ReadinessSnapshot {
            default_wallet_id: wallet_observation.default_wallet_id,
            wallet_count: wallet_observation.wallet_count,
            wallets: wallet_observation.wallets,
            wallet_metadata: wallet_observation.wallet_metadata,
            has_blockhash,
            blockhash_age_ms,
            rpc_ok: wallet_observation.rpc_ok,
            paused,
            pumpapi_executor,
            trade_persistence_mode,
            trade_database_healthy,
        }
    }
}

async fn observe_health<R>(
    cache: &BlockhashCache,
    max_age_ms: u64,
    rpc: impl Future<Output = R>,
    database: impl Future<Output = bool>,
) -> (bool, Option<u128>, R, bool) {
    let rpc_result = rpc.await;
    let database_healthy = database.await;
    // Sample one cache entry after all awaits, not a pre-probe freshness bit.
    let (has_blockhash, blockhash_age_ms) = cache.freshness(max_age_ms);
    (
        has_blockhash,
        blockhash_age_ms,
        rpc_result,
        database_healthy,
    )
}

#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod tests;
