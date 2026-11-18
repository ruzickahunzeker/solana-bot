use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{middleware::auth, response::error},
    models::{
        ResolvePoolRequest, ResolvePoolResponse, ResolveSolHopRequest, ResolveSolHopResponse,
        WarmupRequest, WarmupResponse,
    },
    validation::{parse_hop_quote, parse_resolve_mint, validate_warmup},
    AppState,
};

pub(crate) async fn warmup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WarmupRequest>,
) -> Response {
    if let Err(response) =
        auth::authorize_trade(state.request_settings.api_access_policy(), &headers, peer)
    {
        return response;
    }

    let normalized = match validate_warmup(&request) {
        Ok(normalized) => normalized,
        Err(err) => return error(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let key = match state
        .pool_queries
        .warmup(request.to_route_hint(&normalized))
        .await
    {
        Ok(key) => key,
        Err(err) => return error(StatusCode::BAD_GATEWAY, err.to_string()),
    };
    (
        StatusCode::OK,
        Json(WarmupResponse {
            ok: true,
            cache_key: key,
        }),
    )
        .into_response()
}

pub(crate) async fn resolve_pool(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ResolvePoolRequest>,
) -> Response {
    if let Err(response) =
        auth::authorize_trade(state.request_settings.api_access_policy(), &headers, peer)
    {
        return response;
    }
    let mint = match parse_resolve_mint(&request.mint) {
        Ok(mint) => mint,
        Err(_) => return error(StatusCode::BAD_REQUEST, "mint must be a valid pubkey"),
    };
    match state.pool_queries.resolve(mint).await {
        Ok(resolved) => (StatusCode::OK, Json(ResolvePoolResponse::from(resolved))).into_response(),
        Err(err) => error(StatusCode::BAD_GATEWAY, err.to_string()),
    }
}

pub(crate) async fn resolve_sol_hop(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ResolveSolHopRequest>,
) -> Response {
    if let Err(response) =
        auth::authorize_trade(state.request_settings.api_access_policy(), &headers, peer)
    {
        return response;
    }
    if !state.request_settings.stonkfun_via_sol_enabled() {
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
    let quote_mint = match parse_hop_quote(&request.quote_mint) {
        Ok(mint) => mint,
        Err(_) => return error(StatusCode::BAD_REQUEST, "quote_mint must be a valid pubkey"),
    };
    match state.pool_queries.sol_hop(quote_mint).await {
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
