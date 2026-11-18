mod adapters;
mod aggregator_quote;
mod api;
mod application;
mod auth;
mod blockhash_cache;
mod bootstrap;
mod config;
#[cfg(test)]
mod contract_tests;
mod conversion;
mod domain;
mod executor;
mod http;
#[cfg(test)]
mod http_contract_tests;
mod idempotency;
mod infrastructure;
mod models;
mod param_cache;
mod persistence;
mod protocol;
mod protocol_capabilities;
#[cfg(test)]
mod protocol_contract_tests;
mod pumpapi_client;
mod sanitize;
mod sdk_client;
mod state;
#[cfg(test)]
mod trade_contract_tests;
mod validation;
mod wallet_crypto;
mod wallet_ops;
mod wallet_repository;
mod wallets;

use anyhow::Context;
use axum::Router;
use config::Config;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::adapters::persistence::trade_reads::StoredTradeReadSource;
#[cfg(test)]
use crate::aggregator_quote::QuoteStore;
use crate::application::pool_queries::PoolQueries;
use crate::application::ports::blockhash::CachedBlockhashRead;
use crate::application::ports::ordinary_execution::OrdinaryExecutionBackend;
use crate::application::ports::ordinary_sol_buy::OrdinarySolBuyBackend;
use crate::application::ports::trade_reads::TradeRecordReadSource;
use crate::application::ports::wallet_identity::LoadedWalletDirectory;
use crate::application::quote_preflight::QuotePreflights;
use crate::application::quote_reconciliation::QuoteReconciliations;
use crate::application::readiness::ReadinessQuery;
use crate::application::sol_buy_quotes::SolBuyQuoteQueries;
use crate::application::sol_buy_submit::SolBuySubmissions;
use crate::application::sol_transfers::SolTransfers;
use crate::application::trade_queries::TradeQueryService;
use crate::application::vanity::VanityTasks;
use crate::application::wallet_balances::WalletBalanceQueries;
use crate::application::wallet_management::WalletManagement;
#[cfg(test)]
use crate::blockhash_cache::BlockhashCache;
#[cfg(test)]
use crate::conversion::ConversionStore;
use crate::idempotency::IdempotencyStore;
#[cfg(test)]
use crate::param_cache::ParamCache;
use crate::persistence::TradePersistence;
use crate::state::TradeStore;
#[cfg(test)]
use crate::wallet_ops::WalletOps;
#[cfg(test)]
use crate::wallet_repository::WalletRepository;
#[cfg(test)]
use crate::wallets::WalletRegistry;

#[derive(Clone)]
pub struct AppState {
    pub(crate) trade_tasks: crate::infrastructure::trade_tasks::TradeTasks,
    pub(crate) request_settings: Arc<config::request_settings::RequestSettings>,
    #[cfg(test)]
    pub config: Arc<Config>,
    // Fixture handles only; production services own their original runtime resources.
    #[cfg(test)]
    pub wallets: Arc<WalletRegistry>,
    #[cfg(test)]
    pub wallet_repository: Arc<WalletRepository>,
    pub(crate) cached_blockhash: Arc<dyn CachedBlockhashRead>,
    #[cfg(test)]
    pub params: Arc<ParamCache>,
    pub(crate) param_cache_writer:
        Arc<dyn crate::application::ports::pool_queries::ParamCacheWriter>,
    pub trades: Arc<TradeStore>,
    pub idempotency: Arc<IdempotencyStore>,
    pub persistence: Arc<TradePersistence>,
    #[cfg(test)]
    pub aggregator_quotes: Arc<QuoteStore>,
    #[cfg(test)]
    pub conversions: Option<Arc<ConversionStore>>,
    pub readiness: Arc<dyn ReadinessQuery>,
    pub pool_queries: Arc<dyn PoolQueries>,
    pub(crate) ordinary_execution: Arc<dyn OrdinaryExecutionBackend>,
    pub(crate) ordinary_sol_buy: Arc<dyn OrdinarySolBuyBackend>,
    pub(crate) quote_queries: Arc<dyn SolBuyQuoteQueries>,
    pub(crate) quote_preflights: Arc<dyn QuotePreflights>,
    pub(crate) quote_submissions: Arc<dyn SolBuySubmissions>,
    pub(crate) quote_reconciliations: Arc<dyn QuoteReconciliations>,
    pub(crate) sol_transfers: Arc<dyn SolTransfers>,
    pub(crate) wsol_submissions: Arc<dyn crate::application::wsol_submission::WsolSubmissions>,
    pub(crate) wsol_queries: Arc<dyn crate::application::wsol_queries::WsolQueries>,
    pub(crate) wallet_identity: Arc<dyn LoadedWalletDirectory>,
    pub(crate) wallet_balance_queries: Arc<dyn WalletBalanceQueries>,
    pub(crate) wallet_management: Arc<dyn WalletManagement>,
    pub(crate) vanity_tasks: Arc<dyn VanityTasks>,
}

impl AppState {
    pub(crate) fn trade_intake(
        &self,
    ) -> crate::application::trade_intake::TradeIntakeService<
        '_,
        TradePersistence,
        TradeStore,
        IdempotencyStore,
    > {
        crate::application::trade_intake::TradeIntakeService::new(
            self.persistence.as_ref(),
            self.trades.as_ref(),
            self.idempotency.as_ref(),
            self.wallet_identity.as_ref(),
            &crate::adapters::sdk::address_parser::SdkAddressParser,
            self.request_settings.intake(),
            crate::state::new_request_id,
            crate::state::now_ms,
        )
    }

    pub(crate) fn trade_queries(&self) -> TradeQueryService<impl TradeRecordReadSource + '_> {
        TradeQueryService::new(StoredTradeReadSource::new(
            self.persistence.as_ref(),
            self.trades.as_ref(),
        ))
    }
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
    let bootstrap::Bootstrapped {
        state,
        ordinary_shutdown,
        wallets,
        wallet_repository,
        blockhash_refresher,
        observation_plan,
        vanity_worker,
    } = bootstrap::initialize(config.clone()).await?;

    let intake_stop = ordinary_shutdown.stop_intake();
    let vanity_stop = vanity_worker.clone();
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
        broadcast_disabled = config.security.broadcast == crate::domain::execution_policy::BroadcastPolicy::Disabled,
        wallet_admin_auth_mode = config.security.wallet_admin.mode_name(),
        "trade service listening"
    );
    let shutdown = config.shutdown;
    drop(config);
    let observation_worker = observation_plan.map(|plan| plan.start());
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        intake_stop();
        vanity_stop.request_stop();
    })
    .await;
    let (cleanup, vanity_cleanup) = tokio::join!(
        ordinary_shutdown.finish(
            std::time::Duration::from_millis(shutdown.trade_task_drain_timeout_ms),
            blockhash_refresher,
            observation_worker,
            std::time::Duration::from_millis(shutdown.shadow_drain_timeout_ms),
        ),
        vanity_worker.shutdown(std::time::Duration::from_millis(
            shutdown.trade_task_drain_timeout_ms
        )),
    );
    result?;
    cleanup?;
    vanity_cleanup?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            }
            Err(error) => {
                warn!(%error, "failed to install terminate handler; retaining Ctrl-C shutdown");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
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
