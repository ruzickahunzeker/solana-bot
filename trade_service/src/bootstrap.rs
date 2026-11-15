use anyhow::Context;
use solana_commitment_config::CommitmentConfig;
use std::sync::Arc;

use crate::{
    adapters::{
        aggregator::quotes::AggregatorQuoteSource,
        ordinary_observation_worker::ObservationPlan,
        persistence::wallet_metadata::RepositoryWalletMetadata,
        readiness::RuntimeReadinessSource,
        rpc::read_runtime::SharedReadRuntime,
        sdk::{
            address_parser::SdkAddressParser, ordinary_execution::SdkOrdinaryExecutionBackend,
            ordinary_sol_buy::SdkOrdinarySolBuyBackend, pool_queries::SdkPoolReadBackend,
            quote_preflight::SdkQuotePreflightBackend,
            quote_reconciliation::SdkQuoteReconciliationRuntime,
            quote_submit::SdkQuoteSubmissionRuntime, sol_transfer::SdkSolTransferRuntime,
            wallet_balances::SdkWalletBalanceRuntime, wallet_identity::SdkWalletIdentityDirectory,
            wallet_management::SdkWalletManagement,
        },
    },
    aggregator_quote::QuoteStore,
    application::{
        pool_queries::PoolQueryService, quote_preflight::QuotePreflightService,
        quote_reconciliation::QuoteReconciliationService, readiness::ReadinessService,
        sol_buy_quotes::SolBuyQuoteService, sol_buy_submit::SolBuySubmissionService,
        sol_transfers::SolTransferService, vanity::VanityService,
        wallet_balances::WalletBalanceService, wallet_management::WalletManagementService,
    },
    blockhash_cache::{BlockhashCache, BlockhashRefresher},
    config::{self, Config},
    conversion::ConversionStore,
    idempotency::IdempotencyStore,
    param_cache::ParamCache,
    persistence::TradePersistence,
    state::TradeStore,
    wallet_ops::WalletOps,
    wallet_repository::WalletRepository,
    wallets::WalletRegistry,
    AppState,
};

pub(crate) struct Bootstrapped {
    pub(crate) state: AppState,
    pub(crate) wallets: Arc<WalletRegistry>,
    pub(crate) wallet_repository: Arc<WalletRepository>,
    pub(crate) blockhash_refresher: BlockhashRefresher,
    pub(crate) observation_plan: Option<ObservationPlan>,
}

struct Resources {
    config: Arc<Config>,
    wallets: Arc<WalletRegistry>,
    wallet_repository: Arc<WalletRepository>,
    read_rpc: Arc<SharedReadRuntime>,
    blockhash_cache: Arc<BlockhashCache>,
    persistence: Arc<TradePersistence>,
    wallet_ops: Arc<WalletOps>,
    conversions: Option<Arc<ConversionStore>>,
}

pub(crate) async fn initialize(config: Arc<Config>) -> anyhow::Result<Bootstrapped> {
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
    complete(
        Resources {
            config,
            wallets,
            wallet_repository,
            read_rpc,
            blockhash_cache,
            persistence,
            wallet_ops,
            conversions,
        },
        blockhash_refresher,
    )
}

// Only assembles services around existing resources; it does not load wallets or start workers.
fn complete(
    resources: Resources,
    blockhash_refresher: BlockhashRefresher,
) -> anyhow::Result<Bootstrapped> {
    let Resources {
        config,
        wallets,
        wallet_repository,
        read_rpc,
        blockhash_cache,
        persistence,
        wallet_ops,
        conversions,
    } = resources;
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
    Ok(Bootstrapped {
        state,
        wallets,
        wallet_repository,
        blockhash_refresher,
        observation_plan,
    })
}

#[cfg(test)]
mod tests;
