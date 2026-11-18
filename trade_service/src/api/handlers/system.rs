use axum::{
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::middleware::auth,
    models::{HealthResponse, ReadyResponse},
    AppState,
};

pub(crate) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

pub(crate) async fn ready(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) =
        auth::authorize_trade(state.request_settings.api_access_policy(), &headers, peer)
    {
        return response;
    }
    Json(ReadyResponse::from(state.readiness.inspect().await)).into_response()
}

pub(crate) async fn capabilities(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) =
        auth::authorize_trade(state.request_settings.api_access_policy(), &headers, peer)
    {
        return response;
    }
    Json(crate::protocol_capabilities::current(
        state.request_settings.stonkfun_via_sol_enabled(),
    ))
    .into_response()
}
