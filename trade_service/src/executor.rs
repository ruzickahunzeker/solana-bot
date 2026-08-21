use tracing::{error, info};

use crate::models::{ExecutionMethod, Side, TradeRequest, TradeStatus};
use crate::pumpapi_client::PumpApiTrade;
use crate::sdk_client::{ExecuteResult, ExecuteTrade};
use crate::state::{cache_key, describe_trade};
use crate::validation::NormalizedTrade;
use crate::AppState;

pub async fn execute_trade(
    state: AppState,
    request_id: String,
    request: TradeRequest,
    normalized: NormalizedTrade,
) {
    let idempotency_key = request.idempotency_key.clone();
    let wallet_id = request
        .wallet_id
        .clone()
        .unwrap_or_else(|| state.wallets.default_wallet_id().to_string());
    let detail = describe_trade(
        request.side,
        normalized.amount_lamports,
        normalized.sell_percent,
    );
    info!(
        request_id = %request_id,
        wallet_id = %wallet_id,
        idempotency_key = %idempotency_key,
        side = ?request.side,
        pool = ?request.pool,
        mint = %request.mint,
        %detail,
        status = "building_tx",
        "trade execution started"
    );
    let building = state
        .trades
        .update(&request_id, TradeStatus::BuildingTx, None, None);
    if !persist_before_execution(&state, building.as_ref()).await {
        return;
    }

    let Some(wallet) = state.wallets.get(&wallet_id) else {
        let record = state.trades.update(
            &request_id,
            TradeStatus::Failed,
            None,
            Some(format!("wallet_id {wallet_id} not found")),
        );
        if !persist_before_execution(&state, record.as_ref()).await {
            return;
        }
        return;
    };

    if normalized.execution_method == ExecutionMethod::Pumpapi {
        let record = state
            .trades
            .update(&request_id, TradeStatus::Submitting, None, None);
        if !persist_before_execution(&state, record.as_ref()).await {
            return;
        }

        let simulate = normalized.simulate.unwrap_or(state.config.enable_simulate);
        let trade = PumpApiTrade {
            config: state.config.pumpapi.clone(),
            side: request.side,
            mint: normalized.mint,
            quote_mint: normalized.quote_mint,
            pool_id: normalized.pool_id,
            amount_units: normalized.amount_lamports,
            sell_percent: normalized.sell_percent,
            slippage_bps: normalized.slippage_bps,
            priority_fee_sol: request
                .tip_sol
                .unwrap_or(state.config.pumpapi.default_priority_fee_sol),
            simulate,
            wait_tx_confirmed: state.config.wait_tx_confirmed,
        };

        let result = wallet.pumpapi_trade(trade).await;
        finish_trade(
            &state,
            &request_id,
            &idempotency_key,
            &request,
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
        persist_update(&state, record.as_ref()).await;
        return;
    };

    let submitting = state
        .trades
        .update(&request_id, TradeStatus::Submitting, None, None);
    if !persist_before_execution(&state, submitting.as_ref()).await {
        return;
    }

    let simulate = normalized.simulate.unwrap_or(state.config.enable_simulate);
    let trade = ExecuteTrade {
        pool: request.pool,
        mint: normalized.mint,
        quote_mint: normalized.quote_mint,
        pool_id: normalized.pool_id,
        amount_lamports: normalized.amount_lamports,
        sell_percent: normalized.sell_percent,
        slippage_bps: normalized.slippage_bps,
        tip_sol: normalized.tip_sol,
        recent_blockhash: blockhash.hash,
        wait_tx_confirmed: state.config.wait_tx_confirmed,
        mev_submit_verify_ms: state.config.mev_submit_verify_ms,
        simulate,
        use_mev: normalized.use_mev,
        mev_provider: normalized.mev_provider,
    };

    let result = match request.side {
        Side::Buy => wallet.buy(trade).await,
        Side::Sell => wallet.sell(trade).await,
    };

    finish_trade(
        &state,
        &request_id,
        &idempotency_key,
        &request,
        result,
        simulate,
    )
    .await;
}

async fn finish_trade(
    state: &AppState,
    request_id: &str,
    idempotency_key: &str,
    request: &TradeRequest,
    result: anyhow::Result<ExecuteResult>,
    simulate: bool,
) {
    let (status, signature, error_text) = map_status(result, simulate);
    let record = state
        .trades
        .update(request_id, status, signature.clone(), error_text.clone());
    persist_update(state, record.as_ref()).await;

    let key = cache_key(request.pool, &request.mint, request.pool_id.as_deref());
    state.params.insert(
        key,
        request.pool,
        request.mint.clone(),
        request.pool_id.clone(),
    );

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

async fn persist_update(state: &AppState, record: Option<&crate::models::TradeRecord>) {
    let Some(record) = record else {
        return;
    };
    if let Err(error) = state.persistence.update(record).await {
        error!(request_id = %record.request_id, error = %error, "failed to persist trade status; transaction will not be retried automatically");
    }
}

async fn persist_before_execution(
    state: &AppState,
    record: Option<&crate::models::TradeRecord>,
) -> bool {
    let Some(record) = record else {
        return false;
    };
    match state.persistence.update(record).await {
        Ok(()) => true,
        Err(error) if state.persistence.is_durable() => {
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
    match result {
        Ok(result) if result.success => {
            if simulate {
                return (TradeStatus::Simulated, result.signature, result.error);
            }
            if result.signature.is_some() {
                (TradeStatus::Submitted, result.signature, result.error)
            } else {
                (TradeStatus::Unknown, None, result.error)
            }
        }
        Ok(result) => {
            let known_failed_submission = result.error.as_deref().is_some_and(|error| {
                error.contains("landed with transaction error")
                    || error.contains("submit signature not visible")
                    || error.contains("sendTransaction failed")
            });
            let status = if known_failed_submission {
                TradeStatus::Failed
            } else if result.signature.is_some() {
                TradeStatus::Unknown
            } else {
                TradeStatus::Failed
            };
            (status, result.signature, result.error)
        }
        Err(error) => {
            error!(error = %error, "trade execution call failed");
            (TradeStatus::Failed, None, Some(error.to_string()))
        }
    }
}
