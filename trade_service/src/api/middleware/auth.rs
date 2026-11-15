use crate::{
    auth,
    config::security::{ApiAccessPolicy, WalletAdminAuth},
    models::ErrorResponse,
};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

pub(crate) fn authorize_trade(
    policy: ApiAccessPolicy<'_>,
    headers: &HeaderMap,
    peer: SocketAddr,
) -> Result<(), Response> {
    if !policy.trade_client_ip_allowed(peer.ip()) {
        tracing::warn!(client_ip = %peer.ip(), "trade api client ip rejected");
        return Err(error(StatusCode::FORBIDDEN, "client ip not allowed"));
    }
    if !auth::authorized(headers, policy.trade_token) {
        return Err(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }
    Ok(())
}

pub(crate) fn authorize_internal(
    policy: ApiAccessPolicy<'_>,
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
    authorize_trade(policy, headers, peer)
}

pub(crate) fn authorize_wallet_admin(
    policy: ApiAccessPolicy<'_>,
    headers: &HeaderMap,
    peer: SocketAddr,
    internal: bool,
) -> Result<(), Response> {
    match policy.wallet_admin {
        WalletAdminAuth::Legacy => {
            if internal {
                authorize_internal(policy, headers, peer)
            } else {
                authorize_trade(policy, headers, peer)
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

#[cfg(test)]
mod tests;
