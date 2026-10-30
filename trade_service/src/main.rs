mod aggregator_quote;
mod auth;
mod blockhash_cache;
mod config;
mod executor;
mod http;
mod idempotency;
mod models;
mod param_cache;
mod persistence;
mod pumpapi_client;
mod sanitize;
mod sdk_client;
mod state;
mod validation;
mod wallet_crypto;
mod wallet_ops;
mod wallet_repository;
mod wallets;

use anyhow::Context;
use axum::Router;
use config::Config;
use solana_commitment_config::CommitmentConfig;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::aggregator_quote::QuoteStore;
use crate::blockhash_cache::BlockhashCache;
use crate::idempotency::IdempotencyStore;
use crate::param_cache::ParamCache;
use crate::persistence::TradePersistence;
use crate::state::TradeStore;
use crate::wallet_ops::WalletOps;
use crate::wallet_repository::WalletRepository;
use crate::wallets::WalletRegistry;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub wallets: Arc<WalletRegistry>,
    pub wallet_repository: Arc<WalletRepository>,
    pub wallet_ops: Arc<WalletOps>,
    pub blockhash_cache: Arc<BlockhashCache>,
    pub params: Arc<ParamCache>,
    pub trades: Arc<TradeStore>,
    pub idempotency: Arc<IdempotencyStore>,
    pub persistence: Arc<TradePersistence>,
    pub aggregator_quotes: Arc<QuoteStore>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,pumpswap_global_config=error")
            }),
        )
        .init();

    load_dotenv();
    let config = Arc::new(Config::from_env()?);
    let wallet_repository = Arc::new(
        WalletRepository::new(
            &config.wallet_storage,
            &config.wallets,
            &config.default_wallet_id,
        )
        .await
        .context("initialize wallet repository")?,
    );
    let runtime_wallets = wallet_repository
        .load_wallet_configs(&config.wallets)
        .await
        .context("load runtime wallets")?;
    let wallets = Arc::new(
        WalletRegistry::new(
            &runtime_wallets,
            config.default_wallet_id.clone(),
            config.rpc_urls.clone(),
            config.mev.clone(),
            CommitmentConfig::processed(),
            config.enable_sdk_log,
        )
        .await
        .context("initialize wallets")?,
    );
    let blockhash_cache = Arc::new(BlockhashCache::new(wallets.rpcs()));
    blockhash_cache
        .clone()
        .spawn_refresher(config.blockhash_refresh_ms);

    let persistence = Arc::new(
        TradePersistence::new(&config.trade_persistence)
            .await
            .context("initialize trade persistence")?,
    );
    let wallet_ops = Arc::new(
        WalletOps::new(wallet_repository.clone(), wallets.clone())
            .await
            .context("initialize wallet operations")?,
    );
    let state = AppState {
        config: config.clone(),
        wallets: wallets.clone(),
        wallet_repository: wallet_repository.clone(),
        wallet_ops,
        blockhash_cache,
        params: Arc::new(ParamCache::default()),
        trades: Arc::new(TradeStore::default()),
        idempotency: Arc::new(IdempotencyStore::default()),
        persistence,
        aggregator_quotes: Arc::new(QuoteStore::default()),
    };

    let app: Router = http::router(state).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(config.trade_http_addr)
        .await
        .with_context(|| format!("bind {}", config.trade_http_addr))?;

    info!(
        addr = %config.trade_http_addr,
        wallet = %wallets.default_pubkey(),
        wallet_count = wallets.wallet_count(),
        wallet_database_mode = wallet_repository.database_enabled(),
        rpc_lanes = wallets.default().rpc_count(),
        swqos_lanes = ?wallets.default().swqos_lanes(),
        active_swqos_lanes = ?wallets.default().active_swqos_lanes(),
        require_mev_submit = wallets.default().require_mev_submit(),
        trade_persistence_mode = ?config.trade_persistence.mode,
        "trade service listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

fn load_dotenv() {
    if dotenvy::dotenv().is_ok() {
        return;
    }

    if let Some(path) = env_file_from_exe() {
        if dotenvy::from_path(&path).is_err() {
            warn!(path = %path.display(), "failed to load .env near executable");
        }
    }
}

fn env_file_from_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    find_env_in_ancestors(exe.parent()?)
}

fn find_env_in_ancestors(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join(".env");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
