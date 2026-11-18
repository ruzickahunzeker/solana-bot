//! Status HTTP boundaries with inert artifacts, fake readers and disposable SQLite.
use super::*;
use crate::{
    application::wsol_queries::{status_boundary_tests::invalid_record, tests as queries},
    conversion::ConversionStore,
    domain::execution_policy::BroadcastPolicy,
};

async fn fixture(store: Option<Arc<ConversionStore>>) -> (Router, Arc<queries::Shared>) {
    let (service, shared) = match store {
        Some(store) => queries::harness_with_store(store).await,
        None => queries::harness().await,
    };
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.security.broadcast = BroadcastPolicy::Disabled;
    config.conversion_live_enabled = false;
    state.replace_fixture_config(Arc::new(config));
    state.conversions = Some(shared.store.clone());
    state.wsol_queries = service;
    (http::router(state), shared)
}

async fn get(app: &Router, id: &str) -> Response {
    send(
        app,
        "GET",
        &format!("/internal/conversions/wsol/submissions/{id}?owner_type=telegram&owner_id=42"),
        LOCAL,
        Some(TOKEN),
        None,
    )
    .await
}

async fn fault(
    response: Response,
    code: &str,
    stage: &str,
    id: Option<&str>,
    signature: Option<&str>,
) {
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-wsol-error-code"], code);
    assert_eq!(response.headers()["x-wsol-error-stage"], stage);
    assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(response.headers().get("retry-after").is_none());
    for (name, expected) in [
        ("x-wsol-submission-id", id),
        ("x-wsol-signature", signature),
    ] {
        assert_eq!(
            response
                .headers()
                .get(name)
                .map(|value| value.to_str().unwrap()),
            expected
        );
    }
    let body = json_body(response, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(body.as_object().unwrap().len(), 1);
    let message = body["error"].as_str().unwrap();
    assert!(message.ends_with("WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key"));
    assert!(!message.contains("private-api-key"));
}

#[tokio::test]
async fn wsol_status_boundary_router_never_publishes_new_evidence_or_replaces_terminal_error() {
    for (status, add_signature) in [("failed", true), ("failed", false), ("confirmed", false)] {
        let (app, shared) = fixture(None).await;
        let original = queries::seed(&shared, status).await;
        let mut current = original.clone();
        if add_signature {
            current.signature = Some("late-untrusted-signature".into());
            current.signed_transaction = Some(vec![4, 5, 6]);
        } else {
            current.error = Some("replacement terminal error".into());
        }
        shared.set_status_records(vec![Some(original.clone()), Some(current)]);
        fault(
            get(&app, &original.submission_id).await,
            "wsol_integrity_error",
            "status_final_read",
            Some(&original.submission_id),
            original.signature.as_deref(),
        )
        .await;
        assert_eq!(shared.events(), ["submission", "submission"]);
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
    }
}

#[tokio::test]
async fn wsol_status_boundary_router_rejects_bad_initial_shape_before_lookup_without_context() {
    for field in (0..=20).filter(|field| !matches!(field, 9 | 10)) {
        let (app, shared) = fixture(None).await;
        let original = queries::seed(&shared, "reserved").await;
        shared.set_status_records(vec![Some(invalid_record(&original, field))]);
        fault(
            get(&app, &original.submission_id).await,
            "wsol_integrity_error",
            "status_read",
            None,
            None,
        )
        .await;
        assert_eq!(shared.events(), ["submission"]);
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
    }
}

#[tokio::test]
async fn wsol_status_boundary_router_rejects_bad_final_state_and_keeps_only_original_context() {
    for field in 0..=20 {
        let (app, shared) = fixture(None).await;
        let original = queries::seed(&shared, "reserved").await;
        shared.set_status_records(vec![
            Some(original.clone()),
            Some(invalid_record(&original, field)),
        ]);
        fault(
            get(&app, &original.submission_id).await,
            "wsol_integrity_error",
            "status_final_read",
            Some(&original.submission_id),
            None,
        )
        .await;
        assert_eq!(shared.events(), ["submission", "submission"]);
    }
    for after in ["signed", "reserved", "unsupported"] {
        let (app, shared) = fixture(None).await;
        let original = queries::seed(&shared, "unknown").await;
        let mut current = original.clone();
        current.status = after.into();
        shared.mode(9);
        shared.set_status_records(vec![Some(original.clone()), Some(current)]);
        fault(
            get(&app, &original.submission_id).await,
            "wsol_integrity_error",
            "status_final_read",
            Some(&original.submission_id),
            Some("synthetic-signature"),
        )
        .await;
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
    }
}

#[tokio::test]
async fn wsol_status_boundary_router_classifies_read_constructor_and_observation_faults_without_retries(
) {
    for (mode, code, stage, status) in [
        (17, "wsol_unavailable", "status_read", "signed"),
        (6, "wsol_unavailable", "status_lookup", "signed"),
        (13, "wsol_outcome_unknown", "status_observation", "signed"),
        (
            19,
            "wsol_outcome_unknown",
            "status_observation",
            "submitted",
        ),
        (18, "wsol_unavailable", "status_final_read", "submitted"),
    ] {
        let (app, shared) = fixture(None).await;
        let original = queries::seed(&shared, "signed").await;
        shared.mode(mode);
        fault(
            get(&app, &original.submission_id).await,
            code,
            stage,
            (mode != 17).then_some(original.submission_id.as_str()),
            (mode != 17).then_some("synthetic-signature"),
        )
        .await;
        let current = shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.status, status);
        assert_eq!(current.signature, original.signature);
        assert_eq!(current.signed_transaction, original.signed_transaction);
        assert!(
            shared
                .events()
                .iter()
                .filter(|event| event.as_str() == "lookup")
                .count()
                <= 1
        );
    }
}

#[tokio::test]
async fn wsol_status_boundary_router_preserves_seven_field_receipts_and_owner_missing_errors() {
    for status in [
        "reserved",
        "signed",
        "unknown",
        "submitted",
        "confirmed",
        "failed",
    ] {
        let (app, shared) = fixture(None).await;
        let original = queries::seed(&shared, status).await;
        shared.mode(9);
        let path = format!(
            "/internal/conversions/wsol/submissions/{}?owner_type=telegram&owner_id=foreign",
            original.submission_id
        );
        let response = send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await;
        assert_eq!(response.headers()["x-wsol-error-code"], "wsol_rejected");
        assert!(response.headers().get("x-wsol-submission-id").is_none());
        assert!(response.headers().get("x-wsol-signature").is_none());
        error_body(
            response,
            StatusCode::BAD_REQUEST,
            "conversion submission does not belong to this owner",
        )
        .await;
        let response = get(&app, &original.submission_id).await;
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = json_body(response, StatusCode::OK).await;
        assert_eq!(
            body,
            serde_json::to_value(
                crate::api::dto::wsol_conversions::WsolSubmissionResult::from_record(
                    original.clone(),
                    false
                )
            )
            .unwrap()
        );
        assert_eq!(body.as_object().unwrap().len(), 7);
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
    }
    let (app, shared) = fixture(None).await;
    let response = get(&app, "missing").await;
    assert_eq!(response.headers()["x-wsol-error-code"], "wsol_rejected");
    assert!(response.headers().get("x-wsol-submission-id").is_none());
    error_body(
        response,
        StatusCode::BAD_REQUEST,
        "conversion submission not found",
    )
    .await;
    assert_eq!(shared.events(), ["submission"]);
}

#[tokio::test]
async fn wsol_status_boundary_router_actual_sqlite_bad_rows_do_not_write_or_load_runtime() {
    for (assignment, code) in [
        ("error='dirty reservation'", "wsol_integrity_error"),
        ("status='unsupported'", "wsol_integrity_error"),
        ("signature='orphan'", "wsol_integrity_error"),
        ("quote_id=X'FF'", "wsol_unavailable"),
    ] {
        let dir =
            std::env::temp_dir().join(format!("wsol-status-bad-row-{}", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", dir.join("test.sqlite").display());
        let (app, shared) =
            fixture(Some(Arc::new(ConversionStore::new(&url).await.unwrap()))).await;
        let original = queries::seed(&shared, "reserved").await;
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query(&format!("UPDATE conversion_submissions SET {assignment}"))
            .execute(&pool)
            .await
            .unwrap();
        let snapshot_sql = "SELECT status,signature,signed_transaction,error,hex(quote_id),updated_at_ms FROM conversion_submissions";
        let before: (
            String,
            Option<String>,
            Option<Vec<u8>>,
            Option<String>,
            String,
            i64,
        ) = sqlx::query_as(snapshot_sql).fetch_one(&pool).await.unwrap();
        fault(
            get(&app, &original.submission_id).await,
            code,
            "status_read",
            None,
            None,
        )
        .await;
        let after: (
            String,
            Option<String>,
            Option<Vec<u8>>,
            Option<String>,
            String,
            i64,
        ) = sqlx::query_as(snapshot_sql).fetch_one(&pool).await.unwrap();
        assert_eq!(before, after);
        assert_eq!(shared.events(), ["submission"]);
        drop(app);
        drop(shared);
        pool.close().await;
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}

#[tokio::test]
async fn wsol_status_boundary_router_redacts_old_rpc_urls_without_rewriting_the_original_record() {
    let dir = std::env::temp_dir().join(format!("wsol-status-redact-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let (app, shared) = fixture(Some(Arc::new(ConversionStore::new(&url).await.unwrap()))).await;
    let original = queries::seed(&shared, "signed").await;
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let history = "old failure https://fixture.invalid/private-api-key";
    sqlx::query("UPDATE conversion_submissions SET error=?")
        .bind(history)
        .execute(&pool)
        .await
        .unwrap();
    let before = shared
        .store
        .get_by_id(&original.submission_id)
        .await
        .unwrap()
        .unwrap();
    shared.mode(9);
    let body = json_body(get(&app, &original.submission_id).await, StatusCode::OK).await;
    assert!(!body["error"].as_str().unwrap().contains("private-api-key"));
    assert_eq!(body["signature"], "synthetic-signature");
    assert_eq!(
        shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(before)
    );
    assert_eq!(
        shared.events(),
        ["submission", "status_reader", "lookup", "submission"]
    );
    drop(app);
    drop(shared);
    pool.close().await;
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
