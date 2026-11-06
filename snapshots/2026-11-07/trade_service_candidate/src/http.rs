use anyhow::{bail, Context, Result as AnyResult};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use tokio::time::{sleep, Duration, Instant};
use tracing::{info, warn};

use crate::aggregator::{
    AggregatorQuote, AggregatorQuoteRequest, ProviderId, WalletBalanceSnapshot,
};
use crate::aggregator_quote::{
    self, SolBuyQuoteRequest, SolBuySubmitRequest, SolBuySubmitResponse, SubmissionReservation,
};
use crate::aggregator_service::{self, QuoteCreationError, SubmissionError};
use crate::auth;
use crate::backend::types::{
    ObservedSolBridge, QuoteRequest, ResolvedRoute, RouteHint, TradePlan, VerifiedViaSolRoute,
    ViaSolRouteRequest,
};
use crate::config::WalletConfig;
use crate::conversion::{self, WsolQuoteRequest, WsolSubmission};
use crate::executor;
use crate::idempotency::ReserveResult;
use crate::models::{
    BatchTradeItemResponse, BatchTradeRequest, BatchTradeResponse, CreateVanityJobRequest,
    CreateWalletRequest, ErrorResponse, ExportWalletResponse, HealthResponse,
    ManagedWalletResponse, ManagedWalletsResponse, QuoteBalanceResponse, ReadyResponse,
    ResolvePoolRequest, ResolvePoolResponse, ResolveSolHopRequest, ResolveSolHopResponse,
    SolTransferAmountMode, SolTransferRequest, SolTransferResponse, TradeRequest, TradeResponse,
    TradeStatus, UpdateWalletRequest, WalletBalanceItem, WalletBalancesResponse, WalletsResponse,
    WarmupRequest, WarmupResponse,
};
use crate::persistence::DurableReserve;
use crate::route_registry::{ObservedSdkPreviewSnapshot, SdkTradeDecisionSnapshot};
use crate::sanitize::redact_urls;
use crate::state::{cache_key, new_request_id};
use crate::validation::{validate_trade, validate_warmup};
use crate::wallet_repository::{ManagedWallet, SolTransferRecord};
use crate::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/aggregators/capabilities", get(aggregator_capabilities))
        .route(
            "/v1/aggregators/quotes/compare",
            post(compare_aggregator_quotes),
        )
        .route(
            "/v1/aggregators/shadow/metrics",
            get(aggregator_shadow_metrics),
        )
        .route("/v1/wallets", get(wallets).post(create_wallet))
        .route("/v1/wallets/manage", get(managed_wallets))
        .route(
            "/v1/wallets/:wallet_id",
            patch(update_wallet).delete(delete_wallet),
        )
        .route("/v1/wallets/:wallet_id/export", post(export_wallet))
        .route("/v1/cache/warmup", post(warmup))
        .route("/v1/pools/resolve", post(resolve_pool))
        .route("/v1/pools/resolve-candidates", post(resolve_candidates))
        .route("/v1/routes/decide", post(decide_sdk_route))
        .route(
            "/v1/routes/observations/sdk-preview",
            post(preview_observed_sdk_route),
        )
        .route(
            "/v1/routes/observations/sdk-preview/readback",
            post(readback_observed_sdk_preview),
        )
        .route(
            "/v1/routes/cpmm-sell-preview",
            post(preview_cpmm_sell_route),
        )
        .route(
            "/v1/pools/resolve-via-sol-candidates",
            post(resolve_via_sol_candidates),
        )
        .route("/v1/routes/quote", post(route_quote))
        .route("/v1/routes/simulate", post(route_simulate))
        .route("/v1/pools/sol-hop", post(resolve_sol_hop))
        .route("/v1/quotes/sol-buy", post(sol_buy_quote))
        .route("/internal/conversions/wsol/quotes", post(wsol_quote))
        .route("/internal/conversions/wsol/simulate", post(wsol_simulate))
        .route("/internal/conversions/wsol/submit", post(wsol_submit))
        .route(
            "/internal/conversions/wsol/submissions/:submission_id",
            get(wsol_status),
        )
        .route("/v1/quotes/sol-buy/:quote_id", get(get_sol_buy_quote))
        .route(
            "/v1/quotes/sol-buy/:quote_id/preflight",
            post(preflight_sol_buy_quote),
        )
        .route(
            "/v1/quotes/sol-buy/:quote_id/submit",
            post(submit_sol_buy_quote),
        )
        .route(
            "/v1/quotes/sol-buy/:quote_id/submissions/:idempotency_key",
            get(get_sol_buy_submission),
        )
        .route("/v1/trades", post(create_trade).get(list_trades))
        .route("/v1/batch/trades", post(create_batch_trades))
        .route("/v1/trades/:request_id", get(get_trade))
        .route("/internal/wallet-balances", get(wallet_balances))
        .route(
            "/internal/wallets/:wallet_id/quote-balance/:mint",
            get(quote_balance),
        )
        .route("/internal/wallets/vanity-jobs", post(create_vanity_job))
        .route(
            "/internal/wallets/vanity-jobs/:job_id",
            get(get_vanity_job).delete(cancel_vanity_job),
        )
        .route(
            "/internal/wallets/:wallet_id/transfer-sol",
            post(transfer_sol),
        )
        .with_state(state)
}

async fn capabilities(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    Json(crate::protocol_capabilities::current(
        state.config.stonkfun_via_sol.enabled,
        state.config.simulate_only,
    ))
    .into_response()
}

#[derive(Debug, Serialize)]
struct AggregatorCapabilitiesResponse {
    schema_version: u32,
    simulate_only: bool,
    providers: Vec<AggregatorProviderCapability>,
}

#[derive(Debug, Serialize)]
struct AggregatorProviderCapability {
    provider: crate::aggregator::ProviderId,
    supports_quote: bool,
    supports_build: bool,
    supports_preflight: bool,
    supports_submit: bool,
    read_only: bool,
}

async fn aggregator_capabilities(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let providers = state
        .aggregators
        .capabilities()
        .into_iter()
        .map(|(provider, capabilities)| AggregatorProviderCapability {
            provider,
            supports_quote: capabilities.supports_quote,
            supports_build: capabilities.supports_build,
            supports_preflight: capabilities.supports_preflight,
            supports_submit: capabilities.supports_submit,
            read_only: capabilities.read_only,
        })
        .collect();
    Json(AggregatorCapabilitiesResponse {
        schema_version: 1,
        simulate_only: state.config.simulate_only,
        providers,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
struct AggregatorCompareRequest {
    wallet_id: String,
    mint: String,
    amount_lamports: u64,
    slippage_bps: u16,
    #[serde(default)]
    allowed_aggregators: Option<Vec<ProviderId>>,
}

#[derive(Debug, Serialize)]
struct AggregatorCompareResponse {
    schema_version: u32,
    shadow_only: bool,
    results: Vec<AggregatorCompareResult>,
}

#[derive(Debug, Deserialize)]
struct AggregatorShadowMetricsQuery {
    wallet_id: Option<String>,
    provider: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct AggregatorShadowMetricsResponse {
    schema_version: u32,
    read_only: bool,
    metrics: Vec<aggregator_quote::AggregatorShadowMetric>,
    summary: aggregator_quote::AggregatorShadowSummary,
}

async fn aggregator_shadow_metrics(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<AggregatorShadowMetricsQuery>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if query
        .wallet_id
        .as_deref()
        .is_some_and(|value| value.len() > 128)
    {
        return error(StatusCode::BAD_REQUEST, "wallet_id is too long");
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    match state
        .aggregator_quotes
        .shadow_metrics(query.wallet_id.as_deref(), query.provider.as_deref(), limit)
        .await
    {
        Ok(metrics) => match state
            .aggregator_quotes
            .shadow_summary(query.wallet_id.as_deref(), query.provider.as_deref())
            .await
        {
            Ok(summary) => Json(AggregatorShadowMetricsResponse {
                schema_version: 1,
                read_only: true,
                metrics,
                summary,
            })
            .into_response(),
            Err(err) => error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
        },
        Err(err) => error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
    }
}

#[derive(Debug, Serialize)]
struct AggregatorCompareResult {
    provider: ProviderId,
    status: String,
    latency_ms: u64,
    quote: Option<AggregatorQuote>,
    error: Option<String>,
}

fn validate_aggregator_compare_request(
    request: &AggregatorCompareRequest,
) -> Result<(), &'static str> {
    if request.wallet_id.trim().is_empty() || request.wallet_id.len() > 128 {
        return Err("wallet_id is required");
    }
    if request.amount_lamports == 0 {
        return Err("amount_lamports must be greater than zero");
    }
    if request.slippage_bps == 0 || request.slippage_bps > 3000 {
        return Err("slippage_bps must be between 1 and 3000");
    }
    if request.mint.parse::<solana_sdk::pubkey::Pubkey>().is_err() {
        return Err("mint must be a valid Solana public key");
    }
    if request
        .allowed_aggregators
        .as_ref()
        .is_some_and(Vec::is_empty)
    {
        return Err("allowed_aggregators must not be empty");
    }
    Ok(())
}

fn selected_compare_providers(allowed: Option<Vec<ProviderId>>) -> Vec<ProviderId> {
    allowed.unwrap_or_else(|| vec![ProviderId::Jupiter])
}

fn validate_compare_quote(
    quote: &AggregatorQuote,
    provider: ProviderId,
    request: &AggregatorQuoteRequest,
) -> AnyResult<()> {
    const MAX_COMPARE_QUOTE_TTL_MS: u64 = 60_000;
    if quote.provider != provider {
        bail!("provider quote binding mismatch");
    }
    if quote.wallet_id != request.wallet_id || quote.wallet_pubkey != request.wallet_pubkey {
        bail!("provider quote wallet binding mismatch");
    }
    if quote.input_mint != request.input_mint
        || quote.output_mint != request.output_mint
        || quote.input_amount_units != request.input_amount_units
        || quote.slippage_bps != request.slippage_bps
    {
        bail!("provider quote request binding mismatch");
    }
    if quote.balance_snapshot != request.balance_snapshot {
        bail!("provider quote balance snapshot binding mismatch");
    }
    if !(1..=3000).contains(&request.slippage_bps) {
        bail!("provider quote request slippage is invalid");
    }
    if quote.provider_quote_id.trim().is_empty() || quote.provider_quote_id.len() > 128 {
        bail!("provider quote ID is missing or too long");
    }
    let now_ms = crate::aggregator_quote::current_time_ms();
    if quote.expires_at_ms <= now_ms {
        bail!("provider quote expired");
    }
    if quote.expires_at_ms - now_ms > MAX_COMPARE_QUOTE_TTL_MS {
        bail!("provider quote expiry exceeds compare limit");
    }
    if quote.expected_output_units == 0
        || quote.minimum_output_units == 0
        || quote.expected_output_units < quote.minimum_output_units
    {
        bail!("provider quote has invalid output amount");
    }
    let minimum_allowed = (quote.expected_output_units as u128)
        .saturating_mul(10_000 - request.slippage_bps as u128)
        / 10_000;
    if (quote.minimum_output_units as u128) < minimum_allowed {
        bail!("provider quote exceeds requested slippage");
    }
    validate_route_continuity(quote)?;
    let route_fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&quote.route)?));
    if quote.route_fingerprint != route_fingerprint {
        bail!("provider quote route fingerprint mismatch");
    }
    let request_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(request)?));
    if quote.request_hash != request_hash {
        bail!("provider quote request hash mismatch");
    }
    Ok(())
}

fn validate_route_continuity(quote: &AggregatorQuote) -> AnyResult<()> {
    if quote.route.len() > 4 {
        bail!("provider quote route is too long");
    }
    let Some(first) = quote.route.first() else {
        bail!("provider quote route is empty");
    };
    if first.input_mint != quote.input_mint {
        bail!("provider quote route input mismatch");
    }
    let mut previous_output = first.output_mint.as_str();
    for leg in &quote.route {
        if leg.label.trim().is_empty()
            || leg
                .input_mint
                .parse::<solana_sdk::pubkey::Pubkey>()
                .is_err()
            || leg
                .output_mint
                .parse::<solana_sdk::pubkey::Pubkey>()
                .is_err()
        {
            bail!("provider quote route has invalid leg");
        }
    }
    for leg in quote.route.iter().skip(1) {
        if leg.input_mint != previous_output {
            bail!("provider quote route is discontinuous");
        }
        previous_output = &leg.output_mint;
    }
    if previous_output != quote.output_mint {
        bail!("provider quote route output mismatch");
    }
    Ok(())
}

enum CompareQuoteOutcome {
    Quoted(AggregatorQuote),
    TimedOut,
    Failed,
}

async fn gather_compare_results(
    aggregators: std::sync::Arc<crate::aggregator::AggregatorRegistry>,
    providers: Vec<ProviderId>,
    quote_request: &AggregatorQuoteRequest,
) -> Vec<AggregatorCompareResult> {
    let mut results = Vec::with_capacity(providers.len());
    let mut jobs = Vec::new();
    for provider_id in providers {
        if aggregators
            .get(provider_id)
            .is_none_or(|provider| !provider.capabilities().supports_quote)
        {
            results.push(AggregatorCompareResult {
                provider: provider_id,
                status: "unavailable".to_string(),
                latency_ms: 0,
                quote: None,
                error: Some("provider quote capability is unavailable".to_string()),
            });
            continue;
        }
        let aggregators = aggregators.clone();
        let request = quote_request.clone();
        let job = tokio::spawn(async move {
            let started = Instant::now();
            let outcome = match tokio::time::timeout(
                Duration::from_secs(3),
                aggregators.quote(provider_id, &request),
            )
            .await
            {
                Ok(Ok(quote)) => CompareQuoteOutcome::Quoted(quote),
                Ok(Err(_)) => CompareQuoteOutcome::Failed,
                Err(_) => CompareQuoteOutcome::TimedOut,
            };
            (started.elapsed().as_millis() as u64, outcome)
        });
        jobs.push((provider_id, job));
    }
    // Handles retain the provider ID even when a task panics, so every
    // selected provider contributes exactly one result and Shadow metric.
    for (provider, job) in jobs {
        let (latency_ms, outcome) = match job.await {
            Ok(result) => result,
            Err(_) => (0, CompareQuoteOutcome::Failed),
        };
        let (status, quote, error) = match outcome {
            CompareQuoteOutcome::Quoted(quote) => {
                match validate_compare_quote(&quote, provider, quote_request) {
                    Ok(()) => ("quoted", Some(quote), None),
                    Err(_) => ("failed", None, Some("provider quote validation failed")),
                }
            }
            CompareQuoteOutcome::TimedOut => ("failed", None, Some("provider quote timed out")),
            CompareQuoteOutcome::Failed => ("failed", None, Some("provider quote failed")),
        };
        results.push(AggregatorCompareResult {
            provider,
            status: status.to_string(),
            latency_ms,
            quote,
            error: error.map(str::to_string),
        });
    }
    results.sort_by_key(|result| result.provider.as_str());
    results
}

async fn compare_aggregator_quotes(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<AggregatorCompareRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if let Err(message) = validate_aggregator_compare_request(&request) {
        return error(StatusCode::BAD_REQUEST, message);
    }
    let Some(wallet) = state.wallets.get(&request.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    let selected = selected_compare_providers(request.allowed_aggregators);
    if selected.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "allowed_aggregators must not be empty",
        );
    }
    let mut unique = Vec::new();
    for provider in selected {
        if !unique.contains(&provider) {
            unique.push(provider);
        }
    }
    let balance_snapshot = if unique.iter().any(|provider_id| {
        state
            .aggregators
            .get(*provider_id)
            .is_some_and(|provider| provider.capabilities().supports_quote)
    }) {
        let balance_lamports = match wallet.wallet_balance().await {
            Ok(balance) => balance,
            Err(err) => {
                let err = redact_urls(&err.to_string());
                tracing::warn!(error = %err, "wallet balance snapshot unavailable for aggregator compare");
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "wallet balance unavailable",
                );
            }
        };
        Some(WalletBalanceSnapshot {
            balance_lamports,
            fetched_at_ms: aggregator_quote::current_time_ms(),
        })
    } else {
        None
    };
    let quote_request = AggregatorQuoteRequest {
        wallet_id: request.wallet_id.clone(),
        wallet_pubkey: wallet.wallet().to_string(),
        input_mint: crate::models::WSOL_MINT.to_string(),
        output_mint: request.mint.clone(),
        input_amount_units: request.amount_lamports,
        slippage_bps: request.slippage_bps,
        balance_snapshot,
    };
    let results = gather_compare_results(state.aggregators.clone(), unique, &quote_request).await;
    for result in &results {
        let error_class = result.error.as_deref().map(|_| result.status.as_str());
        if let Err(err) = state
            .aggregator_quotes
            .record_shadow_metric(
                result.provider.as_str(),
                &request.wallet_id,
                crate::models::WSOL_MINT,
                &request.mint,
                request.amount_lamports,
                request.slippage_bps,
                &result.status,
                result.latency_ms,
                result.quote.as_ref(),
                error_class,
            )
            .await
        {
            let err = redact_urls(&err.to_string());
            tracing::warn!(error = %err, provider = %result.provider, "failed to persist aggregator shadow metric");
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "aggregator shadow metric persistence unavailable",
            );
        }
    }
    Json(AggregatorCompareResponse {
        schema_version: 1,
        shadow_only: true,
        results,
    })
    .into_response()
}

async fn wsol_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WsolQuoteRequest>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    match conversion::quote_wsol(&state, request).await {
        Ok(quote) => Json(quote).into_response(),
        Err(err) => error(
            StatusCode::BAD_REQUEST,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
}

async fn wsol_simulate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WsolSubmission>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    match conversion::simulate_wsol(&state, request).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => error(
            StatusCode::BAD_REQUEST,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
}

async fn wsol_submit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WsolSubmission>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    if state.config.simulate_only && !request.simulate {
        return error(
            StatusCode::FORBIDDEN,
            "this Trade Service instance only accepts simulations",
        );
    }
    match conversion::submit_wsol(&state, request).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => error(
            StatusCode::BAD_REQUEST,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
}

async fn wsol_status(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(submission_id): Path<String>,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    match conversion::get_wsol_status(&state, &submission_id, &owner.owner_type, &owner.owner_id)
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(err) => error(
            StatusCode::BAD_REQUEST,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct OwnerQuery {
    owner_type: String,
    owner_id: String,
}

async fn quote_balance(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((wallet_id, mint)): Path<(String, String)>,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    let wallet = match state.wallet_repository.get(&wallet_id).await {
        Ok(wallet) => wallet,
        Err(error_value) => return error(StatusCode::NOT_FOUND, error_value.to_string()),
    };
    if !wallet.enabled
        || !((wallet.owner_type == owner.owner_type && wallet.owner_id == owner.owner_id)
            || (wallet.owner_type == "system" && wallet.owner_id == "global"))
    {
        return error(StatusCode::FORBIDDEN, "wallet owner mismatch");
    }
    let mint = match mint.parse::<solana_sdk::pubkey::Pubkey>() {
        Ok(mint) => mint,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid quote mint"),
    };
    let Some(client) = state.wallets.get(&wallet_id) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "wallet is not loaded");
    };
    match client.quote_balance(&mint).await {
        Ok((decimals, balance_units, balance_lamports)) => Json(QuoteBalanceResponse {
            wallet_id,
            quote_mint: mint.to_string(),
            decimals,
            balance_units,
            balance_lamports,
        })
        .into_response(),
        Err(error_value) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&error_value.to_string()),
        ),
    }
}

async fn wallet_balances(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    let visible = match state.wallet_repository.list().await {
        Ok(wallets) => wallets
            .into_iter()
            .filter(|wallet| {
                wallet.enabled
                    && ((wallet.owner_type == owner.owner_type
                        && wallet.owner_id == owner.owner_id)
                        || (wallet.owner_type == "system" && wallet.owner_id == "global"))
            })
            .collect::<Vec<_>>(),
        Err(error_value) => return error(StatusCode::SERVICE_UNAVAILABLE, error_value.to_string()),
    };
    let pubkeys = match visible
        .iter()
        .map(|wallet| wallet.pubkey.parse())
        .collect::<Result<Vec<solana_sdk::pubkey::Pubkey>, _>>()
    {
        Ok(pubkeys) => pubkeys,
        Err(error_value) => {
            return error(StatusCode::INTERNAL_SERVER_ERROR, error_value.to_string())
        }
    };
    let balances = state.wallets.default().wallet_balances(&pubkeys).await;
    let items = match balances {
        Ok(values) => visible
            .into_iter()
            .zip(values)
            .map(|(wallet, balance)| WalletBalanceItem {
                wallet_id: wallet.wallet_id,
                pubkey: wallet.pubkey,
                balance_lamports: Some(balance),
                error: None,
            })
            .collect(),
        Err(error_value) => {
            let message = redact_urls(&error_value.to_string());
            visible
                .into_iter()
                .map(|wallet| WalletBalanceItem {
                    wallet_id: wallet.wallet_id,
                    pubkey: wallet.pubkey,
                    balance_lamports: None,
                    error: Some(message.clone()),
                })
                .collect()
        }
    };
    Json(WalletBalancesResponse {
        fetched_at_ms: crate::state::now_ms(),
        wallets: items,
    })
    .into_response()
}

async fn create_vanity_job(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CreateVanityJobRequest>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    match state.wallet_ops.create_vanity_job(request).await {
        Ok(job) => (StatusCode::ACCEPTED, Json(job)).into_response(),
        Err(error_value) => error(StatusCode::BAD_REQUEST, error_value.to_string()),
    }
}

async fn get_vanity_job(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    match state.wallet_ops.get_vanity_job(&job_id).await {
        Ok(job) if job.owner_type == owner.owner_type && job.owner_id == owner.owner_id => {
            Json(job).into_response()
        }
        Ok(_) => error(
            StatusCode::FORBIDDEN,
            "wallet generation job owner mismatch",
        ),
        Err(error_value) => error(StatusCode::NOT_FOUND, error_value.to_string()),
    }
}

async fn cancel_vanity_job(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    match state
        .wallet_ops
        .cancel_vanity_job(&job_id, &owner.owner_type, &owner.owner_id)
        .await
    {
        Ok(job) => Json(job).into_response(),
        Err(error_value) => error(StatusCode::BAD_REQUEST, error_value.to_string()),
    }
}

async fn transfer_sol(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
    Json(request): Json<SolTransferRequest>,
) -> Response {
    if let Err(response) = authorize_internal(&state, &headers, peer) {
        return response;
    }
    if state.config.simulate_only {
        return error(
            StatusCode::FORBIDDEN,
            "SOL transfers are disabled in simulation-only mode",
        );
    }
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return error(
            StatusCode::BAD_REQUEST,
            "idempotency_key must be 1..128 characters",
        );
    }
    let wallet = match state.wallet_repository.get(&wallet_id).await {
        Ok(wallet) => wallet,
        Err(error_value) => return error(StatusCode::NOT_FOUND, error_value.to_string()),
    };
    let source_owned =
        wallet.owner_type == request.owner_type && wallet.owner_id == request.owner_id;
    let source_system = state.config.allow_internal_system_wallet_transfer
        && wallet.owner_type == "system"
        && wallet.owner_id == "global";
    if !wallet.enabled || (!source_owned && !source_system) {
        return error(
            StatusCode::FORBIDDEN,
            "source wallet is disabled or does not belong to this owner",
        );
    }
    let destination = match (&request.to_wallet_id, &request.to_address) {
        (Some(target_id), None) => match state.wallet_repository.get(target_id).await {
            Ok(target)
                if target.enabled
                    && ((target.owner_type == request.owner_type
                        && target.owner_id == request.owner_id)
                        || (state.config.allow_internal_system_wallet_transfer
                            && target.owner_type == "system"
                            && target.owner_id == "global")) =>
            {
                target.pubkey
            }
            Ok(_) => {
                return error(
                    StatusCode::FORBIDDEN,
                    "target wallet does not belong to this owner",
                )
            }
            Err(error_value) => return error(StatusCode::BAD_REQUEST, error_value.to_string()),
        },
        (None, Some(address)) => address.trim().to_string(),
        _ => return error(StatusCode::BAD_REQUEST, "provide exactly one destination"),
    };
    let destination_pubkey = match destination.parse::<solana_sdk::pubkey::Pubkey>() {
        Ok(pubkey) => pubkey,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "destination is not a valid Solana address",
            )
        }
    };
    let (fixed_lamports, percent) = match request.amount_mode {
        SolTransferAmountMode::Fixed
            if request.amount_lamports.unwrap_or(0) > 0 && request.percent.is_none() =>
        {
            (request.amount_lamports, None)
        }
        SolTransferAmountMode::Percent
            if matches!(request.percent, Some(50 | 100)) && request.amount_lamports.is_none() =>
        {
            (None, request.percent)
        }
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "fixed requires amount_lamports; percent supports only 50 or 100",
            )
        }
    };
    let transfer_id = format!("transfer_{}", uuid::Uuid::new_v4().simple());
    let record = SolTransferRecord {
        transfer_id: transfer_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        owner_type: request.owner_type.clone(),
        owner_id: request.owner_id.clone(),
        wallet_id: wallet_id.clone(),
        from_pubkey: wallet.pubkey.clone(),
        to_pubkey: destination_pubkey.to_string(),
        amount_mode: request.amount_mode.as_str().to_string(),
        requested_amount_lamports: fixed_lamports,
        requested_percent: percent,
        actual_amount_lamports: None,
        fee_lamports: None,
        balance_before_lamports: None,
        status: "processing".to_string(),
        signature: None,
        error: None,
    };
    let reserved = match state.wallet_repository.reserve_sol_transfer(&record).await {
        Ok(value) => value,
        Err(error_value) => return error(StatusCode::SERVICE_UNAVAILABLE, error_value.to_string()),
    };
    if !reserved {
        return match state
            .wallet_repository
            .get_sol_transfer_by_idempotency(&request.idempotency_key)
            .await
        {
            Ok(existing)
                if existing.owner_type == request.owner_type
                    && existing.owner_id == request.owner_id =>
            {
                Json(sol_transfer_response(existing, true)).into_response()
            }
            Ok(_) => error(
                StatusCode::CONFLICT,
                "idempotency key belongs to another owner",
            ),
            Err(error_value) => error(StatusCode::SERVICE_UNAVAILABLE, error_value.to_string()),
        };
    }
    let client = match state.wallets.get(&wallet_id) {
        Some(client) => client,
        None => {
            let _ = state
                .wallet_repository
                .finish_sol_transfer(
                    &transfer_id,
                    "failed",
                    None,
                    None,
                    None,
                    None,
                    Some("source wallet is not loaded"),
                )
                .await;
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "source wallet is not loaded",
            );
        }
    };
    match client
        .transfer_sol(&destination_pubkey, fixed_lamports, percent)
        .await
    {
        Ok(result) => {
            let _ = state
                .wallet_repository
                .finish_sol_transfer(
                    &transfer_id,
                    &result.status,
                    Some(result.amount_lamports),
                    Some(result.fee_lamports),
                    Some(result.balance_before_lamports),
                    Some(&result.signature),
                    None,
                )
                .await;
        }
        Err(error_value) => {
            let _ = state
                .wallet_repository
                .finish_sol_transfer(
                    &transfer_id,
                    "failed",
                    None,
                    None,
                    None,
                    None,
                    Some(&redact_urls(&error_value.to_string())),
                )
                .await;
        }
    }
    match state
        .wallet_repository
        .get_sol_transfer_by_idempotency(&request.idempotency_key)
        .await
    {
        Ok(record) => Json(sol_transfer_response(record, false)).into_response(),
        Err(error_value) => error(StatusCode::SERVICE_UNAVAILABLE, error_value.to_string()),
    }
}

fn sol_transfer_response(record: SolTransferRecord, duplicate: bool) -> SolTransferResponse {
    SolTransferResponse {
        transfer_id: record.transfer_id,
        wallet_id: record.wallet_id,
        from_pubkey: record.from_pubkey,
        to_pubkey: record.to_pubkey,
        status: record.status,
        amount_lamports: record.actual_amount_lamports,
        fee_lamports: record.fee_lamports,
        balance_before_lamports: record.balance_before_lamports,
        signature: record.signature,
        error: record.error,
        duplicate,
    }
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn ready(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let has_blockhash = state
        .blockhash_cache
        .get_fresh(state.config.blockhash_max_age_ms)
        .is_some();
    let default_wallet = state.wallets.default();
    let rpc_ok = default_wallet.ping_rpc().await;
    Json(ReadyResponse {
        ready: has_blockhash && rpc_ok,
        wallet: default_wallet.wallet().to_string(),
        default_wallet_id: state.wallets.default_wallet_id().to_string(),
        wallet_count: state.wallets.wallet_count(),
        wallets: state.wallets.infos(),
        has_blockhash,
        blockhash_age_ms: state.blockhash_cache.age_ms(),
        rpc_ok,
        paused: false,
        rpc_lanes: default_wallet.rpc_count(),
        swqos_lanes: default_wallet.swqos_lanes(),
        active_swqos_lanes: default_wallet.active_swqos_lanes(),
        mev_protection: default_wallet.mev_protection(),
        require_mev_submit: default_wallet.require_mev_submit(),
        pumpapi_executor: state.config.pumpapi.enabled,
        trade_persistence_mode: format!("{:?}", state.persistence.mode()).to_ascii_lowercase(),
        trade_database_healthy: state.persistence.healthy().await,
    })
    .into_response()
}

async fn wallets(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    (
        StatusCode::OK,
        Json(WalletsResponse {
            default_wallet_id: state.wallets.default_wallet_id().to_string(),
            wallets: state.wallets.infos(),
        }),
    )
        .into_response()
}

async fn managed_wallets(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    match state.wallet_repository.list().await {
        Ok(wallets) => (
            StatusCode::OK,
            Json(ManagedWalletsResponse {
                wallets: wallets
                    .into_iter()
                    .map(|wallet| managed_wallet_response(wallet, false))
                    .collect(),
            }),
        )
            .into_response(),
        Err(err) => error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
    }
}

async fn create_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CreateWalletRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let result = match request.private_key_base58.as_deref() {
        Some(private_key) => state
            .wallet_repository
            .import(
                &request.owner_type,
                &request.owner_id,
                &request.label,
                private_key,
            )
            .await
            .map(|(wallet, created)| (wallet, private_key.to_string(), created)),
        None => state
            .wallet_repository
            .create(&request.owner_type, &request.owner_id, &request.label)
            .await
            .map(|(wallet, private_key)| (wallet, private_key, true)),
    };
    let (wallet, private_key, created) = match result {
        Ok(value) => value,
        Err(err) => return error(StatusCode::BAD_REQUEST, err.to_string()),
    };
    if wallet.enabled {
        let config = WalletConfig {
            wallet_id: wallet.wallet_id.clone(),
            private_key_base58: private_key,
        };
        if let Err(err) = state.wallets.add(&config).await {
            if created {
                let _ = state.wallet_repository.disable(&wallet.wallet_id).await;
            }
            return error(StatusCode::BAD_GATEWAY, err.to_string());
        }
    }
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    (status, Json(managed_wallet_response(wallet, created))).into_response()
}

async fn update_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
    Json(request): Json<UpdateWalletRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    match state
        .wallet_repository
        .rename(&wallet_id, request.label.trim())
        .await
    {
        Ok(wallet) => {
            (StatusCode::OK, Json(managed_wallet_response(wallet, false))).into_response()
        }
        Err(err) => error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

async fn delete_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let wallet = match state.wallet_repository.disable(&wallet_id).await {
        Ok(wallet) => wallet,
        Err(err) => return error(StatusCode::BAD_REQUEST, err.to_string()),
    };
    if let Err(err) = state.wallets.remove(&wallet_id) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }
    (StatusCode::OK, Json(managed_wallet_response(wallet, false))).into_response()
}

async fn export_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    match state.wallet_repository.export(&wallet_id).await {
        Ok(private_key_base58) => (
            StatusCode::OK,
            Json(ExportWalletResponse {
                wallet_id,
                private_key_base58,
            }),
        )
            .into_response(),
        Err(err) => error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

fn managed_wallet_response(wallet: ManagedWallet, created: bool) -> ManagedWalletResponse {
    ManagedWalletResponse {
        wallet_id: wallet.wallet_id,
        owner_type: wallet.owner_type,
        owner_id: wallet.owner_id,
        label: wallet.label,
        pubkey: wallet.pubkey,
        is_default: wallet.is_default,
        enabled: wallet.enabled,
        created,
    }
}

async fn warmup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WarmupRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }

    let normalized = match validate_warmup(&request) {
        Ok(normalized) => normalized,
        Err(err) => return error(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let warmup_hint = crate::backend::types::RouteHint {
        protocol_id: Some(crate::backend::registry::protocol_id(request.pool).to_string()),
        program_id: None,
        pool_id: normalized.pool_id.map(|value| value.to_string()),
        mint: normalized.mint.to_string(),
        quote_mint: Some(normalized.quote_mint.to_string()),
    };
    if let Err(err) = state.execution_backend.warmup(warmup_hint).await {
        return error(StatusCode::BAD_GATEWAY, err.to_string());
    }

    let key = cache_key(request.pool, &request.mint, request.pool_id.as_deref());
    state
        .params
        .insert(key.clone(), request.pool, request.mint, request.pool_id);
    (
        StatusCode::OK,
        Json(WarmupResponse {
            ok: true,
            cache_key: key,
        }),
    )
        .into_response()
}

async fn resolve_pool(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ResolvePoolRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let mint = match request.mint.parse::<solana_sdk::pubkey::Pubkey>() {
        Ok(mint) => mint,
        Err(_) => return error(StatusCode::BAD_REQUEST, "mint must be a valid pubkey"),
    };
    match state
        .execution_backend
        .resolve_pool(crate::backend::types::RouteHint {
            protocol_id: None,
            program_id: None,
            pool_id: None,
            mint: mint.to_string(),
            quote_mint: None,
        })
        .await
    {
        Ok(resolved) => {
            let Some(pool) = crate::backend::registry::pool_for_protocol(&resolved.protocol_id)
            else {
                return error(StatusCode::BAD_GATEWAY, "backend returned unknown protocol");
            };
            let key = cache_key(pool, &resolved.mint, resolved.pool_id.as_deref());
            state
                .params
                .insert(key, pool, resolved.mint.clone(), resolved.pool_id.clone());
            let response = ResolvePoolResponse {
                mint: resolved.mint,
                pool,
                pool_id: resolved.pool_id,
                quote_mint: resolved.quote_mint.unwrap_or_default(),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(err) => error(StatusCode::BAD_GATEWAY, err.to_string()),
    }
}

async fn resolve_candidates(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<RouteHint>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if request.mint.parse::<solana_sdk::pubkey::Pubkey>().is_err() {
        return error(StatusCode::BAD_REQUEST, "route mint must be a valid pubkey");
    }
    let route = match state.execution_backend.resolve_pool(request).await {
        Ok(route) => route,
        Err(err) => return error(StatusCode::BAD_GATEWAY, err.to_string()),
    };
    if let Err(err) =
        crate::route_registry::validate_verified_route(&route, crate::state::now_ms() as u64)
    {
        return error(StatusCode::BAD_GATEWAY, err.to_string());
    }
    if let Err(err) = state
        .route_store
        .insert(route.clone(), crate::state::now_ms() as u64)
        .await
    {
        return error(StatusCode::BAD_GATEWAY, err.to_string());
    }
    Json(route).into_response()
}

#[derive(Serialize)]
struct RouteDecisionResponse {
    schema_version: u32,
    status: &'static str,
    reason: Option<&'static str>,
    wallet_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    wallet_pubkey: Option<String>,
    decision_id: Option<String>,
    route_id: Option<String>,
    request_fingerprint: Option<String>,
    expires_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_bridge: Option<ObservedSolBridge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdk_capability: Option<RouteSdkCapabilityEvidence>,
    simulate_only: bool,
}

#[derive(Serialize)]
struct RouteSdkCapabilityEvidence {
    package_builder_family: &'static str,
    service_adapter: &'static str,
    route_evidence: &'static str,
    sdk_unsupported_proven: bool,
    jupiter_candidate_allowed: bool,
}

fn route_decision_response(
    wallet_id: String,
    wallet_pubkey: Option<&str>,
    simulate_only: bool,
    status: &'static str,
    reason: Option<&'static str>,
    decision: Option<SdkTradeDecisionSnapshot>,
) -> Response {
    route_decision_response_with_observation(
        wallet_id,
        wallet_pubkey,
        simulate_only,
        status,
        reason,
        decision,
        None,
        None,
    )
}

fn route_decision_response_with_observation(
    wallet_id: String,
    wallet_pubkey: Option<&str>,
    simulate_only: bool,
    status: &'static str,
    reason: Option<&'static str>,
    decision: Option<SdkTradeDecisionSnapshot>,
    observed_bridge: Option<ObservedSolBridge>,
    sdk_capability: Option<RouteSdkCapabilityEvidence>,
) -> Response {
    Json(RouteDecisionResponse {
        schema_version: 1,
        status,
        reason,
        wallet_id,
        wallet_pubkey: wallet_pubkey.map(str::to_string),
        decision_id: decision.as_ref().map(|value| value.decision_id.clone()),
        route_id: decision.as_ref().map(|value| value.route_id.clone()),
        request_fingerprint: decision
            .as_ref()
            .map(|value| value.request_fingerprint.clone()),
        expires_at_ms: decision.map(|value| value.expires_at_ms),
        observed_bridge,
        sdk_capability,
        simulate_only,
    })
    .into_response()
}

async fn classify_sol_bridge_intent(
    state: &AppState,
    request: &TradeRequest,
    wallet: &crate::sdk_client::SdkTradeClient,
    wallet_id: &str,
    wallet_pubkey: &str,
    discovered_target: Option<ResolvedRoute>,
) -> (&'static str, Option<ObservedSolBridge>) {
    let target_hint = RouteHint {
        protocol_id: Some(crate::backend::registry::protocol_id(request.pool).to_string()),
        program_id: None,
        pool_id: request.pool_id.clone(),
        mint: request.mint.clone(),
        quote_mint: request.quote_mint.clone(),
    };
    let target_result = match discovered_target {
        Some(route) => Ok(route),
        None => state.execution_backend.resolve_pool(target_hint).await,
    };
    let target = match target_result {
        Ok(route)
            if crate::route_registry::validate_verified_route(
                &route,
                crate::state::now_ms() as u64,
            )
            .is_ok()
                && route.protocol_id == crate::backend::registry::protocol_id(request.pool)
                && route.mint == request.mint
                && request
                    .pool_id
                    .as_deref()
                    .is_none_or(|pool_id| route.pool_id.as_deref() == Some(pool_id))
                && request
                    .quote_mint
                    .as_deref()
                    .is_none_or(|quote_mint| route.quote_mint.as_deref() == Some(quote_mint)) =>
        {
            route
        }
        _ => return ("route_verification_unavailable", None),
    };
    let quote_mint = match target
        .quote_mint
        .as_deref()
        .and_then(|value| value.parse().ok())
    {
        Some(value) => value,
        None => return ("route_verification_unavailable", None),
    };
    let hop_pool_id = match wallet.resolve_wsol_quote_cpmm_hop(&quote_mint).await {
        Ok(value) => value,
        Err(_) => return ("bridge_route_not_resolved", None),
    };
    let hop_hint = RouteHint {
        protocol_id: Some("raydium-cpmm".to_string()),
        program_id: None,
        pool_id: Some(hop_pool_id.to_string()),
        mint: quote_mint.to_string(),
        quote_mint: Some(crate::models::WSOL_MINT.to_string()),
    };
    let hop = match state.execution_backend.resolve_pool(hop_hint).await {
        Ok(value) => value,
        Err(_) => return ("bridge_route_not_resolved", None),
    };
    let now_ms = crate::state::now_ms() as u64;
    let expires_at_ms =
        match crate::route_registry::validate_wsol_quote_cpmm_bridge(&target, &hop, now_ms) {
            Ok(value) => value,
            Err(_) => return ("bridge_route_not_resolved", None),
        };
    let request_fingerprint =
        match crate::route_registry::sol_bridge_request_fingerprint(request, wallet_pubkey) {
            Ok(value) => value,
            Err(_) => return ("bridge_snapshot_unavailable", None),
        };
    let observation = ObservedSolBridge {
        observation_id: format!("observe:sol-bridge:{}", uuid::Uuid::new_v4().simple()),
        wallet_id: wallet_id.to_string(),
        wallet_pubkey: wallet_pubkey.to_string(),
        request_fingerprint,
        target,
        sol_hop: hop,
        expires_at_ms,
    };
    if state
        .route_store
        .insert_observed_sol_bridge(observation.clone(), now_ms)
        .await
        .is_err()
    {
        return ("bridge_snapshot_unavailable", None);
    }
    // Bridge evidence alone cannot authorize an SDK, Router, or Jupiter trade.
    (
        "bridge_verified_sdk_capability_unresolved",
        Some(observation),
    )
}

#[derive(Deserialize)]
struct ObservedSdkPreviewRequest {
    observation_id: String,
    trade: TradeRequest,
    #[serde(default)]
    simulate_transaction: bool,
}

#[derive(Serialize)]
struct ObservedSdkPreviewResponse {
    status: &'static str,
    observation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_id: Option<String>,
    preview: crate::sdk_client::SdkTwoHopBuildPreview,
    executable: bool,
    simulate_only: bool,
}

#[derive(Deserialize)]
struct ObservedSdkPreviewReadbackRequest {
    preview_id: String,
    trade: TradeRequest,
}

#[derive(Serialize)]
struct ObservedSdkPreviewReadbackResponse {
    status: &'static str,
    preview_id: String,
    observation_id: String,
    wallet_id: String,
    input_lamports: u64,
    bridge_min_output_units: u64,
    target_min_output_units: u64,
    target_token_delta_units: u64,
    transaction_bytes: usize,
    transaction_version: &'static str,
    validated_slot: u64,
    simulation_slot: u64,
    expires_at_ms: u64,
    executable: bool,
    simulate_only: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CpmmSellPreviewRequest {
    wallet_id: String,
    target_pool_id: String,
    bridge_pool_id: String,
    target_mint: String,
    quote_mint: String,
    sell_percent: u64,
    slippage_bps: u64,
    #[serde(default)]
    unsigned_simulation: bool,
}

#[derive(Serialize)]
struct CpmmSellPreviewResponse {
    status: &'static str,
    wallet_id: String,
    wallet_pubkey: String,
    preview: crate::sdk_client::SdkTwoHopSellBuildPreview,
    simulate_only: bool,
    executable: bool,
}

async fn preview_cpmm_sell_route(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CpmmSellPreviewRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if !state.config.simulate_only {
        return error(
            StatusCode::FORBIDDEN,
            "SDK sell preview requires simulate-only mode",
        );
    }
    if request.wallet_id.trim().is_empty() || request.wallet_id.len() > 128 {
        return error(StatusCode::BAD_REQUEST, "wallet_id is required");
    }
    if !(1..=100).contains(&request.sell_percent) || !(1..=3_000).contains(&request.slippage_bps) {
        return error(StatusCode::BAD_REQUEST, "invalid sell percent or slippage");
    }
    let parse = |field: &'static str, value: &str| {
        value
            .parse::<solana_sdk::pubkey::Pubkey>()
            .map_err(|_| field)
    };
    let target_pool_id = match parse("target_pool_id", &request.target_pool_id) {
        Ok(value) => value,
        Err(field) => return error(StatusCode::BAD_REQUEST, format!("invalid {field}")),
    };
    let bridge_pool_id = match parse("bridge_pool_id", &request.bridge_pool_id) {
        Ok(value) => value,
        Err(field) => return error(StatusCode::BAD_REQUEST, format!("invalid {field}")),
    };
    let target_mint = match parse("target_mint", &request.target_mint) {
        Ok(value) => value,
        Err(field) => return error(StatusCode::BAD_REQUEST, format!("invalid {field}")),
    };
    let quote_mint = match parse("quote_mint", &request.quote_mint) {
        Ok(value) => value,
        Err(field) => return error(StatusCode::BAD_REQUEST, format!("invalid {field}")),
    };
    let Some(wallet) = state.wallets.get(&request.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    let wallet_pubkey = wallet.wallet().to_string();
    if state.wallet_repository.database_enabled() {
        match state.wallet_repository.get(&request.wallet_id).await {
            Ok(managed) if managed.enabled && managed.pubkey == wallet_pubkey => {}
            _ => return error(StatusCode::FORBIDDEN, "preview wallet has changed"),
        }
    }
    match wallet
        .preview_cpmm_sell_via_cpmm(
            &target_pool_id,
            &bridge_pool_id,
            &target_mint,
            &quote_mint,
            request.sell_percent,
            request.slippage_bps,
            request.unsigned_simulation,
        )
        .await
    {
        Ok(preview) => Json(CpmmSellPreviewResponse {
            status: "sdk_sell_build_preview_read_only",
            wallet_id: request.wallet_id,
            wallet_pubkey,
            preview,
            simulate_only: true,
            executable: false,
        })
        .into_response(),
        Err(error_value) => {
            tracing::warn!(error = %redact_urls(&error_value.to_string()), "SDK sell build preview unavailable");
            error(
                StatusCode::BAD_GATEWAY,
                "SDK sell build preview unavailable",
            )
        }
    }
}

async fn readback_observed_sdk_preview(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(mut body): Json<ObservedSdkPreviewReadbackRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let wallet_id = state
        .wallets
        .resolve_wallet_id(body.trade.wallet_id.as_deref());
    body.trade.wallet_id = Some(wallet_id.clone());
    let Some(wallet) = state.wallets.get(&wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "preview wallet is unavailable");
    };
    let wallet_pubkey = wallet.wallet().to_string();
    if state.wallet_repository.database_enabled() {
        match state.wallet_repository.get(&wallet_id).await {
            Ok(managed) if managed.enabled && managed.pubkey == wallet_pubkey => {}
            _ => return error(StatusCode::FORBIDDEN, "preview wallet has changed"),
        }
    }
    let preview = match state
        .route_store
        .get_observed_sdk_preview(&body.preview_id, crate::state::now_ms() as u64)
        .await
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "SDK preview is unavailable"),
    };
    let fingerprint =
        match crate::route_registry::sol_bridge_request_fingerprint(&body.trade, &wallet_pubkey) {
            Ok(value) => value,
            Err(_) => return error(StatusCode::BAD_REQUEST, "preview request is invalid"),
        };
    if preview.wallet_id != wallet_id
        || preview.wallet_pubkey != wallet_pubkey
        || preview.request_fingerprint != fingerprint
    {
        return error(StatusCode::BAD_REQUEST, "SDK preview does not match trade");
    }
    Json(ObservedSdkPreviewReadbackResponse {
        status: "sdk_preview_verified_read_only",
        preview_id: preview.preview_id,
        observation_id: preview.observation_id,
        wallet_id,
        input_lamports: preview.input_lamports,
        bridge_min_output_units: preview.bridge_min_output_units,
        target_min_output_units: preview.target_min_output_units,
        target_token_delta_units: preview.target_token_delta_units,
        transaction_bytes: preview.transaction_bytes,
        transaction_version: preview.transaction_version,
        validated_slot: preview.validated_slot,
        simulation_slot: preview.simulation_slot,
        expires_at_ms: preview.expires_at_ms,
        executable: false,
        simulate_only: state.config.simulate_only,
    })
    .into_response()
}

async fn preview_observed_sdk_route(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(mut body): Json<ObservedSdkPreviewRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let wallet_id = state
        .wallets
        .resolve_wallet_id(body.trade.wallet_id.as_deref());
    body.trade.wallet_id = Some(wallet_id.clone());
    let Some(wallet) = state.wallets.get(&wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "preview wallet is unavailable");
    };
    let wallet_pubkey = wallet.wallet().to_string();
    if state.wallet_repository.database_enabled() {
        match state.wallet_repository.get(&wallet_id).await {
            Ok(managed) if managed.enabled && managed.pubkey == wallet_pubkey => {}
            _ => return error(StatusCode::FORBIDDEN, "preview wallet has changed"),
        }
    }
    let mut observation = match state
        .route_store
        .get_observed_sol_bridge(&body.observation_id, crate::state::now_ms() as u64)
        .await
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "bridge observation is unavailable"),
    };
    let request_fingerprint =
        match crate::route_registry::sol_bridge_request_fingerprint(&body.trade, &wallet_pubkey) {
            Ok(value) => value,
            Err(_) => return error(StatusCode::BAD_REQUEST, "preview request is invalid"),
        };
    if observation.wallet_id != wallet_id
        || observation.wallet_pubkey != wallet_pubkey
        || observation.request_fingerprint != request_fingerprint
        || observation.target.protocol_id != crate::backend::registry::protocol_id(body.trade.pool)
        || !matches!(
            body.trade.pool,
            crate::models::Pool::Pump
                | crate::models::Pool::PumpAmm
                | crate::models::Pool::RaydiumAmmV4
                | crate::models::Pool::RaydiumCpmm
        )
    {
        return error(
            StatusCode::BAD_REQUEST,
            "preview does not match bridge observation",
        );
    }
    let Some(preview) = preview_observed_sdk_bridge(
        &state,
        &body.trade,
        &wallet,
        &observation,
        body.simulate_transaction,
    )
    .await
    else {
        return error(
            StatusCode::BAD_GATEWAY,
            "SDK preview unavailable for current pool state",
        );
    };
    let preview_id = if body.trade.pool == crate::models::Pool::Pump {
        // Pump.fun V3 token-quote preview remains diagnostic until its live
        // route and execution adapter have their own funded regression.
        None
    } else if let Some(simulation) = preview.simulation.as_ref().filter(|value| {
        value.attempted && value.rpc_accepted && value.ok && value.minimum_output_verified
    }) {
        let Some(target_token_delta_units) = simulation.target_token_delta_units else {
            return error(
                StatusCode::BAD_GATEWAY,
                "SDK simulation output is incomplete",
            );
        };
        let now_ms = crate::state::now_ms() as u64;
        if observation.expires_at_ms <= now_ms.saturating_add(2_000) {
            observation = match refresh_observed_bridge_after_preview(
                &state,
                &observation,
                &wallet_id,
                &wallet_pubkey,
                &request_fingerprint,
            )
            .await
            {
                Ok(value) => value,
                Err(_) => {
                    return error(
                        StatusCode::BAD_GATEWAY,
                        "SDK preview route refresh unavailable; resolve the route again",
                    )
                }
            };
        }
        let now_ms = crate::state::now_ms() as u64;
        let snapshot = ObservedSdkPreviewSnapshot {
            preview_id: format!("observe:sdk-preview:{}", uuid::Uuid::new_v4().simple()),
            observation_id: observation.observation_id.clone(),
            wallet_id: wallet_id.clone(),
            wallet_pubkey: wallet_pubkey.clone(),
            request_fingerprint: request_fingerprint.clone(),
            input_lamports: preview.input_lamports,
            bridge_min_output_units: preview.bridge_min_output_units,
            target_min_output_units: preview.target_min_output_units,
            target_token_delta_units,
            transaction_bytes: simulation.transaction_bytes,
            transaction_version: simulation.transaction_version,
            validated_slot: preview.validated_slot,
            simulation_slot: simulation.slot,
            expires_at_ms: observation.expires_at_ms.min(now_ms.saturating_add(5_000)),
        };
        if state
            .route_store
            .insert_observed_sdk_preview(snapshot.clone(), now_ms)
            .await
            .is_err()
        {
            return error(StatusCode::BAD_GATEWAY, "SDK preview snapshot unavailable");
        }
        Some(snapshot.preview_id)
    } else {
        None
    };
    Json(ObservedSdkPreviewResponse {
        status: "sdk_instruction_preview",
        observation_id: observation.observation_id,
        preview_id,
        preview,
        executable: false,
        simulate_only: state.config.simulate_only,
    })
    .into_response()
}

async fn refresh_observed_bridge_after_preview(
    state: &AppState,
    original: &ObservedSolBridge,
    wallet_id: &str,
    wallet_pubkey: &str,
    request_fingerprint: &str,
) -> anyhow::Result<ObservedSolBridge> {
    let target_hint = RouteHint {
        protocol_id: Some(original.target.protocol_id.clone()),
        program_id: original.target.program_id.clone(),
        pool_id: original.target.pool_id.clone(),
        mint: original.target.mint.clone(),
        quote_mint: original.target.quote_mint.clone(),
    };
    let hop_hint = RouteHint {
        protocol_id: Some(original.sol_hop.protocol_id.clone()),
        program_id: original.sol_hop.program_id.clone(),
        pool_id: original.sol_hop.pool_id.clone(),
        mint: original.sol_hop.mint.clone(),
        quote_mint: original.sol_hop.quote_mint.clone(),
    };
    let (target, sol_hop) = tokio::try_join!(
        state.execution_backend.resolve_pool(target_hint),
        state.execution_backend.resolve_pool(hop_hint),
    )?;
    if state.wallet_repository.database_enabled() {
        let managed = state.wallet_repository.get(wallet_id).await?;
        anyhow::ensure!(
            managed.enabled && managed.pubkey == wallet_pubkey,
            "SDK preview wallet changed during route refresh"
        );
    }
    let now_ms = crate::state::now_ms() as u64;
    let expires_at_ms =
        crate::route_registry::validate_wsol_quote_cpmm_bridge(&target, &sol_hop, now_ms)?;
    let refreshed = ObservedSolBridge {
        observation_id: format!("observe:sol-bridge:{}", uuid::Uuid::new_v4().simple()),
        wallet_id: wallet_id.to_string(),
        wallet_pubkey: wallet_pubkey.to_string(),
        request_fingerprint: request_fingerprint.to_string(),
        target,
        sol_hop,
        expires_at_ms,
    };
    crate::route_registry::validate_refreshed_sol_bridge(original, &refreshed, now_ms)?;
    state
        .route_store
        .insert_observed_sol_bridge(refreshed.clone(), now_ms)
        .await?;
    Ok(refreshed)
}

pub(crate) async fn preview_observed_sdk_bridge(
    state: &AppState,
    request: &TradeRequest,
    wallet: &crate::sdk_client::SdkTradeClient,
    observation: &ObservedSolBridge,
    simulate_transaction: bool,
) -> Option<crate::sdk_client::SdkTwoHopBuildPreview> {
    let attempt = async {
        let target_pool: Option<solana_sdk::pubkey::Pubkey> = observation
            .target
            .pool_id
            .as_deref()
            .map(str::parse)
            .transpose()?;
        let bridge_pool = observation
            .sol_hop
            .pool_id
            .as_deref()
            .context("bridge pool ID missing")?
            .parse()?;
        let target_mint = observation.target.mint.parse()?;
        let quote_mint = observation
            .target
            .quote_mint
            .as_deref()
            .context("target quote mint missing")?
            .parse()?;
        let amount = match (request.amount_lamports, request.amount_sol) {
            (Some(value), None) => value,
            (None, Some(value)) => crate::validation::token_to_units("amount_sol", value, 9)?,
            _ => anyhow::bail!("SOL amount is missing or ambiguous"),
        };
        let slippage_bps = crate::validation::normalize_slippage_bps(request, &state.config)?;
        match request.pool {
            crate::models::Pool::Pump => {
                wallet
                    .preview_pumpfun_v3_usdc_via_cpmm(
                        &bridge_pool,
                        &target_mint,
                        &quote_mint,
                        amount,
                        slippage_bps,
                        simulate_transaction,
                    )
                    .await
            }
            crate::models::Pool::PumpAmm => {
                wallet
                    .preview_pumpswap_via_cpmm(
                        target_pool.as_ref().context("target pool ID missing")?,
                        &bridge_pool,
                        &target_mint,
                        &quote_mint,
                        amount,
                        slippage_bps,
                        simulate_transaction,
                    )
                    .await
            }
            crate::models::Pool::RaydiumAmmV4 => {
                wallet
                    .preview_amm_v4_via_cpmm(
                        target_pool.as_ref().context("target pool ID missing")?,
                        &bridge_pool,
                        &target_mint,
                        &quote_mint,
                        amount,
                        slippage_bps,
                        simulate_transaction,
                    )
                    .await
            }
            crate::models::Pool::RaydiumCpmm => {
                wallet
                    .preview_cpmm_via_cpmm(
                        target_pool.as_ref().context("target pool ID missing")?,
                        &bridge_pool,
                        &target_mint,
                        &quote_mint,
                        amount,
                        slippage_bps,
                        simulate_transaction,
                    )
                    .await
            }
            _ => anyhow::bail!("SDK bridge preview is not integrated for this pool"),
        }
    };
    match attempt.await {
        Ok(preview) => Some(preview),
        Err(error) => {
            warn!(error = %redact_urls(&error.to_string()), "SDK two-hop build preview unavailable");
            None
        }
    }
}

async fn decide_sdk_route(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(mut request): Json<TradeRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let wallet_id = state
        .wallets
        .resolve_wallet_id(request.wallet_id.as_deref());
    let response_pubkey = state
        .wallets
        .get(&wallet_id)
        .map(|wallet| wallet.wallet().to_string());
    let blocked = |reason| {
        route_decision_response(
            wallet_id.clone(),
            response_pubkey.as_deref(),
            state.config.simulate_only,
            "policy_blocked",
            Some(reason),
            None,
        )
    };
    if request.sdk_decision_id.is_some() {
        return blocked("decision_id_not_accepted_for_resolution");
    }
    request.wallet_id = Some(wallet_id.clone());
    let Some(runtime_wallet) = state.wallets.get(&wallet_id) else {
        return blocked("wallet_unavailable");
    };
    let wallet_pubkey = runtime_wallet.wallet().to_string();
    if state.wallet_repository.database_enabled() {
        match state.wallet_repository.get(&wallet_id).await {
            Ok(value) if value.enabled && value.pubkey == wallet_pubkey => {}
            _ => return blocked("wallet_unavailable"),
        }
    }
    if request.sdk_preview_id.is_some() {
        if request.pool == crate::models::Pool::Pump {
            return blocked("pumpfun_token_quote_preview_is_read_only");
        }
        if !state.persistence.is_durable() {
            return blocked("durable_idempotency_required");
        }
        let normalized = match validate_trade(&request, &state.config) {
            Ok(value) if value.execution_method == crate::models::ExecutionMethod::Sdk => value,
            _ => return blocked("invalid_sdk_bridge_request"),
        };
        let now_ms = crate::state::now_ms() as u64;
        let (preview, observation) = match crate::route_registry::validate_sdk_preview_trade(
            &state.route_store,
            &request,
            &normalized,
            &wallet_id,
            &wallet_pubkey,
            now_ms,
        )
        .await
        {
            Ok(value) => value,
            Err(_) => return blocked("sdk_preview_unavailable_or_mismatched"),
        };
        let decision = SdkTradeDecisionSnapshot {
            decision_id: format!("sdk-decision:bridge:{}", uuid::Uuid::new_v4().simple()),
            route_id: observation.observation_id,
            via_sol: false,
            bridge_preview_id: Some(preview.preview_id),
            cpmm_sell_evidence: None,
            cpmm_buy_evidence: None,
            wallet_id: wallet_id.clone(),
            wallet_pubkey: wallet_pubkey.clone(),
            idempotency_key: request.idempotency_key,
            request_fingerprint: preview.request_fingerprint,
            expires_at_ms: preview.expires_at_ms,
        };
        if state
            .route_store
            .insert_sdk_decision(decision.clone(), now_ms)
            .await
            .is_err()
        {
            return blocked("decision_snapshot_unavailable");
        }
        let inventory = crate::sdk_client::bridge_sdk_capability(request.pool);
        return route_decision_response_with_observation(
            wallet_id,
            Some(&wallet_pubkey),
            state.config.simulate_only,
            "sdk_verified",
            None,
            Some(decision),
            None,
            Some(RouteSdkCapabilityEvidence {
                package_builder_family: inventory.package_builder_family,
                service_adapter: inventory.service_adapter,
                route_evidence: "unsigned_simulation_verified",
                sdk_unsupported_proven: false,
                jupiter_candidate_allowed: false,
            }),
        );
    }
    let mut discovered_direct_target = None;
    if crate::validation::sol_buy_requires_quote_discovery(&request, &state.config) {
        let target_hint = RouteHint {
            protocol_id: Some(crate::backend::registry::protocol_id(request.pool).to_string()),
            program_id: None,
            pool_id: request.pool_id.clone(),
            mint: request.mint.clone(),
            quote_mint: None,
        };
        let target = match state.execution_backend.resolve_pool(target_hint).await {
            Ok(route)
                if crate::route_registry::validate_verified_route(
                    &route,
                    crate::state::now_ms() as u64,
                )
                .is_ok() =>
            {
                route
            }
            _ => {
                return route_decision_response(
                    wallet_id,
                    Some(&wallet_pubkey),
                    state.config.simulate_only,
                    "unresolved_retryable",
                    Some("route_verification_unavailable"),
                    None,
                )
            }
        };
        let quote_is_wsol = target.quote_mint.as_deref().is_some_and(|quote| {
            quote == crate::models::WSOL_MINT || quote == crate::models::SOL_MINT
        });
        if quote_is_wsol {
            discovered_direct_target = Some(target);
        } else {
            let (reason, observation) = classify_sol_bridge_intent(
                &state,
                &request,
                runtime_wallet.as_ref(),
                &wallet_id,
                &wallet_pubkey,
                Some(target),
            )
            .await;
            let sdk_capability = observation.as_ref().map(|_| {
                let inventory = crate::sdk_client::bridge_sdk_capability(request.pool);
                RouteSdkCapabilityEvidence {
                    package_builder_family: inventory.package_builder_family,
                    service_adapter: inventory.service_adapter,
                    route_evidence: "chain_verified_bridge_pool_identity",
                    sdk_unsupported_proven: false,
                    jupiter_candidate_allowed: false,
                }
            });
            return route_decision_response_with_observation(
                wallet_id,
                Some(&wallet_pubkey),
                state.config.simulate_only,
                "unresolved_retryable",
                Some(reason),
                None,
                observation,
                sdk_capability,
            );
        }
    }
    if crate::validation::unresolved_sol_bridge_intent(&request, &state.config) {
        let (reason, observation) = classify_sol_bridge_intent(
            &state,
            &request,
            runtime_wallet.as_ref(),
            &wallet_id,
            &wallet_pubkey,
            None,
        )
        .await;
        let sdk_capability = observation.as_ref().map(|_| {
            let inventory = crate::sdk_client::bridge_sdk_capability(request.pool);
            RouteSdkCapabilityEvidence {
                package_builder_family: inventory.package_builder_family,
                service_adapter: inventory.service_adapter,
                route_evidence: "chain_verified_bridge_pool_identity",
                sdk_unsupported_proven: false,
                jupiter_candidate_allowed: false,
            }
        });
        return route_decision_response_with_observation(
            wallet_id,
            Some(&wallet_pubkey),
            state.config.simulate_only,
            "unresolved_retryable",
            Some(reason),
            None,
            observation,
            sdk_capability,
        );
    }
    let normalized = match validate_trade(&request, &state.config) {
        Ok(value) if value.execution_method == crate::models::ExecutionMethod::Sdk => value,
        _ => return blocked("invalid_sdk_trade_request"),
    };
    let cpmm_sell = request.pool == crate::models::Pool::RaydiumCpmm
        && request.side == crate::models::Side::Sell
        && normalized.settlement_asset == crate::models::SettlementAsset::Sol;
    let cpmm_buy = request.pool == crate::models::Pool::RaydiumCpmm
        && request.side == crate::models::Side::Buy
        && normalized.settlement_asset == crate::models::SettlementAsset::Sol;
    let cpmm_sell_evidence = if cpmm_sell {
        if !state.persistence.is_durable() || request.sdk_route_id.is_some() {
            return blocked("durable_cpmm_sell_decision_required");
        }
        let Some(target_pool) = normalized.pool_id.as_ref() else {
            return blocked("cpmm_target_pool_required");
        };
        let Some(bridge_pool) = normalized.sol_hop_pool_id.as_ref() else {
            return blocked("cpmm_bridge_pool_required");
        };
        let preview = match runtime_wallet
            .preview_cpmm_sell_via_cpmm(
                target_pool,
                bridge_pool,
                &normalized.mint,
                &normalized.quote_mint,
                normalized.sell_percent.unwrap_or_default(),
                normalized.slippage_bps,
                true,
            )
            .await
        {
            Ok(value) => value,
            Err(_) => return blocked("cpmm_sell_unsigned_simulation_unavailable"),
        };
        let Some(simulation) = preview.simulation.as_ref() else {
            return blocked("cpmm_sell_unsigned_simulation_unavailable");
        };
        if !simulation.rpc_accepted
            || !simulation.ok
            || !simulation.balance_and_minimum_verified
            || simulation.slot < preview.validated_slot
            || simulation.input_token_debit_units != Some(preview.input_token_units)
            || simulation
                .wsol_credit_units
                .is_none_or(|credit| credit < preview.bridge_min_wsol_units)
        {
            return blocked("cpmm_sell_unsigned_simulation_unavailable");
        }
        Some(crate::route_registry::SdkCpmmSellEvidence {
            input_token_units: preview.input_token_units,
            minimum_wsol_units: preview.bridge_min_wsol_units,
            validated_slot: preview.validated_slot,
            simulation_slot: simulation.slot,
        })
    } else {
        None
    };
    let cpmm_buy_evidence = if cpmm_buy {
        if !state.persistence.is_durable() || request.sdk_route_id.is_some() {
            return blocked("durable_cpmm_buy_decision_required");
        }
        let Some(target_pool) = normalized.pool_id.as_ref() else {
            return blocked("cpmm_target_pool_required");
        };
        let Some(bridge_pool) = normalized.sol_hop_pool_id.as_ref() else {
            return blocked("cpmm_bridge_pool_required");
        };
        let Some(input_lamports) = normalized.amount_lamports else {
            return blocked("cpmm_buy_input_required");
        };
        let preview = match runtime_wallet
            .preview_cpmm_via_cpmm(
                target_pool,
                bridge_pool,
                &normalized.mint,
                &normalized.quote_mint,
                input_lamports,
                normalized.slippage_bps,
                true,
            )
            .await
        {
            Ok(value) => value,
            Err(_) => return blocked("cpmm_buy_unsigned_simulation_unavailable"),
        };
        let Some(simulation) = preview.simulation.as_ref() else {
            return blocked("cpmm_buy_unsigned_simulation_unavailable");
        };
        if !simulation.attempted
            || !simulation.rpc_accepted
            || !simulation.ok
            || !simulation.minimum_output_verified
            || simulation.slot < preview.validated_slot
            || simulation
                .payer_debit_lamports
                .is_none_or(|debit| debit < input_lamports)
            || simulation
                .target_token_delta_units
                .is_none_or(|delta| delta < preview.target_min_output_units)
        {
            return blocked("cpmm_buy_unsigned_simulation_unavailable");
        }
        Some(crate::route_registry::SdkCpmmBuyEvidence {
            input_lamports,
            minimum_target_units: preview.target_min_output_units,
            validated_slot: preview.validated_slot,
            simulation_slot: simulation.slot,
        })
    } else {
        None
    };
    let unresolved = |reason| {
        route_decision_response(
            wallet_id.clone(),
            Some(&wallet_pubkey),
            state.config.simulate_only,
            "unresolved_retryable",
            Some(reason),
            None,
        )
    };
    let (route_id, expires_at_ms, via_sol) = if normalized.settlement_asset
        == crate::models::SettlementAsset::Sol
    {
        let route =
            match executor::resolve_sdk_via_sol_trade_route(&state, &request, &normalized).await {
                Ok(value) => value,
                Err(_) => return unresolved("route_verification_unavailable"),
            };
        (route.route_id, route.expires_at_ms, true)
    } else {
        let hint = RouteHint {
            protocol_id: Some(crate::backend::registry::protocol_id(request.pool).to_string()),
            program_id: None,
            pool_id: request.pool_id.clone(),
            mint: request.mint.clone(),
            quote_mint: Some(normalized.quote_mint.to_string()),
        };
        let route_result = match discovered_direct_target {
            Some(route) => Ok(route),
            None => state.execution_backend.resolve_pool(hint).await,
        };
        let route = match route_result {
            Ok(value) => value,
            Err(_) => return unresolved("route_verification_unavailable"),
        };
        if crate::route_registry::validate_single_sdk_trade_match(
            &route,
            &request,
            &normalized,
            crate::state::now_ms() as u64,
        )
        .is_err()
        {
            return unresolved("route_does_not_match_trade");
        }
        if state
            .route_store
            .insert(route.clone(), crate::state::now_ms() as u64)
            .await
            .is_err()
        {
            return unresolved("route_snapshot_unavailable");
        }
        (route.route_id, route.expires_at_ms, false)
    };
    match crate::sdk_client::direct_sdk_buildability(
        request.pool,
        request.side,
        &normalized.quote_mint,
        normalized.pool_id.as_ref(),
        normalized.sol_hop_pool,
        normalized.sol_hop_pool_id.as_ref(),
    ) {
        crate::sdk_client::DirectSdkBuildability::Available => {}
        crate::sdk_client::DirectSdkBuildability::Unavailable(_) => {
            return unresolved("sdk_adapter_pending")
        }
        crate::sdk_client::DirectSdkBuildability::Incomplete(_) => {
            return unresolved("route_evidence_incomplete")
        }
    }
    let request_fingerprint =
        match crate::persistence::request_hash(&wallet_id, &request, &normalized) {
            Ok(value) => value,
            Err(_) => return unresolved("request_fingerprint_unavailable"),
        };
    let decision_now_ms = crate::state::now_ms() as u64;
    let decision = SdkTradeDecisionSnapshot {
        decision_id: format!("sdk-decision:{}", uuid::Uuid::new_v4().simple()),
        route_id,
        via_sol,
        bridge_preview_id: None,
        cpmm_sell_evidence,
        cpmm_buy_evidence,
        wallet_id: wallet_id.clone(),
        wallet_pubkey: wallet_pubkey.clone(),
        idempotency_key: request.idempotency_key.clone(),
        request_fingerprint,
        expires_at_ms: if cpmm_sell || cpmm_buy {
            expires_at_ms.min(decision_now_ms.saturating_add(5_000))
        } else {
            expires_at_ms
        },
    };
    if state
        .route_store
        .insert_sdk_decision(decision.clone(), decision_now_ms)
        .await
        .is_err()
    {
        return unresolved("decision_snapshot_unavailable");
    }
    let inventory = crate::sdk_client::direct_sdk_capability(request.pool, via_sol);
    route_decision_response_with_observation(
        wallet_id,
        Some(&wallet_pubkey),
        state.config.simulate_only,
        "sdk_verified",
        None,
        Some(decision),
        None,
        Some(RouteSdkCapabilityEvidence {
            package_builder_family: inventory.package_builder_family,
            service_adapter: if cpmm_buy {
                if request.simulate == Some(false) {
                    "cpmm_two_hop_buy_prepared_live"
                } else {
                    "cpmm_two_hop_buy_simulate_only"
                }
            } else if cpmm_sell {
                if request.simulate == Some(false) {
                    "cpmm_two_hop_sell_prepared_live"
                } else {
                    "cpmm_two_hop_sell_simulate_only"
                }
            } else {
                inventory.service_adapter
            },
            route_evidence: if cpmm_sell || cpmm_buy {
                "unsigned_simulation_verified"
            } else if via_sol {
                "chain_verified_via_sol_route"
            } else {
                "chain_verified_single_pool_route"
            },
            sdk_unsupported_proven: false,
            jupiter_candidate_allowed: false,
        }),
    )
}

async fn resolve_via_sol_candidates(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ViaSolRouteRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if !state.config.stonkfun_via_sol.enabled {
        return error(StatusCode::FORBIDDEN, "StonkFun via-SOL route is disabled");
    }
    if !matches!(
        request.target.protocol_id.as_deref(),
        Some("stonkfun" | "stonkfun-swap")
    ) || request.target.pool_id.is_none()
    {
        return error(
            StatusCode::BAD_REQUEST,
            "via-SOL target requires stonkfun or stonkfun-swap protocol and pool_id",
        );
    }
    let Some(target_quote) = request.target.quote_mint.as_deref() else {
        return error(
            StatusCode::BAD_REQUEST,
            "via-SOL target quote_mint is required",
        );
    };
    if target_quote == crate::models::WSOL_MINT
        || target_quote.parse::<solana_sdk::pubkey::Pubkey>().is_err()
        || request
            .sol_hop_pool_id
            .parse::<solana_sdk::pubkey::Pubkey>()
            .is_err()
    {
        return error(
            StatusCode::BAD_REQUEST,
            "via-SOL quote mint or hop pool is invalid",
        );
    }
    let hop_hint = RouteHint {
        protocol_id: Some("raydium-cpmm".to_string()),
        program_id: None,
        pool_id: Some(request.sol_hop_pool_id),
        mint: target_quote.to_string(),
        quote_mint: Some(crate::models::WSOL_MINT.to_string()),
    };
    // The two legs are independent RPC reads. Resolve them concurrently so
    // the target's five-second TTL is not spent waiting for the second leg.
    let (target, sol_hop) = match tokio::try_join!(
        state.execution_backend.resolve_pool(request.target),
        state.execution_backend.resolve_pool(hop_hint)
    ) {
        Ok(routes) => routes,
        Err(err) => return error(StatusCode::BAD_GATEWAY, err.to_string()),
    };
    let route = VerifiedViaSolRoute {
        route_id: format!("sdk11:via-sol:{}", uuid::Uuid::new_v4().simple()),
        expires_at_ms: target.expires_at_ms.min(sol_hop.expires_at_ms),
        target,
        sol_hop,
    };
    if let Err(err) = state
        .route_store
        .insert_via_sol(route.clone(), crate::state::now_ms() as u64)
        .await
    {
        return error(StatusCode::BAD_GATEWAY, err.to_string());
    }
    Json(route).into_response()
}

async fn verify_route_wallet(state: &AppState, wallet_id: &str) -> Result<(), Response> {
    let Some(runtime_wallet) = state.wallets.get(wallet_id) else {
        return Err(error(StatusCode::NOT_FOUND, "wallet not found"));
    };
    if state.wallet_repository.database_enabled() {
        match state.wallet_repository.get(wallet_id).await {
            Ok(wallet)
                if wallet.enabled && wallet.pubkey == runtime_wallet.wallet().to_string() => {}
            Ok(_) => {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    "wallet is disabled or has changed",
                ))
            }
            Err(_) => return Err(error(StatusCode::NOT_FOUND, "wallet not found")),
        }
    }
    Ok(())
}

async fn route_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<QuoteRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if let Err(err) = state
        .route_store
        .admit(&request.route, crate::state::now_ms() as u64)
        .await
    {
        return error(StatusCode::BAD_REQUEST, err.to_string());
    }
    if request.wallet_id.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "wallet_id is required");
    }
    if let Err(response) = verify_route_wallet(&state, &request.wallet_id).await {
        return response;
    }
    match state.execution_backend.quote_exact_in(request).await {
        Ok(quote) => Json(quote).into_response(),
        Err(err) => error(StatusCode::NOT_IMPLEMENTED, err.to_string()),
    }
}

async fn route_simulate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(plan): Json<TradePlan>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if let Err(err) = state
        .route_store
        .admit(&plan.route, crate::state::now_ms() as u64)
        .await
    {
        return error(StatusCode::BAD_REQUEST, err.to_string());
    }
    if let Err(err) = crate::route_registry::validate_trade_plan(
        &plan,
        &plan.quote.wallet_id,
        crate::state::now_ms() as u64,
    ) {
        return error(StatusCode::BAD_REQUEST, err.to_string());
    }
    if let Err(response) = verify_route_wallet(&state, &plan.quote.wallet_id).await {
        return response;
    }
    match state.execution_backend.simulate(plan).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => error(StatusCode::NOT_IMPLEMENTED, err.to_string()),
    }
}

async fn resolve_sol_hop(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ResolveSolHopRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if !state.config.stonkfun_via_sol.enabled {
        return (
            StatusCode::OK,
            Json(ResolveSolHopResponse {
                available: false,
                pool: None,
                pool_id: None,
                reason: Some("STONKFUN_VIA_SOL_ENABLED is false".to_string()),
            }),
        )
            .into_response();
    }
    let quote_mint = match request.quote_mint.parse() {
        Ok(mint) => mint,
        Err(_) => return error(StatusCode::BAD_REQUEST, "quote_mint must be a valid pubkey"),
    };
    match state
        .wallets
        .default()
        .resolve_wsol_quote_cpmm_hop(&quote_mint)
        .await
    {
        Ok(pool_id) => (
            StatusCode::OK,
            Json(ResolveSolHopResponse {
                available: true,
                pool: Some(crate::models::Pool::RaydiumCpmm),
                pool_id: Some(pool_id.to_string()),
                reason: None,
            }),
        )
            .into_response(),
        Err(err) => (
            StatusCode::OK,
            Json(ResolveSolHopResponse {
                available: false,
                pool: None,
                pool_id: None,
                reason: Some(crate::sanitize::redact_urls(&err.to_string())),
            }),
        )
            .into_response(),
    }
}

async fn sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SolBuyQuoteRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let Some(wallet) = state.wallets.get(&request.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    match aggregator_service::create_jupiter_bound_quote(
        &state.aggregator_quotes,
        &request,
        &wallet,
    )
    .await
    {
        Ok(bound) => (StatusCode::OK, Json(bound)).into_response(),
        Err(QuoteCreationError::Invalid(err)) => error(StatusCode::BAD_REQUEST, err.to_string()),
        Err(QuoteCreationError::InsufficientBalance) => error(
            StatusCode::BAD_REQUEST,
            "wallet SOL balance below quote amount",
        ),
        Err(QuoteCreationError::BalanceUnavailable(err)) => {
            let err = redact_urls(&err.to_string());
            tracing::warn!(error = %err, "wallet balance snapshot unavailable for Jupiter quote");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "wallet balance unavailable",
            )
        }
        Err(QuoteCreationError::Provider(err)) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
        Err(QuoteCreationError::Persistence(err)) => {
            error(StatusCode::SERVICE_UNAVAILABLE, err.to_string())
        }
    }
}

#[derive(Deserialize)]
struct QuoteWalletQuery {
    wallet_id: String,
}

async fn get_sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(quote_id): Path<String>,
    Query(query): Query<QuoteWalletQuery>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let Some(wallet) = state.wallets.get(&query.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    match state
        .aggregator_quotes
        .get(&quote_id, &query.wallet_id, &wallet.wallet().to_string())
        .await
    {
        Ok(bound) => (StatusCode::OK, Json(bound)).into_response(),
        Err(err) => error(StatusCode::GONE, err.to_string()),
    }
}

async fn preflight_sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(quote_id): Path<String>,
    Query(query): Query<QuoteWalletQuery>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let Some(wallet) = state.wallets.get(&query.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    let bound = match state
        .aggregator_quotes
        .get(&quote_id, &query.wallet_id, &wallet.wallet().to_string())
        .await
    {
        Ok(bound) => bound,
        Err(err) => return error(StatusCode::GONE, err.to_string()),
    };
    if bound.status == "failed" {
        return error(StatusCode::GONE, "quote is terminal; request a new quote");
    }
    match aggregator_quote::preflight(&bound, &wallet).await {
        Ok(result) => {
            if let Err(err) = state
                .aggregator_quotes
                .record_preflight(&quote_id, &result)
                .await
            {
                return error(StatusCode::SERVICE_UNAVAILABLE, err.to_string());
            }
            let status = if result.local_simulation_ok {
                "preflighted"
            } else {
                "failed"
            };
            if let Err(err) = state
                .aggregator_quotes
                .mark_quote_status(&quote_id, status)
                .await
            {
                return error(StatusCode::SERVICE_UNAVAILABLE, err.to_string());
            }
            match state
                .aggregator_quotes
                .get(&quote_id, &query.wallet_id, &wallet.wallet().to_string())
                .await
            {
                Ok(_) => (StatusCode::OK, Json(result)).into_response(),
                Err(err) => error(StatusCode::GONE, err.to_string()),
            }
        }
        Err(err) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
}

async fn submit_sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(quote_id): Path<String>,
    Json(request): Json<SolBuySubmitRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if state.config.simulate_only && !request.simulate {
        return error(
            StatusCode::FORBIDDEN,
            "this Trade Service instance only accepts simulations",
        );
    }
    let Some(wallet) = state.wallets.get(&request.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    let bound = match state
        .aggregator_quotes
        .get(&quote_id, &request.wallet_id, &wallet.wallet().to_string())
        .await
    {
        Ok(bound) => bound,
        Err(err) => return error(StatusCode::GONE, err.to_string()),
    };
    if bound.status == "failed" {
        return error(StatusCode::GONE, "quote is terminal; request a new quote");
    }
    match aggregator_service::submit_jupiter_bound_quote(
        &state.aggregator_quotes,
        &bound,
        &wallet,
        &request,
        state.config.simulate_only,
    )
    .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(SubmissionError::SimulateOnly) => error(
            StatusCode::FORBIDDEN,
            "this Trade Service instance only accepts simulations",
        ),
        Err(SubmissionError::Invalid(message)) => error(StatusCode::BAD_REQUEST, message),
        Err(SubmissionError::Conflict) => error(
            StatusCode::CONFLICT,
            "idempotency key was used for another quote",
        ),
        Err(SubmissionError::InFlight) => error(
            StatusCode::CONFLICT,
            "idempotency key is already reserved; recover the original submission",
        ),
        Err(SubmissionError::Reservation(err) | SubmissionError::MarkSubmitting(err)) => {
            error(StatusCode::SERVICE_UNAVAILABLE, err.to_string())
        }
        Err(SubmissionError::Finalize(err)) => {
            error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
        }
        Err(SubmissionError::Execution(err)) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
}

#[derive(Debug, Serialize)]
struct AggregatorSubmissionStatusResponse {
    schema_version: u32,
    quote_id: String,
    wallet_id: String,
    idempotency_key: String,
    state: String,
    response: Option<SolBuySubmitResponse>,
}

async fn get_sol_buy_submission(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((quote_id, idempotency_key)): Path<(String, String)>,
    Query(query): Query<QuoteWalletQuery>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if query.wallet_id.trim().is_empty() || idempotency_key.trim().is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "wallet_id and idempotency_key are required",
        );
    }
    if state.wallets.get(&query.wallet_id).is_none() {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    }
    let key = format!("{}:{}", query.wallet_id, idempotency_key);
    match state
        .aggregator_quotes
        .submission_matches_quote(&key, &quote_id)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return error(
                StatusCode::NOT_FOUND,
                "submission not found or quote mismatch",
            )
        }
        Err(err) => return error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
    }
    let reservation = match state.aggregator_quotes.lookup_submission(&key).await {
        Ok(Some(value)) => value,
        Ok(None) => return error(StatusCode::NOT_FOUND, "submission not found"),
        Err(err) => return error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
    };
    let (state_name, response) = match reservation {
        SubmissionReservation::Existing(value) => {
            if value.quote_id != quote_id {
                return error(StatusCode::CONFLICT, "submission quote mismatch");
            }
            (value.status.clone(), Some(value))
        }
        SubmissionReservation::InFlight => ("submitting".to_string(), None),
        SubmissionReservation::Unknown => ("unknown".to_string(), None),
        SubmissionReservation::Reserved => ("reserved".to_string(), None),
        SubmissionReservation::Conflict => {
            return error(StatusCode::CONFLICT, "submission quote mismatch")
        }
    };
    Json(AggregatorSubmissionStatusResponse {
        schema_version: 1,
        quote_id,
        wallet_id: query.wallet_id,
        idempotency_key,
        state: state_name,
        response,
    })
    .into_response()
}

async fn create_trade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<TradeRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }

    match accept_trade_for_wallet(state, request, None).await {
        Ok(outcome) => (outcome.http_status, Json(outcome.response)).into_response(),
        Err((status, message)) => error(status, message),
    }
}

struct AcceptTradeOutcome {
    http_status: StatusCode,
    wallet_id: String,
    response: TradeResponse,
}

#[derive(Deserialize)]
struct ListTradesQuery {
    limit: Option<usize>,
}

async fn accept_trade_for_wallet(
    state: AppState,
    mut request: TradeRequest,
    forced_wallet_id: Option<String>,
) -> Result<AcceptTradeOutcome, (StatusCode, String)> {
    let wallet_id = forced_wallet_id.unwrap_or_else(|| {
        state
            .wallets
            .resolve_wallet_id(request.wallet_id.as_deref())
    });
    request.wallet_id = Some(wallet_id.clone());

    let Some(wallet) = state.wallets.get(&wallet_id) else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("wallet_id {wallet_id} not found"),
        ));
    };
    let wallet_pubkey = wallet.wallet().to_string();

    // The legacy explicit aggregator trade path has no server-issued proof
    // that every applicable SDK builder is unsupported for this route. Keep
    // Jupiter quote/Shadow diagnostics available, but do not reserve a trade
    // or run its simulate/submit state machine on a client-selected backend.
    if request.execution_method == Some(crate::models::ExecutionMethod::Aggregator) {
        return Err((
            StatusCode::CONFLICT,
            "aggregator trade requires a server-issued sdk_unsupported decision".to_string(),
        ));
    }

    let normalized = match state
        .backend_catalog
        .backend_for_pool(request.pool)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no execution backend registered for protocol {}",
                crate::backend::registry::protocol_id(request.pool)
            )
        })
        .and_then(|_| validate_trade(&request, &state.config))
    {
        Ok(normalized) => normalized,
        Err(error) => {
            // A supplied decision is a pre-admission credential. Invalid
            // request shape must not create a rejected trade or reserve an
            // idempotency key before that credential can be checked.
            if request.sdk_decision_id.is_some() || request.sdk_preview_id.is_some() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "SDK credential is expired or does not match this trade".to_string(),
                ));
            }
            let record = state.trades.rejected(
                &request,
                wallet_id.clone(),
                wallet_pubkey,
                redact_urls(&error.to_string()),
            );
            warn!(
                request_id = %record.request_id,
                wallet_id = %record.wallet_id,
                idempotency_key = %request.idempotency_key,
                side = ?request.side,
                pool = ?request.pool,
                mint = %request.mint,
                status = ?record.status,
                error = ?record.error.as_deref().map(redact_urls),
                "trade rejected"
            );
            if let Err(persist_error) = state.persistence.mirror(&record, &request, None).await {
                let persist_error = redact_urls(&persist_error.to_string());
                warn!(error = %persist_error, request_id = %record.request_id, "failed to persist rejected trade");
            }
            return Ok(AcceptTradeOutcome {
                http_status: StatusCode::BAD_REQUEST,
                wallet_id,
                response: TradeResponse::from(record),
            });
        }
    };

    if request.pool == crate::models::Pool::RaydiumCpmm
        && normalized.settlement_asset == crate::models::SettlementAsset::Sol
        && request.sdk_decision_id.is_none()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "CPMM two-hop trade requires a server-issued SDK decision".to_string(),
        ));
    }

    if request.sdk_decision_id.is_some() || request.sdk_preview_id.is_some() {
        if state.wallet_repository.database_enabled() {
            let managed = state.wallet_repository.get(&wallet_id).await.map_err(|_| {
                (
                    StatusCode::FORBIDDEN,
                    "SDK credential wallet is unavailable".to_string(),
                )
            })?;
            if !managed.enabled || managed.pubkey != wallet_pubkey {
                return Err((
                    StatusCode::FORBIDDEN,
                    "SDK credential wallet is disabled or has changed".to_string(),
                ));
            }
        }
        if !state.persistence.is_durable() {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "SDK decision trades require durable idempotency".to_string(),
            ));
        }
        // An exact retry must read the original order before checking a
        // short-lived SDK decision. This applies to direct single-pool
        // decisions as well as bridge previews, including after restart.
        match state
            .persistence
            .lookup_idempotency(&wallet_id, &request, &normalized)
            .await
        {
            Ok(Some(DurableReserve::Duplicate(mut existing))) => {
                existing.duplicate = true;
                return Ok(AcceptTradeOutcome {
                    http_status: StatusCode::OK,
                    wallet_id,
                    response: TradeResponse::from(existing),
                });
            }
            Ok(Some(DurableReserve::Conflict)) => {
                return Err((
                    StatusCode::CONFLICT,
                    "idempotency key was already used for a different trade".to_string(),
                ));
            }
            Ok(None) | Ok(Some(DurableReserve::Created)) => {}
            Err(_) => {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "trade persistence unavailable; trade was not started".to_string(),
                ));
            }
        }
        let now_ms = crate::state::now_ms() as u64;
        let credential_valid = if request.sdk_preview_id.is_some() {
            crate::route_registry::validate_sdk_preview_trade(
                &state.route_store,
                &request,
                &normalized,
                &wallet_id,
                &wallet_pubkey,
                now_ms,
            )
            .await
            .map(|_| ())
        } else {
            crate::route_registry::validate_sdk_decision_trade(
                &state.route_store,
                &request,
                &normalized,
                &wallet_id,
                &wallet_pubkey,
                now_ms,
            )
            .await
            .map(|_| ())
        };
        credential_valid.map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "SDK credential is expired or does not match this trade".to_string(),
            )
        })?;
    }

    let request_id = new_request_id();
    if !state.persistence.is_durable() {
        let scoped_idempotency_key = format!("{wallet_id}:{}", request.idempotency_key);
        match state
            .idempotency
            .reserve(&scoped_idempotency_key, &request_id)
        {
            ReserveResult::Duplicate(existing_request_id) => {
                if let Some(mut existing) = state.trades.get(&existing_request_id) {
                    existing.duplicate = true;
                    return Ok(AcceptTradeOutcome {
                        http_status: StatusCode::OK,
                        wallet_id,
                        response: TradeResponse::from(existing),
                    });
                }
                return Err((
                    StatusCode::CONFLICT,
                    "duplicate idempotency key points to missing trade".to_string(),
                ));
            }
            ReserveResult::Reserved => {}
        }
    }

    let record =
        state
            .trades
            .create_with_id(request_id, &request, wallet_id.clone(), wallet_pubkey);
    if state.persistence.is_durable() {
        match state
            .persistence
            .reserve(&record, &request, &normalized)
            .await
        {
            Ok(DurableReserve::Created) => {}
            Ok(DurableReserve::Duplicate(mut existing)) => {
                state.trades.remove(&record.request_id);
                existing.duplicate = true;
                return Ok(AcceptTradeOutcome {
                    http_status: StatusCode::OK,
                    wallet_id,
                    response: TradeResponse::from(existing),
                });
            }
            Ok(DurableReserve::Conflict) => {
                state.trades.remove(&record.request_id);
                return Err((
                    StatusCode::CONFLICT,
                    "idempotency key was already used for a different trade".to_string(),
                ));
            }
            Err(error) => {
                state.trades.remove(&record.request_id);
                let error = redact_urls(&error.to_string());
                warn!(error = %error, "durable idempotency reservation failed; trade not started");
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "trade persistence unavailable; trade was not started".to_string(),
                ));
            }
        }
    } else if let Err(error) = state
        .persistence
        .mirror(&record, &request, Some(&normalized))
        .await
    {
        let error = redact_urls(&error.to_string());
        warn!(error = %error, request_id = %record.request_id, "shadow trade persistence failed");
    }
    info!(
        request_id = %record.request_id,
        wallet_id = %record.wallet_id,
        idempotency_key = %request.idempotency_key,
        side = ?request.side,
        pool = ?request.pool,
        mint = %request.mint,
        status = ?TradeStatus::Accepted,
        "trade accepted"
    );

    let state_for_task = state.clone();
    let request_id = record.request_id.clone();
    tokio::spawn(async move {
        executor::execute_trade(state_for_task, request_id, request, normalized).await;
    });

    let response_record = wait_for_trade_result(
        &state,
        &record.request_id,
        state.config.trade_response_wait_ms,
    )
    .await
    .unwrap_or(record);
    let http_status = if is_pending_status(response_record.status) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    let response = TradeResponse::from(response_record);
    Ok(AcceptTradeOutcome {
        http_status,
        wallet_id,
        response,
    })
}

async fn create_batch_trades(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(batch): Json<BatchTradeRequest>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if batch.wallet_ids.is_empty() {
        return error(StatusCode::BAD_REQUEST, "wallet_ids is required");
    }

    let mut items = Vec::with_capacity(batch.wallet_ids.len());
    for wallet_id in batch.wallet_ids {
        match accept_trade_for_wallet(state.clone(), batch.trade.clone(), Some(wallet_id.clone()))
            .await
        {
            Ok(outcome) => items.push(BatchTradeItemResponse {
                wallet_id: outcome.wallet_id,
                request_id: Some(outcome.response.request_id),
                status: Some(outcome.response.status),
                error: outcome.response.error,
                duplicate: outcome.response.duplicate,
            }),
            Err((status, message)) => items.push(BatchTradeItemResponse {
                wallet_id,
                request_id: None,
                status: None,
                error: Some(format!("{status}: {message}")),
                duplicate: false,
            }),
        }
    }

    (StatusCode::ACCEPTED, Json(BatchTradeResponse { items })).into_response()
}

async fn list_trades(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<ListTradesQuery>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    if state.persistence.is_durable() {
        return match state.persistence.recent(limit).await {
            Ok(records) => (StatusCode::OK, Json(records)).into_response(),
            Err(error_value) => error(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("trade persistence unavailable: {error_value}"),
            ),
        };
    }
    (StatusCode::OK, Json(state.trades.recent(limit))).into_response()
}

async fn get_trade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Response {
    if let Err(response) = authorize_protected(&state, &headers, peer) {
        return response;
    }
    if state.persistence.is_durable() {
        return match state.persistence.get(&request_id).await {
            Ok(Some(record)) => (StatusCode::OK, Json(record)).into_response(),
            Ok(None) => error(StatusCode::NOT_FOUND, "trade not found"),
            Err(error_value) => error(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("trade persistence unavailable: {error_value}"),
            ),
        };
    }
    match state.trades.get(&request_id) {
        Some(record) => (StatusCode::OK, Json(record)).into_response(),
        None => error(StatusCode::NOT_FOUND, "trade not found"),
    }
}

fn authorize_protected(
    state: &AppState,
    headers: &HeaderMap,
    peer: SocketAddr,
) -> Result<(), Response> {
    if !state.config.trade_client_ip_allowed(peer.ip()) {
        warn!(client_ip = %peer.ip(), "trade api client ip rejected");
        return Err(error(StatusCode::FORBIDDEN, "client ip not allowed"));
    }
    if !auth::authorized(headers, &state.config.local_trade_api_token) {
        return Err(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }
    Ok(())
}

fn authorize_internal(
    state: &AppState,
    headers: &HeaderMap,
    peer: SocketAddr,
) -> Result<(), Response> {
    if !peer.ip().is_loopback() {
        warn!(client_ip = %peer.ip(), "internal wallet api client ip rejected");
        return Err(error(
            StatusCode::FORBIDDEN,
            "internal wallet API is loopback-only",
        ));
    }
    authorize_protected(state, headers, peer)
}

async fn wait_for_trade_result(
    state: &AppState,
    request_id: &str,
    wait_ms: u64,
) -> Option<crate::models::TradeRecord> {
    if wait_ms == 0 {
        if state.persistence.is_durable() {
            return state.persistence.get(request_id).await.ok().flatten();
        }
        return state.trades.get(request_id);
    }
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        let record = if state.persistence.is_durable() {
            state.persistence.get(request_id).await.ok().flatten()?
        } else {
            state.trades.get(request_id)?
        };
        if !is_pending_status(record.status) || Instant::now() >= deadline {
            return Some(record);
        }
        sleep(Duration::from_millis(20)).await;
    }
}

fn is_pending_status(status: TradeStatus) -> bool {
    matches!(
        status,
        TradeStatus::Accepted
            | TradeStatus::WarmingParams
            | TradeStatus::BuildingTx
            | TradeStatus::Submitting
    )
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: redact_urls(&message.into()),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod aggregator_compare_tests {
    use super::*;
    use crate::aggregator::{AggregatorBackend, AggregatorCapabilities, AggregatorRegistry};
    use axum::body::to_bytes;

    #[derive(Clone, Copy)]
    enum MockMode {
        Valid,
        SecretError,
        Panic,
    }

    struct MockProvider {
        id: ProviderId,
        mode: MockMode,
    }

    #[async_trait::async_trait]
    impl AggregatorBackend for MockProvider {
        fn provider(&self) -> ProviderId {
            self.id
        }

        fn capabilities(&self) -> AggregatorCapabilities {
            AggregatorCapabilities::read_only_quote()
        }

        async fn quote(&self, request: &AggregatorQuoteRequest) -> AnyResult<AggregatorQuote> {
            match self.mode {
                MockMode::SecretError => anyhow::bail!("api-key=secret provider body=secret"),
                MockMode::Panic => panic!("mock provider task failed"),
                MockMode::Valid => {}
            }
            let route = vec![crate::aggregator::AggregatorRouteLeg {
                label: "fixture".into(),
                input_mint: request.input_mint.clone(),
                output_mint: request.output_mint.clone(),
            }];
            Ok(AggregatorQuote {
                provider: self.id,
                provider_quote_id: "fixture-quote".into(),
                wallet_id: request.wallet_id.clone(),
                wallet_pubkey: request.wallet_pubkey.clone(),
                input_mint: request.input_mint.clone(),
                output_mint: request.output_mint.clone(),
                input_amount_units: request.input_amount_units,
                expected_output_units: 100,
                minimum_output_units: 99,
                slippage_bps: request.slippage_bps,
                route_fingerprint: format!("{:x}", Sha256::digest(serde_json::to_vec(&route)?)),
                request_hash: format!("{:x}", Sha256::digest(serde_json::to_vec(request)?)),
                created_at_ms: aggregator_quote::current_time_ms(),
                expires_at_ms: aggregator_quote::current_time_ms() + 10_000,
                price_impact_percent: None,
                route,
                balance_snapshot: request.balance_snapshot.clone(),
            })
        }
    }

    #[tokio::test]
    async fn compare_keeps_partial_results_and_never_returns_provider_error_body() {
        let request = AggregatorQuoteRequest {
            wallet_id: "fixture-wallet".into(),
            wallet_pubkey: solana_sdk::pubkey::Pubkey::new_unique().to_string(),
            input_mint: crate::models::WSOL_MINT.into(),
            output_mint: crate::models::USDC_MINT.into(),
            input_amount_units: 1_000_000,
            slippage_bps: 100,
            balance_snapshot: None,
        };
        for failure_mode in [MockMode::SecretError, MockMode::Panic] {
            let mut registry = AggregatorRegistry::new();
            registry.register(std::sync::Arc::new(MockProvider {
                id: ProviderId::Jupiter,
                mode: MockMode::Valid,
            }));
            registry.register(std::sync::Arc::new(MockProvider {
                id: ProviderId::Dflow,
                mode: failure_mode,
            }));
            let results = gather_compare_results(
                std::sync::Arc::new(registry),
                vec![ProviderId::Jupiter, ProviderId::Dflow],
                &request,
            )
            .await;
            assert_eq!(results.len(), 2);
            assert_eq!(results[0].provider, ProviderId::Dflow);
            assert_eq!(results[0].status, "failed");
            assert!(results[0].quote.is_none());
            let failure = results[0].error.as_deref().unwrap();
            assert!(!failure.contains("secret"));
            assert!(!failure.contains("api-key"));
            assert_eq!(results[1].provider, ProviderId::Jupiter);
            assert_eq!(results[1].status, "quoted");
            assert!(results[1].quote.is_some());
        }
    }

    fn request() -> AggregatorCompareRequest {
        AggregatorCompareRequest {
            wallet_id: "main".into(),
            mint: crate::models::USDC_MINT.into(),
            amount_lamports: 1_000_000,
            slippage_bps: 100,
            allowed_aggregators: None,
        }
    }

    #[test]
    fn compare_validation_accepts_bounded_request() {
        assert!(validate_aggregator_compare_request(&request()).is_ok());
    }

    #[test]
    fn compare_validation_rejects_invalid_mint_amount_and_slippage() {
        let mut invalid = request();
        invalid.mint = "invalid".into();
        assert_eq!(
            validate_aggregator_compare_request(&invalid),
            Err("mint must be a valid Solana public key")
        );
        invalid = request();
        invalid.amount_lamports = 0;
        assert_eq!(
            validate_aggregator_compare_request(&invalid),
            Err("amount_lamports must be greater than zero")
        );
        invalid = request();
        invalid.slippage_bps = 3001;
        assert_eq!(
            validate_aggregator_compare_request(&invalid),
            Err("slippage_bps must be between 1 and 3000")
        );
    }

    #[test]
    fn compare_validation_rejects_empty_provider_filter() {
        let mut invalid = request();
        invalid.allowed_aggregators = Some(Vec::new());
        assert_eq!(
            validate_aggregator_compare_request(&invalid),
            Err("allowed_aggregators must not be empty")
        );
    }

    #[tokio::test]
    async fn error_responses_redact_provider_urls() {
        let response = error(
            StatusCode::BAD_GATEWAY,
            "provider https://rpc.example.test/?api-key=secret failed",
        );
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"], "provider <redacted-url> failed");
    }

    #[test]
    fn compare_defaults_to_jupiter_only() {
        assert_eq!(selected_compare_providers(None), vec![ProviderId::Jupiter]);
        assert_eq!(
            selected_compare_providers(Some(vec![ProviderId::Dflow])),
            vec![ProviderId::Dflow]
        );
    }

    #[test]
    fn compare_quote_binding_rejects_wallet_and_route_mismatch() {
        let request = AggregatorQuoteRequest {
            wallet_id: "main".into(),
            wallet_pubkey: "wallet-pubkey".into(),
            input_mint: crate::models::WSOL_MINT.into(),
            output_mint: crate::models::USDC_MINT.into(),
            input_amount_units: 1_000_000,
            slippage_bps: 100,
            balance_snapshot: None,
        };
        let route = vec![crate::aggregator::AggregatorRouteLeg {
            label: "jupiter".into(),
            input_mint: request.input_mint.clone(),
            output_mint: request.output_mint.clone(),
        }];
        let mut quote = AggregatorQuote {
            provider: ProviderId::Jupiter,
            provider_quote_id: "q".into(),
            wallet_id: request.wallet_id.clone(),
            wallet_pubkey: request.wallet_pubkey.clone(),
            input_mint: request.input_mint.clone(),
            output_mint: request.output_mint.clone(),
            input_amount_units: request.input_amount_units,
            expected_output_units: 100,
            minimum_output_units: 99,
            slippage_bps: request.slippage_bps,
            route_fingerprint: format!("{:x}", Sha256::digest(serde_json::to_vec(&route).unwrap())),
            request_hash: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&request).unwrap())
            ),
            created_at_ms: 1,
            expires_at_ms: crate::aggregator_quote::current_time_ms() + 10_000,
            price_impact_percent: None,
            route,
            balance_snapshot: None,
        };
        assert!(validate_compare_quote(&quote, ProviderId::Jupiter, &request).is_ok());
        quote.provider_quote_id.clear();
        assert!(
            validate_compare_quote(&quote, ProviderId::Jupiter, &request)
                .unwrap_err()
                .to_string()
                .contains("quote ID")
        );
        quote.provider_quote_id = "q".into();
        quote.expires_at_ms = crate::aggregator_quote::current_time_ms() + 61_000;
        assert!(
            validate_compare_quote(&quote, ProviderId::Jupiter, &request)
                .unwrap_err()
                .to_string()
                .contains("expiry")
        );
        quote.expires_at_ms = crate::aggregator_quote::current_time_ms() + 10_000;
        quote.balance_snapshot = Some(WalletBalanceSnapshot {
            balance_lamports: 2_000_000,
            fetched_at_ms: aggregator_quote::current_time_ms(),
        });
        assert!(validate_compare_quote(&quote, ProviderId::Jupiter, &request).is_err());
        quote.balance_snapshot = None;
        quote.wallet_id = "other".into();
        assert!(validate_compare_quote(&quote, ProviderId::Jupiter, &request).is_err());
        quote.wallet_id = request.wallet_id.clone();
        quote.route_fingerprint = "wrong".into();
        assert!(validate_compare_quote(&quote, ProviderId::Jupiter, &request).is_err());
        quote.route_fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&quote.route).unwrap())
        );
        quote.minimum_output_units = 90;
        assert!(
            validate_compare_quote(&quote, ProviderId::Jupiter, &request)
                .unwrap_err()
                .to_string()
                .contains("slippage")
        );
        quote.minimum_output_units = 0;
        assert!(validate_compare_quote(&quote, ProviderId::Jupiter, &request).is_err());
    }

    #[test]
    fn compare_quote_binding_rejects_discontinuous_route() {
        let request = AggregatorQuoteRequest {
            wallet_id: "main".into(),
            wallet_pubkey: "wallet-pubkey".into(),
            input_mint: crate::models::WSOL_MINT.into(),
            output_mint: crate::models::USDC_MINT.into(),
            input_amount_units: 1_000_000,
            slippage_bps: 100,
            balance_snapshot: None,
        };
        let route = vec![
            crate::aggregator::AggregatorRouteLeg {
                label: "first".into(),
                input_mint: request.input_mint.clone(),
                output_mint: solana_sdk::pubkey::Pubkey::new_unique().to_string(),
            },
            crate::aggregator::AggregatorRouteLeg {
                label: "second".into(),
                input_mint: solana_sdk::pubkey::Pubkey::new_unique().to_string(),
                output_mint: request.output_mint.clone(),
            },
        ];
        let quote = AggregatorQuote {
            provider: ProviderId::Jupiter,
            provider_quote_id: "q".into(),
            wallet_id: request.wallet_id.clone(),
            wallet_pubkey: request.wallet_pubkey.clone(),
            input_mint: request.input_mint.clone(),
            output_mint: request.output_mint.clone(),
            input_amount_units: request.input_amount_units,
            expected_output_units: 100,
            minimum_output_units: 99,
            slippage_bps: request.slippage_bps,
            route_fingerprint: format!("{:x}", Sha256::digest(serde_json::to_vec(&route).unwrap())),
            request_hash: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&request).unwrap())
            ),
            created_at_ms: 1,
            expires_at_ms: crate::aggregator_quote::current_time_ms() + 10_000,
            price_impact_percent: None,
            route,
            balance_snapshot: None,
        };
        assert!(
            validate_compare_quote(&quote, ProviderId::Jupiter, &request)
                .unwrap_err()
                .to_string()
                .contains("discontinuous")
        );
    }

    #[test]
    fn compare_quote_rejects_malformed_or_excessive_route() {
        let mut quote = AggregatorQuote {
            provider: ProviderId::Jupiter,
            provider_quote_id: "q".into(),
            wallet_id: "main".into(),
            wallet_pubkey: "wallet-pubkey".into(),
            input_mint: crate::models::WSOL_MINT.into(),
            output_mint: crate::models::USDC_MINT.into(),
            input_amount_units: 1_000_000,
            expected_output_units: 100,
            minimum_output_units: 99,
            slippage_bps: 100,
            route_fingerprint: String::new(),
            request_hash: String::new(),
            created_at_ms: 1,
            expires_at_ms: crate::aggregator_quote::current_time_ms() + 10_000,
            price_impact_percent: None,
            route: vec![crate::aggregator::AggregatorRouteLeg {
                label: "jupiter".into(),
                input_mint: crate::models::WSOL_MINT.into(),
                output_mint: crate::models::USDC_MINT.into(),
            }],
            balance_snapshot: None,
        };
        quote.route[0].label.clear();
        assert!(validate_route_continuity(&quote)
            .unwrap_err()
            .to_string()
            .contains("invalid leg"));
        quote.route[0].label = "jupiter".into();
        quote.route = vec![quote.route[0].clone(); 5];
        assert!(validate_route_continuity(&quote)
            .unwrap_err()
            .to_string()
            .contains("too long"));
    }
}

#[cfg(test)]
mod sdk_sell_preview_tests {
    use super::CpmmSellPreviewRequest;

    #[test]
    fn sell_build_preview_does_not_accept_trade_credentials() {
        let mut request = serde_json::json!({
            "wallet_id": "2TE",
            "target_pool_id": "gyYigqG8vdEMkDNTTVL6at2msbHdTDWyb6bccyR238U",
            "bridge_pool_id": "fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2",
            "target_mint": "3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG",
            "quote_mint": crate::models::USDC_MINT,
            "sell_percent": 10,
            "slippage_bps": 100,
        });
        assert!(serde_json::from_value::<CpmmSellPreviewRequest>(request.clone()).is_ok());
        request["sdk_decision_id"] = serde_json::json!("forged");
        assert!(serde_json::from_value::<CpmmSellPreviewRequest>(request).is_err());
    }
}
