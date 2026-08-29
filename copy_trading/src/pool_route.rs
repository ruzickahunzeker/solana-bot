use crate::{
    db::{models::PumpApiEvent, Database},
    trade_client::{ResolvedPoolRoute, TradeServiceClient},
};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Duration,
};
use tracing::{debug, warn};

const MEMORY_CACHE_CAPACITY: usize = 4096;

#[derive(Clone)]
pub struct PoolRouteCache {
    db: Database,
    trade_client: TradeServiceClient,
    resolve_timeout: Duration,
    memory: Arc<RwLock<HashMap<String, ResolvedPoolRoute>>>,
}

impl PoolRouteCache {
    pub fn new(db: Database, trade_client: TradeServiceClient, timeout_secs: u64) -> Self {
        Self {
            db,
            trade_client,
            resolve_timeout: Duration::from_secs(timeout_secs.max(1)),
            memory: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn resolve(&self, mint: &str, force_refresh: bool) -> Result<ResolvedPoolRoute> {
        if !force_refresh {
            if let Some(route) = self
                .memory
                .read()
                .expect("pool route memory cache poisoned")
                .get(mint)
                .cloned()
            {
                return Ok(route);
            }
            if let Some(observed) = self.db.latest_observed_pool_route(mint).await? {
                let route = ResolvedPoolRoute {
                    mint: mint.to_string(),
                    pool: observed.pool,
                    pool_id: observed.pool_id,
                    quote_mint: observed.quote_mint,
                };
                self.remember(route.clone());
                return Ok(route);
            }
        }

        let route =
            tokio::time::timeout(self.resolve_timeout, self.trade_client.resolve_pool(mint))
                .await
                .context("pool resolution timeout")??;
        self.store(&route, "trade_service").await;
        Ok(route)
    }

    pub async fn observe_event(&self, event: &PumpApiEvent) {
        let Some(route) = route_from_event(event) else {
            return;
        };
        if self.remember(route.clone()) {
            self.persist(&route, "pumpapi").await;
        }
    }

    async fn store(&self, route: &ResolvedPoolRoute, source: &str) {
        self.remember(route.clone());
        self.persist(route, source).await;
    }

    async fn persist(&self, route: &ResolvedPoolRoute, source: &str) {
        if let Err(err) = self
            .db
            .upsert_pool_route(
                &route.mint,
                &route.pool,
                route.pool_id.as_deref(),
                &route.quote_mint,
                source,
            )
            .await
        {
            warn!(?err, mint = route.mint, "failed to persist mint pool route");
        }
    }

    fn remember(&self, route: ResolvedPoolRoute) -> bool {
        let mut memory = self
            .memory
            .write()
            .expect("pool route memory cache poisoned");
        if memory.get(&route.mint) == Some(&route) {
            return false;
        }
        if memory.len() >= MEMORY_CACHE_CAPACITY && !memory.contains_key(&route.mint) {
            debug!(
                entries = memory.len(),
                "clearing full pool route memory cache"
            );
            memory.clear();
        }
        memory.insert(route.mint.clone(), route);
        true
    }
}

fn route_from_event(event: &PumpApiEvent) -> Option<ResolvedPoolRoute> {
    let mint = event.mint.as_deref()?.trim();
    let pool = event.pool.as_deref()?.trim().to_ascii_lowercase();
    let quote_mint = event.quote_mint.as_deref()?.trim();
    if mint.is_empty() || quote_mint.is_empty() || !supported_pool(&pool) {
        return None;
    }
    let pool_id = event
        .pool_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    if !matches!(pool.as_str(), "pump" | "bonk") && pool_id.is_none() {
        return None;
    }
    Some(ResolvedPoolRoute {
        mint: mint.to_string(),
        pool,
        pool_id,
        quote_mint: quote_mint.to_string(),
    })
}

fn supported_pool(pool: &str) -> bool {
    matches!(
        pool,
        "pump"
            | "pump-amm"
            | "bonk"
            | "meteora-damm-v2"
            | "meteora-dbc"
            | "raydium-amm-v4"
            | "raydium-cpmm"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(pool_id: Option<&str>) -> PumpApiEvent {
        PumpApiEvent {
            signature: Some("signature-1".to_string()),
            mint: Some("mint-1".to_string()),
            pool: Some("pump-amm".to_string()),
            pool_id: pool_id.map(ToString::to_string),
            quote_mint: Some("quote-1".to_string()),
            tx_type: Some("buy".to_string()),
            signer: Some("signer-1".to_string()),
            sol_in_pool: None,
            tokens_in_pool: None,
            quote_in_pool: None,
            event_timestamp_ms: None,
            received_unix_ms: 1,
            raw_json: "{}".to_string(),
        }
    }

    #[test]
    fn event_route_requires_pool_id_for_pumpswap() {
        assert!(route_from_event(&event(None)).is_none());
        let route = route_from_event(&event(Some("pool-1"))).unwrap();
        assert_eq!(route.pool, "pump-amm");
        assert_eq!(route.pool_id.as_deref(), Some("pool-1"));
    }
}
