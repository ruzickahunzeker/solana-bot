use std::{future::Future, sync::Arc};

use crate::{
    adapters::rpc::read_runtime::SharedReadRuntime,
    application::ports::readiness::ReadinessSource,
    blockhash_cache::BlockhashCache,
    config::Config,
    domain::{execution_policy::BroadcastPolicy, readiness::ReadinessSnapshot},
    persistence::TradePersistence,
    wallets::WalletRegistry,
};

pub(crate) struct RuntimeReadinessSource {
    config: Arc<Config>,
    wallets: Arc<WalletRegistry>,
    blockhash_cache: Arc<BlockhashCache>,
    persistence: Arc<TradePersistence>,
    read_rpc: Arc<SharedReadRuntime>,
}

impl RuntimeReadinessSource {
    pub fn new(
        config: Arc<Config>,
        wallets: Arc<WalletRegistry>,
        blockhash_cache: Arc<BlockhashCache>,
        persistence: Arc<TradePersistence>,
        read_rpc: Arc<SharedReadRuntime>,
    ) -> Self {
        Self {
            config,
            wallets,
            blockhash_cache,
            persistence,
            read_rpc,
        }
    }
}

impl ReadinessSource for RuntimeReadinessSource {
    async fn read(&self) -> ReadinessSnapshot {
        let probe = self.wallets.default().readiness_probe();
        let (
            has_blockhash,
            blockhash_age_ms,
            (rpc_ok, default_wallet_id, wallet_count, wallets, wallet_metadata),
            trade_database_healthy,
        ) = observe_health(
            &self.blockhash_cache,
            self.config.blockhash_max_age_ms,
            async {
                let rpc_ok = probe.ping(&self.read_rpc).await;
                let default_wallet_id = self.wallets.default_wallet_id().to_string();
                let wallet_count = self.wallets.wallet_count();
                let wallets = self.wallets.public_infos();
                (
                    rpc_ok,
                    default_wallet_id,
                    wallet_count,
                    wallets,
                    probe.into_metadata(),
                )
            },
            self.persistence.healthy(),
        )
        .await;
        let paused = self.config.security.broadcast == BroadcastPolicy::Disabled;
        let pumpapi_executor = self.config.pumpapi.enabled;
        let trade_persistence_mode = format!("{:?}", self.persistence.mode()).to_ascii_lowercase();
        ReadinessSnapshot {
            default_wallet_id,
            wallet_count,
            wallets,
            wallet_metadata,
            has_blockhash,
            blockhash_age_ms,
            rpc_ok,
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
mod tests;
