use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::{
        dto::wsol_conversions::{
            WsolOwnerQuery, WsolQuotePreview, WsolQuoteRequest, WsolRecoveryQuery,
            WsolSimulationResult, WsolSubmission, WsolSubmissionResult,
        },
        middleware::auth,
        response::error,
    },
    application::{
        wsol_failure::{WsolFailure, WsolFailureKind},
        wsol_recovery::WsolRecoveryFailure,
        wsol_submission::WsolSubmitPolicy,
    },
    AppState,
};

pub(crate) async fn wsol_recover(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<WsolRecoveryQuery>,
) -> Response {
    if let Err(response) =
        auth::authorize_internal(state.config.api_access_policy(), &headers, peer)
    {
        return response;
    }
    let mut response = match state.wsol_queries.recover(query.input()).await {
        Ok(result) => Json(WsolSubmissionResult::from(result)).into_response(),
        Err(cause) => {
            let failure = cause
                .downcast::<WsolRecoveryFailure>()
                .unwrap_or(WsolRecoveryFailure::Unavailable);
            let status = match failure {
                WsolRecoveryFailure::Rejected => StatusCode::BAD_REQUEST,
                WsolRecoveryFailure::NotFound => StatusCode::NOT_FOUND,
                WsolRecoveryFailure::Unavailable | WsolRecoveryFailure::Integrity => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            };
            let mut response = error(status, failure.to_string());
            response.headers_mut().insert(
                "x-wsol-error-code",
                HeaderValue::from_static(failure.code()),
            );
            response.headers_mut().insert(
                "x-wsol-error-stage",
                HeaderValue::from_static("recovery_lookup"),
            );
            response
        }
    };
    let headers = response.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("x-wsol-auto-retry", HeaderValue::from_static("false"));
    headers.insert(
        "x-wsol-recovery",
        HeaderValue::from_static("inspect-original-operation"),
    );
    response
}

pub(crate) async fn wsol_quote(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WsolQuoteRequest>,
) -> Response {
    if let Err(response) =
        auth::authorize_internal(state.config.api_access_policy(), &headers, peer)
    {
        return response;
    }
    let mut response = match state.wsol_queries.quote(request.input()).await {
        Ok(quote) => Json(WsolQuotePreview::from(quote)).into_response(),
        Err(err) => operation_error(err),
    };
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn wsol_simulate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WsolSubmission>,
) -> Response {
    if let Err(response) =
        auth::authorize_internal(state.config.api_access_policy(), &headers, peer)
    {
        return response;
    }
    let mut response = match state.wsol_submissions.simulate(request.input()).await {
        Ok(result) => Json(WsolSimulationResult::from(result)).into_response(),
        Err(err) => operation_error(err),
    };
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn wsol_submit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WsolSubmission>,
) -> Response {
    if let Err(response) =
        auth::authorize_internal(state.config.api_access_policy(), &headers, peer)
    {
        return response;
    }
    if let Err(err) = state.config.security.broadcast.ensure_allowed() {
        return error(StatusCode::FORBIDDEN, err.to_string());
    }
    let policy = WsolSubmitPolicy {
        broadcast: state.config.security.broadcast,
        live_enabled: state.config.conversion_live_enabled,
    };
    match state.wsol_submissions.submit(request.input(), policy).await {
        Ok(result) => Json(WsolSubmissionResult::from(result)).into_response(),
        Err(err) => operation_error(err),
    }
}

fn operation_error(error_value: anyhow::Error) -> Response {
    let failure = error_value
        .downcast::<WsolFailure>()
        .unwrap_or_else(WsolFailure::unclassified);
    let status = match failure.kind {
        WsolFailureKind::Rejected => StatusCode::BAD_REQUEST,
        WsolFailureKind::Conflict => StatusCode::CONFLICT,
        WsolFailureKind::Unavailable
        | WsolFailureKind::Integrity
        | WsolFailureKind::OutcomeUnknown => StatusCode::SERVICE_UNAVAILABLE,
    };
    let mut response = error(status, failure.client_message());
    let headers = response.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "x-wsol-error-code",
        HeaderValue::from_static(failure.kind.code()),
    );
    headers.insert(
        "x-wsol-error-stage",
        HeaderValue::from_static(failure.stage.as_str()),
    );
    headers.insert("x-wsol-auto-retry", HeaderValue::from_static("false"));
    let recovery = match failure.kind {
        WsolFailureKind::Rejected => "review-request",
        WsolFailureKind::Conflict => "inspect-original-operation",
        WsolFailureKind::Unavailable => "retry-read-only",
        WsolFailureKind::Integrity | WsolFailureKind::OutcomeUnknown => {
            "reconcile-original-operation"
        }
    };
    headers.insert("x-wsol-recovery", HeaderValue::from_static(recovery));
    for (name, value) in [
        ("x-wsol-submission-id", failure.submission_id.as_deref()),
        ("x-wsol-signature", failure.signature.as_deref()),
    ] {
        if let Some(value) = value.and_then(|value| HeaderValue::from_str(value).ok()) {
            headers.insert(name, value);
        }
    }
    response
}

pub(crate) async fn wsol_status(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(owner): Query<WsolOwnerQuery>,
) -> Response {
    if let Err(response) =
        auth::authorize_internal(state.config.api_access_policy(), &headers, peer)
    {
        return response;
    }
    let mut response = match state
        .wsol_queries
        .status(&id, &owner.owner_type, &owner.owner_id)
        .await
    {
        Ok(result) => Json(WsolSubmissionResult::from(result)).into_response(),
        Err(err) => operation_error(err),
    };
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{
        wsol_failure::{WsolFailureScope, WsolFailureStage as Stage},
        wsol_submission_types::WsolSubmissionRecord,
    };

    #[tokio::test]
    async fn wsol_submit_error_mapping_preserves_one_field_json_and_explicit_no_retry_contract() {
        for (stage, cause, status, code) in [
            (
                Stage::Admission,
                anyhow::anyhow!("bad input"),
                StatusCode::BAD_REQUEST,
                "wsol_rejected",
            ),
            (
                Stage::Reservation,
                WsolFailure::conflict("key conflict").into(),
                StatusCode::CONFLICT,
                "wsol_conflict",
            ),
            (
                Stage::QuoteLookup,
                anyhow::anyhow!("read failure"),
                StatusCode::SERVICE_UNAVAILABLE,
                "wsol_unavailable",
            ),
            (
                Stage::Reservation,
                WsolFailure::integrity("bad row").into(),
                StatusCode::SERVICE_UNAVAILABLE,
                "wsol_integrity_error",
            ),
            (
                Stage::SignedPersistence,
                anyhow::anyhow!("write failure"),
                StatusCode::SERVICE_UNAVAILABLE,
                "wsol_outcome_unknown",
            ),
        ] {
            let original = cause.to_string();
            let mut scope = WsolFailureScope::new();
            scope.stage(stage);
            let response = operation_error(scope.classify(cause));
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-wsol-error-code"], code);
            assert_eq!(response.headers()["x-wsol-error-stage"], stage.as_str());
            assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
            assert!(response.headers().get("retry-after").is_none());
            let body: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 16_384)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(body.as_object().unwrap().len(), 1);
            if status == StatusCode::SERVICE_UNAVAILABLE {
                assert!(body["error"].as_str().unwrap().starts_with(&original));
                assert!(body["error"]
                    .as_str()
                    .unwrap()
                    .contains("do not automatically retry"));
            } else {
                assert_eq!(body, serde_json::json!({"error":original}));
            }
        }
    }

    #[tokio::test]
    async fn wsol_submit_unclassified_error_is_uncertain_not_a_parameter_rejection() {
        let response = operation_error(anyhow::anyhow!("conversion quote expired"));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers()["x-wsol-error-code"],
            "wsol_outcome_unknown"
        );
        assert_eq!(response.headers()["x-wsol-error-stage"], "unclassified");
        assert!(response.headers().get("x-wsol-submission-id").is_none());
        assert!(response.headers().get("x-wsol-signature").is_none());
    }

    #[tokio::test]
    async fn wsol_submit_error_headers_reject_injection_and_do_not_export_wallet_or_key_material() {
        let mut scope = WsolFailureScope::new();
        scope.stage(Stage::BroadcastReceipt);
        scope.remember_record(&WsolSubmissionRecord {
            submission_id: "bad\nidentifier".into(),
            idempotency_key: "must-not-export-this-key".into(),
            fingerprint: "fingerprint".into(),
            quote_id: "quote".into(),
            wallet_id: "wallet".into(),
            wallet_pubkey: "public".into(),
            owner_type: "telegram".into(),
            owner_id: "must-not-export-owner".into(),
            status: "signed".into(),
            signature: None,
            signed_transaction: None,
            error: None,
        });
        scope.remember_signature("bad\nsignature");
        let response = operation_error(scope.classify(anyhow::anyhow!(
            "storage failure https://fixture.invalid/private-api-key"
        )));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(response.headers().get("x-wsol-submission-id").is_none());
        assert!(response.headers().get("x-wsol-signature").is_none());
        let body = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 16_384)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        for secret in [
            "private-api-key",
            "must-not-export-this-key",
            "must-not-export-owner",
            "bad\\nidentifier",
            "bad\\nsignature",
        ] {
            assert!(!body.contains(secret));
        }
    }
}
