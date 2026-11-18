use crate::{
    adapters::persistence::ordinary_claims::RuntimeExecutionClaims,
    application::{
        ordinary_runtime::OrdinaryExecutionService,
        ports::ordinary_claims::OrdinaryExecutionClaimer, trade_admission::AdmittedTrade,
    },
    AppState,
};

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
    execute_with_claims(
        state,
        &AdmittedTrade::unreserved_fixture(&record, command),
        &UnreservedFixtureClaims,
    )
    .await;
}

pub(crate) async fn execute_admitted_trade(state: AppState, execution: AdmittedTrade) {
    let claims = RuntimeExecutionClaims::new(
        state.persistence.clone(),
        state.trades.clone(),
        state.idempotency.clone(),
    );
    execute_with_claims(state, &execution, &claims).await;
}

async fn execute_with_claims<C: OrdinaryExecutionClaimer>(
    state: AppState,
    execution: &AdmittedTrade,
    claims: &C,
) {
    OrdinaryExecutionService::new(
        state.persistence.as_ref(),
        state.trades.as_ref(),
        state.blockhash_cache.as_ref(),
        state.ordinary_execution.as_ref(),
        state.ordinary_sol_buy.as_ref(),
        state.param_cache_writer.as_ref(),
        state.request_settings.execution(),
    )
    .execute(execution, claims)
    .await;
}
