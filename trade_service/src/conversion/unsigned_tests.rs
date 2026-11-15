//! Internal admission/failure transactions with temporary SQLite and invalid synthetic artifacts.
use super::*;
use crate::adapters::persistence::wsol_conversion::reservation_fingerprint;
use std::sync::Arc;

async fn count(store: &ConversionStore) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM conversion_submissions")
        .fetch_one(&store.pool)
        .await
        .unwrap()
}
async fn reserve(store: &ConversionStore) -> ConversionRecord {
    store
        .reserve_live(&contract_tests::submission(), &contract_tests::quote(), 1)
        .await
        .unwrap()
        .0
}
async fn stamp(store: &ConversionStore) -> i64 {
    sqlx::query_scalar(
        "SELECT updated_at_ms FROM conversion_submissions WHERE idempotency_key='fixture-key'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn wsol_reservation_post_insert_faults_roll_back_the_entire_new_operation() {
    let mut faults = vec![
        (
            "BEFORE",
            "SELECT RAISE(FAIL,'synthetic insert rejection');".to_owned(),
        ),
        ("BEFORE", "SELECT RAISE(IGNORE);".to_owned()),
        (
            "AFTER",
            "DELETE FROM conversion_submissions WHERE submission_id=NEW.submission_id;".to_owned(),
        ),
    ];
    for assignment in [
        "submission_id='corrupt'",
        "idempotency_key='corrupt'",
        "fingerprint='corrupt'",
        "quote_id='corrupt'",
        "wallet_id='corrupt'",
        "wallet_pubkey='corrupt'",
        "owner_type='corrupt'",
        "owner_id='corrupt'",
        "status='failed'",
        "status='corrupt'",
        "signature='unexpected'",
        "signed_transaction=X'04'",
        "error='unexpected'",
        "created_at_ms=99",
        "updated_at_ms=99",
        "status='signed',signature='synthetic',signed_transaction=X'04'",
    ] {
        faults.push(("AFTER", format!("UPDATE conversion_submissions SET {assignment} WHERE submission_id=NEW.submission_id;")));
    }
    for (timing, body) in faults {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(&format!("CREATE TRIGGER corrupt_reserve {timing} INSERT ON conversion_submissions BEGIN {body} END;"))
            .execute(&store.pool).await.unwrap();
        assert!(
            store
                .reserve_live(&contract_tests::submission(), &contract_tests::quote(), 1)
                .await
                .is_err(),
            "{body}"
        );
        assert_eq!(count(&store).await, 0, "{body}");
    }
}

#[tokio::test]
async fn wsol_historical_reservation_requires_full_binding_and_coherent_evidence_not_only_hash() {
    for assignment in [
        "submission_id=''",
        "quote_id='corrupt'",
        "wallet_id='corrupt'",
        "wallet_pubkey='corrupt'",
        "owner_type='corrupt'",
        "owner_id='corrupt'",
        "status='corrupt'",
        "error='dirty reservation'",
        "signature='unexpected'",
        "signed_transaction=X'04'",
        "status='signed'",
        "status='confirmed'",
        "status='failed',signature='unexpected'",
        "status='submitted',signature='',signed_transaction=X'04'",
    ] {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        reserve(&store).await;
        sqlx::query(&format!("UPDATE conversion_submissions SET {assignment}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let before = store.get_by_key("fixture-key").await.unwrap().unwrap();
        assert!(
            store
                .reserve_live(
                    &contract_tests::submission(),
                    &contract_tests::quote(),
                    2_000
                )
                .await
                .is_err(),
            "{assignment}"
        );
        assert_eq!(store.get_by_key("fixture-key").await.unwrap(), Some(before));
        assert_eq!(count(&store).await, 1);
        assert_eq!(stamp(&store).await, 1);
    }
}

#[tokio::test]
async fn wsol_internal_reservation_preserves_v1_fingerprint_history_priority_and_utf8_keys() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let quote = WsolQuoteSnapshot::from(&contract_tests::quote());
    let input = contract_tests::submission().input();
    let fingerprint = reservation_fingerprint(&input, &quote).unwrap();
    assert_eq!(
        fingerprint,
        "ce2b1bb83be3c1f5bc569a1bd8842846469e71d3051ad9e28b8b0777ee30700e"
    );
    let (record, created) = store
        .reserve_bound(&input, &quote, &fingerprint, 1)
        .await
        .unwrap();
    assert!(created);
    assert_eq!(
        store
            .reserve_bound(&input, &quote, &fingerprint, u64::MAX)
            .await
            .unwrap(),
        (record.clone(), false)
    );
    let mut changed = input.clone();
    changed.confirmed = !changed.confirmed;
    let changed_hash = reservation_fingerprint(&changed, &quote).unwrap();
    assert_eq!(
        store
            .reserve_bound(&changed, &quote, &changed_hash, 2)
            .await
            .unwrap_err()
            .to_string(),
        "idempotency key was used with different conversion parameters"
    );
    changed = input.clone();
    changed.idempotency_key = "new-key".into();
    let changed_hash = reservation_fingerprint(&changed, &quote).unwrap();
    assert_eq!(
        store
            .reserve_bound(&changed, &quote, &changed_hash, quote.expires_at_ms)
            .await
            .unwrap_err()
            .to_string(),
        "conversion quote expired"
    );
    for (key, accepted) in [("\u{e9}".repeat(64), true), ("\u{e9}".repeat(65), false)] {
        let mut next = input.clone();
        next.idempotency_key = key;
        assert_eq!(reservation_fingerprint(&next, &quote).is_ok(), accepted);
    }
    assert_eq!(store.get_by_key("fixture-key").await.unwrap(), Some(record));
    assert_eq!(stamp(&store).await, 1);
}

#[tokio::test]
async fn wsol_unsigned_failure_binds_identity_and_rejects_dirty_or_already_signed_rows() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let original = reserve(&store).await;
    for field in 0..8 {
        let mut wrong = original.clone();
        crate::application::wsol_submission::tests::corrupt_record(&mut wrong, field);
        assert!(store
            .fail_before_sign_bound(&wrong, "synthetic failure", 2)
            .await
            .is_err());
        assert_eq!(
            store.get_by_key("fixture-key").await.unwrap(),
            Some(original.clone())
        );
    }
    assert!(store
        .fail_before_sign_bound(&original, "synthetic", u64::MAX)
        .await
        .is_err());
    for assignment in [
        "signature='unexpected'",
        "signed_transaction=X'04'",
        "error='dirty'",
        "status='unknown'",
        "status='failed'",
    ] {
        let other = ConversionStore::new("sqlite::memory:").await.unwrap();
        let reserved = reserve(&other).await;
        sqlx::query(&format!("UPDATE conversion_submissions SET {assignment}"))
            .execute(&other.pool)
            .await
            .unwrap();
        let before = other.get_by_key("fixture-key").await.unwrap().unwrap();
        assert!(other
            .fail_before_sign_bound(&reserved, "synthetic", 2)
            .await
            .is_err());
        assert!(other
            .fail_before_sign_bound(&before, "synthetic", 2)
            .await
            .is_err());
        assert_eq!(other.get_by_key("fixture-key").await.unwrap(), Some(before));
    }
    store
        .record_signed(
            &original.submission_id,
            "synthetic-signature",
            &[1, 2, 3],
            2,
        )
        .await
        .unwrap();
    let signed = store.get_by_key("fixture-key").await.unwrap().unwrap();
    assert!(store
        .fail_before_sign_bound(&original, "late prepare failure", 3)
        .await
        .is_err());
    assert_eq!(store.get_by_key("fixture-key").await.unwrap(), Some(signed));
}

#[tokio::test]
async fn wsol_unsigned_failure_write_and_readback_corruption_roll_back_and_errors_are_redacted() {
    let mut faults = vec![
        (
            "BEFORE",
            "SELECT RAISE(FAIL,'synthetic failure write rejection');".to_owned(),
        ),
        ("BEFORE", "SELECT RAISE(IGNORE);".to_owned()),
        (
            "AFTER",
            "DELETE FROM conversion_submissions WHERE submission_id=NEW.submission_id;".to_owned(),
        ),
    ];
    for assignment in [
        "submission_id='corrupt'",
        "idempotency_key='corrupt'",
        "fingerprint='corrupt'",
        "quote_id='corrupt'",
        "wallet_id='corrupt'",
        "wallet_pubkey='corrupt'",
        "owner_type='corrupt'",
        "owner_id='corrupt'",
        "status='reserved'",
        "signature='unexpected'",
        "signed_transaction=X'04'",
        "error='different'",
        "error=NULL",
        "created_at_ms=99",
        "updated_at_ms=99",
    ] {
        faults.push(("AFTER", format!("UPDATE conversion_submissions SET {assignment} WHERE submission_id=NEW.submission_id;")));
    }
    for (timing, body) in faults {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        let original = reserve(&store).await;
        sqlx::raw_sql(&format!("CREATE TRIGGER corrupt_failure {timing} UPDATE ON conversion_submissions WHEN OLD.status='reserved' AND NEW.status='failed' BEGIN {body} END;"))
            .execute(&store.pool).await.unwrap();
        assert!(
            store
                .fail_before_sign_bound(&original, "synthetic failure", 2)
                .await
                .is_err(),
            "{body}"
        );
        assert_eq!(
            store.get_by_key("fixture-key").await.unwrap(),
            Some(original)
        );
        assert_eq!(stamp(&store).await, 1);
    }
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let original = reserve(&store).await;
    let error = "synthetic failure https://fixture.invalid/private-key";
    let failed = store
        .fail_before_sign_bound(&original, error, 2)
        .await
        .unwrap();
    assert_eq!(
        failed.error.as_deref(),
        Some(crate::sanitize::redact_urls(error).as_str())
    );
    assert!(!failed.error.as_ref().unwrap().contains("private-key"));
    assert!(failed.signature.is_none() && failed.signed_transaction.is_none());
    assert!(store
        .fail_before_sign_bound(&original, "replacement", 3)
        .await
        .is_err());
    assert_eq!(store.get_by_key("fixture-key").await.unwrap(), Some(failed));
}

#[tokio::test]
async fn wsol_unsigned_failure_and_signing_compete_once_and_history_survives_file_reopen() {
    let dir = std::env::temp_dir().join(format!("wsol-unsigned-race-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let a = Arc::new(ConversionStore::new(&url).await.unwrap());
    let original = Arc::new(reserve(&a).await);
    let b = Arc::new(ConversionStore::new(&url).await.unwrap());
    let barrier = Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = vec![];
    for index in 0..16 {
        let store = if index % 2 == 0 { a.clone() } else { b.clone() };
        let expected = original.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            if index % 2 == 0 {
                store
                    .fail_before_sign_bound(&expected, "synthetic failure", 2)
                    .await
            } else {
                store
                    .record_signed_bound(
                        WsolSignedOperation {
                            record: &expected,
                            signature: "synthetic-signature",
                            signed_bytes: &[1, 2, 3],
                        },
                        2,
                    )
                    .await
            }
        }));
    }
    let mut winners = vec![];
    for task in tasks {
        if let Ok(record) = task.await.unwrap() {
            winners.push(record);
        }
    }
    assert_eq!(winners.len(), 1);
    let winner = winners.pop().unwrap();
    assert_eq!(winner.signature.is_some(), winner.status == "signed");
    assert_eq!(
        winner.signed_transaction.is_some(),
        winner.status == "signed"
    );
    a.pool.close().await;
    b.pool.close().await;
    let reopened = ConversionStore::new(&url).await.unwrap();
    assert_eq!(
        reopened.get_by_key("fixture-key").await.unwrap(),
        Some(winner.clone())
    );
    assert!(
        !reopened
            .reserve_live(
                &contract_tests::submission(),
                &contract_tests::quote(),
                2_000
            )
            .await
            .unwrap()
            .1
    );
    assert!(reopened
        .fail_before_sign_bound(&original, "late failure", 3)
        .await
        .is_err());
    assert_eq!(
        reopened.get_by_key("fixture-key").await.unwrap(),
        Some(winner.clone())
    );
    let mut other = contract_tests::submission();
    other.idempotency_key = "another-key".into();
    assert_eq!(
        reopened
            .reserve_live(&other, &contract_tests::quote(), 3)
            .await
            .is_ok(),
        winner.status == "failed"
    );
    reopened.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
