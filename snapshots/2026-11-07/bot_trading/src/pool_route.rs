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
            if let Some(route) = self.observed_hint(mint).await? {
                return Ok(route);
            }
        }

        let route =
            tokio::time::timeout(self.resolve_timeout, self.trade_client.resolve_pool(mint))
                .await
                .context("pool resolution timeout")??;
        anyhow::ensure!(
            route.mint == mint,
            "resolved pool Mint does not match requested Mint"
        );
        crate::trade_client::validate_pool_quote(&route.pool, Some(&route.quote_mint))?;
        self.store(&route, "trade_service").await;
        Ok(route)
    }

    /// A cached or PumpAPI-observed pool is only a lookup hint. Callers must
    /// revalidate it through the execution service before using it to trade.
    pub async fn observed_hint(&self, mint: &str) -> Result<Option<ResolvedPoolRoute>> {
        if let Some(route) = self
            .memory
            .read()
            .expect("pool route memory cache poisoned")
            .get(mint)
            .cloned()
        {
            if route.pool.eq_ignore_ascii_case("raydium-cpmm")
                || route.pool.eq_ignore_ascii_case("pump")
            {
                crate::trade_client::validate_pool_quote(&route.pool, Some(&route.quote_mint))?;
            }
            if crate::trade_client::validate_pool_quote(&route.pool, Some(&route.quote_mint))
                .is_ok()
            {
                return Ok(Some(route));
            }
        }
        if let Some(observed) = self.db.latest_observed_pool_route(mint).await? {
            let route = ResolvedPoolRoute {
                mint: mint.to_string(),
                pool: observed.pool,
                pool_id: observed.pool_id,
                quote_mint: observed.quote_mint,
            };
            if route.pool.eq_ignore_ascii_case("raydium-cpmm")
                || route.pool.eq_ignore_ascii_case("pump")
            {
                crate::trade_client::validate_pool_quote(&route.pool, Some(&route.quote_mint))?;
            }
            if crate::trade_client::validate_pool_quote(&route.pool, Some(&route.quote_mint))
                .is_ok()
            {
                self.remember(route.clone());
                return Ok(Some(route));
            }
        }
        Ok(None)
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
        // PumpAPI emits a high-cardinality stream of new mints. Persisting
        // every observed route turns this cache into an unbounded event log;
        // the bounded in-memory cache is sufficient for live routing.
        if source == "pumpapi"
            && !matches!(
                std::env::var("POOL_ROUTE_PERSIST_OBSERVATIONS")
                    .ok()
                    .as_deref(),
                Some("1") | Some("true") | Some("yes")
            )
        {
            return;
        }
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
    crate::trade_client::validate_pool_quote(&pool, Some(quote_mint)).ok()?;
    let pool_id = if matches!(pool.as_str(), "pump" | "bonk") {
        None
    } else {
        event
            .pool_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    };
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
            | "stonkfun"
            | "stonkfun-swap"
            | "meteora-damm-v2"
            | "meteora-dbc"
            | "raydium-amm-v4"
            | "raydium-cpmm"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cached_unsupported_pump_reports_quote_without_remote_resolution() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        db.upsert_pool_route(
            "mint-1",
            "pump",
            None,
            "WXMRyRZhsa19ety5erZhHg4N3xj3EVN92u94422teJp",
            "pumpapi",
        )
        .await
        .unwrap();
        let cache = PoolRouteCache::new(
            db,
            TradeServiceClient::new("http://127.0.0.1:0".into(), None, 1).unwrap(),
            1,
        );
        assert!(cache
            .resolve("mint-1", false)
            .await
            .unwrap_err()
            .to_string()
            .contains("Pump 池"));
    }

    #[tokio::test]
    async fn cached_unsupported_cpmm_reports_quote_without_remote_resolution() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        db.upsert_pool_route(
            "mint-1",
            "raydium-cpmm",
            Some("pool-1"),
            "DJTu7vi8norVzdVAffgvb39VP7wjKeTsgaMBJrzfxvoF",
            "pumpapi",
        )
        .await
        .unwrap();
        let cache = PoolRouteCache::new(
            db,
            TradeServiceClient::new("http://127.0.0.1:0".into(), None, 1).unwrap(),
            1,
        );
        let error = cache.resolve("mint-1", false).await.unwrap_err();
        assert!(error.to_string().contains("报价币不受支持"));
        let supported = ResolvedPoolRoute {
            mint: "mint-1".into(),
            pool: "raydium-cpmm".into(),
            pool_id: Some("pool-1".into()),
            quote_mint: crate::trade_client::WSOL_MINT.into(),
        };
        cache.store(&supported, "pumpapi").await;
        assert_eq!(cache.resolve("mint-1", false).await.unwrap(), supported);
        assert!(cache.resolve("mint-1", true).await.is_err());
    }

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

    #[test]
    fn unsupported_meteora_event_does_not_replace_supported_route() {
        let mut event = event(Some("custom-pool"));
        event.pool = Some("meteora-damm-v2".into());
        event.quote_mint = Some("2JnYVAeY4gYjRSmGRPQi9EudJm88AT6LgVKeKvxeb9Qp".into());
        assert!(route_from_event(&event).is_none());
        for quote in [
            crate::trade_client::WSOL_MINT,
            crate::trade_client::USDC_MINT,
        ] {
            event.quote_mint = Some(quote.into());
            let route = route_from_event(&event).unwrap();
            assert_eq!(route.pool_id.as_deref(), Some("custom-pool"));
            assert_eq!(route.quote_mint, quote);
        }
    }

    #[test]
    fn pump_event_route_discards_pool_id() {
        let mut event = event(Some("bonding-curve"));
        event.pool = Some("pump".to_string());
        event.quote_mint = Some(crate::trade_client::WSOL_MINT.to_string());
        let route = route_from_event(&event).unwrap();
        assert_eq!(route.pool, "pump");
        assert_eq!(route.pool_id, None);
    }
}
