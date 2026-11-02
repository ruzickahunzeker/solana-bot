use crate::{
    db::{models::PumpApiEvent, Database},
    trade_client::{ResolvedPoolRoute, TradeServiceClient},
};
use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tracing::{debug, warn};

const MEMORY_CACHE_CAPACITY: usize = 4096;
const VERIFIED_ROUTE_TTL: Duration = Duration::from_secs(15);
const CAPABILITIES_TTL: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct PoolRouteCache {
    db: Database,
    trade_client: TradeServiceClient,
    resolve_timeout: Duration,
    memory: Arc<RwLock<HashMap<String, Vec<ResolvedPoolRoute>>>>,
    verified_until: Arc<RwLock<HashMap<String, Instant>>>,
    verified_routes: Arc<RwLock<HashMap<String, ResolvedPoolRoute>>>,
    live_protocols: Arc<RwLock<Option<HashSet<String>>>>,
    capabilities_until: Arc<RwLock<Option<Instant>>>,
}

impl PoolRouteCache {
    pub fn new(db: Database, trade_client: TradeServiceClient, timeout_secs: u64) -> Self {
        Self {
            db,
            trade_client,
            resolve_timeout: Duration::from_secs(timeout_secs.max(1)),
            memory: Arc::new(RwLock::new(HashMap::new())),
            verified_until: Arc::new(RwLock::new(HashMap::new())),
            verified_routes: Arc::new(RwLock::new(HashMap::new())),
            live_protocols: Arc::new(RwLock::new(None)),
            capabilities_until: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn resolve(&self, mint: &str, force_refresh: bool) -> Result<ResolvedPoolRoute> {
        if !force_refresh {
            if let Some(route) = self.preferred_memory_route(mint) {
                return Ok(route);
            }
            if let Some(route) = self.preferred_db_route(mint).await? {
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

    pub async fn resolve_verified(&self, mint: &str) -> Result<ResolvedPoolRoute> {
        let now = Instant::now();
        let cached = self
            .verified_until
            .read()
            .expect("verified route cache poisoned")
            .get(mint)
            .copied()
            .is_some_and(|until| until > now);
        if cached {
            if let Some(route) = self
                .verified_routes
                .read()
                .expect("verified route cache poisoned")
                .get(mint)
                .cloned()
            {
                return validate_verified_route(route);
            }
        }

        // PumpAPI events already carry the pool address. Verify that candidate
        // directly first; a full mint scan is only the fallback for cache misses
        // or stale routes.
        for candidate in self.cached_routes(mint).await? {
            self.ensure_live_protocol(&candidate.pool).await?;
            let candidate_label = route_label(&candidate);
            let verified = tokio::time::timeout(
                self.resolve_timeout,
                self.trade_client.warmup_route(&candidate),
            )
            .await;
            if matches!(verified, Ok(Ok(()))) {
                let route = validate_verified_route(candidate)?;
                self.store(&route, "trade_service_verified").await;
                self.verified_until
                    .write()
                    .expect("verified route cache poisoned")
                    .insert(mint.to_string(), now + VERIFIED_ROUTE_TTL);
                self.verified_routes
                    .write()
                    .expect("verified route cache poisoned")
                    .insert(mint.to_string(), route.clone());
                return Ok(route);
            }
            if let Ok(Err(error)) = verified {
                warn!(mint, route = %candidate_label, ?error, "cached pool route verification failed; trying next candidate");
            } else {
                warn!(mint, route = %candidate_label, "cached pool route verification timed out; trying next candidate");
            }
        }

        let route =
            tokio::time::timeout(self.resolve_timeout, self.trade_client.resolve_pool(mint))
                .await
                .context("verified pool resolution timeout")??;
        self.ensure_live_protocol(&route.pool).await?;
        let route = validate_verified_route(route)?;
        self.store(&route, "trade_service_verified").await;
        self.verified_until
            .write()
            .expect("verified route cache poisoned")
            .insert(mint.to_string(), now + VERIFIED_ROUTE_TTL);
        self.verified_routes
            .write()
            .expect("verified route cache poisoned")
            .insert(mint.to_string(), route.clone());
        Ok(route)
    }

    async fn ensure_live_protocol(&self, pool: &str) -> Result<()> {
        let now = Instant::now();
        let cache_valid = self
            .capabilities_until
            .read()
            .expect("capabilities cache poisoned")
            .is_some_and(|until| until > now);
        if !cache_valid {
            let response = self
                .trade_client
                .capabilities()
                .await
                .context("trade_service capabilities unavailable")?;
            let live = response
                .protocols
                .into_iter()
                .filter(|capability| {
                    capability.status == "live"
                        && capability.supports_buy
                        && capability.supports_sell
                })
                .map(|capability| capability.protocol_id)
                .collect::<HashSet<_>>();
            *self
                .live_protocols
                .write()
                .expect("capabilities cache poisoned") = Some(live);
            *self
                .capabilities_until
                .write()
                .expect("capabilities cache poisoned") = Some(now + CAPABILITIES_TTL);
        }

        let allowed = self
            .live_protocols
            .read()
            .expect("capabilities cache poisoned")
            .as_ref()
            .is_some_and(|protocols| protocols.contains(pool));
        if !allowed {
            anyhow::bail!("trade protocol {pool} is not live in trade_service capabilities");
        }
        Ok(())
    }

    async fn cached_routes(&self, mint: &str) -> Result<Vec<ResolvedPoolRoute>> {
        let mut routes = self
            .memory
            .read()
            .expect("pool route memory cache poisoned")
            .get(mint)
            .cloned()
            .unwrap_or_default();
        if routes.is_empty() {
            routes = self
                .db
                .observed_pool_route_candidates(mint)
                .await?
                .into_iter()
                .map(|observed| ResolvedPoolRoute {
                    mint: mint.to_string(),
                    pool: observed.pool,
                    pool_id: observed.pool_id,
                    quote_mint: observed.quote_mint,
                })
                .collect();
        }
        if routes.is_empty() {
            if let Some(observed) = self.db.latest_observed_pool_route(mint).await? {
                routes.push(ResolvedPoolRoute {
                    mint: mint.to_string(),
                    pool: observed.pool,
                    pool_id: observed.pool_id,
                    quote_mint: observed.quote_mint,
                });
            }
        }
        sort_routes(&mut routes);
        Ok(routes)
    }

    async fn preferred_db_route(&self, mint: &str) -> Result<Option<ResolvedPoolRoute>> {
        let routes = self.cached_routes(mint).await?;
        Ok(routes.into_iter().next())
    }

    fn preferred_memory_route(&self, mint: &str) -> Option<ResolvedPoolRoute> {
        let mut routes = self
            .memory
            .read()
            .expect("pool route memory cache poisoned")
            .get(mint)
            .cloned()?;
        sort_routes(&mut routes);
        routes.into_iter().next()
    }

    pub async fn observe_event(&self, event: &PumpApiEvent) {
        let Some(route) = route_from_event(event) else {
            return;
        };
        let verified = self
            .verified_until
            .read()
            .expect("verified route cache poisoned")
            .get(&route.mint)
            .copied()
            .is_some_and(|until| until > Instant::now());
        if verified {
            return;
        }
        self.remember(route.clone());
        self.persist(&route, "pumpapi").await;
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
        let has_mint = memory.contains_key(&route.mint);
        if memory.len() >= MEMORY_CACHE_CAPACITY && !has_mint {
            debug!(
                entries = memory.len(),
                "clearing full pool route memory cache"
            );
            memory.clear();
        }
        let routes = memory.entry(route.mint.clone()).or_default();
        if routes.iter().any(|candidate| candidate == &route) {
            return false;
        }
        routes.push(route);
        true
    }
}

pub(crate) fn validate_verified_route(route: ResolvedPoolRoute) -> Result<ResolvedPoolRoute> {
    Ok(route)
}

fn route_label(route: &ResolvedPoolRoute) -> String {
    format!(
        "{}:{}:{}",
        route.pool,
        route.pool_id.as_deref().unwrap_or("-"),
        route.quote_mint
    )
}

fn sort_routes(routes: &mut [ResolvedPoolRoute]) {
    routes.sort_by_key(|route| {
        let quote_priority = match route.quote_mint.as_str() {
            "So11111111111111111111111111111111111111112"
            | "So11111111111111111111111111111111111111111" => 0,
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => 1,
            _ => 2,
        };
        let pool_priority = match route.pool.as_str() {
            "meteora-damm-v2" => 0,
            "pump-amm" => 1,
            _ => 2,
        };
        (quote_priority, pool_priority, route.pool_id.clone())
    });
}

fn route_from_event(event: &PumpApiEvent) -> Option<ResolvedPoolRoute> {
    let mint = event.mint.as_deref()?.trim();
    let pool = event.pool.as_deref()?.trim().to_ascii_lowercase();
    let quote_mint = event.quote_mint.as_deref()?.trim();
    if mint.is_empty() || quote_mint.is_empty() || !supported_pool(&pool) {
        return None;
    }
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
    fn verified_pump_amm_route_accepts_custom_quote_for_direct_trade() {
        let route = ResolvedPoolRoute {
            mint: "2qQVrHpQaWzGf8iGYbpVUjYELYN2Q911XQWxFbsGpump".to_string(),
            pool: "pump-amm".to_string(),
            pool_id: Some("pool-id".to_string()),
            quote_mint: "CARDSccUMFKoPRZxt5vt3ksUbxEFEcnZ3H2pd3dKxYjp".to_string(),
        };
        assert_eq!(validate_verified_route(route).unwrap().pool, "pump-amm");
    }

    #[test]
    fn standard_quote_route_is_preferred_over_custom_quote() {
        let mut routes = vec![
            ResolvedPoolRoute {
                mint: "mint-1".to_string(),
                pool: "pump-amm".to_string(),
                pool_id: Some("custom-pool".to_string()),
                quote_mint: "CustomQuote1111111111111111111111111111111111".to_string(),
            },
            ResolvedPoolRoute {
                mint: "mint-1".to_string(),
                pool: "meteora-damm-v2".to_string(),
                pool_id: Some("wsol-pool".to_string()),
                quote_mint: "So11111111111111111111111111111111111111112".to_string(),
            },
        ];

        sort_routes(&mut routes);

        assert_eq!(routes.first().unwrap().pool, "meteora-damm-v2");
        assert_eq!(
            routes.first().unwrap().quote_mint,
            "So11111111111111111111111111111111111111112"
        );
    }

    #[test]
    fn pump_event_route_discards_pool_id() {
        let mut event = event(Some("bonding-curve"));
        event.pool = Some("pump".to_string());
        let route = route_from_event(&event).unwrap();
        assert_eq!(route.pool, "pump");
        assert_eq!(route.pool_id, None);
    }
}
