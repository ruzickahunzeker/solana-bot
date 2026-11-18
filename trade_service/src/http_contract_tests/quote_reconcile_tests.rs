use super::*;
use crate::{
    application::{
        ports::reconciliation::ReconciliationJournal,
        quote_reconciliation::test_support::{
            harness, Answer, Harness, KEY, QUOTE, SIGNATURE, WALLET,
        },
    },
    domain::{
        execution_policy::BroadcastPolicy,
        reconciliation::{ChainCommitment, ChainObservation},
        trade::Address,
    },
};

async fn injected(signed: bool) -> (AppState, Harness) {
    let h = harness(signed).await;
    let mut state = state().await;
    state.quote_reconciliations = h.service.clone();
    (state, h)
}

fn path() -> String {
    format!("/v1/quotes/sol-buy/{QUOTE}/submission?wallet_id={WALLET}&idempotency_key={KEY}")
}

async fn body(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn quote_reconcile_baseline_validates_key_before_missing_wallet_and_keeps_query_rules() {
    let state = state().await;
    let app = http::router(state.clone());
    for (wallet, key, message) in [
        (
            "",
            "",
            "wallet_id is required and must be at most 128 characters",
        ),
        (
            "missing",
            "",
            "idempotency_key is required and must be at most 128 characters",
        ),
        ("missing", "key", "wallet_id not found"),
    ] {
        error_body(
            send(
                &app, "GET",
                &format!("/v1/quotes/sol-buy/missing/submission?wallet_id={wallet}&idempotency_key={key}"),
                LOCAL, Some(TOKEN), None,
            ).await,
            StatusCode::BAD_REQUEST, message,
        ).await;
    }
    let valid = "/v1/quotes/sol-buy/missing/submission?wallet_id=missing&idempotency_key=key";
    assert_eq!(
        send(&app, "GET", valid, LOCAL, None, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/quotes/sol-buy/missing/submission",
            LOCAL,
            None,
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&app, "POST", valid, LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(state.wallets.wallet_count(), 0);
    assert!(state.trades.recent(10).is_empty());
}

#[tokio::test]
async fn quote_reconcile_http_keeps_full_v1_response_and_no_store_even_when_broadcast_disabled() {
    for disabled in [false, true] {
        let (mut state, h) = injected(true).await;
        if disabled {
            state.edit_fixture_config(|config| {
                config.security.broadcast = BroadcastPolicy::Disabled
            });
        }
        let app = http::router(state.clone());
        let value = body(send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await).await;
        assert_eq!(
            value,
            json!({
                "submission_id":h.submission_id, "quote_id":QUOTE, "wallet_id":WALLET,
                "status":"confirmed", "signature":SIGNATURE, "error":null,
                "commitment":"confirmed", "observed_slot":102,
                "reconciliation_status":"updated", "reconciliation_error":null,
            })
        );
        assert_eq!(
            h.trace.events(),
            [
                "resolve", "journal", "reader", "pubkey", "lookup", "rpc", "observe", "lookup",
                "drop"
            ]
        );
        assert!(state.trades.recent(10).is_empty());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}

#[tokio::test]
async fn quote_reconcile_http_unsigned_and_finalized_records_skip_status_queries() {
    for signed in [false, true] {
        let (state, h) = injected(signed).await;
        if signed {
            h.journal()
                .record_observation(
                    &h.submission_id,
                    &ChainObservation {
                        signature: SIGNATURE.into(),
                        commitment: ChainCommitment::Finalized,
                        slot: 999,
                        error: None,
                    },
                )
                .await
                .unwrap();
        }
        *h.trace.answer.lock().unwrap() = Answer::Pending;
        let app = http::router(state);
        let value = body(send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await).await;
        assert_eq!(
            value["status"],
            if signed { "finalized" } else { "submitting" }
        );
        assert_eq!(
            value["signature"],
            if signed {
                json!(SIGNATURE)
            } else {
                Value::Null
            }
        );
        assert_eq!(value["reconciliation_status"], "not_required");
        assert!(value["reconciliation_error"].is_null());
        assert!(h.trace.signatures.lock().unwrap().is_empty());
        assert_eq!(
            h.trace.events(),
            ["resolve", "journal", "reader", "pubkey", "lookup", "drop"]
        );
    }
}

#[tokio::test]
async fn quote_reconcile_http_preserves_key_durable_storage_missing_and_identity_error_statuses() {
    for mode in 0..7 {
        let (state, h) = injected(true).await;
        let mut request_path = path();
        match mode {
            0 => {
                request_path = format!(
                    "/v1/quotes/sol-buy/{QUOTE}/submission?wallet_id={WALLET}&idempotency_key="
                )
            }
            1 => h.trace.faults.lock().unwrap().no_journal = true,
            2 => h.trace.faults.lock().unwrap().lookup_at = Some(1),
            3 => {
                request_path = format!(
                    "/v1/quotes/sol-buy/{QUOTE}/submission?wallet_id={WALLET}&idempotency_key=missing"
                );
            }
            4 => {
                request_path = format!(
                    "/v1/quotes/sol-buy/different/submission?wallet_id={WALLET}&idempotency_key={KEY}"
                );
            }
            5 => {
                h.trace
                    .wallets
                    .lock()
                    .unwrap()
                    .insert(WALLET.into(), Address::from_bytes([9; 32]));
            }
            6 => h.trace.faults.lock().unwrap().observe = true,
            _ => unreachable!(),
        }
        let expected = [
            StatusCode::BAD_REQUEST,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::NOT_FOUND,
            StatusCode::CONFLICT,
            StatusCode::CONFLICT,
            StatusCode::SERVICE_UNAVAILABLE,
        ][mode];
        let app = http::router(state);
        let response = send(&app, "GET", &request_path, LOCAL, Some(TOKEN), None).await;
        assert_eq!(response.status(), expected, "mode {mode}");
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert!(value["error"].is_string());
        assert!(!value.to_string().contains("https://"));
        assert_eq!(
            h.trace.signatures.lock().unwrap().len(),
            usize::from(mode == 6)
        );
    }
}

#[tokio::test]
async fn quote_reconcile_http_misses_errors_and_mismatched_signature_do_not_hide_original_order() {
    for (answer, expected) in [
        (Answer::Missing, "not_found"),
        (Answer::Error, "unavailable"),
        (Answer::Mismatch, "unavailable"),
    ] {
        let (state, h) = injected(true).await;
        *h.trace.answer.lock().unwrap() = answer;
        let app = http::router(state);
        let value = body(send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await).await;
        assert_eq!(value["status"], "unknown");
        assert_eq!(value["signature"], SIGNATURE);
        assert_eq!(value["reconciliation_status"], expected);
        assert!(!value.to_string().contains("https://"));
        assert_eq!(*h.trace.signatures.lock().unwrap(), [SIGNATURE]);
        assert!(!h.trace.events().contains(&"observe"));
    }
}

#[tokio::test]
async fn quote_reconcile_http_finalized_failure_is_terminal_and_error_urls_are_redacted() {
    let (state, h) = injected(true).await;
    *h.trace.answer.lock().unwrap() = Answer::Found(
        ChainCommitment::Finalized,
        Some("synthetic chain error https://rpc.example.invalid/private".into()),
    );
    let app = http::router(state);
    let value = body(send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await).await;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["signature"], SIGNATURE);
    assert_eq!(value["commitment"], "finalized");
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("synthetic chain error"));
    assert!(!value.to_string().contains("https://"));
    h.trace.clear();
    *h.trace.answer.lock().unwrap() = Answer::Pending;
    let replay = body(send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await).await;
    assert_eq!(replay["status"], "failed");
    assert_eq!(replay["signature"], value["signature"]);
    assert_eq!(replay["error"], value["error"]);
    assert_eq!(replay["reconciliation_status"], "not_required");
    assert!(!h.trace.events().contains(&"rpc"));
}

#[tokio::test]
async fn quote_reconcile_http_registry_changes_during_lookup_do_not_change_retained_identity() {
    for remove in [false, true] {
        let (state, h) = injected(true).await;
        h.trace.faults.lock().unwrap().replace_wallet = !remove;
        h.trace.faults.lock().unwrap().remove_wallet = remove;
        let app = http::router(state);
        let value = body(send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await).await;
        assert_eq!(value["status"], "confirmed");
        assert_eq!(*h.trace.resolved_ids.lock().unwrap(), [WALLET]);
        h.trace.clear();
        let response = send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await;
        assert_eq!(
            response.status(),
            if remove {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::CONFLICT
            }
        );
        assert!(!h.trace.events().contains(&"rpc"));
    }
}

#[tokio::test]
async fn quote_reconcile_http_concurrent_finality_queries_cas_once_and_keep_one_original_record() {
    let (state, h) = injected(true).await;
    *h.trace.answer.lock().unwrap() = Answer::Found(ChainCommitment::Finalized, None);
    *h.trace.barrier.lock().unwrap() = Some(Arc::new(tokio::sync::Barrier::new(8)));
    let app = http::router(state);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            body(send(&app, "GET", &path(), LOCAL, Some(TOKEN), None).await).await
        }));
    }
    let mut updated = 0;
    for task in tasks {
        let value = task.await.unwrap();
        assert_eq!(value["status"], "finalized");
        assert_eq!(value["signature"], SIGNATURE);
        updated += usize::from(value["reconciliation_status"] == "updated");
    }
    assert_eq!(updated, 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_submissions")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let bytes: Vec<u8> = sqlx::query_scalar("SELECT signed_transaction FROM quote_submissions")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
}

#[tokio::test]
async fn quote_reconcile_http_auth_query_and_method_rejections_never_resolve_runtime() {
    let (state, h) = injected(true).await;
    let app = http::router(state);
    for (method, request_path, peer, token, expected) in [
        ("GET", path(), LOCAL, None, StatusCode::UNAUTHORIZED),
        ("GET", path(), DENIED, Some(TOKEN), StatusCode::FORBIDDEN),
        (
            "GET",
            "/v1/quotes/sol-buy/missing/submission".into(),
            LOCAL,
            None,
            StatusCode::BAD_REQUEST,
        ),
        (
            "POST",
            path(),
            LOCAL,
            Some(TOKEN),
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ] {
        assert_eq!(
            send(&app, method, &request_path, peer, token, None)
                .await
                .status(),
            expected
        );
        assert!(h.trace.events().is_empty());
    }
}
