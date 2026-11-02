mod alpha;
mod alpha_shadow;
mod bot;
mod config;
mod crypto;
mod db;
mod execution_params;
mod executor;
mod limit_order;
mod order;
mod pool_route;
mod preflight;
mod price_oracle;
mod retention;
mod risk;
mod strategy;
mod stream;
mod trade_client;
mod wallet;

use anyhow::Result;
use bot::BotState;
use config::AppConfig;
use crypto::SecretBox;
use db::Database;
use executor::ExecutionRouter;
use limit_order::LimitOrderEngine;
use order::OrderReconciler;
use pool_route::PoolRouteCache;
use price_oracle::PriceOracle;
use risk::RiskEngine;
use strategy::StrategyEngine;
use teloxide::Bot;
use tracing::{info, warn};
use trade_client::TradeServiceClient;
use wallet::WalletService;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("copy_trading=info")),
        )
        .init();

    let config = AppConfig::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;
    if config.alpha_retention_enabled {
        retention::start_alpha_retention(
            db.clone(),
            retention::AlphaRetentionConfig {
                interval_secs: config.alpha_retention_interval_secs,
                observation_retention_hours: config.alpha_observation_retention_hours,
                paper_signal_retention_days: config.alpha_paper_signal_retention_days,
                shadow_order_retention_days: config.alpha_shadow_order_retention_days,
            },
        );
        info!(
            interval_secs = config.alpha_retention_interval_secs,
            observation_retention_hours = config.alpha_observation_retention_hours,
            paper_signal_retention_days = config.alpha_paper_signal_retention_days,
            shadow_order_retention_days = config.alpha_shadow_order_retention_days,
            "alpha retention enabled"
        );
    }

    let secret_box = SecretBox::from_hex(&config.master_key_hex)?;
    let trade_client = TradeServiceClient::new(
        config.trade_service_url.clone(),
        config.trade_service_token.clone(),
        config.trade_service_timeout_secs,
    )?;
    let wallet_service = WalletService::new(db.clone(), secret_box, trade_client.clone());
    let pool_routes = PoolRouteCache::new(
        db.clone(),
        trade_client.clone(),
        config.pool_resolve_timeout_secs,
    );
    if config.migrate_local_wallets_to_trade_service {
        let migrated = wallet_service.migrate_legacy_wallets().await?;
        info!(
            migrated,
            "local wallets migrated to trade_service wallet database"
        );
    }
    let bot = Bot::new(config.telegram_bot_token.clone());
    let price_oracle = PriceOracle::new(
        config.sol_usd_price,
        config.sol_usd_price_api_url.clone(),
        config.sol_usd_price_api_key.clone(),
        config.sol_usd_refresh_secs,
    );
    price_oracle.clone().start();
    let risk = RiskEngine::new();
    let executor = ExecutionRouter::new(
        db.clone(),
        risk.clone(),
        trade_client.clone(),
        pool_routes.clone(),
        config.trade_submit_max_attempts,
        config.trade_submit_retry_delay_ms,
    );
    OrderReconciler::new(
        db.clone(),
        trade_client.clone(),
        config.trade_reconcile_interval_secs,
        config.trade_submit_max_attempts,
    )
    .start();
    alpha_shadow::start_reconciler(alpha_shadow::AlphaShadowExecutor::new(
        db.clone(),
        trade_client.clone(),
    ));
    let limit_orders = LimitOrderEngine::new(
        db.clone(),
        executor.clone(),
        Some(bot.clone()),
        price_oracle.clone(),
    );
    let strategy = StrategyEngine::new(
        config.clone(),
        db.clone(),
        risk,
        executor,
        Some(bot.clone()),
    );
    let cached_strategies = strategy.refresh_cache().await?;
    info!(cached_strategies, "fast follow strategy cache initialized");
    strategy.start_cache_refresh();

    match trade_client.health().await {
        Ok(body) => info!(body, "trade_service health ok"),
        Err(err) => warn!(
            ?err,
            "trade_service health unavailable; bot can still manage wallets"
        ),
    }

    if config.enable_stream_consumer {
        stream::spawn_zmq_consumer(
            config.pumpapi_zmq_endpoint.clone(),
            db.clone(),
            strategy.clone(),
            limit_orders,
            pool_routes.clone(),
            config.fast_follow_limit_queue_capacity,
            trade_client.clone(),
        );
    }

    if config.enable_telegram_bot {
        bot::run_bot(
            bot,
            BotState::new(
                config,
                db,
                wallet_service,
                trade_client,
                price_oracle,
                strategy,
                pool_routes,
            ),
        )
        .await?;
    } else {
        info!("telegram bot disabled; waiting for ctrl-c");
        tokio::signal::ctrl_c().await?;
    }

    Ok(())
}
