use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;
use tracing::{info, warn};

use crate::{
    adapters::persistence::trade_admission::{AdmissionPayloadV1, RejectionPayloadV1},
    api::{dto::trades::ListTradesQuery, middleware::auth, response::error},
    application::{
        trade_admission::{self, AdmissionDecision, AdmissionError},
        trade_queries::is_pending_status,
    },
    executor,
    models::{
        BatchTradeItemResponse, BatchTradeRequest, BatchTradeResponse, TradeRecord, TradeRequest,
        TradeResponse, TradeStatus,
    },
    state::new_request_id,
    validation::{validate_execution_policy, validate_trade},
    AppState,
};

pub(crate) async fn create_trade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<TradeRequest>,
) -> Response {
    if let Err(response) = auth::authorize_trade(&state.config, &headers, peer) {
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
    let wallet_id = forced_wallet_id.unwrap_or_else(|| {
        state
            .wallet_identity
            .resolve_id(request.wallet_id.as_deref())
    });
    request.wallet_id = Some(wallet_id.clone());

    let Some(wallet) = state.wallet_identity.lookup(&wallet_id) else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("wallet_id {wallet_id} not found"),
        ));
    };
    let wallet_pubkey = wallet.pubkey().to_string();

    let normalized = match validate_trade(&request, &state.config) {
        Ok(normalized) => normalized,
        Err(error) => {
            let rejection_error = error.to_string();
            let payload = RejectionPayloadV1::new(&wallet_id, &wallet_pubkey, &request);
            let record = trade_admission::reject(
                state.trades.as_ref(),
                &payload,
                new_request_id(),
                rejection_error,
                crate::state::now_ms,
            );
            warn!(
                request_id = %record.request_id,
                wallet_id = %record.wallet_id,
                idempotency_key = %request.idempotency_key,
                side = ?request.side,
                pool = ?request.pool,
                mint = %request.mint,
                status = ?record.status,
                error = ?record.error,
                "trade rejected"
            );
            if let Err(persist_error) =
                trade_admission::mirror_rejection(state.persistence.as_ref(), &payload, &record)
                    .await
            {
                warn!(error = %persist_error, request_id = %record.request_id, "failed to persist rejected trade");
            }
            return Ok(AcceptTradeOutcome {
                http_status: StatusCode::BAD_REQUEST,
                wallet_id,
                response: TradeResponse::from(record),
            });
        }
    };

    validate_execution_policy(&normalized, &state.config)
        .map_err(|err| (StatusCode::FORBIDDEN, err.to_string()))?;
    let command = request
        .to_command(wallet_id.clone(), &normalized, &state.config)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let payload =
        AdmissionPayloadV1::new(&wallet_id, &wallet_pubkey, &request, &normalized, &command);
    let (record, execution) = match trade_admission::accept(
        state.persistence.as_ref(),
        state.trades.as_ref(),
        state.idempotency.as_ref(),
        &payload,
        new_request_id(),
    )
    .await
    {
        Ok(AdmissionDecision::Duplicate(existing)) => {
            return Ok(AcceptTradeOutcome {
                http_status: StatusCode::OK,
                wallet_id,
                response: TradeResponse::from(existing),
            })
        }
        Ok(AdmissionDecision::Created {
            record,
            shadow_error,
            execution,
        }) => {
            if let Some(error) = shadow_error {
                warn!(error = %error, request_id = %record.request_id, "shadow trade persistence failed");
            }
            (record, execution)
        }
        Err(AdmissionError::Internal(error)) => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
        }
        Err(AdmissionError::Conflict(message)) => {
            return Err((StatusCode::CONFLICT, message.into()))
        }
        Err(AdmissionError::PersistenceUnavailable(error)) => {
            warn!(error = %error, "durable idempotency reservation failed; trade not started");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "trade persistence unavailable; trade was not started".into(),
            ));
        }
    };
    info!(
        request_id = %record.request_id,
        wallet_id = %record.wallet_id,
        idempotency_key = %request.idempotency_key,
        side = ?request.side,
        pool = ?request.pool,
        mint = %request.mint,
        status = ?TradeStatus::Accepted,
        "trade accepted"
    );

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
    if let Err(response) = auth::authorize_trade(&state.config, &headers, peer) {
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
    if let Err(response) = auth::authorize_trade(&state.config, &headers, peer) {
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
    if let Err(response) = auth::authorize_trade(&state.config, &headers, peer) {
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
