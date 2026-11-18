use crate::{
    adapters::persistence::ordinary_claims::RuntimeExecutionClaims,
    application::{
        ordinary_runtime::OrdinaryExecutionService,
        ports::{
            blockhash::CachedBlockhashRead, ordinary_claims::OrdinaryExecutionClaimer,
            ordinary_execution::OrdinaryExecutionBackend, ordinary_sol_buy::OrdinarySolBuyBackend,
            pool_queries::ParamCacheWriter,
        },
        trade_admission::AdmittedTrade,
        trade_settings::OrdinaryExecutionSettings,
    },
    persistence::TradePersistence,
    state::TradeStore,
    AppState,
};
use std::{future::Future, sync::Arc};

#[cfg(test)]
use crate::{
    application::{
        ordinary_runtime::{map_status, persist_before_execution, persist_update},
        ports::{
            ordinary_claims::{ExecutionClaim, ExecutionOwner},
            ordinary_execution::SdkExecutionContext,
            ordinary_journal::{OrdinaryTradeJournal, TradeJournalMode},
        },
        trade_command::TradeCommand,
        trade_execution::TradeExecutionResult as ExecuteResult,
        trade_record::TradeRecordV1 as TradeRecord,
    },
    domain::{
        route::PoolRouteHint,
        trade::{ExecutionMethod, TradeStatus},
    },
    state::cache_key,
};

#[cfg(test)]
mod claim_tests;
#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod execution_tests;
#[cfg(test)]
mod journal_tests;
#[cfg(test)]
mod sdk_journal_tests;

#[cfg(test)]
pub(crate) async fn execute_trade(state: AppState, request_id: String, command: TradeCommand) {
    let Some(record) = state.trades.get(&request_id) else {
        return;
    };
    struct UnreservedFixtureClaims;
    impl OrdinaryExecutionClaimer for UnreservedFixtureClaims {
        async fn claim(&self, execution: &AdmittedTrade) -> anyhow::Result<ExecutionClaim> {
            Ok(ExecutionClaim::Acquired(ExecutionOwner::fixture(execution)))
        }
    }
    ExecutionResources::from_state(&state)
        .execute(
            &AdmittedTrade::unreserved_fixture(&record, command),
            &UnreservedFixtureClaims,
        )
        .await;
}

/// The dispatched task owns only its execution resources, not the HTTP state.
pub(crate) fn execute_admitted_trade(
    state: &AppState,
    execution: AdmittedTrade,
) -> impl Future<Output = ()> + Send + 'static + use<> {
    let resources = ExecutionResources::from_state(state);
    let claims = RuntimeExecutionClaims::new(
        resources.persistence.clone(),
        resources.trades.clone(),
        state.idempotency.clone(),
    );
    async move { resources.execute(&execution, &claims).await }
}

struct ExecutionResources {
    persistence: Arc<TradePersistence>,
    trades: Arc<TradeStore>,
    blockhash: Arc<dyn CachedBlockhashRead>,
    ordinary: Arc<dyn OrdinaryExecutionBackend>,
    sol_buy: Arc<dyn OrdinarySolBuyBackend>,
    params: Arc<dyn ParamCacheWriter>,
    settings: OrdinaryExecutionSettings,
}

impl ExecutionResources {
    fn from_state(state: &AppState) -> Self {
        Self {
            persistence: state.persistence.clone(),
            trades: state.trades.clone(),
            blockhash: state.cached_blockhash.clone(),
            ordinary: state.ordinary_execution.clone(),
            sol_buy: state.ordinary_sol_buy.clone(),
            params: state.param_cache_writer.clone(),
            settings: state.request_settings.execution(),
        }
    }

    async fn execute<C: OrdinaryExecutionClaimer>(&self, execution: &AdmittedTrade, claims: &C) {
        OrdinaryExecutionService::new(
            self.persistence.as_ref(),
            self.trades.as_ref(),
            self.blockhash.as_ref(),
            self.ordinary.as_ref(),
            self.sol_buy.as_ref(),
            self.params.as_ref(),
            self.settings,
        )
        .execute(execution, claims)
        .await;
    }
}
