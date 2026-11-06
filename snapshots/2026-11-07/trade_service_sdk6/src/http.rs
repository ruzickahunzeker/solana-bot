use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::net::SocketAddr;
use tokio::time::{sleep, Duration, Instant};
use tracing::{info, warn};

use crate::aggregator_quote::{self, SolBuyQuoteRequest, SolBuySubmitRequest};
use crate::auth;
use crate::config::WalletConfig;
use crate::conversion::{self, WsolQuoteRequest, WsolSubmission};
use crate::executor;
use crate::idempotency::ReserveResult;
use crate::models::{
    BatchTradeItemResponse, BatchTradeRequest, BatchTradeResponse, CreateVanityJobRequest,
    CreateWalletRequest, ErrorResponse, ExportWalletResponse, HealthResponse,
    ManagedWalletResponse, ManagedWalletsResponse, QuoteBalanceResponse, ReadyResponse,
    ResolvePoolRequest, ResolveSolHopRequest, ResolveSolHopResponse, SolTransferAmountMode,
    SolTransferRequest, SolTransferResponse, TradeRequest, TradeResponse, TradeStatus,
    UpdateWalletRequest, WalletBalanceItem, WalletBalancesResponse, WalletsResponse, WarmupRequest,
    WarmupResponse,
};
use crate::persistence::DurableReserve;
use crate::state::{cache_key, new_request_id};
use crate::validation::{validate_trade, validate_warmup};
use crate::wallet_repository::{ManagedWallet, SolTransferRecord};
use crate::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/wallets", get(wallets).post(create_wallet))
        .route("/v1/wallets/manage", get(managed_wallets))
        .route(
            "/v1/wallets/:wallet_id",
            patch(update_wallet).delete(delete_wallet),
        )
        .route("/v1/wallets/:wallet_id/export", post(export_wallet))
        .route("/v1/cache/warmup", post(warmup))
        .route("/v1/pools/resolve", post(resolve_pool))
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
    ))
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
            let message = error_value.to_string();
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
                    Some(&error_value.to_string()),
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

    if let Err(err) = state
        .wallets
        .default()
        .warmup_params(
            request.pool,
            &normalized.mint,
            &normalized.quote_mint,
            normalized.pool_id.as_ref(),
        )
        .await
    {
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
    let mint = match request.mint.parse() {
        Ok(mint) => mint,
        Err(_) => return error(StatusCode::BAD_REQUEST, "mint must be a valid pubkey"),
    };
    match state.wallets.default().resolve_pool(&mint).await {
        Ok(resolved) => {
            let key = cache_key(resolved.pool, &resolved.mint, resolved.pool_id.as_deref());
            state.params.insert(
                key,
                resolved.pool,
                resolved.mint.clone(),
                resolved.pool_id.clone(),
            );
            (StatusCode::OK, Json(resolved)).into_response()
        }
        Err(err) => error(StatusCode::BAD_GATEWAY, err.to_string()),
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
        .resolve_stonkfun_sol_hop(&quote_mint)
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
    if let Err(err) = aggregator_quote::validate_request(&request) {
        return error(StatusCode::BAD_REQUEST, err.to_string());
    }
    let Some(wallet) = state.wallets.get(&request.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    match aggregator_quote::quote(&request).await {
        Ok(quote) => {
            match state
                .aggregator_quotes
                .insert(&request, wallet.wallet().to_string(), quote)
            {
                Ok(bound) => (StatusCode::OK, Json(bound)).into_response(),
                Err(err) => error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
            }
        }
        Err(err) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
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
    let bound =
        match state
            .aggregator_quotes
            .get(&quote_id, &query.wallet_id, &wallet.wallet().to_string())
        {
            Ok(bound) => bound,
            Err(err) => return error(StatusCode::GONE, err.to_string()),
        };
    match aggregator_quote::preflight(&bound, &wallet).await {
        Ok(result) => match state.aggregator_quotes.get(
            &quote_id,
            &query.wallet_id,
            &wallet.wallet().to_string(),
        ) {
            Ok(_) => (StatusCode::OK, Json(result)).into_response(),
            Err(err) => error(StatusCode::GONE, err.to_string()),
        },
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
    let Some(wallet) = state.wallets.get(&request.wallet_id) else {
        return error(StatusCode::BAD_REQUEST, "wallet_id not found");
    };
    let bound = match state.aggregator_quotes.get(
        &quote_id,
        &request.wallet_id,
        &wallet.wallet().to_string(),
    ) {
        Ok(bound) => bound,
        Err(err) => return error(StatusCode::GONE, err.to_string()),
    };
    let key = format!("{}:{}", request.wallet_id, request.idempotency_key);
    if let Some(existing) = state.aggregator_quotes.get_submission(&key) {
        if existing.quote_id != quote_id {
            return error(
                StatusCode::CONFLICT,
                "idempotency key was used for another quote",
            );
        }
        return (StatusCode::OK, Json(existing)).into_response();
    }
    match aggregator_quote::submit(&bound, &wallet, &request).await {
        Ok(response) => {
            state
                .aggregator_quotes
                .record_submission(key, response.clone());
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(err) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
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

    let normalized = match validate_trade(&request, &state.config) {
        Ok(normalized) => normalized,
        Err(error) => {
            let record = state.trades.rejected(
                &request,
                wallet_id.clone(),
                wallet_pubkey,
                error.to_string(),
            );
            warn!(
                request_id = %record.request_id,
                wallet_id = %record.wallet_id,
                idempotency_key = %request.idempotency_key,
                side = ?request.side,
                pool = ?request.pool,
                mint = %request.mint,
                status = ?record.status,
                error = ?record.error,
                "trade rejected"
            );
            if let Err(persist_error) = state.persistence.mirror(&record, &request, None).await {
                warn!(error = %persist_error, request_id = %record.request_id, "failed to persist rejected trade");
            }
            return Ok(AcceptTradeOutcome {
                http_status: StatusCode::BAD_REQUEST,
                wallet_id,
                response: TradeResponse::from(record),
            });
        }
    };

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
            error: message.into(),
        }),
    )
        .into_response()
}
