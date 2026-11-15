//! Legacy WSOL storage/wire baselines; no signer, AppState, RPC or production files.
use super::*;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Barrier;

pub(crate) fn quote() -> WsolQuote {
    WsolQuote {
        quote_id: "cq_fixture".into(),
        wallet_id: "fixture-wallet".into(),
        wallet_pubkey: crate::domain::trade::Address::from_bytes([1; 32]).to_string(),
        owner_type: "telegram".into(),
        owner_id: "42".into(),
        direction: WsolDirection::Unwrap,
        amount_lamports: 1_000_000,
        source_balance_lamports: 2_000_000,
        ata_exists: true,
        estimated_network_fee_lamports: 10_000,
        temporary_rent_lamports: 2_039_280,
        rent_refund_lamports: 0,
        expires_at_ms: 1_000,
    }
}

pub(crate) fn submission() -> WsolSubmission {
    WsolSubmission {
        quote_id: "cq_fixture".into(),
        wallet_id: "fixture-wallet".into(),
        owner_type: "telegram".into(),
        owner_id: "42".into(),
        simulate: false,
        confirmed: true,
        idempotency_key: "fixture-key".into(),
    }
}

async fn count(store: &ConversionStore) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM conversion_submissions")
        .fetch_one(&store.pool)
        .await
        .unwrap()
}

#[test]
fn wsol_v1_fingerprint_has_frozen_legacy_serialized_bytes() {
    let quote = quote();
    let input = submission();
    let expected=concat!(
        "[{\"quote_id\":\"cq_fixture\",\"wallet_id\":\"fixture-wallet\",\"owner_type\":\"telegram\",\"owner_id\":\"42\",\"simulate\":false,\"confirmed\":true,\"idempotency_key\":\"fixture-key\"},",
        "{\"quote_id\":\"cq_fixture\",\"wallet_id\":\"fixture-wallet\",\"wallet_pubkey\":\"4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi\",\"owner_type\":\"telegram\",\"owner_id\":\"42\",\"direction\":\"unwrap\",",
        "\"amount_lamports\":1000000,\"source_balance_lamports\":2000000,\"ata_exists\":true,\"estimated_network_fee_lamports\":10000,\"temporary_rent_lamports\":2039280,\"rent_refund_lamports\":0,\"expires_at_ms\":1000}]"
    );
    assert_eq!(serde_json::to_string(&(&input, &quote)).unwrap(), expected);
    assert_eq!(
        input.fingerprint(&quote).unwrap(),
        "ce2b1bb83be3c1f5bc569a1bd8842846469e71d3051ad9e28b8b0777ee30700e"
    );
}

#[test]
fn wsol_wire_defaults_exact_receipts_and_stored_quote_json() {
    let quote = quote();
    let expected = json!({
        "quote_id":"cq_fixture", "wallet_id":"fixture-wallet",
        "wallet_pubkey":crate::domain::trade::Address::from_bytes([1;32]).to_string(),
        "owner_type":"telegram", "owner_id":"42", "direction":"unwrap",
        "amount_lamports":1_000_000, "source_balance_lamports":2_000_000,
        "ata_exists":true, "estimated_network_fee_lamports":10_000,
        "temporary_rent_lamports":2_039_280, "rent_refund_lamports":0,
        "expires_at_ms":1_000,
    });
    assert_eq!(serde_json::to_value(&quote).unwrap(), expected);
    let preview = WsolQuotePreview {
        quote,
        executable: false,
    };
    assert_eq!(
        serde_json::to_value(preview).unwrap(),
        json!({"quote":expected,"executable":false})
    );
    let request: WsolQuoteRequest = serde_json::from_value(json!({
        "wallet_id":"fixture-wallet", "owner_type":"telegram", "owner_id":"42",
        "direction":"wrap", "unknown":"ignored"
    }))
    .unwrap();
    assert!(!request.all);
    assert_eq!(request.amount_lamports, None);
    let mut input = serde_json::to_value(submission()).unwrap();
    input.as_object_mut().unwrap().remove("confirmed");
    assert!(
        !serde_json::from_value::<WsolSubmission>(input.clone())
            .unwrap()
            .confirmed
    );
    input.as_object_mut().unwrap().remove("simulate");
    assert!(serde_json::from_value::<WsolSubmission>(input).is_err());
    let simulation = WsolSimulationResult {
        status: "simulated",
        quote_id: "cq_fixture".into(),
        network_fee_lamports: Some(10_000),
        units_consumed: None,
    };
    assert_eq!(
        serde_json::to_value(simulation).unwrap(),
        json!({
            "status":"simulated", "quote_id":"cq_fixture", "network_fee_lamports":10_000,
            "units_consumed":null,
        })
    );
}

#[tokio::test]
async fn wsol_receipt_seven_fields_and_quote_bytes_are_preserved() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let quote = quote();
    store.save_quote(&quote).await.unwrap();
    let raw: String = sqlx::query_scalar("SELECT quote_json FROM conversion_quotes")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(raw, serde_json::to_string(&quote).unwrap());
    let (record, created) = store.reserve_live(&submission(), &quote, 1).await.unwrap();
    assert!(created);
    let id = record.submission_id.clone();
    assert_eq!(
        serde_json::to_value(WsolSubmissionResult::from_record(record, false)).unwrap(),
        json!({
            "submission_id":id, "quote_id":"cq_fixture", "wallet_id":"fixture-wallet",
            "status":"reserved", "signature":null, "error":null, "duplicate":false,
        })
    );
}

#[test]
fn wsol_fingerprint_binds_all_quote_fields_flags_and_literal_key_bytes() {
    let quote = quote();
    let request = submission();
    let fingerprint = request.fingerprint(&quote).unwrap();
    let quote_json = serde_json::to_value(&quote).unwrap();
    for field in quote_json.as_object().unwrap().keys() {
        let mut changed = quote_json.clone();
        let value = changed.get_mut(field).unwrap();
        *value = match value {
            serde_json::Value::String(s) => json!(format!("changed-{s}")),
            serde_json::Value::Number(n) => json!(n.as_u64().unwrap() + 1),
            serde_json::Value::Bool(b) => json!(!*b),
            _ => unreachable!(),
        };
        if field == "direction" {
            changed[field] = json!("wrap");
        }
        let changed: WsolQuote = serde_json::from_value(changed).unwrap();
        match request.fingerprint(&changed) {
            Ok(value) => assert_ne!(value, fingerprint, "{field}"),
            Err(error) => assert_eq!(
                error.to_string(),
                "submission does not match conversion quote"
            ),
        }
    }
    for field in ["simulate", "confirmed", "idempotency_key"] {
        let mut changed = serde_json::to_value(&request).unwrap();
        changed[field] = if field == "idempotency_key" {
            json!(" fixture-key ")
        } else {
            json!(!changed[field].as_bool().unwrap())
        };
        let changed: WsolSubmission = serde_json::from_value(changed).unwrap();
        assert_ne!(changed.fingerprint(&quote).unwrap(), fingerprint);
    }
    for (key, accepted) in [
        (" ".into(), false),
        ("a".repeat(128), true),
        ("a".repeat(129), false),
        ("\u{e9}".repeat(64), true),
        ("\u{e9}".repeat(65), false),
    ] {
        let mut changed = request.clone();
        changed.idempotency_key = key;
        assert_eq!(changed.fingerprint(&quote).is_ok(), accepted);
    }
}

#[tokio::test]
async fn wsol_expired_duplicates_read_history_but_new_keys_and_simulation_do_not_reserve() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let request = submission();
    let quote = quote();
    let (first, _) = store.reserve_live(&request, &quote, 999).await.unwrap();
    let (duplicate, created) = store.reserve_live(&request, &quote, 1_000).await.unwrap();
    assert!(!created);
    assert_eq!(first, duplicate);
    let mut changed = request.clone();
    changed.idempotency_key = "new".into();
    assert_eq!(
        store
            .reserve_live(&changed, &quote, 1_000)
            .await
            .unwrap_err()
            .to_string(),
        "conversion quote expired"
    );
    changed.simulate = true;
    assert_eq!(
        store
            .reserve_live(&changed, &quote, 1)
            .await
            .unwrap_err()
            .to_string(),
        "simulation does not reserve a live idempotency key"
    );
    assert_eq!(count(&store).await, 1);
}

#[tokio::test]
async fn wsol_write_faults_leave_previous_evidence_and_terminal_states_reject_late_writes() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let request = submission();
    let (reserved, _) = store.reserve_live(&request, &quote(), 1).await.unwrap();
    let id = &reserved.submission_id;
    for (status, action) in [("signed", 0), ("failed", 1), ("submitted", 2)] {
        sqlx::raw_sql(&format!("CREATE TRIGGER reject_update BEFORE UPDATE ON conversion_submissions WHEN NEW.status='{status}' BEGIN SELECT RAISE(FAIL,'synthetic conversion write failure'); END;"))
            .execute(&store.pool).await.unwrap();
        let before = store
            .get_by_key(&request.idempotency_key)
            .await
            .unwrap()
            .unwrap();
        let result = match action {
            0 => {
                store
                    .record_signed(id, "synthetic-signature", &[1, 2, 3], 2)
                    .await
            }
            1 => {
                store
                    .mark_failed_before_sign(id, "synthetic failure", 2)
                    .await
            }
            _ => store.mark_broadcast_result(id, "submitted", None, 3).await,
        };
        assert!(result.is_err());
        assert_eq!(store.get_by_id(id).await.unwrap(), Some(before));
        sqlx::raw_sql("DROP TRIGGER reject_update")
            .execute(&store.pool)
            .await
            .unwrap();
        if action == 1 {
            store
                .record_signed(id, "synthetic-signature", &[1, 2, 3], 2)
                .await
                .unwrap();
        }
    }
    store
        .mark_broadcast_result(id, "confirmed", None, 4)
        .await
        .unwrap();
    let terminal = store.get_by_id(id).await.unwrap().unwrap();
    assert_eq!(terminal.signature.as_deref(), Some("synthetic-signature"));
    assert_eq!(terminal.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
    assert!(store
        .mark_broadcast_result(id, "unknown", None, 5)
        .await
        .is_err());
    assert!(store.mark_failed_before_sign(id, "late", 5).await.is_err());
    assert!(store
        .record_signed(id, "replacement", &[4], 5)
        .await
        .is_err());
    assert_eq!(store.get_by_id(id).await.unwrap(), Some(terminal));
}

#[tokio::test]
async fn wsol_two_pool_contention_has_one_reservation_and_pending_wallet_is_global() {
    let dir = std::env::temp_dir().join(format!("wsol-contention-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let a = Arc::new(ConversionStore::new(&url).await.unwrap());
    let b = Arc::new(ConversionStore::new(&url).await.unwrap());
    let barrier = Arc::new(Barrier::new(16));
    let mut tasks = Vec::new();
    for i in 0..16 {
        let store = if i % 2 == 0 { a.clone() } else { b.clone() };
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .reserve_live(&submission(), &quote(), 1)
                .await
                .unwrap()
        }));
    }
    let mut created = 0;
    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        let (record, inserted) = task.await.unwrap();
        created += usize::from(inserted);
        ids.insert(record.submission_id);
    }
    assert_eq!(created, 1);
    assert_eq!(ids.len(), 1);
    assert_eq!(count(&a).await, 1);
    let mut other_request = submission();
    let mut other_quote = quote();
    other_request.idempotency_key = "other-owner-key".into();
    other_request.owner_id = "other-owner".into();
    other_quote.owner_id = "other-owner".into();
    assert_eq!(
        b.reserve_live(&other_request, &other_quote, 1)
            .await
            .unwrap_err()
            .to_string(),
        "wallet has a pending conversion; reconcile it before submitting another"
    );
    assert_eq!(count(&b).await, 1);
    a.pool.close().await;
    b.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn wsol_file_reopen_preserves_reserved_signed_unknown_and_terminal_without_replay() {
    let dir = std::env::temp_dir().join(format!("wsol-reopen-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let store = ConversionStore::new(&url).await.unwrap();
    let mut expected = Vec::new();
    for status in [
        "reserved",
        "signed",
        "submitted",
        "unknown",
        "confirmed",
        "failed",
    ] {
        let mut quote = quote();
        let mut request = submission();
        quote.wallet_id = format!("wallet-{status}");
        request.wallet_id = quote.wallet_id.clone();
        request.idempotency_key = format!("key-{status}");
        let (record, _) = store.reserve_live(&request, &quote, 1).await.unwrap();
        if status == "failed" {
            store
                .mark_failed_before_sign(&record.submission_id, "fixture failure", 2)
                .await
                .unwrap();
        } else if status != "reserved" {
            store
                .record_signed(&record.submission_id, "synthetic-signature", &[1, 2, 3], 2)
                .await
                .unwrap();
            if status != "signed" {
                store
                    .mark_broadcast_result(&record.submission_id, status, None, 3)
                    .await
                    .unwrap();
            }
        }
        expected.push((
            request,
            quote,
            store
                .get_by_id(&record.submission_id)
                .await
                .unwrap()
                .unwrap(),
        ));
    }
    store.pool.close().await;
    let reopened = ConversionStore::new(&url).await.unwrap();
    for (request, quote, record) in expected {
        assert_eq!(
            reopened.get_by_id(&record.submission_id).await.unwrap(),
            Some(record.clone())
        );
        let (duplicate, created) = reopened
            .reserve_live(&request, &quote, 2_000)
            .await
            .unwrap();
        assert!(!created);
        assert_eq!(duplicate, record);
    }
    assert_eq!(count(&reopened).await, 6);
    reopened.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn wsol_late_unknown_no_longer_downgrades_observed_submission() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let (record, _) = store
        .reserve_live(&submission(), &quote(), 1)
        .await
        .unwrap();
    store
        .record_signed(&record.submission_id, "synthetic-signature", &[1, 2, 3], 2)
        .await
        .unwrap();
    store
        .mark_broadcast_result(&record.submission_id, "submitted", None, 3)
        .await
        .unwrap();
    // The original defect baseline is now a monotonic-state regression test.
    store
        .mark_broadcast_result(&record.submission_id, "unknown", Some("late result"), 4)
        .await
        .unwrap();
    let stored = store
        .get_by_id(&record.submission_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "submitted");
    assert_eq!(stored.signature.as_deref(), Some("synthetic-signature"));
}
