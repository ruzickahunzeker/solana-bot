use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{dto::wallets::OwnerQuery, middleware::auth as access, response::error},
    application::vanity::{VanityError, VanityFailureKind},
    models::CreateVanityJobRequest,
    AppState,
};

pub(crate) async fn create_vanity_job(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CreateVanityJobRequest>,
) -> Response {
    if let Err(response) =
        access::authorize_wallet_admin(state.config.api_access_policy(), &headers, peer, true)
    {
        return response;
    }
    match state.vanity_tasks.create(request.into()).await {
        Ok(job) => (StatusCode::ACCEPTED, Json(job)).into_response(),
        Err(error_value) => vanity_failure(error_value),
    }
}

pub(crate) async fn get_vanity_job(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) =
        access::authorize_wallet_admin(state.config.api_access_policy(), &headers, peer, true)
    {
        return response;
    }
    match state
        .vanity_tasks
        .get(&job_id, &owner.owner_type, &owner.owner_id)
        .await
    {
        Ok(job) => Json(job).into_response(),
        Err(error_value) => vanity_failure(error_value),
    }
}

pub(crate) async fn cancel_vanity_job(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Query(owner): Query<OwnerQuery>,
) -> Response {
    if let Err(response) =
        access::authorize_wallet_admin(state.config.api_access_policy(), &headers, peer, true)
    {
        return response;
    }
    match state
        .vanity_tasks
        .cancel(&job_id, &owner.owner_type, &owner.owner_id)
        .await
    {
        Ok(job) => Json(job).into_response(),
        Err(error_value) => vanity_failure(error_value),
    }
}

fn vanity_failure(error_value: VanityError) -> Response {
    let status = match error_value.kind {
        VanityFailureKind::InvalidRequest => StatusCode::BAD_REQUEST,
        VanityFailureKind::NotFound => StatusCode::NOT_FOUND,
        VanityFailureKind::OwnerMismatch => StatusCode::FORBIDDEN,
    };
    error(status, error_value.message)
}
