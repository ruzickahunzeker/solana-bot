use super::*;
use crate::application::vanity::{test_support::Worker, VanityService};

const OWNER: &str = "owner_type=telegram&owner_id=42";

fn input() -> Value {
    json!({"owner_type":"telegram","owner_id":"42","label":" new vanity ","match_side":"prefix","pattern":"9999"})
}

async fn setup(worker: &Worker) -> AppState {
    let mut state = state().await;
    state.vanity_tasks = Arc::new(VanityService::new(worker.clone()));
    state
}

#[tokio::test]
async fn vanity_application_http_creation_query_progress_and_cancel_keep_exact_v1_public_job() {
    let worker = Worker::new();
    let state = setup(&worker).await;
    let app = http::router(state.clone());
    let mut expected = json!({"job_id":"public-job","owner_type":"telegram","owner_id":"42","label":"new vanity","match_side":"prefix","pattern":"9999",
        "status":"queued","attempts":0,"max_attempts":30_000_000,"wallet_id":null,"pubkey":null,"error":null});
    assert_eq!(
        json_body(
            send(
                &app,
                "POST",
                "/internal/wallets/vanity-jobs",
                LOCAL,
                Some(TOKEN),
                Some(input())
            )
            .await,
            StatusCode::ACCEPTED
        )
        .await,
        expected
    );
    worker.state.lock().unwrap().job.attempts = 123;
    expected["attempts"] = json!(123);
    let path = format!("/internal/wallets/vanity-jobs/public-job?{OWNER}");
    assert_eq!(
        json_body(
            send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK
        )
        .await,
        expected
    );
    expected["status"] = json!("cancel_requested");
    assert_eq!(
        json_body(
            send(&app, "DELETE", &path, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK
        )
        .await,
        expected
    );
    assert_eq!(
        worker.events(),
        ["create", "get:public-job", "cancel:public-job:telegram:42"]
    );
    assert_eq!(state.wallets.wallet_count(), 0);
    assert!(state.trades.recent(10).is_empty());
}

#[tokio::test]
async fn vanity_application_http_preserves_distinct_owner_errors_and_worker_failure_statuses() {
    let worker = Worker::new();
    let app = http::router(setup(&worker).await);
    let other = "/internal/wallets/vanity-jobs/public-job?owner_type=system&owner_id=global";
    error_body(
        send(&app, "GET", other, LOCAL, Some(TOKEN), None).await,
        StatusCode::FORBIDDEN,
        "wallet generation job owner mismatch",
    )
    .await;
    error_body(
        send(&app, "DELETE", other, LOCAL, Some(TOKEN), None).await,
        StatusCode::BAD_REQUEST,
        "wallet generation job does not belong to this owner",
    )
    .await;
    assert_eq!(
        worker.events(),
        ["get:public-job", "cancel:public-job:system:global"]
    );
    for (op, method, path, body, status) in [
        (
            "create",
            "POST",
            "/internal/wallets/vanity-jobs".into(),
            Some(input()),
            StatusCode::BAD_REQUEST,
        ),
        (
            "get",
            "GET",
            format!("/internal/wallets/vanity-jobs/public-job?{OWNER}"),
            None,
            StatusCode::NOT_FOUND,
        ),
        (
            "cancel",
            "DELETE",
            format!("/internal/wallets/vanity-jobs/public-job?{OWNER}"),
            None,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let worker = Worker::new();
        worker.state.lock().unwrap().failure = Some(op);
        let app = http::router(setup(&worker).await);
        error_body(
            send(&app, method, &path, LOCAL, Some(TOKEN), body).await,
            status,
            &format!("synthetic {op} failure"),
        )
        .await;
        assert_eq!(worker.events().len(), 1);
    }
}

#[tokio::test]
async fn vanity_application_http_keeps_extractor_admin_loopback_and_broadcast_policy_order() {
    use crate::{config::security::WalletAdminAuth, domain::execution_policy::BroadcastPolicy};
    let worker = Worker::new();
    let mut state = setup(&worker).await;
    let config = Arc::get_mut(&mut state.config).unwrap();
    config.security.wallet_admin = WalletAdminAuth::Separate {
        token: "vanity-test-admin-only".into(),
    };
    config.security.broadcast = BroadcastPolicy::Disabled;
    let app = http::router(state);
    let admin = "Bearer vanity-test-admin-only";
    for (method, path, body) in [
        (
            "POST",
            "/internal/wallets/vanity-jobs".into(),
            Some(input()),
        ),
        (
            "GET",
            format!("/internal/wallets/vanity-jobs/public-job?{OWNER}"),
            None,
        ),
        (
            "DELETE",
            format!("/internal/wallets/vanity-jobs/public-job?{OWNER}"),
            None,
        ),
    ] {
        error_body(
            send(&app, method, &path, LOCAL, Some(TOKEN), body.clone()).await,
            StatusCode::UNAUTHORIZED,
            "wallet admin authorization required",
        )
        .await;
        error_body(
            send(&app, method, &path, ALLOWED, Some(admin), body).await,
            StatusCode::FORBIDDEN,
            "wallet admin API is loopback-only",
        )
        .await;
    }
    let mut bad = input();
    bad["match_side"] = json!("Prefix");
    assert_eq!(
        send(
            &app,
            "POST",
            "/internal/wallets/vanity-jobs",
            DENIED,
            None,
            Some(bad)
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/internal/wallets/vanity-jobs/public-job",
            DENIED,
            None,
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(worker.events().is_empty());
    let mut invalid = input();
    invalid["pattern"] = json!("0");
    error_body(
        send(
            &app,
            "POST",
            "/internal/wallets/vanity-jobs",
            LOCAL,
            Some(admin),
            Some(invalid),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "pattern contains invalid Base58 characters (0, O, I and l are not allowed)",
    )
    .await;
    assert!(worker.events().is_empty());
    assert_eq!(
        send(
            &app,
            "POST",
            "/internal/wallets/vanity-jobs",
            LOCAL,
            Some(admin),
            Some(input())
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(worker.events(), ["create"]);
}
