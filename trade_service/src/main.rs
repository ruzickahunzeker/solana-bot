mod adapters;
mod aggregator_quote;
mod api;
mod application;
mod auth;
mod blockhash_cache;
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
use solana_commitment_config::CommitmentConfig;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::adapters::aggregator::quotes::AggregatorQuoteSource;
use crate::adapters::persistence::trade_reads::StoredTradeReadSource;
use crate::adapters::persistence::wallet_metadata::RepositoryWalletMetadata;
use crate::adapters::readiness::RuntimeReadinessSource;
use crate::adapters::sdk::address_parser::SdkAddressParser;
use crate::adapters::sdk::ordinary_execution::SdkOrdinaryExecutionBackend;
use crate::adapters::sdk::ordinary_sol_buy::SdkOrdinarySolBuyBackend;
use crate::adapters::sdk::pool_queries::SdkPoolReadBackend;
use crate::adapters::sdk::quote_preflight::SdkQuotePreflightBackend;
use crate::adapters::sdk::quote_reconciliation::SdkQuoteReconciliationRuntime;
use crate::adapters::sdk::quote_submit::SdkQuoteSubmissionRuntime;
use crate::adapters::sdk::sol_transfer::SdkSolTransferRuntime;
use crate::adapters::sdk::wallet_balances::SdkWalletBalanceRuntime;
use crate::adapters::sdk::wallet_identity::SdkWalletIdentityDirectory;
use crate::adapters::sdk::wallet_management::SdkWalletManagement;
use crate::aggregator_quote::QuoteStore;
use crate::application::pool_queries::{PoolQueries, PoolQueryService};
use crate::application::ports::ordinary_execution::OrdinaryExecutionBackend;
use crate::application::ports::ordinary_sol_buy::OrdinarySolBuyBackend;
use crate::application::ports::trade_reads::TradeRecordReadSource;
use crate::application::ports::wallet_identity::LoadedWalletDirectory;
use crate::application::quote_preflight::{QuotePreflightService, QuotePreflights};
use crate::application::quote_reconciliation::{QuoteReconciliationService, QuoteReconciliations};
use crate::application::readiness::{ReadinessQuery, ReadinessService};
use crate::application::sol_buy_quotes::{SolBuyQuoteQueries, SolBuyQuoteService};
use crate::application::sol_buy_submit::{SolBuySubmissionService, SolBuySubmissions};
use crate::application::sol_transfers::{SolTransferService, SolTransfers};
use crate::application::trade_queries::TradeQueryService;
use crate::application::vanity::{VanityService, VanityTasks};
use crate::application::wallet_balances::{WalletBalanceQueries, WalletBalanceService};
use crate::application::wallet_management::{WalletManagement, WalletManagementService};
use crate::blockhash_cache::BlockhashCache;
use crate::conversion::ConversionStore;
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
    // Fixture handles only; production services own their original runtime resources.
    #[cfg(test)]
    pub wallets: Arc<WalletRegistry>,
    #[cfg(test)]
    pub wallet_repository: Arc<WalletRepository>,
    pub blockhash_cache: Arc<BlockhashCache>,
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
    anyhow::ensure!(
        !config.ordinary_observation.enabled
            || config.trade_persistence.mode == config::TradePersistenceMode::Durable,
        "ordinary observation worker requires durable persistence"
    );
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
            config.security.broadcast,
        )
        .await
        .context("initialize wallets")?,
    );
    let read_rpc = Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(
        &config.rpc_urls,
    )?);
    let blockhash_cache = Arc::new(BlockhashCache::default());
    let blockhash_refresher = blockhash_cache.clone().spawn_refresher(
        crate::adapters::rpc::blockhash::RpcBlockhashReader::new(read_rpc.clone()),
        config.blockhash_refresh_ms,
    );

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
    let conversions = if config.conversion_quotes_enabled {
        Some(Arc::new(
            ConversionStore::new(&config.conversion_database_url)
                .await
                .context("initialize conversion database")?,
        ))
    } else {
        None
    };
    let readiness = Arc::new(ReadinessService::new(RuntimeReadinessSource::new(
        config.clone(),
        wallets.clone(),
        blockhash_cache.clone(),
        persistence.clone(),
        read_rpc.clone(),
    )));
    let params = Arc::new(ParamCache::default());
    let pool_queries = Arc::new(PoolQueryService::new(
        SdkPoolReadBackend::new(wallets.clone()),
        params.clone(),
    ));
    let ordinary_execution = Arc::new(SdkOrdinaryExecutionBackend::new(
        wallets.clone(),
        config.pumpapi.clone(),
        read_rpc.clone(),
        persistence.clone(),
    ));
    let wallet_identity = Arc::new(SdkWalletIdentityDirectory::new(wallets.clone()));
    let aggregator_quotes = Arc::new(QuoteStore::default());
    let observation_plan = crate::adapters::ordinary_observation_worker::ObservationPlan::prepare(
        &config.ordinary_observation,
        persistence.as_ref(),
        read_rpc.clone(),
    )?;
    let mut ordinary_sol_buy = SdkOrdinarySolBuyBackend::new(
        wallets.clone(),
        aggregator_quotes.clone(),
        persistence.clone(),
    );
    if let Some(plan) = &observation_plan {
        ordinary_sol_buy = ordinary_sol_buy.with_observation_reader(plan.reader());
    }
    let ordinary_sol_buy = Arc::new(ordinary_sol_buy);
    let quote_queries = Arc::new(SolBuyQuoteService::new(
        AggregatorQuoteSource,
        wallet_identity.clone(),
        aggregator_quotes.clone(),
    ));
    let quote_preflights = Arc::new(QuotePreflightService::new(
        Arc::new(SdkQuotePreflightBackend::new(wallets.clone())),
        aggregator_quotes.clone(),
    ));
    let quote_submissions = Arc::new(SolBuySubmissionService::new(
        SdkQuoteSubmissionRuntime::new(wallets.clone(), persistence.clone()),
        aggregator_quotes.clone(),
    ));
    let quote_reconciliations = Arc::new(QuoteReconciliationService::new(
        SdkQuoteReconciliationRuntime::new(wallets.clone(), persistence.clone()),
    ));
    let sol_transfers = Arc::new(SolTransferService::new(
        SdkSolTransferRuntime::new(wallets.clone(), wallet_repository.clone()),
        SdkAddressParser,
    ));
    let wallet_management = SdkWalletManagement::new(wallet_repository.clone(), wallets.clone());
    let state = AppState {
        vanity_tasks: Arc::new(VanityService::new(wallet_ops)),
        wallet_management: Arc::new(WalletManagementService::new(
            wallet_management.clone(),
            wallet_management,
        )),
        wallet_balance_queries: Arc::new(WalletBalanceService::new(
            RepositoryWalletMetadata::new(wallet_repository.clone()),
            SdkWalletBalanceRuntime::new(wallets.clone(), read_rpc.clone()),
            SdkAddressParser,
            crate::state::now_ms,
        )),
        wsol_queries: Arc::new(crate::application::wsol_queries::WsolQueryService::new(
            crate::adapters::sdk::wsol_queries::SdkWsolQuerySource::new(
                wallets.clone(),
                wallet_repository.clone(),
            ),
            conversions
                .clone()
                .map(crate::adapters::persistence::wsol_conversion::SqliteWsolJournal::new),
            SdkAddressParser,
        )),
        wsol_submissions: Arc::new(
            crate::application::wsol_submission::WsolSubmissionService::new(
                crate::adapters::sdk::wsol_conversion::SdkWsolRuntime::new(
                    wallets.clone(),
                    wallet_repository.clone(),
                ),
                conversions
                    .clone()
                    .map(crate::adapters::persistence::wsol_conversion::SqliteWsolJournal::new),
            ),
        ),
        config: config.clone(),
        #[cfg(test)]
        wallets: wallets.clone(),
        #[cfg(test)]
        wallet_repository: wallet_repository.clone(),
        blockhash_cache,
        #[cfg(test)]
        params: params.clone(),
        param_cache_writer: params,
        trades: Arc::new(TradeStore::default()),
        idempotency: Arc::new(IdempotencyStore::default()),
        persistence,
        #[cfg(test)]
        aggregator_quotes,
        #[cfg(test)]
        conversions,
        readiness,
        pool_queries,
        ordinary_execution,
        ordinary_sol_buy,
        quote_queries,
        quote_preflights,
        quote_submissions,
        quote_reconciliations,
        sol_transfers,
        wallet_identity,
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
        broadcast_disabled = config.security.broadcast == crate::domain::execution_policy::BroadcastPolicy::Disabled,
        wallet_admin_auth_mode = config.security.wallet_admin.mode_name(),
        "trade service listening"
    );
    let observation_worker = observation_plan.map(|plan| plan.start());
    let observation_stop = observation_worker
        .as_ref()
        .map(|worker| worker.stop_signal());
    let blockhash_stop = blockhash_refresher.abort_handle();
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        blockhash_stop.abort();
        if let Some(stop) = observation_stop {
            stop.request();
        }
    })
    .await;
    if let Some(worker) = observation_worker {
        worker.shutdown().await?;
    }
    blockhash_refresher.shutdown().await?;
    result?;
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
