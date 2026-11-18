use super::*;
use crate::{
    application::{
        quote_submission_types::QuoteSubmissionCommand,
        sol_buy_submit::test_support::{harness, harness_with_pool, Harness},
    },
    domain::execution_policy::BroadcastPolicy,
};

async fn injected() -> (AppState, Harness) {
    let h = harness().await;
    let mut state = state().await;
    state.quote_submissions = h.service.clone();
    state.aggregator_quotes = h.cache.inner.clone();
    (state, h)
}

fn path(h: &Harness) -> String {
    format!("/v1/quotes/sol-buy/{}/submit", h.bound.quote_id)
}

fn command_body(request: &QuoteSubmissionCommand) -> Value {
    json!({
        "wallet_id": request.wallet_id,
        "idempotency_key": request.idempotency_key,
        "confirmed": request.confirmed,
        "simulate": request.simulate,
    })
}

async fn body(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn quote_submit_missing_wallet_baseline_precedes_validation_and_keeps_extractors() {
    let state = state().await;
    let app = http::router(state.clone());
    let path = "/v1/quotes/sol-buy/missing/submit";
    for simulate in [false, true] {
        error_body(
            send(
                &app,
                "POST",
                path,
                LOCAL,
                Some(TOKEN),
                Some(json!({
                    "wallet_id":"missing", "confirmed":false,
                    "simulate":simulate, "idempotency_key":""
                })),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "wallet_id not found",
        )
        .await;
    }
    assert_eq!(
        send(&app, "POST", path, LOCAL, None, Some(json!({})))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        send(
            &app,
            "POST",
            path,
            LOCAL,
            None,
            Some(json!({
                "wallet_id":"missing", "idempotency_key":"key",
                "confirmed":true, "simulate":false
            })),
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "GET", path, LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert!(state.trades.recent(10).is_empty());
}

#[tokio::test]
async fn quote_submit_http_simulation_and_live_keep_complete_v1_response_and_shared_cache() {
    let (state, h) = injected().await;
    let app = http::router(state.clone());
    let mut fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    fixture["preflight"]["quote_id"] = json!(h.bound.quote_id);
    fixture["preflight"]["wallet_id"] = json!(h.bound.wallet_id);
    for simulate in [true, false] {
        let response = body(
            send(
                &app,
                "POST",
                &path(&h),
                LOCAL,
                Some(TOKEN),
                Some(command_body(&h.command(simulate))),
            )
            .await,
        )
        .await;
        assert_eq!(
            response,
            json!({
                "quote_id": h.bound.quote_id,
                "wallet_id": h.bound.wallet_id,
                "status": if simulate { "simulated" } else { "submitted" },
                "signature": if simulate { Value::Null } else { json!("synthetic-signature") },
                "error": null,
                "preflight": fixture["preflight"],
            })
        );
    }
    let legacy = state
        .aggregator_quotes
        .get(
            &h.bound.quote_id,
            &h.bound.wallet_id,
            &h.bound.wallet_pubkey,
        )
        .unwrap();
    assert_eq!(legacy.quote_id, h.bound.quote_id);
    assert_eq!(state.wallets.wallet_count(), 0);
    assert!(state.trades.recent(10).is_empty());
}

#[tokio::test]
async fn quote_submit_http_durable_replay_survives_lost_cache_and_conflicts_before_cache() {
    let (state, h) = injected().await;
    let app = http::router(state);
    let request = command_body(&h.command(false));
    let original = body(
        send(
            &app,
            "POST",
            &path(&h),
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
    )
    .await;
    h.trace.faults.lock().unwrap().cache = true;
    h.trace.clear();
    let replay = body(
        send(
            &app,
            "POST",
            &path(&h),
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(replay, original);
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
    h.trace.clear();
    error_body(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy/different/submit",
            LOCAL,
            Some(TOKEN),
            Some(request),
        )
        .await,
        StatusCode::CONFLICT,
        "idempotency key was used for another quote or wallet identity",
    )
    .await;
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
}

#[tokio::test]
async fn quote_submit_http_preserves_validation_storage_cache_policy_and_backend_statuses() {
    for mode in 0..7 {
        let (state, h) = injected().await;
        let mut request = h.command(false);
        {
            let mut f = h.trace.faults.lock().unwrap();
            match mode {
                0 => request.idempotency_key.clear(),
                1 => request.confirmed = false,
                2 => f.no_journal = true,
                3 => f.get_at = Some(1),
                4 => f.cache = true,
                5 => f.forbidden = true,
                6 => f.prepare = true,
                _ => unreachable!(),
            }
        }
        let expected = [
            StatusCode::BAD_REQUEST,
            StatusCode::BAD_REQUEST,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GONE,
            StatusCode::FORBIDDEN,
            StatusCode::BAD_GATEWAY,
        ][mode];
        let app = http::router(state);
        let response = send(
            &app,
            "POST",
            &path(&h),
            LOCAL,
            Some(TOKEN),
            Some(command_body(&request)),
        )
        .await;
        assert_eq!(response.status(), expected, "mode {mode}");
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert!(value["error"].is_string());
        assert!(!value.to_string().contains("https://"));
        assert!(!h.trace.events().contains(&"sign"));
        assert!(!h.trace.events().contains(&"broadcast"));
        if mode == 6 {
            assert!(value["error"]
                .as_str()
                .unwrap()
                .contains("synthetic prepare failure"));
        }
    }
}

#[tokio::test]
async fn quote_submit_http_signature_write_failure_blocks_send_and_replay_never_signs_again() {
    let (state, h) = injected().await;
    h.trace.faults.lock().unwrap().record_signed = true;
    let app = http::router(state);
    let request = command_body(&h.command(false));
    let response = send(
        &app,
        "POST",
        &path(&h),
        LOCAL,
        Some(TOKEN),
        Some(request.clone()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(h.trace.events().contains(&"sign"));
    assert!(!h.trace.events().contains(&"broadcast"));
    h.trace.clear();
    h.trace.faults.lock().unwrap().cache = true;
    let replay = body(send(&app, "POST", &path(&h), LOCAL, Some(TOKEN), Some(request)).await).await;
    assert_eq!(replay["status"], "submitting");
    assert!(replay["signature"].is_null());
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
}

#[tokio::test]
async fn quote_submit_http_timeout_returns_unknown_with_original_signature_without_rebroadcast() {
    let (state, h) = injected().await;
    h.trace.faults.lock().unwrap().timeout = true;
    let app = http::router(state);
    let request = command_body(&h.command(false));
    let original = body(
        send(
            &app,
            "POST",
            &path(&h),
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(original["status"], "unknown");
    assert_eq!(original["signature"], "synthetic-signature");
    assert!(!original["error"].as_str().unwrap().contains("https://"));
    h.trace.clear();
    h.trace.faults.lock().unwrap().cache = true;
    let replay = body(send(&app, "POST", &path(&h), LOCAL, Some(TOKEN), Some(request)).await).await;
    assert_eq!(original, replay);
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
}

#[tokio::test]
async fn quote_submit_http_auth_json_and_method_rejections_do_not_resolve_wallet() {
    let (state, h) = injected().await;
    let app = http::router(state);
    let valid = command_body(&h.command(true));
    let cases = [
        (
            "POST",
            LOCAL,
            None,
            Some(valid.clone()),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "POST",
            DENIED,
            Some(TOKEN),
            Some(valid),
            StatusCode::FORBIDDEN,
        ),
        (
            "POST",
            LOCAL,
            None,
            Some(json!({})),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "GET",
            LOCAL,
            Some(TOKEN),
            None,
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ];
    for (method, peer, token, request, expected) in cases {
        assert_eq!(
            send(&app, method, &path(&h), peer, token, request)
                .await
                .status(),
            expected
        );
        assert!(h.trace.events().is_empty());
    }
}

#[tokio::test]
async fn quote_submit_http_unsigned_simulation_ignores_live_policy_without_creating_live_record() {
    let (mut state, h) = injected().await;
    state.edit_fixture_config(|config| config.security.broadcast = BroadcastPolicy::Disabled);
    h.trace.faults.lock().unwrap().forbidden = true;
    let app = http::router(state);
    let response = body(
        send(
            &app,
            "POST",
            &path(&h),
            LOCAL,
            Some(TOKEN),
            Some(command_body(&h.command(true))),
        )
        .await,
    )
    .await;
    assert_eq!(response["status"], "simulated");
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "cache", "prepare", "drop"]
    );
}

#[tokio::test]
async fn quote_submit_http_concurrent_submissions_keep_atomic_single_synthetic_send() {
    let (state, h) = injected().await;
    *h.trace.barrier.lock().unwrap() = Some(Arc::new(tokio::sync::Barrier::new(8)));
    let app = http::router(state);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let app = app.clone();
        let path = path(&h);
        let command = command_body(&h.command(false));
        tasks.push(tokio::spawn(async move {
            body(send(&app, "POST", &path, LOCAL, Some(TOKEN), Some(command)).await).await
        }));
    }
    for task in tasks {
        let value = task.await.unwrap();
        assert!(matches!(
            value["status"].as_str(),
            Some("submitted" | "submitting" | "unknown")
        ));
    }
    let events = h.trace.events();
    assert_eq!(events.iter().filter(|&&e| e == "sign").count(), 1);
    assert_eq!(events.iter().filter(|&&e| e == "broadcast").count(), 1);
}

#[tokio::test]
async fn quote_submit_http_file_restart_replays_saved_signature_without_original_cache() {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    let dir = std::env::temp_dir().join(format!("quote-submit-http-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.join("submissions.sqlite"))
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    let h = harness_with_pool(pool.clone()).await;
    let mut state = state().await;
    state.quote_submissions = h.service.clone();
    state.aggregator_quotes = h.cache.inner.clone();
    let app = http::router(state);
    let original_path = path(&h);
    let request = command_body(&h.command(false));
    h.trace.faults.lock().unwrap().record_broadcast = true;
    assert_eq!(
        send(
            &app,
            "POST",
            &original_path,
            LOCAL,
            Some(TOKEN),
            Some(request.clone())
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        h.trace
            .events()
            .iter()
            .filter(|&&e| e == "broadcast")
            .count(),
        1
    );
    drop(app);
    drop(h);
    pool.close().await;

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let restored = harness_with_pool(pool.clone()).await;
    let mut state = super::state().await;
    state.quote_submissions = restored.service.clone();
    state.aggregator_quotes = restored.cache.inner.clone();
    restored.trace.faults.lock().unwrap().cache = true;
    let app = http::router(state);
    let replay = body(
        send(
            &app,
            "POST",
            &original_path,
            LOCAL,
            Some(TOKEN),
            Some(request),
        )
        .await,
    )
    .await;
    assert_eq!(replay["status"], "unknown");
    assert_eq!(replay["signature"], "synthetic-signature");
    assert_eq!(
        restored.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT signed_transaction FROM quote_submissions WHERE idempotency_key='fixture-key'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
    drop(app);
    drop(restored);
    pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
