use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{dto::trades::ListTradesQuery, middleware::auth, response::error},
    application::trade_queries::is_pending_status,
    executor,
    models::{
        BatchTradeItemResponse, BatchTradeRequest, BatchTradeResponse, TradeRecord, TradeRequest,
        TradeResponse,
    },
    AppState,
};

pub(crate) async fn create_trade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<TradeRequest>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
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

async fn accept_trade_for_wallet(
    state: AppState,
    mut request: TradeRequest,
    forced_wallet_id: Option<String>,
) -> Result<AcceptTradeOutcome, (StatusCode, String)> {
    let outcome = state
        .trade_intake()
        .accept(&mut request, forced_wallet_id)
        .await
        .map_err(|error| {
            use crate::application::trade_intake::IntakeFailureKind;
            let status = match error.kind {
                IntakeFailureKind::UnknownWallet => StatusCode::BAD_REQUEST,
                IntakeFailureKind::Forbidden => StatusCode::FORBIDDEN,
                IntakeFailureKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
                IntakeFailureKind::Conflict => StatusCode::CONFLICT,
                IntakeFailureKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            };
            (status, error.message)
        })?;
    let _wallet_identity = outcome.wallet_identity;
    let wallet_id = outcome.wallet_id;
    let record = outcome.record;
    let execution = match outcome.decision {
        crate::application::trade_intake::IntakeDecision::Rejected => {
            return Ok(AcceptTradeOutcome {
                http_status: StatusCode::BAD_REQUEST,
                wallet_id,
                response: TradeResponse::from(record),
            })
        }
        crate::application::trade_intake::IntakeDecision::Duplicate => {
            return Ok(AcceptTradeOutcome {
                http_status: StatusCode::OK,
                wallet_id,
                response: TradeResponse::from(record),
            })
        }
        crate::application::trade_intake::IntakeDecision::Created(execution) => execution,
    };

    let state_for_task = state.clone();
    tokio::spawn(async move {
        executor::execute_admitted_trade(state_for_task, execution).await;
    });

    let response_record = state
        .trade_queries()
        .wait(&record.request_id, state.config.trade_response_wait_ms)
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

pub(crate) async fn create_batch_trades(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(batch): Json<BatchTradeRequest>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
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

pub(crate) async fn list_trades(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<ListTradesQuery>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
        return response;
    }
    let records: anyhow::Result<Vec<TradeRecord>> = state.trade_queries().recent(query.limit).await;
    match records {
        Ok(records) => (StatusCode::OK, Json(records)).into_response(),
        Err(error_value) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("trade persistence unavailable: {error_value}"),
        ),
    }
}

pub(crate) async fn get_trade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Response {
    if let Err(response) = auth::authorize_trade(state.config.api_access_policy(), &headers, peer) {
        return response;
    }
    match state.trade_queries().get(&request_id).await {
        Ok(Some(record)) => (StatusCode::OK, Json(record)).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "trade not found"),
        Err(error_value) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("trade persistence unavailable: {error_value}"),
        ),
    }
}
