use crate::{
    db::{models::AlphaPaperSignal, Database},
    executor::{normalize_trade_status, trade_service_pool_id},
    trade_client::{ResolvedPoolRoute, TradeRequest, TradeResponse, TradeServiceClient},
};
use anyhow::{Context, Result};
use chrono::Utc;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct AlphaShadowConfig {
    pub enabled: bool,
    pub wallet_id: Option<String>,
    pub buy_amount_sol: f64,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub poll_attempts: usize,
    pub max_inflight: usize,
    pub min_score: f64,
    pub max_submissions_per_minute: usize,
    pub require_allowlist: bool,
    pub allowed_pools: Option<HashSet<String>>,
    pub allowed_mints: Option<HashSet<String>>,
}

impl AlphaShadowConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: env_bool("ALPHA_SHADOW_ENABLED", false),
            wallet_id: std::env::var("ALPHA_SHADOW_WALLET_ID")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            buy_amount_sol: env_f64("ALPHA_SHADOW_BUY_SOL", 0.01).max(0.000001),
            slippage_bps: env_u64("ALPHA_SHADOW_SLIPPAGE_BPS", 1500),
            tip_sol: env_f64("ALPHA_SHADOW_TIP_SOL", 0.0001).max(0.0),
            poll_attempts: env_usize("ALPHA_SHADOW_POLL_ATTEMPTS", 5).clamp(1, 20),
            max_inflight: env_usize("ALPHA_SHADOW_MAX_INFLIGHT", 2).clamp(1, 32),
            min_score: env_f64("ALPHA_SHADOW_MIN_SCORE", 0.90).clamp(0.0, 1.0),
            max_submissions_per_minute: env_usize("ALPHA_SHADOW_MAX_PER_MINUTE", 10).max(1),
            // Fail closed: an enabled shadow run must explicitly name both its
            // pool and mint universe before it can reach Trade Service.
            require_allowlist: env_bool("ALPHA_SHADOW_REQUIRE_ALLOWLIST", true),
            allowed_pools: env_csv_set("ALPHA_SHADOW_ALLOWED_POOLS"),
            allowed_mints: env_csv_exact_set("ALPHA_SHADOW_ALLOWED_MINTS"),
        }
    }
}

#[derive(Clone)]
pub struct AlphaShadowExecutor {
    config: AlphaShadowConfig,
    db: Database,
    trade_client: TradeServiceClient,
    inflight: Arc<Semaphore>,
    submission_times: Arc<Mutex<VecDeque<Instant>>>,
}

impl AlphaShadowExecutor {
    pub fn new(db: Database, trade_client: TradeServiceClient) -> Self {
        let config = AlphaShadowConfig::from_env();
        Self {
            inflight: Arc::new(Semaphore::new(config.max_inflight)),
            config,
            db,
            trade_client,
            submission_times: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub async fn submit(&self, signal: AlphaPaperSignal) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        if signal.score < self.config.min_score {
            return Ok(());
        }
        if !self.is_allowlisted(&signal) {
            return Ok(());
        }
        let _permit = self
            .inflight
            .clone()
            .acquire_owned()
            .await
            .context("alpha shadow concurrency gate closed")?;
        let wallet_id = self
            .config
            .wallet_id
            .as_deref()
            .context("ALPHA_SHADOW_WALLET_ID is required when shadow is enabled")?;
        let route = self.route(&signal)?;
        let idempotency_key = format!("alpha-shadow:{}", signal.signal_id);
        let amount_sol = (signal.side == "buy").then_some(self.config.buy_amount_sol);
        let sell_percent = (signal.side == "sell").then_some(100_u64);
        let request = self.build_request(
            &signal,
            wallet_id,
            &route,
            idempotency_key.clone(),
            amount_sol,
            sell_percent,
        );
        let request_json = serde_json::json!({
            "signal_id": signal.signal_id,
            "mode": "shadow",
            "simulate": true,
            "trade_request": request,
        });
        let request: TradeRequest = serde_json::from_value(
            request_json
                .get("trade_request")
                .cloned()
                .context("shadow trade request serialization failed")?,
        )?;
        let request_json = serde_json::to_string(&request_json)?;
        let now_ms = Utc::now().timestamp_millis();
        let (order_id, created) = self
            .db
            .reserve_alpha_shadow_order(
                &signal,
                &idempotency_key,
                &request_json,
                amount_sol,
                sell_percent.map(|value| value as i64),
                now_ms,
            )
            .await?;
        if !created {
            return Ok(());
        }
        self.db
            .update_alpha_paper_signal_link(
                &signal.signal_id,
                Some(&order_id),
                None,
                Some("created"),
                now_ms,
            )
            .await?;
        self.db
            .update_alpha_shadow_order(&order_id, "submitting", None, None, None, now_ms)
            .await?;
        if !self.take_submission_slot().await {
            self.persist_error(
                &signal,
                &order_id,
                "blocked",
                "alpha shadow per-minute submission limit reached",
            )
            .await?;
            return Ok(());
        }
        let readiness = self.trade_client.ready().await.and_then(|ready| {
            anyhow::ensure!(
                ready.ready && ready.rpc_ok && ready.has_blockhash,
                "trade_service readiness gate failed for alpha shadow"
            );
            anyhow::ensure!(
                ready
                    .wallets
                    .iter()
                    .any(|wallet| wallet.wallet_id == wallet_id),
                "configured alpha shadow wallet is not present in trade_service readiness"
            );
            Ok(())
        });
        if let Err(err) = readiness {
            self.persist_error(&signal, &order_id, "blocked", &err.to_string())
                .await?;
            return Err(err);
        }
        if let Err(err) = self.trade_client.warmup_route(&route).await {
            self.persist_error(&signal, &order_id, "blocked", &err.to_string())
                .await?;
            return Err(err);
        }
        let response = match self.trade_client.submit_trade(&request).await {
            Ok(response) => response,
            Err(err) => {
                let message = err.to_string();
                self.persist_error(&signal, &order_id, "unknown", &message)
                    .await?;
                return Err(err);
            }
        };
        self.persist_response(&signal, &order_id, &response).await?;
        if let Some(request_id) = response.request_id.as_deref() {
            self.poll_request(&signal, &order_id, request_id).await?;
        }
        Ok(())
    }

    pub async fn reconcile_once(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        for order in self.db.list_alpha_shadow_reconcilable(50).await? {
            let Some(request_id) = order.trade_service_request_id.as_deref() else {
                continue;
            };
            match self.trade_client.get_trade(request_id).await {
                Ok(response) => {
                    let signal = AlphaPaperSignal {
                        signal_id: order.signal_id.clone(),
                        event_id: String::new(),
                        mint: order.mint.clone(),
                        pool: Some(order.pool.clone()),
                        pool_id: order.pool_id.clone(),
                        quote_mint: Some(order.quote_mint.clone()),
                        side: order.side.clone(),
                        score: 0.0,
                        status: "paper".to_string(),
                        features_json: "{}".to_string(),
                        created_at_ms: order.created_at_ms,
                        order_id: Some(order.id.clone()),
                        trade_service_request_id: Some(request_id.to_string()),
                        remote_status: Some(order.status.clone()),
                        updated_at_ms: Some(order.updated_at_ms),
                    };
                    self.persist_response(&signal, &order.id, &response).await?;
                }
                Err(err) => warn!(?err, request_id, "alpha shadow reconcile failed"),
            }
        }
        Ok(())
    }

    fn is_allowlisted(&self, signal: &AlphaPaperSignal) -> bool {
        if self.config.require_allowlist
            && (self.config.allowed_pools.is_none() || self.config.allowed_mints.is_none())
        {
            return false;
        }
        let pool_allowed = self.config.allowed_pools.as_ref().map_or(true, |pools| {
            signal
                .pool
                .as_deref()
                .map(|pool| pool.to_ascii_lowercase())
                .is_some_and(|pool| pools.contains(&pool))
        });
        let mint_allowed = self
            .config
            .allowed_mints
            .as_ref()
            .map_or(true, |mints| mints.contains(&signal.mint));
        pool_allowed && mint_allowed
    }

    async fn poll_request(
        &self,
        signal: &AlphaPaperSignal,
        order_id: &str,
        request_id: &str,
    ) -> Result<()> {
        for attempt in 0..self.config.poll_attempts {
            let response = self.trade_client.get_trade(request_id).await?;
            let status = normalize_trade_status(&response);
            self.persist_response(signal, order_id, &response).await?;
            if matches!(
                status,
                "simulated" | "submitted" | "confirmed" | "failed" | "unknown"
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(if attempt == 0 { 100 } else { 500 })).await;
        }
        Ok(())
    }

    async fn persist_response(
        &self,
        signal: &AlphaPaperSignal,
        order_id: &str,
        response: &TradeResponse,
    ) -> Result<()> {
        let status = normalize_trade_status(response);
        let response_json = serde_json::to_string(response)?;
        let now_ms = Utc::now().timestamp_millis();
        self.db
            .update_alpha_shadow_order(
                order_id,
                status,
                response.request_id.as_deref(),
                Some(&response_json),
                response.error.as_deref(),
                now_ms,
            )
            .await?;
        self.db
            .update_alpha_paper_signal_link(
                &signal.signal_id,
                Some(order_id),
                response.request_id.as_deref(),
                Some(status),
                now_ms,
            )
            .await?;
        info!(signal_id = %signal.signal_id, order_id, status, "alpha shadow response persisted");
        Ok(())
    }

    async fn persist_error(
        &self,
        signal: &AlphaPaperSignal,
        order_id: &str,
        status: &str,
        error: &str,
    ) -> Result<()> {
        let now_ms = Utc::now().timestamp_millis();
        self.db
            .update_alpha_shadow_order(order_id, status, None, None, Some(error), now_ms)
            .await?;
        self.db
            .update_alpha_paper_signal_link(
                &signal.signal_id,
                Some(order_id),
                None,
                Some(status),
                now_ms,
            )
            .await?;
        Ok(())
    }

    async fn take_submission_slot(&self) -> bool {
        let now = Instant::now();
        let mut times = self.submission_times.lock().await;
        while times
            .front()
            .is_some_and(|timestamp| now.duration_since(*timestamp) >= Duration::from_secs(60))
        {
            times.pop_front();
        }
        if times.len() >= self.config.max_submissions_per_minute {
            return false;
        }
        times.push_back(now);
        true
    }

    fn route(&self, signal: &AlphaPaperSignal) -> Result<ResolvedPoolRoute> {
        let pool = signal
            .pool
            .as_deref()
            .unwrap_or("pump")
            .trim()
            .to_ascii_lowercase();
        anyhow::ensure!(
            matches!(
                pool.as_str(),
                "pump"
                    | "pump-amm"
                    | "bonk"
                    | "stonkfun"
                    | "stonkfun-swap"
                    | "meteora-damm-v2"
                    | "meteora-dbc"
                    | "raydium-amm-v4"
                    | "raydium-cpmm"
            ),
            "unsupported alpha shadow pool {pool}"
        );
        let quote_mint = signal
            .quote_mint
            .clone()
            .context("alpha signal is missing quote_mint")?;
        Ok(ResolvedPoolRoute {
            pool_id: trade_service_pool_id(&pool, signal.pool_id.as_deref()),
            mint: signal.mint.clone(),
            pool,
            quote_mint,
        })
    }

    fn build_request(
        &self,
        signal: &AlphaPaperSignal,
        wallet_id: &str,
        route: &ResolvedPoolRoute,
        idempotency_key: String,
        amount_sol: Option<f64>,
        sell_percent: Option<u64>,
    ) -> TradeRequest {
        TradeRequest {
            wallet_id: Some(wallet_id.to_string()),
            execution_method: "sdk".to_string(),
            side: signal.side.clone(),
            pool: route.pool.clone(),
            mint: signal.mint.clone(),
            pool_id: route.pool_id.clone(),
            quote_mint: Some(route.quote_mint.clone()),
            amount_quote_units: None,
            settlement_asset: None,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_sol,
            sell_percent,
            slippage_percent: self.config.slippage_bps as f64 / 100.0,
            tip_sol: self.config.tip_sol,
            use_mev: false,
            mev_provider: None,
            simulate: true,
            source: Some("pumpapi-alpha-shadow".to_string()),
            source_signature: Some(signal.event_id.clone()),
            idempotency_key,
        }
    }
}

pub fn start_reconciler(executor: AlphaShadowExecutor) {
    if !executor.enabled() {
        return;
    }
    tokio::spawn(async move {
        loop {
            if let Err(err) = executor.reconcile_once().await {
                warn!(?err, "alpha shadow reconciler failed");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &f64| value.is_finite())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_csv_set(name: &str) -> Option<HashSet<String>> {
    let values: HashSet<String> = std::env::var(name)
        .ok()?
        .split(',')
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    (!values.is_empty()).then_some(values)
}

fn env_csv_exact_set(name: &str) -> Option<HashSet<String>> {
    let values: HashSet<String> = std::env::var(name)
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect();
    (!values.is_empty()).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::AlphaPaperSignal;

    fn signal(pool: &str, pool_id: Option<&str>) -> AlphaPaperSignal {
        AlphaPaperSignal {
            signal_id: "sig-1:alpha-flow-v1".to_string(),
            event_id: "sig-1:buy:mint-1:pool-1".to_string(),
            mint: "mint-1".to_string(),
            pool: Some(pool.to_string()),
            pool_id: pool_id.map(ToString::to_string),
            quote_mint: Some("So11111111111111111111111111111111111111112".to_string()),
            side: "buy".to_string(),
            score: 0.8,
            status: "paper".to_string(),
            features_json: "{}".to_string(),
            created_at_ms: 1,
            order_id: None,
            trade_service_request_id: None,
            remote_status: None,
            updated_at_ms: None,
        }
    }

    #[tokio::test]
    async fn shadow_route_keeps_pool_id_for_pool_specific_routes() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let client = TradeServiceClient::new("http://127.0.0.1:1".to_string(), None, 1).unwrap();
        let executor = AlphaShadowExecutor::new(db, client);
        let route = executor
            .route(&signal("raydium-cpmm", Some("pool-1")))
            .unwrap();
        assert_eq!(route.pool_id.as_deref(), Some("pool-1"));
        assert_eq!(
            route.quote_mint,
            "So11111111111111111111111111111111111111112"
        );
    }

    #[tokio::test]
    async fn shadow_route_drops_pool_id_for_pump_and_rejects_unknown_pool() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let client = TradeServiceClient::new("http://127.0.0.1:1".to_string(), None, 1).unwrap();
        let executor = AlphaShadowExecutor::new(db, client);
        let route = executor.route(&signal("pump", Some("ignored"))).unwrap();
        assert!(route.pool_id.is_none());
        assert!(executor.route(&signal("unknown", None)).is_err());
    }

    #[tokio::test]
    async fn shadow_request_is_always_simulated_and_idempotent() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let client = TradeServiceClient::new("http://127.0.0.1:1".to_string(), None, 1).unwrap();
        let executor = AlphaShadowExecutor::new(db, client);
        let signal = signal("raydium-cpmm", Some("pool-1"));
        let route = executor.route(&signal).unwrap();
        let request = executor.build_request(
            &signal,
            "wallet-1",
            &route,
            "alpha-shadow:sig-1:alpha-flow-v1".to_string(),
            Some(0.01),
            None,
        );
        assert!(request.simulate);
        assert_eq!(request.source.as_deref(), Some("pumpapi-alpha-shadow"));
        assert_eq!(
            request.source_signature.as_deref(),
            Some(signal.event_id.as_str())
        );
        assert_eq!(request.idempotency_key, "alpha-shadow:sig-1:alpha-flow-v1");
    }

    #[test]
    fn csv_allowlist_normalizes_values() {
        std::env::set_var("ALPHA_SHADOW_ALLOWED_POOLS", " pump-amm, PUMP ");
        let values = env_csv_set("ALPHA_SHADOW_ALLOWED_POOLS").unwrap();
        assert!(values.contains("pump-amm"));
        assert!(values.contains("pump"));
        std::env::remove_var("ALPHA_SHADOW_ALLOWED_POOLS");
    }

    #[tokio::test]
    async fn enabled_shadow_fails_closed_without_both_allowlists() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let client = TradeServiceClient::new("http://127.0.0.1:1".to_string(), None, 1).unwrap();
        let mut executor = AlphaShadowExecutor::new(db, client);
        executor.config.require_allowlist = true;
        executor.config.allowed_pools = None;
        executor.config.allowed_mints = Some(["mint-1".to_string()].into_iter().collect());
        assert!(!executor.is_allowlisted(&signal("pump", None)));
    }

    #[tokio::test]
    async fn allowlist_matches_pool_case_insensitively_and_mint_exactly() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let client = TradeServiceClient::new("http://127.0.0.1:1".to_string(), None, 1).unwrap();
        let mut executor = AlphaShadowExecutor::new(db, client);
        executor.config.require_allowlist = true;
        executor.config.allowed_pools = Some(["pump-amm".to_string()].into_iter().collect());
        executor.config.allowed_mints = Some(["mint-1".to_string()].into_iter().collect());
        assert!(executor.is_allowlisted(&signal("PUMP-AMM", None)));
        assert!(!executor.is_allowlisted(&AlphaPaperSignal {
            mint: "MINT-1".to_string(),
            ..signal("pump-amm", None)
        }));
    }
}
