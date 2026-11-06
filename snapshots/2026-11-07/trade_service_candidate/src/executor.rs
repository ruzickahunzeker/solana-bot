use anyhow::Context;
use solana_sdk::signature::Signature;
use std::future::Future;
use tracing::{error, info};

use crate::aggregator_quote::{
    BoundSolBuyQuote, SolBuyQuoteRequest, SolBuySubmitRequest, SolBuySubmitResponse,
    SubmissionReservation,
};
use crate::aggregator_service;
use crate::backend::types::{RouteHint, VerifiedViaSolRoute};
use crate::models::{
    AggregatorSimulationReport, ExecutionMethod, SettlementAsset, Side, SimulationReport,
    TradeRequest, TradeStatus, WSOL_MINT,
};
use crate::pumpapi_client::PumpApiTrade;
use crate::sanitize::redact_urls;
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
        if !aggregator_simulation_allowed(state.config.simulate_only, simulate) {
            let record = state.trades.update(
                &request_id,
                TradeStatus::Rejected,
                None,
                Some("this Trade Service instance only accepts aggregator simulations".into()),
            );
            if !persist_before_execution(&state, record.as_ref()).await {
                return;
            }
            return;
        }
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
            let submission_key = format!("{}:{}", wallet_id, idempotency_key);
            if let Some(reservation) = state
                .aggregator_quotes
                .lookup_submission(&submission_key)
                .await?
            {
                match reservation {
                    SubmissionReservation::Existing(existing) => {
                        let simulation = Some(aggregator_simulation_report(
                            &existing,
                            None,
                            Some(&request_id),
                        ));
                        return Ok::<_, anyhow::Error>(ExecuteResult {
                            success: existing.status == "submitted" || existing.status == "simulated",
                            signature: existing.signature,
                            error: existing.error,
                            simulation,
                        });
                    }
                    SubmissionReservation::InFlight | SubmissionReservation::Unknown => {
                        anyhow::bail!("aggregator idempotency key is already reserved; recover the original submission")
                    }
                    SubmissionReservation::Reserved | SubmissionReservation::Conflict => {
                        anyhow::bail!("aggregator idempotency key has an invalid reservation state")
                    }
                }
            }
            let bound = aggregator_service::create_jupiter_bound_quote(
                &state.aggregator_quotes,
                &quote_request,
                &wallet,
            )
            .await?;
            let submission = aggregator_service::submit_jupiter_bound_quote(
                &state.aggregator_quotes,
                &bound,
                &wallet,
                &SolBuySubmitRequest {
                    wallet_id: wallet_id.clone(),
                    confirmed: !simulate,
                    simulate,
                    idempotency_key: idempotency_key.clone(),
                },
                state.config.simulate_only,
            )
            .await?;
            let simulation = Some(aggregator_simulation_report(
                &submission,
                Some(&bound),
                Some(&request_id),
            ));
            Ok::<_, anyhow::Error>(ExecuteResult {
                success: submission.status == "submitted" || submission.status == "simulated",
                signature: submission.signature,
                error: submission.error,
                simulation,
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

    if request.pool == crate::models::Pool::RaydiumCpmm
        && normalized.settlement_asset == SettlementAsset::Sol
    {
        let simulate = normalized.simulate == Some(true);
        let result = match (request.side, simulate) {
            (Side::Buy, true) => {
                simulate_cpmm_two_hop_buy_order(&state, &wallet, &request, &normalized, &wallet_id)
                    .await
            }
            (Side::Buy, false) => {
                submit_cpmm_two_hop_buy_order(
                    &state,
                    &wallet,
                    &request_id,
                    &request,
                    &normalized,
                    &wallet_id,
                )
                .await
            }
            (Side::Sell, true) => {
                simulate_cpmm_two_hop_sell_order(&state, &wallet, &request, &normalized, &wallet_id)
                    .await
            }
            (Side::Sell, false) => {
                submit_cpmm_two_hop_sell_order(
                    &state,
                    &wallet,
                    &request_id,
                    &request,
                    &normalized,
                    &wallet_id,
                )
                .await
            }
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
        return;
    }

    if request.sdk_preview_id.is_some() || crate::validation::has_bridge_sdk_decision_id(&request) {
        // This route is an unsigned, simulate-only SDK adapter. It bypasses
        // the direct single-pool buy path, which cannot build this two-hop
        // quote-funded transaction. Both preview and bridge-decision credentials
        // are checked again after admission, before any RPC work begins.
        let result = if crate::validation::has_bridge_sdk_decision_id(&request) {
            let validation = async {
                let wallet_pubkey = wallet.wallet().to_string();
                crate::route_registry::validate_sdk_decision_trade(
                    &state.route_store,
                    &request,
                    &normalized,
                    &wallet_id,
                    &wallet_pubkey,
                    crate::state::now_ms() as u64,
                )
                .await?;
                let decision = state
                    .route_store
                    .get_sdk_decision(
                        request
                            .sdk_decision_id
                            .as_deref()
                            .context("bridge decision missing")?,
                        crate::state::now_ms() as u64,
                    )
                    .await?;
                let mut preview_request = request.clone();
                preview_request.sdk_decision_id = None;
                preview_request.sdk_preview_id = decision.bridge_preview_id;
                simulate_observed_sdk_bridge(
                    &state,
                    &wallet,
                    &preview_request,
                    &normalized,
                    &wallet_id,
                )
                .await
            };
            validation.await
        } else {
            simulate_observed_sdk_bridge(&state, &wallet, &request, &normalized, &wallet_id).await
        };
        finish_trade(
            &state,
            &request_id,
            &idempotency_key,
            &request,
            result,
            true,
        )
        .await;
        return;
    }

    let route_check: anyhow::Result<Option<solana_sdk::pubkey::Pubkey>> =
        if request.sdk_decision_id.is_some() {
            let wallet_pubkey = wallet.wallet().to_string();
            let wallet_binding_valid = if state.wallet_repository.database_enabled() {
                matches!(
                    state.wallet_repository.get(&wallet_id).await,
                    Ok(managed) if managed.enabled && managed.pubkey == wallet_pubkey
                )
            } else {
                true
            };
            if wallet_binding_valid {
                // The decision snapshot binds the wallet pubkey in both file
                // and database modes. Database mode additionally checks the
                // current enabled row before every execution attempt.
                crate::route_registry::validate_sdk_decision_trade(
                    &state.route_store,
                    &request,
                    &normalized,
                    &wallet_id,
                    &wallet_pubkey,
                    crate::state::now_ms() as u64,
                )
                .await
            } else {
                Err(anyhow::anyhow!(
                    "decision wallet is disabled or has changed"
                ))
            }
        } else {
            verify_sdk_via_sol_before_trade(&state, &request, &normalized)
                .await
                .map(|_| None)
        };
    let decision_pool_id = match route_check {
        Ok(pool_id) => pool_id,
        Err(error) => {
            let record = state.trades.update(
                &request_id,
                TradeStatus::Rejected,
                None,
                Some(redact_urls(&error.to_string())),
            );
            persist_update(&state, record.as_ref()).await;
            return;
        }
    };

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
        pool_id: decision_pool_id.or(normalized.pool_id),
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

async fn submit_cpmm_two_hop_sell_order(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request_id: &str,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
) -> anyhow::Result<ExecuteResult> {
    let prepared =
        prepare_cpmm_two_hop_sell_order(state, wallet, request, normalized, wallet_id, true)
            .await?;
    let minimum_context_slot = prepared
        .preview
        .simulation
        .as_ref()
        .context("CPMM sell simulation is missing before send")?
        .slot;
    let signer = wallet.conversion_signer();
    let transaction = prepared.into_signed_transaction(signer.as_ref())?;
    submit_signed_two_hop_transaction(state, wallet, request_id, transaction, minimum_context_slot)
        .await
}

async fn submit_signed_two_hop_transaction(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request_id: &str,
    transaction: solana_sdk::transaction::VersionedTransaction,
    minimum_context_slot: u64,
) -> anyhow::Result<ExecuteResult> {
    let signature = transaction.signatures[0];
    let mut result = reserve_and_send_prepared_sdk_transaction(
        &state.persistence,
        request_id,
        signature,
        || async {
            // The database already has the same signature; expose it to HTTP
            // readers before the sole RPC request is made.
            state.trades.update(
                request_id,
                TradeStatus::Submitting,
                Some(signature.to_string()),
                None,
            );
            wallet
                .send_prepared_sdk_transaction_once(&transaction, minimum_context_slot)
                .await
        },
    )
    .await?;
    match wallet
        .observe_prepared_sdk_signature(signature, state.config.mev_submit_verify_ms)
        .await
    {
        Ok(Some(true)) => {
            result.success = true;
            result.error = None;
        }
        Ok(Some(false)) => {
            result.success = false;
            result.error = Some("SDK transaction landed with transaction error".into());
        }
        Ok(None) => {
            result.success = false;
            result.error = Some("SDK signature not visible; reconcile order status".into());
        }
        Err(error) => {
            result.success = false;
            result.error = Some(format!(
                "SDK signature readback unavailable: {}; reconcile order status",
                redact_urls(&error.to_string())
            ));
        }
    }
    Ok(result)
}

async fn submit_cpmm_two_hop_buy_order(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request_id: &str,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
) -> anyhow::Result<ExecuteResult> {
    let prepared =
        prepare_cpmm_two_hop_buy_order(state, wallet, request, normalized, wallet_id, true).await?;
    let minimum_context_slot = prepared
        .preview
        .simulation
        .as_ref()
        .context("CPMM buy simulation is missing before send")?
        .slot;
    let signer = wallet.conversion_signer();
    let transaction = prepared.into_signed_transaction(signer.as_ref())?;
    submit_signed_two_hop_transaction(state, wallet, request_id, transaction, minimum_context_slot)
        .await
}

async fn simulate_cpmm_two_hop_buy_order(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
) -> anyhow::Result<ExecuteResult> {
    let prepared =
        prepare_cpmm_two_hop_buy_order(state, wallet, request, normalized, wallet_id, false)
            .await?;
    let simulation = prepared
        .preview
        .simulation
        .context("CPMM buy RPC simulation missing")?;
    Ok(ExecuteResult {
        success: true,
        signature: None,
        error: None,
        simulation: Some(SimulationReport {
            ok: true,
            protocol: "raydium-cpmm".to_string(),
            source: "sdk-11:cpmm-two-hop-buy".to_string(),
            units_consumed: simulation.units_consumed,
            logs: Vec::new(),
            aggregator: None,
        }),
    })
}

async fn prepare_cpmm_two_hop_buy_order(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
    live: bool,
) -> anyhow::Result<crate::sdk_client::SdkTwoHopBuyPrepared> {
    anyhow::ensure!(
        if live {
            !state.config.simulate_only
                && state.config.sdk_two_hop_live_enabled
                && request.simulate == Some(false)
        } else {
            state.config.simulate_only && request.simulate == Some(true)
        } && state.persistence.is_durable()
            && request.sdk_decision_id.is_some(),
        "CPMM two-hop buy order requires a durable SDK decision and an enabled execution mode"
    );
    let wallet_pubkey = wallet.wallet().to_string();
    if state.wallet_repository.database_enabled() {
        let managed = state.wallet_repository.get(wallet_id).await?;
        anyhow::ensure!(
            managed.enabled && managed.pubkey == wallet_pubkey,
            "CPMM buy wallet is disabled or has changed"
        );
    }
    let now_ms = crate::state::now_ms() as u64;
    crate::route_registry::validate_sdk_decision_trade(
        &state.route_store,
        request,
        normalized,
        wallet_id,
        &wallet_pubkey,
        now_ms,
    )
    .await?;
    let decision = state
        .route_store
        .get_sdk_decision(
            request
                .sdk_decision_id
                .as_deref()
                .context("CPMM buy decision is missing")?,
            now_ms,
        )
        .await?;
    let evidence = decision
        .cpmm_buy_evidence
        .context("CPMM buy decision has no unsigned simulation evidence")?;
    let prepared = wallet
        .prepare_cpmm_buy_via_cpmm(
            normalized
                .pool_id
                .as_ref()
                .context("CPMM target pool missing")?,
            normalized
                .sol_hop_pool_id
                .as_ref()
                .context("CPMM bridge pool missing")?,
            &normalized.mint,
            &normalized.quote_mint,
            normalized
                .amount_lamports
                .context("CPMM buy input missing")?,
            normalized.slippage_bps,
            true,
        )
        .await?;
    let preview = &prepared.preview;
    let simulation = preview
        .simulation
        .as_ref()
        .context("CPMM buy RPC simulation missing")?;
    anyhow::ensure!(
        simulation.attempted
            && simulation.rpc_accepted
            && simulation.ok
            && simulation.minimum_output_verified
            && preview.input_lamports == evidence.input_lamports
            && preview.target_min_output_units >= evidence.minimum_target_units
            && preview.validated_slot >= evidence.validated_slot
            && simulation.slot >= evidence.simulation_slot
            && simulation
                .payer_debit_lamports
                .is_some_and(|debit| debit >= evidence.input_lamports)
            && simulation.target_token_delta_units.is_some_and(|delta| {
                delta >= evidence.minimum_target_units && delta >= preview.target_min_output_units
            }),
        "CPMM buy re-simulation changed the bound input or protected output"
    );
    Ok(prepared)
}

async fn simulate_cpmm_two_hop_sell_order(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
) -> anyhow::Result<ExecuteResult> {
    let prepared =
        prepare_cpmm_two_hop_sell_order(state, wallet, request, normalized, wallet_id, false)
            .await?;
    let simulation = prepared
        .preview
        .simulation
        .context("CPMM sell RPC simulation missing")?;
    Ok(ExecuteResult {
        success: true,
        signature: None,
        error: None,
        simulation: Some(SimulationReport {
            ok: true,
            protocol: "raydium-cpmm".to_string(),
            source: "sdk-11:cpmm-two-hop-sell".to_string(),
            units_consumed: simulation.units_consumed,
            logs: Vec::new(),
            aggregator: None,
        }),
    })
}

async fn prepare_cpmm_two_hop_sell_order(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
    live: bool,
) -> anyhow::Result<crate::sdk_client::SdkTwoHopSellPrepared> {
    anyhow::ensure!(
        if live {
            !state.config.simulate_only
                && state.config.sdk_two_hop_live_enabled
                && request.simulate == Some(false)
        } else {
            state.config.simulate_only && request.simulate == Some(true)
        } && state.persistence.is_durable()
            && request.sdk_decision_id.is_some(),
        "CPMM two-hop sell order requires a durable SDK decision and an enabled execution mode"
    );
    let wallet_pubkey = wallet.wallet().to_string();
    if state.wallet_repository.database_enabled() {
        let managed = state.wallet_repository.get(wallet_id).await?;
        anyhow::ensure!(
            managed.enabled && managed.pubkey == wallet_pubkey,
            "CPMM sell wallet is disabled or has changed"
        );
    }
    let now_ms = crate::state::now_ms() as u64;
    crate::route_registry::validate_sdk_decision_trade(
        &state.route_store,
        request,
        normalized,
        wallet_id,
        &wallet_pubkey,
        now_ms,
    )
    .await?;
    let decision = state
        .route_store
        .get_sdk_decision(
            request
                .sdk_decision_id
                .as_deref()
                .context("CPMM sell decision is missing")?,
            now_ms,
        )
        .await?;
    let evidence = decision
        .cpmm_sell_evidence
        .context("CPMM sell decision has no unsigned simulation evidence")?;
    let prepared = wallet
        .prepare_cpmm_sell_via_cpmm(
            normalized
                .pool_id
                .as_ref()
                .context("CPMM target pool missing")?,
            normalized
                .sol_hop_pool_id
                .as_ref()
                .context("CPMM bridge pool missing")?,
            &normalized.mint,
            &normalized.quote_mint,
            normalized
                .sell_percent
                .context("CPMM sell percent missing")?,
            normalized.slippage_bps,
            true,
        )
        .await?;
    let preview = &prepared.preview;
    let simulation = preview
        .simulation
        .as_ref()
        .context("CPMM sell RPC simulation missing")?;
    anyhow::ensure!(
        simulation.attempted
            && simulation.rpc_accepted
            && simulation.ok
            && simulation.balance_and_minimum_verified
            && preview.input_token_units == evidence.input_token_units
            && preview.bridge_min_wsol_units >= evidence.minimum_wsol_units
            && preview.validated_slot >= evidence.validated_slot
            && simulation.slot >= evidence.simulation_slot
            && simulation.input_token_debit_units == Some(evidence.input_token_units)
            && simulation.wsol_credit_units.is_some_and(|credit| {
                credit >= evidence.minimum_wsol_units && credit >= preview.bridge_min_wsol_units
            }),
        "CPMM sell re-simulation changed the bound input or protected output"
    );
    Ok(prepared)
}

/// Reserve a signed SDK message before its sole transport attempt. The local
/// signature remains in the result even if the RPC reply is lost or times out.
async fn reserve_and_send_prepared_sdk_transaction<F, Fut>(
    persistence: &crate::persistence::TradePersistence,
    request_id: &str,
    signature: Signature,
    send_once: F,
) -> anyhow::Result<ExecuteResult>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<Signature>>,
{
    anyhow::ensure!(
        persistence
            .reserve_sdk_signature_for_send(request_id, &signature)
            .await?,
        "SDK signature reservation was not acquired; transaction was not sent"
    );
    let result = send_once().await;
    let (success, error) = match result {
        Ok(returned) if returned == signature => (true, None),
        Ok(_) => (
            false,
            Some("SDK RPC returned a signature different from the reserved signature".into()),
        ),
        Err(error) => (false, Some(redact_urls(&error.to_string()))),
    };
    Ok(ExecuteResult {
        success,
        signature: Some(signature.to_string()),
        error,
        simulation: None,
    })
}

async fn simulate_observed_sdk_bridge(
    state: &AppState,
    wallet: &crate::sdk_client::SdkTradeClient,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
) -> anyhow::Result<ExecuteResult> {
    anyhow::ensure!(
        state.config.simulate_only && request.simulate == Some(true),
        "SDK preview path is simulate-only"
    );
    let wallet_pubkey = wallet.wallet().to_string();
    if state.wallet_repository.database_enabled() {
        let managed = state.wallet_repository.get(wallet_id).await?;
        anyhow::ensure!(
            managed.enabled && managed.pubkey == wallet_pubkey,
            "SDK preview wallet is disabled or has changed"
        );
    }
    let (credential, observation) = crate::route_registry::validate_sdk_preview_trade(
        &state.route_store,
        request,
        normalized,
        wallet_id,
        &wallet_pubkey,
        crate::state::now_ms() as u64,
    )
    .await?;
    let target_pool = observation
        .target
        .pool_id
        .as_deref()
        .context("target pool missing")?
        .parse()?;
    let bridge_pool = observation
        .sol_hop
        .pool_id
        .as_deref()
        .context("bridge pool missing")?
        .parse()?;
    let target_quote_mint = observation
        .target
        .quote_mint
        .as_deref()
        .context("verified target quote mint missing")?
        .parse()?;
    let (preview, source) = match request.pool {
        crate::models::Pool::PumpAmm => (
            wallet
                .preview_pumpswap_via_cpmm(
                    &target_pool,
                    &bridge_pool,
                    &normalized.mint,
                    &target_quote_mint,
                    credential.input_lamports,
                    normalized.slippage_bps,
                    true,
                )
                .await?,
            "sdk-11:pumpswap-quote-route",
        ),
        crate::models::Pool::RaydiumAmmV4 => (
            wallet
                .preview_amm_v4_via_cpmm(
                    &target_pool,
                    &bridge_pool,
                    &normalized.mint,
                    &target_quote_mint,
                    credential.input_lamports,
                    normalized.slippage_bps,
                    true,
                )
                .await?,
            "sdk-11:stonkfun-quote-route",
        ),
        crate::models::Pool::RaydiumCpmm => (
            wallet
                .preview_cpmm_via_cpmm(
                    &target_pool,
                    &bridge_pool,
                    &normalized.mint,
                    &target_quote_mint,
                    credential.input_lamports,
                    normalized.slippage_bps,
                    true,
                )
                .await?,
            "sdk-11:stonkfun-quote-route",
        ),
        _ => anyhow::bail!("SDK bridge trade protocol is not supported"),
    };
    let simulation = preview
        .simulation
        .context("SDK two-hop simulation missing")?;
    let failure_reason = if !simulation.attempted || !simulation.rpc_accepted {
        Some("SDK two-hop RPC simulation was not accepted")
    } else if !simulation.ok || !simulation.minimum_output_verified {
        Some("SDK two-hop RPC simulation failed its current minimum output")
    } else if simulation
        .payer_debit_lamports
        .is_none_or(|debit| debit < credential.input_lamports)
    {
        Some("SDK two-hop payer debit is below the requested input")
    } else if preview.target_min_output_units < credential.target_min_output_units {
        Some("SDK two-hop current minimum output fell below bound preview")
    } else if preview.validated_slot < credential.validated_slot
        || simulation.slot < credential.simulation_slot
    {
        Some("SDK two-hop pool or simulation slot regressed")
    } else if !simulation.target_token_delta_units.is_some_and(|delta| {
        delta >= credential.target_min_output_units && delta >= preview.target_min_output_units
    }) {
        Some("SDK two-hop target token delta fell below bound minimum")
    } else {
        None
    };
    let output_valid = failure_reason.is_none();
    let report = SimulationReport {
        ok: output_valid,
        protocol: crate::backend::registry::protocol_id(request.pool).to_string(),
        source: source.to_string(),
        units_consumed: simulation.units_consumed,
        logs: Vec::new(),
        aggregator: None,
    };
    Ok(ExecuteResult {
        success: output_valid,
        signature: None,
        error: failure_reason.map(str::to_string),
        simulation: Some(report),
    })
}

async fn verify_sdk_via_sol_before_trade(
    state: &AppState,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
) -> anyhow::Result<()> {
    if normalized.settlement_asset != SettlementAsset::Sol {
        return Ok(());
    }
    resolve_sdk_via_sol_trade_route(state, request, normalized)
        .await
        .map(|_| ())
}

pub(crate) async fn resolve_sdk_via_sol_trade_route(
    state: &AppState,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
) -> anyhow::Result<VerifiedViaSolRoute> {
    let now_ms = crate::state::now_ms() as u64;
    let route = if let Some(route_id) = request.sdk_route_id.as_deref() {
        state.route_store.get_via_sol(route_id, now_ms).await?
    } else {
        let target_hint = RouteHint {
            protocol_id: Some(crate::backend::registry::protocol_id(request.pool).to_string()),
            program_id: None,
            pool_id: request.pool_id.clone(),
            mint: request.mint.clone(),
            quote_mint: Some(normalized.quote_mint.to_string()),
        };
        let sol_hop_hint = RouteHint {
            protocol_id: Some("raydium-cpmm".to_string()),
            program_id: None,
            pool_id: request.sol_hop_pool_id.clone(),
            mint: normalized.quote_mint.to_string(),
            quote_mint: Some(WSOL_MINT.to_string()),
        };
        let (target, sol_hop) = tokio::try_join!(
            state.execution_backend.resolve_pool(target_hint),
            state.execution_backend.resolve_pool(sol_hop_hint)
        )?;
        let route = VerifiedViaSolRoute {
            route_id: format!("sdk11:via-sol:{}", uuid::Uuid::new_v4().simple()),
            expires_at_ms: target.expires_at_ms.min(sol_hop.expires_at_ms),
            target,
            sol_hop,
        };
        state
            .route_store
            .insert_via_sol(route.clone(), now_ms)
            .await?;
        route
    };
    crate::route_registry::validate_via_sol_trade_match(
        &route,
        request,
        normalized,
        crate::state::now_ms() as u64,
    )?;
    Ok(route)
}

fn aggregator_simulation_allowed(simulate_only: bool, simulate: bool) -> bool {
    !simulate_only || simulate
}

async fn finish_trade(
    state: &AppState,
    request_id: &str,
    idempotency_key: &str,
    request: &TradeRequest,
    result: anyhow::Result<ExecuteResult>,
    simulate: bool,
) {
    let result = result.map(|mut value| {
        if simulate && value.simulation.is_none() {
            value.simulation = Some(SimulationReport {
                ok: value.success,
                protocol: crate::backend::registry::protocol_id(request.pool).to_string(),
                source: if request.execution_method == Some(ExecutionMethod::Pumpapi) {
                    "pumpapi".to_string()
                } else {
                    "sdk".to_string()
                },
                units_consumed: None,
                logs: Vec::new(),
                aggregator: None,
            });
        }
        value
    });
    let simulation = result
        .as_ref()
        .ok()
        .and_then(|value| value.simulation.clone());
    let (status, signature, error_text) = map_status(result, simulate);
    let simulation = simulation.or_else(|| {
        simulate.then(|| SimulationReport {
            ok: status == TradeStatus::Simulated,
            protocol: crate::backend::registry::protocol_id(request.pool).to_string(),
            source: if request.execution_method == Some(ExecutionMethod::Pumpapi) {
                "pumpapi".to_string()
            } else {
                "sdk".to_string()
            },
            units_consumed: None,
            logs: error_text.clone().into_iter().collect(),
            aggregator: None,
        })
    });
    let mut record = state
        .trades
        .update(request_id, status, signature.clone(), error_text.clone());
    if let Some(simulation) = simulation {
        record = state
            .trades
            .set_simulation(request_id, simulation)
            .or(record);
    }
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

fn aggregator_simulation_report(
    submission: &SolBuySubmitResponse,
    bound: Option<&BoundSolBuyQuote>,
    request_id: Option<&str>,
) -> SimulationReport {
    let preflight = &submission.preflight;
    SimulationReport {
        ok: preflight.local_simulation_ok,
        protocol: "aggregator-sol-buy".to_string(),
        source: "aggregator:jupiter".to_string(),
        units_consumed: preflight.simulated_units_consumed,
        logs: vec![preflight.reason.clone()],
        aggregator: Some(AggregatorSimulationReport {
            provider: "jupiter".to_string(),
            quote_id: preflight.quote_id.clone(),
            request_id: request_id.map(str::to_string),
            route_fingerprint: bound.map(|value| value.route_fingerprint.clone()),
            message_hash: bound.and_then(|value| value.message_hash.clone()),
            expected_output_units: preflight.expected_output_units,
            minimum_output_units: preflight.minimum_output_units,
            prioritization_fee_lamports: preflight.prioritization_fee_lamports,
            compute_unit_limit: preflight.compute_unit_limit,
            explicit_system_spend_lamports: preflight.explicit_system_spend_lamports,
            simulated_fee_lamports: preflight.simulated_fee_lamports,
            simulated_payer_debit_lamports: preflight.simulated_payer_debit_lamports,
            simulated_target_token_delta_units: preflight.simulated_target_token_delta_units,
            simulated_target_token_post_units: preflight.simulated_target_token_post_units,
            simulated_units_consumed: preflight.simulated_units_consumed,
            simulation_slot: preflight.simulation_slot,
            instruction_count: preflight.instruction_count,
            address_lookup_table_count: preflight.address_lookup_table_count,
            resolved_account_count: preflight.resolved_account_count,
            instruction_audit_passed: preflight.instruction_audit_passed,
            executable: preflight.executable,
        }),
    }
}

async fn persist_update(state: &AppState, record: Option<&crate::models::TradeRecord>) {
    let Some(record) = record else {
        return;
    };
    if let Err(error) = state.persistence.update(record).await {
        let error = redact_urls(&error.to_string());
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
            let error = redact_urls(&error.to_string());
            error!(request_id = %record.request_id, error = %error, "durable trade status could not be saved; trade execution stopped before broadcast");
            false
        }
        Err(error) => {
            let error = redact_urls(&error.to_string());
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
        Ok(result) if simulate => {
            let status = if result.success {
                TradeStatus::Simulated
            } else {
                TradeStatus::Failed
            };
            // A simulation may carry a locally derived transaction signature
            // from an SDK. It is not evidence of a submitted transaction.
            (status, None, result.error.map(|value| redact_urls(&value)))
        }
        Ok(result) if result.success => {
            let error = result.error.map(|value| redact_urls(&value));
            if result.signature.is_some() {
                (TradeStatus::Submitted, result.signature, error)
            } else {
                (TradeStatus::Unknown, None, error)
            }
        }
        Ok(result) => {
            let known_failed_submission = result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("landed with transaction error"));
            let error = result.error.map(|value| redact_urls(&value));
            let status = if known_failed_submission {
                TradeStatus::Failed
            } else {
                // The SDK can return a failed send without exposing a
                // signature after the lane accepted the transaction.
                TradeStatus::Unknown
            };
            (status, result.signature, error)
        }
        Err(error) => {
            let error = redact_urls(&error.to_string());
            error!(error = %error, "trade execution call failed");
            // The SDK call can fail after its send request has reached a lane
            // without returning a signature. Preserve uncertainty for a live
            // order so a caller does not treat it as safe to rebuild and send.
            (
                if simulate {
                    TradeStatus::Failed
                } else {
                    TradeStatus::Unknown
                },
                None,
                Some(error),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        aggregator_simulation_allowed, map_status, reserve_and_send_prepared_sdk_transaction,
    };
    use crate::config::{TradePersistenceConfig, TradePersistenceMode};
    use crate::models::{TradeRequest, TradeStatus};
    use crate::persistence::{DurableReserve, TradePersistence};
    use crate::sdk_client::ExecuteResult;
    use crate::state::TradeStore;
    use crate::validation::NormalizedTrade;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::{Keypair, Signer};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn reserved_sdk_timeout_keeps_signature_and_never_calls_sender_twice() {
        let persistence = TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: "sqlite::memory:".to_string(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap();
        let mint = Pubkey::new_unique();
        let request: TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id": "main",
            "side": "sell",
            "pool": "raydium-cpmm",
            "mint": mint.to_string(),
            "sell_percent": 100,
            "execution_method": "sdk",
            "simulate": false,
            "idempotency_key": "offline-signed-timeout"
        }))
        .unwrap();
        let normalized = NormalizedTrade {
            mint,
            quote_mint: crate::models::WSOL_MINT.parse().unwrap(),
            pool_id: None,
            settlement_asset: crate::models::SettlementAsset::Quote,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: None,
            sell_percent: Some(100),
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: crate::models::ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(false),
            aggregator_sol: false,
        };
        let store = TradeStore::default();
        let mut trade = store.create_with_id(
            "tr_offline_signed_timeout".to_string(),
            &request,
            "main".to_string(),
            Keypair::new().pubkey().to_string(),
        );
        assert!(matches!(
            persistence
                .reserve(&trade, &request, &normalized)
                .await
                .unwrap(),
            DurableReserve::Created
        ));
        trade.status = TradeStatus::BuildingTx;
        persistence.update(&trade).await.unwrap();
        let signature = Keypair::new().sign_message(b"prepared sdk message");
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = calls.clone();
        let result = reserve_and_send_prepared_sdk_transaction(
            &persistence,
            &trade.request_id,
            signature,
            move || async move {
                first_calls.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("transport timeout"))
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            result.signature.as_deref(),
            Some(signature.to_string().as_str())
        );
        let (status, returned_signature, _) = map_status(Ok(result), false);
        assert_eq!(status, TradeStatus::Unknown);
        assert_eq!(returned_signature, Some(signature.to_string()));
        let saved = persistence.get(&trade.request_id).await.unwrap().unwrap();
        assert_eq!(saved.status, TradeStatus::Submitting);
        assert_eq!(saved.signature, Some(signature.to_string()));

        let second_calls = calls.clone();
        assert!(reserve_and_send_prepared_sdk_transaction(
            &persistence,
            &trade.request_id,
            signature,
            move || async move {
                second_calls.fetch_add(1, Ordering::SeqCst);
                Ok(signature)
            }
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn simulate_only_rejects_non_simulated_aggregator_execution() {
        assert!(!aggregator_simulation_allowed(true, false));
        assert!(aggregator_simulation_allowed(true, true));
        assert!(aggregator_simulation_allowed(false, false));
    }

    #[test]
    fn map_status_redacts_urls_from_success_and_failure_errors() {
        let success = ExecuteResult {
            success: true,
            signature: None,
            error: Some("rpc https://rpc.example.test/?api-key=secret failed".into()),
            simulation: None,
        };
        let (_, _, success_error) = map_status(Ok(success), true);
        assert_eq!(success_error.as_deref(), Some("rpc <redacted-url> failed"));

        let failure = ExecuteResult {
            success: false,
            signature: None,
            error: Some("provider wss://provider.example.test/?token=secret failed".into()),
            simulation: None,
        };
        let (_, _, failure_error) = map_status(Ok(failure), false);
        assert_eq!(
            failure_error.as_deref(),
            Some("provider <redacted-url> failed")
        );
    }

    #[test]
    fn simulated_sdk_signature_is_never_exposed_as_a_submission() {
        for success in [true, false] {
            let result = ExecuteResult {
                success,
                signature: Some("locally-signed-simulation".into()),
                error: None,
                simulation: None,
            };
            let (status, signature, _) = map_status(Ok(result), true);
            assert_eq!(
                status,
                if success {
                    crate::models::TradeStatus::Simulated
                } else {
                    crate::models::TradeStatus::Failed
                }
            );
            assert!(signature.is_none());
        }
    }

    #[test]
    fn live_signature_without_visible_status_remains_unknown() {
        let result = ExecuteResult {
            success: false,
            signature: Some("submitted-signature".into()),
            error: Some("submit signature not visible after 500ms; reconcile order status".into()),
            simulation: None,
        };
        let (status, signature, error) = map_status(Ok(result), false);
        assert_eq!(status, crate::models::TradeStatus::Unknown);
        assert_eq!(signature.as_deref(), Some("submitted-signature"));
        assert!(error.unwrap().contains("reconcile order status"));
    }

    #[test]
    fn live_sdk_error_without_signature_remains_unknown() {
        let (status, signature, _) = map_status(
            Err(anyhow::anyhow!("lane request timed out after send")),
            false,
        );
        assert_eq!(status, crate::models::TradeStatus::Unknown);
        assert!(signature.is_none());
    }

    #[test]
    fn live_failed_send_without_signature_remains_unknown() {
        let result = ExecuteResult {
            success: false,
            signature: None,
            error: Some("sendTransaction failed after transport timeout".into()),
            simulation: None,
        };
        let (status, signature, _) = map_status(Ok(result), false);
        assert_eq!(status, crate::models::TradeStatus::Unknown);
        assert!(signature.is_none());
    }
}
