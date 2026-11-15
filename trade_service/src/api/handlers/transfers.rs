use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{
        dto::sol_transfers::{SolTransferRequest, SolTransferResponse},
        middleware::auth,
        response::error,
    },
    application::sol_transfers::{SolTransferPolicy, TransferFailureKind},
    AppState,
};

pub(crate) async fn transfer_sol(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
    Json(request): Json<SolTransferRequest>,
) -> Response {
    if let Err(response) =
        auth::authorize_internal(state.config.api_access_policy(), &headers, peer)
    {
        return response;
    }
    let policy = SolTransferPolicy {
        broadcast: state.config.security.broadcast,
        allow_system_wallet: state.config.allow_internal_system_wallet_transfer,
    };
    match state
        .sol_transfers
        .submit(&wallet_id, request.input(), policy)
        .await
    {
        Ok(outcome) => Json(SolTransferResponse::from(outcome)).into_response(),
        Err(err) => {
            let status = match err.kind {
                TransferFailureKind::BadRequest => StatusCode::BAD_REQUEST,
                TransferFailureKind::Forbidden => StatusCode::FORBIDDEN,
                TransferFailureKind::NotFound => StatusCode::NOT_FOUND,
                TransferFailureKind::Conflict => StatusCode::CONFLICT,
                TransferFailureKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            };
            error(status, err.message)
        }
    }
}
