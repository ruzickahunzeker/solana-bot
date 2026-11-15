use tracing::{error, info};

use crate::adapters::persistence::ordinary_claims::RuntimeExecutionClaims;
use crate::application::ports::ordinary_claims::ExecutionOwner;
use crate::application::ports::ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer};
use crate::application::ports::ordinary_execution::SdkExecutionContext;
use crate::application::ports::ordinary_journal::{OrdinaryTradeJournal, TradeJournalMode};
use crate::application::ports::ordinary_sol_buy::{JournaledSolBuyOutcome, SolBuyExecutionOutcome};
use crate::application::trade_execution::{classify_result, TradeExecutionResult as ExecuteResult};
use crate::application::trade_record::TradeRecordV1 as TradeRecord;
use crate::application::{trade_admission::AdmittedTrade, trade_command::TradeCommand};
use crate::domain::trade::{ExecutionMethod, TradeStatus};
use crate::state::{cache_key, describe_trade};
use crate::AppState;

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
    let reservation = execution.reservation();
    let request_id = reservation.request_id().to_owned();
    let command = execution.command();
    let mut owner = if command.aggregator_sol && !command.execution.mode.simulate() {
        match claims.claim(execution).await {
            Ok(ExecutionClaim::Acquired(owner)) => Some(owner),
            Ok(ExecutionClaim::AlreadyHeld) => return,
            Err(error) => {
                error!(request_id = %request_id, error = %crate::sanitize::redact_urls(&error.to_string()), "ordinary execution claim unavailable or inconsistent; execution stopped without retry");
                return;
            }
        }
    } else {
        None
    };
    if let Err(error) = state.config.security.broadcast.ensure_trade_allowed(
        command.execution.method == ExecutionMethod::Sdk && !command.aggregator_sol,
        command.execution.mode.simulate(),
    ) {
        let record = state.trades.update(
            &request_id,
            TradeStatus::Rejected,
            None,
            Some(error.to_string()),
        );
        persist_update(state.persistence.as_ref(), record.as_ref()).await;
        return;
    }
    if matches!(
        crate::application::ordinary_claims::operation_for_claim(execution, reservation.mode()),
        Ok(crate::application::ordinary_claims::ClaimOperation::SdkTrade)
    ) {
        match claims.claim(execution).await {
            Ok(ExecutionClaim::Acquired(acquired)) => owner = Some(acquired),
            Ok(ExecutionClaim::AlreadyHeld) => return,
            Err(error) => {
                error!(request_id = %request_id, error = %crate::sanitize::redact_urls(&error.to_string()), "original SDK execution claim unavailable; no execution or retry");
                return;
            }
        }
    }
    let idempotency_key = reservation.idempotency_key().to_owned();
    let wallet_id = reservation.wallet_id().to_owned();
    let detail = describe_trade(
        command.amount.side(),
        command.amount.buy_units(),
        command.amount.sell_percent(),
    );
    info!(
        request_id = %request_id,
        wallet_id = %wallet_id,
        idempotency_key = %idempotency_key,
        request_hash = %reservation.request_hash(),
        reservation_mode = ?reservation.mode(),
        side = ?command.amount.side(),
        pool = ?command.route.pool,
        mint = %command.route.mint,
        %detail,
        status = "building_tx",
        "trade execution started"
    );
    let building = state
        .trades
        .update(&request_id, TradeStatus::BuildingTx, None, None);
    if !persist_before_execution(state.persistence.as_ref(), building.as_ref()).await {
        return;
    }

    if owner.as_ref().is_some_and(|owner| {
        owner.operation() == crate::application::ordinary_claims::ClaimOperation::SdkTrade
    }) {
        execute_original_sdk(
            &state,
            execution,
            owner.take().expect("checked original SDK owner"),
        )
        .await;
        return;
    }

    if command.aggregator_sol {
        let Some(wallet) = state.ordinary_sol_buy.resolve(&wallet_id) else {
            fail_missing_wallet(&state, &request_id, &wallet_id).await;
            return;
        };
        let simulate = command.execution.mode.simulate();
        let result = wallet.buy(&execution, owner.as_ref()).await;
        match result {
            Ok(SolBuyExecutionOutcome::Legacy(result)) => {
                finish_trade(
                    &state,
                    &request_id,
                    &idempotency_key,
                    command,
                    Ok(result),
                    simulate,
                )
                .await
            }
            Ok(SolBuyExecutionOutcome::Journaled(result)) => {
                finish_journaled(&state, &request_id, &idempotency_key, command, result)
            }
            Err(error) => {
                finish_trade(
                    &state,
                    &request_id,
                    &idempotency_key,
                    command,
                    Err(error),
                    simulate,
                )
                .await
            }
        }
        return;
    }

    let Some(wallet) = state.ordinary_execution.resolve(&wallet_id) else {
        fail_missing_wallet(&state, &request_id, &wallet_id).await;
        return;
    };

    if command.execution.method == ExecutionMethod::Pumpapi {
        let record = state
            .trades
            .update(&request_id, TradeStatus::Submitting, None, None);
        if !persist_before_execution(state.persistence.as_ref(), record.as_ref()).await {
            return;
        }

        let simulate = command.execution.mode.simulate();
        let result = wallet
            .pumpapi(&command, state.config.wait_tx_confirmed)
            .await;
        finish_trade(
            &state,
            &request_id,
            &idempotency_key,
            &command,
            result,
            simulate,
        )
        .await;
        return;
    }

    let Some(blockhash) = state
        .blockhash_cache
        .get_fresh(state.config.blockhash_max_age_ms)
    else {
        let record = state.trades.update(
            &request_id,
            TradeStatus::Rejected,
            None,
            Some("blockhash cache is empty or stale".to_string()),
        );
        persist_update(state.persistence.as_ref(), record.as_ref()).await;
        return;
    };

    let submitting = state
        .trades
        .update(&request_id, TradeStatus::Submitting, None, None);
    if !persist_before_execution(state.persistence.as_ref(), submitting.as_ref()).await {
        return;
    }

    let simulate = command.execution.mode.simulate();
    let result = wallet
        .sdk(
            &command,
            SdkExecutionContext {
                blockhash: blockhash.hash,
                wait_tx_confirmed: state.config.wait_tx_confirmed,
                mev_submit_verify_ms: state.config.mev_submit_verify_ms,
            },
        )
        .await;

    finish_trade(
        &state,
        &request_id,
        &idempotency_key,
        &command,
        result,
        simulate,
    )
    .await;
}

async fn execute_original_sdk(state: &AppState, execution: &AdmittedTrade, owner: ExecutionOwner) {
    let anchor = execution.reservation();
    let command = execution.command();
    let Some(journal) = state.persistence.ordinary_execution_claims() else {
        finish_journaled(
            state,
            anchor.request_id(),
            anchor.idempotency_key(),
            command,
            JournaledSolBuyOutcome {
                record: None,
                signature: None,
                error: Some("original SDK durable journal is unavailable; do not resubmit".into()),
            },
        );
        return;
    };
    let Some(wallet) = state.ordinary_execution.resolve(anchor.wallet_id()) else {
        let outcome = crate::application::ordinary_sdk_submission::reject_preparation(
            &journal,
            execution,
            owner,
            anyhow::anyhow!("wallet_id {} not found", anchor.wallet_id()),
        )
        .await;
        finish_journaled(
            state,
            anchor.request_id(),
            anchor.idempotency_key(),
            command,
            outcome,
        );
        return;
    };
    let Some(blockhash) = state
        .blockhash_cache
        .get_fresh(state.config.blockhash_max_age_ms)
    else {
        let outcome = crate::application::ordinary_sdk_submission::reject_context(
            &journal,
            execution,
            owner,
            "blockhash cache is empty or stale",
        )
        .await;
        finish_journaled(
            state,
            anchor.request_id(),
            anchor.idempotency_key(),
            command,
            outcome,
        );
        return;
    };
    let outcome = wallet
        .sdk_journaled(
            execution,
            owner,
            SdkExecutionContext {
                blockhash: blockhash.hash,
                wait_tx_confirmed: state.config.wait_tx_confirmed,
                mev_submit_verify_ms: state.config.mev_submit_verify_ms,
            },
        )
        .await;
    finish_journaled(
        state,
        anchor.request_id(),
        anchor.idempotency_key(),
        command,
        outcome,
    );
}

async fn fail_missing_wallet(state: &AppState, request_id: &str, wallet_id: &str) {
    let record = state.trades.update(
        request_id,
        TradeStatus::Failed,
        None,
        Some(format!("wallet_id {wallet_id} not found")),
    );
    let _ = persist_before_execution(state.persistence.as_ref(), record.as_ref()).await;
}

async fn finish_trade(
    state: &AppState,
    request_id: &str,
    idempotency_key: &str,
    command: &TradeCommand,
    result: anyhow::Result<ExecuteResult>,
    simulate: bool,
) {
    let (status, signature, error_text) = map_status(result, simulate);
    let record = state
        .trades
        .update(request_id, status, signature.clone(), error_text.clone());
    persist_update(state.persistence.as_ref(), record.as_ref()).await;

    after_trade(state, idempotency_key, command, record);
}

fn finish_journaled(
    state: &AppState,
    request_id: &str,
    key: &str,
    command: &TradeCommand,
    outcome: JournaledSolBuyOutcome,
) {
    let record = match outcome.record {
        Some(record) => {
            state.trades.insert_record(&record);
            Some(record)
        }
        None => state.trades.update(
            request_id,
            TradeStatus::Unknown,
            outcome.signature,
            outcome.error,
        ),
    };
    // Durable journal owns the parent write. Never mirror a stale executor snapshot over it.
    after_trade(state, key, command, record);
}

fn after_trade(
    state: &AppState,
    idempotency_key: &str,
    command: &TradeCommand,
    record: Option<TradeRecord>,
) {
    let mint = command.route.mint.to_string();
    let pool_id = command.route.pool_id.map(|v| v.to_string());
    let key = cache_key(command.route.pool, &mint, pool_id.as_deref());
    state.params.insert(key, command.route.pool, mint, pool_id);

    if let Some(record) = record {
        info!(
            request_id = %record.request_id,
            wallet_id = %record.wallet_id,
            idempotency_key = %idempotency_key,
            side = ?record.side,
            pool = ?record.pool,
            mint = %record.mint,
            execution_method = ?record.execution_method,
            status = ?record.status,
            signature = ?record.signature,
            error = ?record.error,
            "trade execution finished"
        );
    }
}

async fn persist_update(journal: &impl OrdinaryTradeJournal, record: Option<&TradeRecord>) {
    let Some(record) = record else {
        return;
    };
    if let Err(error) = journal.update_record(record).await {
        error!(request_id = %record.request_id, error = %error, "failed to persist trade status; transaction will not be retried automatically");
    }
}

async fn persist_before_execution(
    journal: &impl OrdinaryTradeJournal,
    record: Option<&TradeRecord>,
) -> bool {
    let Some(record) = record else {
        return false;
    };
    match journal.update_record(record).await {
        Ok(()) => true,
        Err(error) if journal.mode() == TradeJournalMode::Durable => {
            error!(request_id = %record.request_id, error = %error, "durable trade status could not be saved; trade execution stopped before broadcast");
            false
        }
        Err(error) => {
            error!(request_id = %record.request_id, error = %error, "shadow trade status could not be saved; original in-memory execution continues");
            true
        }
    }
}

fn map_status(
    result: anyhow::Result<ExecuteResult>,
    simulate: bool,
) -> (TradeStatus, Option<String>, Option<String>) {
    if let Err(error) = &result {
        error!(error = %error, "trade execution call failed");
    }
    classify_result(result, simulate)
}
