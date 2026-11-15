use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{
        dto::{
            quote_submissions::{QuoteSubmissionQuery, QuoteSubmissionStatusResponse},
            sol_buy::{
                BoundSolBuyQuote, QuoteWalletQuery, SolBuyPreflightResponse, SolBuyQuoteRequest,
                SolBuySubmitRequest, SolBuySubmitResponse,
            },
        },
        middleware::auth,
        response::error,
    },
    application::{
        quote_preflight::QuotePreflightError,
        quote_submission::{FailureKind, SubmissionError},
        quote_submission_types::QuoteSubmissionCommand,
        sol_buy_quotes::QuoteQueryError,
    },
    AppState,
};

fn quote_query_error(err: QuoteQueryError) -> Response {
    match err {
        QuoteQueryError::InvalidRequest(err) => error(StatusCode::BAD_REQUEST, err.to_string()),
        QuoteQueryError::WalletMissing => error(StatusCode::BAD_REQUEST, "wallet_id not found"),
        QuoteQueryError::Backend(err) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
        QuoteQueryError::CacheWrite(err) => error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
        QuoteQueryError::CacheRead(err) => error(StatusCode::GONE, err.to_string()),
    }
}

pub(crate) async fn sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SolBuyQuoteRequest>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
        return response;
    }
    match state.quote_queries.create(request.query_input()).await {
        Ok(bound) => (StatusCode::OK, Json(BoundSolBuyQuote::from(bound))).into_response(),
        Err(err) => quote_query_error(err),
    }
}

pub(crate) async fn get_sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(quote_id): Path<String>,
    Query(query): Query<QuoteWalletQuery>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
        return response;
    }
    match state.quote_queries.get(&quote_id, &query.wallet_id) {
        Ok(bound) => (StatusCode::OK, Json(BoundSolBuyQuote::from(bound))).into_response(),
        Err(err) => quote_query_error(err),
    }
}

pub(crate) async fn preflight_sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(quote_id): Path<String>,
    Query(query): Query<QuoteWalletQuery>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
        return response;
    }
    match state
        .quote_preflights
        .inspect(&quote_id, &query.wallet_id)
        .await
    {
        Ok(report) => (StatusCode::OK, Json(SolBuyPreflightResponse::from(report))).into_response(),
        Err(QuotePreflightError::WalletMissing) => {
            error(StatusCode::BAD_REQUEST, "wallet_id not found")
        }
        Err(QuotePreflightError::Cache(err)) => error(StatusCode::GONE, err.to_string()),
        Err(QuotePreflightError::Backend(err)) => error(
            StatusCode::BAD_GATEWAY,
            crate::sanitize::redact_urls(&err.to_string()),
        ),
    }
}

pub(crate) async fn submit_sol_buy_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(quote_id): Path<String>,
    Json(request): Json<SolBuySubmitRequest>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
        return response;
    }
    let request = QuoteSubmissionCommand::from(request);
    match state.quote_submissions.submit(&quote_id, &request).await {
        Ok(response) => {
            (StatusCode::OK, Json(SolBuySubmitResponse::from(response))).into_response()
        }
        Err(err) => quote_submission_error(err),
    }
}

pub(crate) async fn reconcile_sol_buy_submission(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(quote_id): Path<String>,
    Query(query): Query<QuoteSubmissionQuery>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
        return response;
    }
    match state
        .quote_reconciliations
        .reconcile(&quote_id, &query.wallet_id, &query.idempotency_key)
        .await
    {
        Ok(result) => (
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(QuoteSubmissionStatusResponse::from(result)),
        )
            .into_response(),
        Err(err) => quote_submission_error(err),
    }
}

fn quote_submission_error(err: SubmissionError) -> Response {
    let status = match err.kind {
        FailureKind::InvalidRequest => StatusCode::BAD_REQUEST,
        FailureKind::Forbidden => StatusCode::FORBIDDEN,
        FailureKind::NotFound => StatusCode::NOT_FOUND,
        FailureKind::Expired => StatusCode::GONE,
        FailureKind::Conflict => StatusCode::CONFLICT,
        FailureKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        FailureKind::Backend => StatusCode::BAD_GATEWAY,
    };
    error(status, err.message)
}
