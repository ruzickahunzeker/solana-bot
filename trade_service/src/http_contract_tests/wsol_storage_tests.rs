//! Actual Router + file SQLite faults, with keyless fake execution/query sources.
use super::*;
use crate::{
    application::{wsol_queries::tests as queries, wsol_submission::tests as submissions},
    conversion::{ConversionStore, WsolSubmission},
};

const BASE: &str = "/internal/conversions/wsol";

struct Fixture {
    app: Router,
    pool: sqlx::SqlitePool,
    submitted: Arc<submissions::Shared>,
    queried: Arc<queries::Shared>,
    dir: std::path::PathBuf,
}
impl Fixture {
    async fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("wsol-http-quote-store-{}", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", dir.join("test.sqlite").display());
        let store = Arc::new(ConversionStore::new(&url).await.unwrap());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        let (submissions, submitted) = submissions::harness_with_store(store.clone()).await;
        let (queries, queried) = queries::harness_with_store(store.clone()).await;
        let mut state = state().await;
        let mut config = (*state.config).clone();
        config.conversion_live_enabled = true;
        config.conversion_quotes_enabled = true;
        state.replace_fixture_config(Arc::new(config));
        state.conversions = Some(store);
        state.wsol_submissions = submissions;
        state.wsol_queries = queries;
        Self {
            app: http::router(state),
            pool,
            submitted,
            queried,
            dir,
        }
    }
    async fn post(&self, suffix: &str, input: Value) -> Response {
        send(
            &self.app,
            "POST",
            &format!("{BASE}/{suffix}"),
            LOCAL,
            Some(TOKEN),
            Some(input),
        )
        .await
    }
    async fn close(self) {
        drop(self.app);
        drop(self.submitted);
        drop(self.queried);
        self.pool.close().await;
        tokio::fs::remove_dir_all(self.dir).await.unwrap();
    }
}

#[tokio::test]
async fn wsol_quote_storage_router_corrupt_reads_fail_before_reservation_with_no_panic_or_execution(
) {
    for (assignment, message) in [
        ("quote_json=X'FF'", "stored WSOL quote could not be decoded"),
        (
            "quote_json='null'",
            "stored WSOL quote could not be decoded",
        ),
        (
            "expires_at_ms=-1",
            "stored WSOL quote identity or expiry does not match its row",
        ),
    ] {
        let fixture = Fixture::new().await;
        sqlx::query(&format!("UPDATE conversion_quotes SET {assignment}"))
            .execute(&fixture.pool)
            .await
            .unwrap();
        let wire = serde_json::to_value(WsolSubmission::from(&submissions::input())).unwrap();
        let response = fixture.post("submit", wire.clone()).await;
        assert_eq!(
            response.headers()["x-wsol-error-code"],
            "wsol_integrity_error"
        );
        assert_eq!(response.headers()["x-wsol-error-stage"], "quote_lookup");
        assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
        assert!(response.headers().get("x-wsol-submission-id").is_none());
        assert!(response.headers().get("x-wsol-signature").is_none());
        error_body(response, StatusCode::SERVICE_UNAVAILABLE, &format!("{message}; WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key")).await;
        let mut simulation = wire;
        simulation["simulate"] = json!(true);
        let response = fixture.post("simulate", simulation).await;
        assert_eq!(
            response.headers()["x-wsol-error-code"],
            "wsol_integrity_error"
        );
        assert_eq!(response.headers()["x-wsol-error-stage"], "unsigned_quote");
        error_body(response, StatusCode::SERVICE_UNAVAILABLE,
            &format!("{message}; WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key")).await;
        assert!(fixture
            .submitted
            .store
            .get_by_key("key")
            .await
            .unwrap()
            .is_none());
        assert_eq!(fixture.submitted.events(), ["quote", "quote"]);
        assert!(fixture.queried.events().is_empty());
        fixture.close().await;
    }
}

#[tokio::test]
async fn wsol_quote_storage_router_failed_quote_readback_returns_no_quote_and_rolls_back_all_changes(
) {
    let fixture = Fixture::new().await;
    let original = fixture
        .submitted
        .store
        .get_quote("q1")
        .await
        .unwrap()
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER quote_fault AFTER INSERT ON conversion_quotes BEGIN UPDATE conversion_quotes SET quote_json='null'; END;")
        .execute(&fixture.pool).await.unwrap();
    let response = fixture.post("quotes", json!({"wallet_id":"fixture-wallet","owner_type":"telegram","owner_id":"42","direction":"wrap","amount_lamports":1000000})).await;
    assert_eq!(
        response.headers()["x-wsol-error-code"],
        "wsol_integrity_error"
    );
    assert_eq!(response.headers()["x-wsol-error-stage"], "quote_save");
    error_body(response, StatusCode::SERVICE_UNAVAILABLE,
        "stored WSOL quote could not be decoded; WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key").await;
    assert_eq!(
        fixture.submitted.store.get_quote("q1").await.unwrap(),
        Some(original)
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversion_quotes")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        fixture.queried.events(),
        [
            "metadata",
            "parse_owner",
            "quote_reader",
            "estimate:1",
            "save"
        ]
    );
    assert!(fixture.submitted.events().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn wsol_quote_storage_router_original_key_recovery_survives_corrupt_quote_without_reposting()
{
    let fixture = Fixture::new().await;
    fixture.submitted.set_write_failure(7);
    let response = fixture
        .post(
            "submit",
            serde_json::to_value(WsolSubmission::from(&submissions::input())).unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get("x-wsol-submission-id").is_none());
    let original = fixture
        .submitted
        .store
        .get_by_key("key")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE conversion_quotes SET quote_json=X'FF'")
        .execute(&fixture.pool)
        .await
        .unwrap();
    let path = format!("{BASE}/submissions/by-key?wallet_id=w1&owner_type=telegram&owner_id=123&idempotency_key=key");
    let body = json_body(
        send(&fixture.app, "GET", &path, LOCAL, Some(TOKEN), None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(body["submission_id"], original.submission_id);
    assert_eq!(body["status"], "reserved");
    assert_eq!(body["signature"], Value::Null);
    assert_eq!(
        fixture
            .submitted
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(original)
    );
    assert_eq!(fixture.submitted.events(), ["quote", "reserve"]);
    assert_eq!(fixture.queried.events(), ["recovery"]);
    fixture.close().await;
}
