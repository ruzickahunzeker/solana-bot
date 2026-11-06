use anyhow::Context;
use tracing::{error, info};

use crate::aggregator_quote::{self, SolBuyQuoteRequest, SolBuySubmitRequest};
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

    if normalized.aggregator_sol {
        let simulate = normalized.simulate.unwrap_or(state.config.enable_simulate);
        let result = async {
            let amount_lamports = normalized
                .amount_lamports
                .context("SOL aggregator buy amount is missing")?;
            let quote_request = SolBuyQuoteRequest {
                wallet_id: wallet_id.clone(),
                mint: request.mint.clone(),
                amount_lamports,
                slippage_bps: u16::try_from(normalized.slippage_bps)
                    .context("aggregator slippage exceeds supported range")?,
            };
            aggregator_quote::validate_request(&quote_request)?;
            let quote = aggregator_quote::quote(&quote_request).await?;
            let bound = state.aggregator_quotes.insert(
                &quote_request,
                wallet.wallet().to_string(),
                quote,
            )?;
            let submission = aggregator_quote::submit(
                &bound,
                &wallet,
                &SolBuySubmitRequest {
                    wallet_id: wallet_id.clone(),
                    confirmed: !simulate,
                    simulate,
                    idempotency_key: idempotency_key.clone(),
                },
            )
            .await?;
            Ok::<_, anyhow::Error>(ExecuteResult {
                success: submission.status == "submitted" || submission.status == "simulated",
                signature: submission.signature,
                error: submission.error,
            })
        }
        .await;
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
        sol_hop_pool: normalized.sol_hop_pool,
        sol_hop_pool_id: normalized.sol_hop_pool_id,
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
                // The SDK may report a locally signed simulation fingerprint.
                // It is not a submitted transaction signature.
                return (TradeStatus::Simulated, None, result.error);
            }
            if result.signature.is_some() {
                (TradeStatus::Submitted, result.signature, result.error)
            } else {
                (TradeStatus::Unknown, None, result.error)
            }
        }
        Ok(result) => {
            if simulate {
                return (TradeStatus::Failed, None, result.error);
            }
            let known_failed_submission = result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("landed with transaction error"));
            let status = if known_failed_submission {
                TradeStatus::Failed
            } else {
                // A failed SDK send without a returned signature may already
                // have reached the lane; keep the order pending review.
                TradeStatus::Unknown
            };
            (status, result.signature, result.error)
        }
        Err(error) => {
            error!(error = %error, "trade execution call failed");
            // A live transport failure may occur after the lane accepted the
            // transaction; without a signature the result still needs review.
            (
                if simulate {
                    TradeStatus::Failed
                } else {
                    TradeStatus::Unknown
                },
                None,
                Some(error.to_string()),
            )
        }
    }
}

#[cfg(test)]
mod status_tests {
    use super::map_status;
    use crate::models::TradeStatus;
    use crate::sdk_client::ExecuteResult;

    #[test]
    fn invisible_live_signature_is_unknown_and_preserved() {
        let result = ExecuteResult {
            success: false,
            signature: Some("submitted-signature".into()),
            error: Some("submit signature not visible; reconcile order status".into()),
        };
        let (status, signature, _) = map_status(Ok(result), false);
        assert_eq!(status, TradeStatus::Unknown);
        assert_eq!(signature.as_deref(), Some("submitted-signature"));
    }

    #[test]
    fn live_transport_error_without_signature_is_unknown() {
        let (status, signature, _) = map_status(Err(anyhow::anyhow!("timeout")), false);
        assert_eq!(status, TradeStatus::Unknown);
        assert!(signature.is_none());
    }

    #[test]
    fn failed_live_send_without_signature_is_unknown() {
        let result = ExecuteResult {
            success: false,
            signature: None,
            error: Some("sendTransaction failed after timeout".into()),
        };
        let (status, signature, _) = map_status(Ok(result), false);
        assert_eq!(status, TradeStatus::Unknown);
        assert!(signature.is_none());
    }

    #[test]
    fn simulation_signature_is_not_exposed() {
        for success in [true, false] {
            let result = ExecuteResult {
                success,
                signature: Some("locally-signed-simulation".into()),
                error: None,
            };
            let (status, signature, _) = map_status(Ok(result), true);
            assert_eq!(
                status,
                if success {
                    TradeStatus::Simulated
                } else {
                    TradeStatus::Failed
                }
            );
            assert!(signature.is_none());
        }
    }
}
