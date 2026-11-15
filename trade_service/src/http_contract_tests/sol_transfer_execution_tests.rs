use super::*;
use crate::{
    application::sol_transfers::test_support::{
        harness, harness_with_repository, Harness, Stage, KEY, SIG,
    },
    domain::trade::Address,
};
use std::time::Duration;

const PATH: &str = "/internal/wallets/source/transfer-sol";
fn payload(patch: Value) -> Value {
    let mut value = json!({"owner_type":"telegram","owner_id":"42","to_address":Address::from_bytes([2;32]).to_string(),
        "amount_mode":"fixed","amount_lamports":1_000_000,"idempotency_key":KEY});
    value
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    value
}
async fn app(h: &Harness) -> Router {
    let mut state = state().await;
    state.wallet_repository = h.repo.clone();
    state.sol_transfers = h.service.clone();
    http::router(state)
}
async fn submit(app: &Router, patch: Value) -> Response {
    send(app, "POST", PATH, LOCAL, Some(TOKEN), Some(payload(patch))).await
}
async fn body(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn transfer_http_uses_journal_and_keeps_full_v1_receipt_for_fixed_half_and_all() {
    for (patch, amount) in [
        (json!({}), 1_000_000),
        (
            json!({"amount_mode":"percent","amount_lamports":null,"percent":50}),
            547_060,
        ),
        (
            json!({"amount_mode":"percent","amount_lamports":null,"percent":100}),
            1_995_000,
        ),
    ] {
        let h = harness().await;
        let app = app(&h).await;
        let value = body(submit(&app, patch).await).await;
        let id = value["transfer_id"].as_str().unwrap();
        assert!(id.starts_with("transfer_"));
        assert_eq!(
            value,
            json!({"transfer_id":id,"wallet_id":"source","from_pubkey":Address::from_bytes([1;32]).to_string(),
            "to_pubkey":Address::from_bytes([2;32]).to_string(),"status":"confirmed","amount_lamports":amount,"fee_lamports":5000,
            "balance_before_lamports":2_000_000,"signature":SIG,"error":null,"duplicate":false})
        );
        assert_eq!(h.trace.count("sign"), 1);
        assert_eq!(h.trace.count("broadcast"), 1);
    }
}

#[tokio::test]
async fn transfer_http_equivalent_destination_alias_and_label_change_replay_same_intent() {
    let h = harness().await;
    let app = app(&h).await;
    let original = body(submit(&app, json!({})).await).await;
    sqlx::query("UPDATE wallets SET label='renamed' WHERE wallet_id='target'")
        .execute(&h.pool)
        .await
        .unwrap();
    h.trace.clear();
    let replay = body(submit(&app, json!({"to_address":null,"to_wallet_id":"target"})).await).await;
    assert_eq!(replay["transfer_id"], original["transfer_id"]);
    assert_eq!(replay["signature"], original["signature"]);
    assert_eq!(replay["duplicate"], true);
    assert_eq!(h.trace.count("resolve"), 0);
    assert_eq!(h.trace.count("sign"), 0);
}

#[tokio::test]
async fn transfer_http_prepare_and_sign_failures_keep_failed_receipt_without_broadcast() {
    for sign in [false, true] {
        let h = harness().await;
        h.trace.faults.lock().unwrap().prepare = !sign;
        h.trace.faults.lock().unwrap().sign = sign;
        let app = app(&h).await;
        let value = body(submit(&app, json!({})).await).await;
        assert_eq!(value["status"], "failed");
        assert!(value["signature"].is_null());
        assert!(!value["error"].as_str().unwrap().contains("https://"));
        assert_eq!(h.trace.count("broadcast"), 0);
        assert_eq!(value.as_object().unwrap().len(), 11);
    }
}

#[tokio::test]
async fn transfer_http_write_failure_after_sign_or_send_is_unavailable_and_duplicate_does_not_retry(
) {
    for sent in [false, true] {
        let h = harness().await;
        let condition = if sent {
            "NEW.status='submitted'"
        } else {
            "NEW.signature IS NOT NULL AND NEW.status='unknown'"
        };
        sqlx::raw_sql(&format!("CREATE TRIGGER reject_stage BEFORE UPDATE ON wallet_sol_transfers WHEN {condition} BEGIN SELECT RAISE(FAIL,'synthetic write failure'); END;")).execute(&h.pool).await.unwrap();
        let app = app(&h).await;
        error_body(
            submit(&app, json!({})).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "SOL transfer journal unavailable; do not resubmit or use a new key",
        )
        .await;
        assert_eq!(h.trace.count("sign"), 1);
        assert_eq!(h.trace.count("broadcast"), usize::from(sent));
        h.trace.clear();
        let replay = body(submit(&app, json!({})).await).await;
        assert_eq!(
            replay["status"],
            if sent { "unknown" } else { "processing" }
        );
        assert_eq!(replay["signature"].is_string(), sent);
        assert_eq!(replay["duplicate"], true);
        assert_eq!(h.trace.count("sign"), 0);
        assert_eq!(h.trace.count("broadcast"), 0);
    }
}

#[tokio::test]
async fn transfer_http_send_errors_keep_unknown_original_signature_and_do_not_confirm_or_resend() {
    for mismatch in [false, true] {
        let h = harness().await;
        h.trace.faults.lock().unwrap().broadcast = !mismatch;
        h.trace.faults.lock().unwrap().different_receipt = mismatch;
        let app = app(&h).await;
        let value = body(submit(&app, json!({})).await).await;
        assert_eq!(value["status"], "unknown");
        assert_eq!(value["signature"], SIG);
        assert!(!value.to_string().contains("https://"));
        assert_eq!(h.trace.count("confirm"), 0);
        h.trace.clear();
        let replay = body(submit(&app, json!({})).await).await;
        assert_eq!(replay["signature"], SIG);
        assert_eq!(replay["duplicate"], true);
        assert_eq!(h.trace.count("broadcast"), 0);
    }
}

#[tokio::test]
async fn transfer_http_cancelled_request_after_signed_record_recovers_without_new_send() {
    let h = harness().await;
    let app = app(&h).await;
    let (entered, _release) = h.trace.gate(Stage::Broadcast);
    let worker = app.clone();
    let task = tokio::spawn(async move { submit(&worker, json!({})).await });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    h.journal().initialize().await.unwrap();
    h.trace.clear();
    let value = body(submit(&app, json!({})).await).await;
    assert_eq!(value["status"], "unknown");
    assert_eq!(value["signature"], SIG);
    assert_eq!(value["duplicate"], true);
    assert_eq!(h.trace.count("sign"), 0);
    assert_eq!(h.trace.count("broadcast"), 0);
}

#[tokio::test]
async fn transfer_http_concurrent_same_key_only_prepares_signs_and_sends_once() {
    let h = harness().await;
    let app = app(&h).await;
    let (entered, release) = h.trace.gate(Stage::Prepare);
    let first = app.clone();
    let task = tokio::spawn(async move { body(submit(&first, json!({})).await).await });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let mut workers = Vec::new();
    for _ in 0..16 {
        let app = app.clone();
        workers.push(tokio::spawn(async move {
            body(submit(&app, json!({})).await).await
        }));
    }
    for worker in workers {
        let value = worker.await.unwrap();
        assert_eq!(value["status"], "processing");
        assert_eq!(value["duplicate"], true);
    }
    release.notify_one();
    assert_eq!(task.await.unwrap()["status"], "confirmed");
    assert_eq!(h.trace.count("prepare"), 1);
    assert_eq!(h.trace.count("sign"), 1);
    assert_eq!(h.trace.count("broadcast"), 1);
}

#[tokio::test]
async fn transfer_http_file_restart_keeps_signed_bytes_and_returns_original_unknown_record() {
    let dir = std::env::temp_dir().join(format!("native-http-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = WalletStorageConfig {
        mode: WalletStorageMode::Database,
        database_url: format!("sqlite://{}", dir.join("wallets.sqlite").display()),
        busy_timeout_ms: 1000,
        passphrase: None,
    };
    let repo = Arc::new(WalletRepository::new(&config, &[], "source").await.unwrap());
    let pool = repo.pool_for_transfer_tests();
    let h = harness_with_repository(repo, pool).await;
    let app = app(&h).await;
    sqlx::raw_sql("CREATE TRIGGER reject_receipt BEFORE UPDATE ON wallet_sol_transfers WHEN NEW.status='submitted' BEGIN SELECT RAISE(FAIL,'synthetic receipt write failure'); END;").execute(&h.pool).await.unwrap();
    assert_eq!(
        submit(&app, json!({})).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.trace.count("broadcast"), 1);
    h.pool.close().await;
    drop(app);
    drop(h);
    let repo = Arc::new(WalletRepository::new(&config, &[], "source").await.unwrap());
    let pool = repo.pool_for_transfer_tests();
    let h = harness_with_repository(repo, pool).await;
    let restored = self::app(&h).await;
    let value = body(submit(&restored, json!({})).await).await;
    assert_eq!(value["status"], "unknown");
    assert_eq!(value["signature"], SIG);
    assert_eq!(value["duplicate"], true);
    assert_eq!(h.trace.count("resolve"), 0);
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT signed_transaction FROM wallet_sol_transfer_attempts")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
    h.pool.close().await;
    drop(restored);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn transfer_http_confirmation_absence_keeps_submitted_signature_without_claiming_confirmation(
) {
    let h = harness().await;
    h.trace.faults.lock().unwrap().unconfirmed = true;
    let app = app(&h).await;
    let value = body(submit(&app, json!({})).await).await;
    assert_eq!(value["status"], "submitted");
    assert_eq!(value["signature"], SIG);
    assert!(value["error"].is_null());
    assert_eq!(h.trace.count("record_confirmed"), 0);
    assert_eq!(h.trace.count("broadcast"), 1);
}
