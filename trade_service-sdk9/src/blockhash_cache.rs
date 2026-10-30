use crate::sanitize::redact_urls;
use crate::state::now_ms;
use dashmap::DashMap;
use sol_trade_sdk::common::SolanaRpcClient;
use solana_sdk::hash::Hash;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{debug, warn};

#[derive(Clone)]
pub struct CachedBlockhash {
    pub hash: Hash,
    pub updated_at_ms: u128,
}

pub struct BlockhashCache {
    rpcs: Vec<Arc<SolanaRpcClient>>,
    inner: DashMap<&'static str, CachedBlockhash>,
}

impl BlockhashCache {
    pub fn new(rpcs: Vec<Arc<SolanaRpcClient>>) -> Self {
        Self {
            rpcs,
            inner: DashMap::new(),
        }
    }

    pub fn spawn_refresher(self: Arc<Self>, refresh_ms: u64) {
        tokio::spawn(async move {
            loop {
                let mut refreshed = false;
                for (idx, rpc) in self.rpcs.iter().enumerate() {
                    match rpc.get_latest_blockhash().await {
                        Ok(hash) => {
                            self.inner.insert(
                                "latest",
                                CachedBlockhash {
                                    hash,
                                    updated_at_ms: now_ms(),
                                },
                            );
                            debug!(rpc_lane = idx, "blockhash refreshed");
                            refreshed = true;
                            break;
                        }
                        Err(error) => {
                            let error = redact_urls(&error.to_string());
                            warn!(rpc_lane = idx, error = %error, "blockhash refresh failed")
                        }
                    }
                }
                if !refreshed {
                    warn!("all RPC lanes failed to refresh blockhash");
                }
                sleep(Duration::from_millis(refresh_ms)).await;
            }
        });
    }

    pub fn get_fresh(&self, max_age_ms: u64) -> Option<CachedBlockhash> {
        let cached = self.inner.get("latest")?.clone();
        if now_ms().saturating_sub(cached.updated_at_ms) > u128::from(max_age_ms) {
            return None;
        }
        Some(cached)
    }

    pub fn age_ms(&self) -> Option<u128> {
        self.inner
            .get("latest")
            .map(|cached| now_ms().saturating_sub(cached.updated_at_ms))
    }
}
