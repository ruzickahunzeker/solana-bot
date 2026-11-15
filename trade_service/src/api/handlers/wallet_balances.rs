use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{dto::wallets::OwnerQuery, middleware::auth as access, response::error},
    application::wallet_balances::{BalanceFailureKind, BalanceQueryError},
    models::{QuoteBalanceResponse, WalletBalancesResponse},
    AppState,
};

pub(crate) async fn quote_balance(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((wallet_id, mint)): Path<(String, String)>,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) = access::authorize_internal(&state.config, &headers, peer) {
        return response;
    }
    match state
        .wallet_balance_queries
        .quote(&wallet_id, &mint, &owner.owner_type, &owner.owner_id)
        .await
    {
        Ok(balance) => Json(QuoteBalanceResponse::from(balance)).into_response(),
        Err(query_error) => query_failure(query_error),
    }
}

pub(crate) async fn wallet_balances(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) = access::authorize_internal(&state.config, &headers, peer) {
        return response;
    }
    match state
        .wallet_balance_queries
        .native(&owner.owner_type, &owner.owner_id)
        .await
    {
        Ok(balances) => Json(WalletBalancesResponse::from(balances)).into_response(),
        Err(query_error) => query_failure(query_error),
    }
}

fn query_failure(query_error: BalanceQueryError) -> Response {
    let status = match query_error.kind {
        BalanceFailureKind::NotFound => StatusCode::NOT_FOUND,
        BalanceFailureKind::OwnerMismatch => StatusCode::FORBIDDEN,
        BalanceFailureKind::InvalidMint => StatusCode::BAD_REQUEST,
        BalanceFailureKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        BalanceFailureKind::InvalidMetadata => StatusCode::INTERNAL_SERVER_ERROR,
        BalanceFailureKind::Backend => StatusCode::BAD_GATEWAY,
    };
    error(status, query_error.message)
}
