use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{middleware::auth as access, response::error},
    application::wallet_management::{WalletFailureKind, WalletManagementError},
    models::{
        CreateWalletRequest, ExportWalletResponse, ManagedWalletResponse, ManagedWalletsResponse,
        UpdateWalletRequest, WalletsResponse,
    },
    AppState,
};

pub(crate) async fn wallets(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = access::authorize_trade(&state.config, &headers, peer) {
        return response;
    }
    (
        StatusCode::OK,
        Json(WalletsResponse::from(state.wallet_management.loaded())),
    )
        .into_response()
}

pub(crate) async fn managed_wallets(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = access::authorize_wallet_admin(&state.config, &headers, peer, false) {
        return response;
    }
    match state.wallet_management.managed().await {
        Ok(wallets) => {
            (StatusCode::OK, Json(ManagedWalletsResponse::from(wallets))).into_response()
        }
        Err(error) => management_failure(error),
    }
}

pub(crate) async fn create_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CreateWalletRequest>,
) -> Response {
    if let Err(response) = access::authorize_wallet_admin(&state.config, &headers, peer, false) {
        return response;
    }
    match state.wallet_management.create(request.into()).await {
        Ok(view) => {
            let status = if view.created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (status, Json(ManagedWalletResponse::from(view))).into_response()
        }
        Err(error) => management_failure(error),
    }
}

pub(crate) async fn update_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
    Json(request): Json<UpdateWalletRequest>,
) -> Response {
    if let Err(response) = access::authorize_wallet_admin(&state.config, &headers, peer, false) {
        return response;
    }
    match state
        .wallet_management
        .rename(&wallet_id, &request.label)
        .await
    {
        Ok(view) => (StatusCode::OK, Json(ManagedWalletResponse::from(view))).into_response(),
        Err(error) => management_failure(error),
    }
}

pub(crate) async fn delete_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
) -> Response {
    if let Err(response) = access::authorize_wallet_admin(&state.config, &headers, peer, false) {
        return response;
    }
    match state.wallet_management.delete(&wallet_id).await {
        Ok(view) => (StatusCode::OK, Json(ManagedWalletResponse::from(view))).into_response(),
        Err(error) => management_failure(error),
    }
}

pub(crate) async fn export_wallet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
) -> Response {
    if let Err(response) = access::authorize_wallet_admin(&state.config, &headers, peer, false) {
        return response;
    }
    match state.wallet_management.export(&wallet_id).await {
        Ok(exported) => (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(ExportWalletResponse::from(exported)),
        )
            .into_response(),
        Err(error) => management_failure(error),
    }
}

fn management_failure(error_value: WalletManagementError) -> Response {
    let status = match error_value.kind {
        WalletFailureKind::InvalidRequest => StatusCode::BAD_REQUEST,
        WalletFailureKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        WalletFailureKind::LoadFailed => StatusCode::BAD_GATEWAY,
        WalletFailureKind::RemoveFailed => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error(status, error_value.message)
}
