use crate::{
    config::AppConfig,
    db::{
        models::{MintSignalDetails, PumpApiEvent, StrategyConfig},
        Database,
    },
    executor::{ExecutionRequest, ExecutionRouter, FastFollowExecutionRequest},
    risk::RiskEngine,
};
use chrono::Utc;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, RwLock},
    time::{Duration, Instant},
};
use teloxide::Bot;
use tokio::{
    runtime::Handle,
    sync::{Mutex, Semaphore},
};
use tracing::{debug, info, warn};

const WSOL: &str = "So11111111111111111111111111111111111111112";
const SOL: &str = "So11111111111111111111111111111111111111111";

#[derive(Clone)]
struct CachedFollow {
    watch: crate::db::models::WatchMatch,
    strategy: StrategyConfig,
    trade_service_wallet_id: Option<String>,
    allow_mev: bool,
}

#[derive(Clone)]
pub struct StrategyEngine {
    config: AppConfig,
    db: Database,
    risk: RiskEngine,
    executor: ExecutionRouter,
    bot: Option<Bot>,
    cache: Arc<RwLock<HashMap<String, Vec<CachedFollow>>>>,
    recent_events: Arc<StdMutex<HashMap<String, Instant>>>,
    inflight: Arc<Semaphore>,
    serializers: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl StrategyEngine {
    pub fn new(
        config: AppConfig,
        db: Database,
        risk: RiskEngine,
        executor: ExecutionRouter,
        bot: Option<Bot>,
    ) -> Self {
        let max_inflight = config.fast_follow_max_inflight;
        Self {
            config,
            db,
            risk,
            executor,
            bot,
            cache: Arc::new(RwLock::new(HashMap::new())),
            recent_events: Arc::new(StdMutex::new(HashMap::new())),
            inflight: Arc::new(Semaphore::new(max_inflight)),
            serializers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn refresh_cache(&self) -> anyhow::Result<usize> {
        let watches = self.db.list_active_watch_matches().await?;
        let mut next: HashMap<String, Vec<CachedFollow>> = HashMap::new();
        for watch in watches {
            let active_wallet = self.db.active_wallet(watch.user_id).await?;
            let strategy = self
                .db
                .get_or_create_strategy_for_watch(
                    watch.user_id,
                    watch.watch_id,
                    active_wallet.as_ref().map(|item| item.id),
                    &format!("跟单 {}", watch.watch_label),
                    self.config.default_buy_amount_sol,
                    self.config.default_slippage_bps,
                    self.config.default_tip_sol,
                )
                .await?;
            let execution_wallet = if let Some(wallet_id) = strategy.wallet_id {
                self.db.get_wallet(watch.user_id, wallet_id).await.ok()
            } else {
                active_wallet
            };
            let allow_mev = self
                .db
                .user_mev_enabled(watch.user_id)
                .await
                .unwrap_or(false);
            next.entry(watch.watch_address.clone())
                .or_default()
                .push(CachedFollow {
                    watch,
                    strategy,
                    trade_service_wallet_id: execution_wallet
                        .and_then(|item| item.external_wallet_id),
                    allow_mev,
                });
        }
        let count = next.values().map(Vec::len).sum();
        *self.cache.write().expect("fast follow cache poisoned") = next;
        debug!(count, "fast follow strategy cache refreshed");
        Ok(count)
    }

    pub fn start_cache_refresh(&self) {
        let engine = self.clone();
        let interval = Duration::from_millis(self.config.fast_follow_cache_refresh_ms);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if let Err(err) = engine.refresh_cache().await {
                    warn!(?err, "failed to refresh fast follow strategy cache");
                }
            }
        });
    }

    pub fn fast_follow_enabled(&self) -> bool {
        self.config.fast_follow_enabled
    }

    pub fn panic_pause(&self) {
        self.risk.set_paused(true);
    }

    pub fn dispatch_event(&self, handle: &Handle, event: PumpApiEvent) -> bool {
        if !self.config.fast_follow_enabled {
            return false;
        }
        let Some(signer) = event.signer.as_deref() else {
            return false;
        };
        let matches = self
            .cache
            .read()
            .expect("fast follow cache poisoned")
            .get(signer)
            .cloned()
            .unwrap_or_default();
        if matches.is_empty() {
            return false;
        }
        if let Some(signature) = event.signature.as_deref() {
            let key = format!(
                "{}:{}:{}",
                signature,
                event.tx_type.as_deref().unwrap_or(""),
                signer
            );
            let mut recent = self
                .recent_events
                .lock()
                .expect("recent event cache poisoned");
            let now = Instant::now();
            recent.retain(|_, seen| now.duration_since(*seen) < Duration::from_secs(600));
            if recent.insert(key, now).is_some() {
                return true;
            }
        }

        for cached in matches {
            let Ok(permit) = self.inflight.clone().try_acquire_owned() else {
                warn!(
                    signer,
                    mint = event.mint.as_deref().unwrap_or(""),
                    strategy_id = cached.strategy.id,
                    "fast follow concurrency limit reached; dropping signal"
                );
                continue;
            };
            let engine = self.clone();
            let event = event.clone();
            handle.spawn(async move {
                let _permit = permit;
                let mint = event.mint.clone().unwrap_or_default();
                let wallet_key = cached
                    .trade_service_wallet_id
                    .clone()
                    .unwrap_or_else(|| format!("local:{}", cached.strategy.wallet_id.unwrap_or(0)));
                let serial_key = format!("{wallet_key}:{mint}");
                let serial = {
                    let mut locks = engine.serializers.lock().await;
                    locks
                        .entry(serial_key.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(())))
                        .clone()
                };
                let _serial_guard = serial.lock().await;
                engine.observe_cached(cached, event).await;
                let mut locks = engine.serializers.lock().await;
                if Arc::strong_count(&serial) == 2 {
                    locks.remove(&serial_key);
                }
            });
        }
        true
    }

    async fn observe_cached(&self, cached: CachedFollow, event: PumpApiEvent) {
        let Some(mint) = event.mint.clone() else {
            return;
        };
        let Some(side) = normalize_side(event.tx_type.as_deref()) else {
            return;
        };
        let mut details = signal_details(&cached.strategy, side, &event, &cached.watch, &mint);
        let block_reason = strategy_block_reason(&cached.strategy, side, &event).or_else(|| {
            fast_route_block_reason(
                &cached.strategy,
                side,
                &event,
                cached.trade_service_wallet_id.as_deref(),
            )
        });
        if let Some(reason) = block_reason {
            details.filter_reason = Some(reason.clone());
            self.persist_cached_signal(&cached, &event, &mint, side, "blocked", &details)
                .await;
            return;
        }

        let metadata_json = encode_signal_details(&details);
        let reason = signal_reason(&cached.watch, side, &event);
        if cached.strategy.dry_run {
            let source_event_id = match self.db.insert_or_get_pumpapi_event(&event).await {
                Ok(id) => id,
                Err(err) => {
                    warn!(?err, "failed to store dry-run fast follow event");
                    return;
                }
            };
            let signal_id = match self
                .db
                .insert_signal(
                    cached.watch.user_id,
                    Some(cached.strategy.id),
                    source_event_id,
                    &mint,
                    side,
                    &reason,
                    "dry_run",
                    Some(&metadata_json),
                )
                .await
            {
                Ok(id) => id,
                Err(err) => {
                    warn!(?err, "failed to store dry-run fast follow signal");
                    return;
                }
            };
            if let Err(err) = self
                .executor
                .execute(ExecutionRequest {
                    strategy: &cached.strategy,
                    signal_id,
                    source_event_id,
                    mint: &mint,
                    side,
                    event: &event,
                })
                .await
            {
                warn!(
                    ?err,
                    signal_id, "failed to execute dry-run fast follow signal"
                );
            }
            return;
        }

        let Some(trade_service_wallet_id) = cached.trade_service_wallet_id else {
            return;
        };
        let request = FastFollowExecutionRequest {
            strategy: cached.strategy,
            event,
            mint,
            side: side.to_string(),
            trade_service_wallet_id,
            allow_mev: cached.allow_mev,
            signal_reason: reason,
            signal_metadata_json: metadata_json,
        };
        if let Err(err) = self.executor.execute_fast_follow(request).await {
            warn!(?err, "fast follow live execution failed");
        }
    }

    async fn persist_cached_signal(
        &self,
        cached: &CachedFollow,
        event: &PumpApiEvent,
        mint: &str,
        side: &str,
        status: &str,
        details: &MintSignalDetails,
    ) {
        let source_event_id = match self.db.insert_or_get_pumpapi_event(event).await {
            Ok(id) => id,
            Err(err) => {
                warn!(?err, "failed to store blocked fast follow event");
                return;
            }
        };
        let metadata_json = encode_signal_details(details);
        let reason = signal_reason(&cached.watch, side, event);
        if let Err(err) = self
            .db
            .insert_signal(
                cached.watch.user_id,
                Some(cached.strategy.id),
                source_event_id,
                mint,
                side,
                &reason,
                status,
                Some(&metadata_json),
            )
            .await
        {
            warn!(?err, "failed to store blocked fast follow signal");
        }
    }

    pub async fn observe_event(&self, source_event_id: i64, event: &PumpApiEvent) {
        let decision = self.risk.check_live_trade_allowed(true);
        info!(
            mint = ?event.mint,
            tx_type = ?event.tx_type,
            allowed = decision.allowed,
            reason = decision.reason,
            "strategy observed event"
        );

        let Some(signer) = event.signer.as_deref() else {
            return;
        };
        let Some(mint) = event.mint.as_deref() else {
            return;
        };
        let side = normalize_side(event.tx_type.as_deref());
        if !matches!(side, Some("buy" | "sell")) {
            return;
        }
        let side = side.unwrap();

        let matches = match self.db.find_watch_matches(signer).await {
            Ok(matches) => matches,
            Err(err) => {
                warn!(?err, signer, "failed to lookup watch matches");
                return;
            }
        };

        for item in matches {
            let wallet = match self.db.active_wallet(item.user_id).await {
                Ok(wallet) => wallet,
                Err(err) => {
                    warn!(
                        ?err,
                        user_id = item.user_id,
                        "failed to lookup active wallet"
                    );
                    None
                }
            };
            let strategy = match self
                .db
                .get_or_create_strategy_for_watch(
                    item.user_id,
                    item.watch_id,
                    wallet.as_ref().map(|wallet| wallet.id),
                    &format!("跟单 {}", item.watch_label),
                    self.config.default_buy_amount_sol,
                    self.config.default_slippage_bps,
                    self.config.default_tip_sol,
                )
                .await
            {
                Ok(strategy) => strategy,
                Err(err) => {
                    warn!(?err, watch_id = item.watch_id, "failed to get strategy");
                    continue;
                }
            };
            let mut details = signal_details(&strategy, side, event, &item, mint);
            if let Some(reason) = strategy_block_reason(&strategy, side, event) {
                details.filter_reason = Some(reason.clone());
                let metadata_json = encode_signal_details(&details);
                let signal_reason = signal_reason(&item, side, event);
                info!(
                    strategy_id = strategy.id,
                    reason, "signal blocked by strategy"
                );
                if let Err(err) = self
                    .db
                    .insert_signal(
                        item.user_id,
                        Some(strategy.id),
                        source_event_id,
                        mint,
                        side,
                        &signal_reason,
                        "blocked",
                        Some(&metadata_json),
                    )
                    .await
                {
                    warn!(?err, "failed to insert blocked signal");
                }
                continue;
            }

            let metadata_json = encode_signal_details(&details);
            let reason = signal_reason(&item, side, event);
            let signal_id = match self
                .db
                .insert_signal(
                    item.user_id,
                    Some(strategy.id),
                    source_event_id,
                    mint,
                    side,
                    &reason,
                    if strategy.dry_run { "dry_run" } else { "live" },
                    Some(&metadata_json),
                )
                .await
            {
                Ok(signal_id) => signal_id,
                Err(err) => {
                    warn!(?err, "failed to insert dry-run signal");
                    continue;
                }
            };

            if let Err(err) = self
                .executor
                .execute(ExecutionRequest {
                    strategy: &strategy,
                    signal_id,
                    source_event_id,
                    mint,
                    side,
                    event,
                })
                .await
            {
                warn!(?err, signal_id, "failed to execute signal");
            }

            let _ = &self.bot;
        }
    }
}

fn signal_details(
    strategy: &StrategyConfig,
    side: &str,
    event: &PumpApiEvent,
    item: &crate::db::models::WatchMatch,
    mint: &str,
) -> MintSignalDetails {
    let notification_unix_ms = Utc::now().timestamp_millis();
    MintSignalDetails {
        strategy_id: Some(strategy.id),
        watch_label: item.watch_label.clone(),
        watch_wallet: item.watch_address.clone(),
        signer: event.signer.clone(),
        signature: event.signature.clone(),
        mint: mint.to_string(),
        side: side.to_string(),
        pool: event.pool.clone(),
        pool_id: event.pool_id.clone(),
        quote_mint: event.quote_mint.clone(),
        sol_in_pool: event.sol_in_pool,
        tokens_in_pool: event.tokens_in_pool,
        quote_in_pool: event.quote_in_pool,
        event_timestamp_ms: event.event_timestamp_ms,
        received_unix_ms: Some(event.received_unix_ms),
        ingest_delay_ms: event
            .event_timestamp_ms
            .map(|timestamp| event.received_unix_ms - timestamp),
        notification_unix_ms: Some(notification_unix_ms),
        notification_delay_ms: Some(notification_unix_ms - event.received_unix_ms),
        default_buy_amount_sol: effective_buy_amount_sol(strategy, event),
        filter_reason: None,
        target_buy_sol: json_number(&event.raw_json, "quoteAmount"),
        market_cap_quote: json_number(&event.raw_json, "marketCapQuote"),
    }
}

fn encode_signal_details(details: &MintSignalDetails) -> String {
    match serde_json::to_string(details) {
        Ok(value) => value,
        Err(err) => {
            warn!(?err, "failed to encode signal metadata");
            "{}".to_string()
        }
    }
}

fn signal_reason(item: &crate::db::models::WatchMatch, side: &str, event: &PumpApiEvent) -> String {
    format!(
        "watch_id={} wallet={} tx_type={} signature={}",
        item.watch_id,
        item.watch_address,
        side,
        event.signature.as_deref().unwrap_or("")
    )
}

fn strategy_block_reason(
    strategy: &StrategyConfig,
    side: &str,
    event: &PumpApiEvent,
) -> Option<String> {
    if !strategy.enabled {
        return Some("strategy disabled".to_string());
    }
    if side == "buy" && !strategy.copy_buy {
        return Some("copy buy disabled".to_string());
    }
    if side == "sell" && !strategy.copy_sell {
        return Some("copy sell disabled".to_string());
    }
    if strategy.min_sol_in_pool > 0.0 && event.sol_in_pool.unwrap_or(0.0) < strategy.min_sol_in_pool
    {
        return Some(format!(
            "sol pool too small: actual={} min={}",
            event.sol_in_pool.unwrap_or(0.0),
            strategy.min_sol_in_pool
        ));
    }
    let mcap = json_number(&event.raw_json, "marketCapQuote");
    if strategy.min_mcap > 0.0 && mcap.unwrap_or(0.0) < strategy.min_mcap {
        return Some(format!(
            "market cap too small: actual={} min={}",
            mcap.unwrap_or(0.0),
            strategy.min_mcap
        ));
    }
    if strategy.max_mcap > 0.0 && mcap.map(|value| value > strategy.max_mcap).unwrap_or(false) {
        return Some(format!(
            "market cap too high: actual={} max={}",
            mcap.unwrap_or(0.0),
            strategy.max_mcap
        ));
    }
    let target_buy = json_number(&event.raw_json, "quoteAmount");
    if strategy.min_target_buy_sol > 0.0 && target_buy.unwrap_or(0.0) < strategy.min_target_buy_sol
    {
        return Some(format!(
            "target buy too small: actual={} min={}",
            target_buy.unwrap_or(0.0),
            strategy.min_target_buy_sol
        ));
    }
    if strategy.max_target_buy_sol > 0.0
        && target_buy
            .map(|value| value > strategy.max_target_buy_sol)
            .unwrap_or(false)
    {
        return Some(format!(
            "target buy too high: actual={} max={}",
            target_buy.unwrap_or(0.0),
            strategy.max_target_buy_sol
        ));
    }
    if let Some(max_delay) = strategy.max_signal_delay_ms {
        let delay = event
            .event_timestamp_ms
            .map(|timestamp| event.received_unix_ms - timestamp);
        if delay.map(|value| value > max_delay).unwrap_or(false) {
            return Some(format!(
                "signal delay too high: actual={}ms max={}ms",
                delay.unwrap_or(0),
                max_delay
            ));
        }
    }
    if !strategy.allowed_pools.is_empty() {
        let pool = event.pool.as_deref().unwrap_or("");
        if !strategy.allowed_pools.iter().any(|item| item == pool) {
            return Some(format!("pool not allowed: actual={}", pool));
        }
    }
    None
}

fn fast_route_block_reason(
    strategy: &StrategyConfig,
    side: &str,
    event: &PumpApiEvent,
    trade_service_wallet_id: Option<&str>,
) -> Option<String> {
    if strategy.dry_run {
        return None;
    }
    if event.signature.as_deref().unwrap_or("").is_empty() {
        return Some("live fast follow requires source signature".to_string());
    }
    if trade_service_wallet_id.is_none() {
        return Some("execution wallet is not bound to trade_service".to_string());
    }
    if event.pool.as_deref().unwrap_or("").is_empty() {
        return Some("fast follow event is missing pool".to_string());
    }
    let pool = event.pool.as_deref().unwrap_or("");
    if !matches!(
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
    ) {
        return Some(format!("unsupported trade_service pool {pool}"));
    }
    if !matches!(pool, "pump" | "bonk") && event.pool_id.as_deref().unwrap_or("").is_empty() {
        return Some(format!("fast follow event is missing pool_id for {pool}"));
    }
    if side == "buy" {
        if let Some(quote_mint) = event.quote_mint.as_deref() {
            if !matches!(quote_mint, WSOL | SOL) {
                if matches!(pool, "stonkfun" | "stonkfun-swap") {
                    return Some(
                        "自定义 Quote 跟买尚未设置 Quote 金额；当前 SOL 策略金额不可用于该池"
                            .to_string(),
                    );
                }
                return Some(format!("unsupported fast follow quote mint {quote_mint}"));
            }
        }
    }
    None
}

pub fn effective_buy_amount_sol(strategy: &StrategyConfig, event: &PumpApiEvent) -> f64 {
    if strategy.buy_mode == "percentage" {
        let target_buy = json_number(&event.raw_json, "quoteAmount").unwrap_or(0.0);
        let amount = target_buy * strategy.buy_ratio_percent / 100.0;
        if strategy.max_per_mint_sol > 0.0 {
            amount.min(strategy.max_per_mint_sol)
        } else {
            amount
        }
    } else {
        strategy.buy_amount_sol
    }
}

fn json_number(raw_json: &str, field: &str) -> Option<f64> {
    let value: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    value.get(field).and_then(|item| {
        item.as_f64()
            .or_else(|| item.as_str().and_then(|text| text.parse::<f64>().ok()))
    })
}

fn normalize_side(value: Option<&str>) -> Option<&'static str> {
    let value = value?.trim().to_ascii_lowercase();
    match value.as_str() {
        "buy" | "create" => Some("buy"),
        "sell" => Some("sell"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{StrategyEngine, WSOL};
    use crate::{
        config::AppConfig,
        db::{models::PumpApiEvent, Database},
        executor::ExecutionRouter,
        risk::RiskEngine,
        trade_client::TradeServiceClient,
    };
    use std::{collections::HashSet, time::Duration};

    fn test_config() -> AppConfig {
        AppConfig {
            telegram_bot_token: "test".to_string(),
            allowed_telegram_user_ids: HashSet::from([1]),
            database_url: "sqlite::memory:".to_string(),
            master_key_hex: "00".repeat(32),
            pumpapi_zmq_endpoint: "tcp://127.0.0.1:8939".to_string(),
            enable_stream_consumer: false,
            enable_telegram_bot: false,
            trade_service_url: "http://127.0.0.1:1".to_string(),
            trade_service_token: None,
            route_decision_url: None,
            route_decision_token: None,
            web_api_base_url: None,
            web_api_token: None,
            pnl_position_wallets: Vec::new(),
            trade_service_timeout_secs: 1,
            pool_resolve_timeout_secs: 1,
            trade_submission_enabled: true,
            isolated_sdk_simulation_enabled: false,
            trade_submit_max_attempts: 1,
            trade_submit_retry_delay_ms: 1,
            trade_reconcile_interval_secs: 1,
            fast_follow_enabled: true,
            fast_follow_max_inflight: 16,
            fast_follow_cache_refresh_ms: 5000,
            fast_follow_limit_queue_capacity: 4096,
            migrate_local_wallets_to_trade_service: false,
            dry_run_default: true,
            default_buy_amount_sol: 0.01,
            quick_buy_presets_sol: [0.001, 0.005, 0.01, 0.05, 0.1],
            quick_sell_presets_percent: [10.0, 25.0, 50.0, 75.0, 100.0],
            default_slippage_bps: 1500,
            default_tip_sol: 0.0001,
            use_mev: false,
            default_mev_provider: "auto".to_string(),
            sol_usd_price: Some(150.0),
            sol_usd_price_api_url: "http://127.0.0.1:1".to_string(),
            sol_usd_price_api_key: None,
            sol_usd_refresh_secs: 0,
            alpha_retention_enabled: true,
            alpha_retention_interval_secs: 900,
            alpha_observation_retention_hours: 72,
            alpha_paper_signal_retention_days: 7,
            alpha_observation_max_rows: 50_000,
            alpha_paper_signal_max_rows: 20_000,
            alpha_retention_batch_size: 1_000,
        }
    }

    #[tokio::test]
    async fn watched_buy_creates_dry_run_signal_order_and_position() {
        let config = test_config();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(1, Some("test")).await.unwrap();
        let signer = "5aLY85pyxiuX3fd4RgM3Yc1e3MAL6b7UgaZz6MS3JUfG";
        db.add_watch_wallet(user.id, signer, "Replay")
            .await
            .unwrap();
        let trade_client = TradeServiceClient::new(
            config.trade_service_url.clone(),
            None,
            config.trade_service_timeout_secs,
        )
        .unwrap()
        .with_submission_enabled(true);
        let risk = RiskEngine::new();
        let executor = ExecutionRouter::new(db.clone(), risk.clone(), trade_client, 1, 1);
        let engine = StrategyEngine::new(config, db.clone(), risk, executor, None);
        let event = PumpApiEvent {
            signature: Some("replay-signature-1".to_string()),
            mint: Some("B8aCJxpe8MwVTqkBqnc8j21sYGKtFADMczagRHDLpump".to_string()),
            pool: Some("pump".to_string()),
            pool_id: None,
            quote_mint: None,
            tx_type: Some("buy".to_string()),
            signer: Some(signer.to_string()),
            sol_in_pool: Some(20.0),
            tokens_in_pool: Some(1_000_000.0),
            quote_in_pool: None,
            event_timestamp_ms: Some(1_780_000_000_000),
            received_unix_ms: 1_780_000_000_020,
            raw_json: r#"{"quoteAmount":0.5,"marketCapQuote":50000}"#.to_string(),
        };
        let source_event_id = db.insert_pumpapi_event(&event).await.unwrap();
        engine.observe_event(source_event_id, &event).await;

        let signals = db.list_recent_signals(user.id, 10).await.unwrap();
        let orders = db.list_recent_orders(user.id, 10).await.unwrap();
        let positions = db.list_open_dry_run_positions(user.id, 10).await.unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].status, "dry_run");
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].status, "dry_run");
        assert_eq!(orders[0].amount_sol, Some(0.01));
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].amount_sol, 0.01);
    }

    #[tokio::test]
    async fn fast_dispatch_uses_cached_strategy_and_deduplicates_event() {
        let config = test_config();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(2, Some("fast-test")).await.unwrap();
        db.insert_external_wallet(user.id, "wallet-fast", "Fast", "pubkey-fast")
            .await
            .unwrap();
        let signer = "VJSDW6S74YXR4rRR9P4xwhMvLZJQMhrUb8XMFirUsy1";
        db.add_watch_wallet(user.id, signer, "Fast Replay")
            .await
            .unwrap();
        let trade_client = TradeServiceClient::new(
            config.trade_service_url.clone(),
            None,
            config.trade_service_timeout_secs,
        )
        .unwrap()
        .with_submission_enabled(true);
        let risk = RiskEngine::new();
        let executor = ExecutionRouter::new(db.clone(), risk.clone(), trade_client, 1, 1);
        let engine = StrategyEngine::new(config, db.clone(), risk, executor, None);
        assert_eq!(engine.refresh_cache().await.unwrap(), 1);
        let event = PumpApiEvent {
            signature: Some("fast-replay-signature".to_string()),
            mint: Some("B8aCJxpe8MwVTqkBqnc8j21sYGKtFADMczagRHDLpump".to_string()),
            pool: Some("pump".to_string()),
            pool_id: None,
            quote_mint: Some(WSOL.to_string()),
            tx_type: Some("buy".to_string()),
            signer: Some(signer.to_string()),
            sol_in_pool: Some(20.0),
            tokens_in_pool: Some(1_000_000.0),
            quote_in_pool: None,
            event_timestamp_ms: Some(1_780_000_000_000),
            received_unix_ms: 1_780_000_000_002,
            raw_json: r#"{"quoteAmount":0.5,"marketCapQuote":50000}"#.to_string(),
        };
        assert!(engine.dispatch_event(&tokio::runtime::Handle::current(), event.clone()));
        assert!(engine.dispatch_event(&tokio::runtime::Handle::current(), event));

        for _ in 0..50 {
            if !db
                .list_recent_signals(user.id, 10)
                .await
                .unwrap()
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let signals = db.list_recent_signals(user.id, 10).await.unwrap();
        let orders = db.list_recent_orders(user.id, 10).await.unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].status, "dry_run");
    }
}
