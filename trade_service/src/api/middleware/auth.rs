use crate::{
    auth,
    config::{security::WalletAdminAuth, Config},
    models::ErrorResponse,
};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

pub(crate) fn authorize_trade(
    config: &Config,
    headers: &HeaderMap,
    peer: SocketAddr,
) -> Result<(), Response> {
    if !config.trade_client_ip_allowed(peer.ip()) {
        tracing::warn!(client_ip = %peer.ip(), "trade api client ip rejected");
        return Err(error(StatusCode::FORBIDDEN, "client ip not allowed"));
    }
    if !auth::authorized(headers, &config.local_trade_api_token) {
        return Err(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }
    Ok(())
}

pub(crate) fn authorize_internal(
    config: &Config,
    headers: &HeaderMap,
    peer: SocketAddr,
) -> Result<(), Response> {
    if !peer.ip().is_loopback() {
        tracing::warn!(client_ip = %peer.ip(), "internal wallet api client ip rejected");
        return Err(error(
            StatusCode::FORBIDDEN,
            "internal wallet API is loopback-only",
        ));
    }
    authorize_trade(config, headers, peer)
}

pub(crate) fn authorize_wallet_admin(
    config: &Config,
    headers: &HeaderMap,
    peer: SocketAddr,
    internal: bool,
) -> Result<(), Response> {
    match &config.security.wallet_admin {
        WalletAdminAuth::Legacy => {
            if internal {
                authorize_internal(config, headers, peer)
            } else {
                authorize_trade(config, headers, peer)
            }
        }
        WalletAdminAuth::Separate { token } => {
            if !peer.ip().is_loopback() {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    "wallet admin API is loopback-only",
                ));
            }
            if !auth::authorized(headers, token) {
                return Err(error(
                    StatusCode::UNAUTHORIZED,
                    "wallet admin authorization required",
                ));
            }
            Ok(())
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}
