//! Bound journal writes with synthetic bytes and temporary SQLite only.
use super::*;
use std::sync::Arc;

async fn reserve(store: &ConversionStore) -> ConversionRecord {
    store
        .reserve_live(&contract_tests::submission(), &contract_tests::quote(), 1)
        .await
        .unwrap()
        .0
}
async fn sign(store: &ConversionStore, record: &ConversionRecord) -> Result<ConversionRecord> {
    store
        .record_signed_bound(
            WsolSignedOperation {
                record,
                signature: "synthetic-signature",
                signed_bytes: &[1, 2, 3],
            },
            2,
        )
        .await
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
async fn wsol_signed_write_binds_all_eight_identity_fields_and_rejects_empty_artifacts() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let original = reserve(&store).await;
    for field in 0..8 {
        let mut wrong = original.clone();
        crate::application::wsol_submission::tests::corrupt_record(&mut wrong, field);
        assert!(sign(&store, &wrong).await.is_err(), "field {field}");
        assert_eq!(
            store.get_by_id(&original.submission_id).await.unwrap(),
            Some(original.clone())
        );
        assert_eq!(stamp(&store).await, 1);
    }
    for (signature, signed_bytes) in [("", &[1, 2, 3][..]), ("synthetic-signature", &[][..])] {
        assert!(store
            .record_signed_bound(
                WsolSignedOperation {
                    record: &original,
                    signature,
                    signed_bytes
                },
                2
            )
            .await
            .is_err());
    }
    let signed = sign(&store, &original).await.unwrap();
    assert_eq!(signed.status, "signed");
    assert_eq!(signed.signature.as_deref(), Some("synthetic-signature"));
    assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
    original.validate_readback(&signed).unwrap();
    assert_eq!(
        store.get_by_id(&original.submission_id).await.unwrap(),
        Some(signed.clone())
    );
    assert_eq!(stamp(&store).await, 2);
    assert!(sign(&store, &original).await.is_err());
    assert_eq!(
        store.get_by_id(&original.submission_id).await.unwrap(),
        Some(signed)
    );
}

#[tokio::test]
async fn wsol_signed_write_rejects_dirty_reservations_and_stale_or_malformed_snapshots() {
    for sql in [
        "UPDATE conversion_submissions SET status='failed'",
        "UPDATE conversion_submissions SET status='signed'",
        "UPDATE conversion_submissions SET status='unknown'",
        "UPDATE conversion_submissions SET status='submitted'",
        "UPDATE conversion_submissions SET status='confirmed'",
        "UPDATE conversion_submissions SET signature='other'",
        "UPDATE conversion_submissions SET signed_transaction=X'04'",
        "UPDATE conversion_submissions SET error='unexpected reservation error'",
    ] {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        let original = reserve(&store).await;
        sqlx::query(sql).execute(&store.pool).await.unwrap();
        let before = store
            .get_by_id(&original.submission_id)
            .await
            .unwrap()
            .unwrap();
        assert!(sign(&store, &original).await.is_err(), "{sql}");
        assert!(sign(&store, &before).await.is_err(), "{sql}");
        assert_eq!(
            store.get_by_id(&original.submission_id).await.unwrap(),
            Some(before)
        );
        assert_eq!(stamp(&store).await, 1);
    }
    {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        let original = reserve(&store).await;
        sqlx::query("UPDATE conversion_submissions SET fingerprint='corrupt'")
            .execute(&store.pool)
            .await
            .unwrap();
        let changed = store
            .get_by_id(&original.submission_id)
            .await
            .unwrap()
            .unwrap();
        assert!(sign(&store, &original).await.is_err());
        assert_eq!(
            store.get_by_id(&original.submission_id).await.unwrap(),
            Some(changed)
        );
    }
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let original = reserve(&store).await;
    for field in 0..4 {
        let mut malformed = original.clone();
        match field {
            0 => malformed.status = "signed".into(),
            1 => malformed.signature = Some("unexpected".into()),
            2 => malformed.signed_transaction = Some(vec![4]),
            _ => malformed.error = Some("unexpected".into()),
        }
        assert!(sign(&store, &malformed).await.is_err());
    }
    assert!(store
        .record_signed_bound(
            WsolSignedOperation {
                record: &original,
                signature: "synthetic-signature",
                signed_bytes: &[1, 2, 3]
            },
            u64::MAX
        )
        .await
        .is_err());
    assert_eq!(
        store.get_by_id(&original.submission_id).await.unwrap(),
        Some(original)
    );
}

#[tokio::test]
async fn wsol_signed_write_rolls_back_write_rejection_ignore_delete_and_every_readback_corruption()
{
    let mut bodies = vec![
        (
            "BEFORE",
            "SELECT RAISE(FAIL,'synthetic signed write failure');".to_owned(),
        ),
        ("BEFORE", "SELECT RAISE(IGNORE);".to_owned()),
        (
            "AFTER",
            "DELETE FROM conversion_submissions WHERE submission_id=NEW.submission_id;".to_owned(),
        ),
    ];
    for assignment in [
        "submission_id='corrupt-id'",
        "idempotency_key='corrupt-key'",
        "fingerprint='corrupt'",
        "quote_id='corrupt'",
        "wallet_id='corrupt'",
        "wallet_pubkey='corrupt'",
        "owner_type='corrupt'",
        "owner_id='corrupt'",
        "status='unknown'",
        "signature='corrupt'",
        "signed_transaction=X'04'",
        "signed_transaction=NULL",
        "error='corrupt'",
        "created_at_ms=99",
        "updated_at_ms=99",
    ] {
        bodies.push(("AFTER", format!("UPDATE conversion_submissions SET {assignment} WHERE submission_id=NEW.submission_id;")));
    }
    for (timing, body) in bodies {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        let original = reserve(&store).await;
        sqlx::raw_sql(&format!("CREATE TRIGGER corrupt_signed {timing} UPDATE ON conversion_submissions WHEN OLD.status='reserved' AND NEW.status='signed' BEGIN {body} END;"))
            .execute(&store.pool).await.unwrap();
        assert!(sign(&store, &original).await.is_err(), "{body}");
        assert_eq!(
            store.get_by_id(&original.submission_id).await.unwrap(),
            Some(original),
            "{body}"
        );
        assert_eq!(stamp(&store).await, 1);
    }
}

#[tokio::test]
async fn wsol_signed_write_two_pools_have_one_exact_winner_and_reopen_never_resigns() {
    let dir = std::env::temp_dir().join(format!("wsol-signed-write-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let a = Arc::new(ConversionStore::new(&url).await.unwrap());
    let original = Arc::new(reserve(&a).await);
    let b = Arc::new(ConversionStore::new(&url).await.unwrap());
    let barrier = Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = vec![];
    for index in 0..16u8 {
        let store = if index % 2 == 0 { a.clone() } else { b.clone() };
        let record = original.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            let signature = format!("synthetic-signature-{index}");
            let bytes = [index, 1, 2];
            barrier.wait().await;
            store
                .record_signed_bound(
                    WsolSignedOperation {
                        record: &record,
                        signature: &signature,
                        signed_bytes: &bytes,
                    },
                    2,
                )
                .await
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
    assert_eq!(
        a.get_by_id(&original.submission_id).await.unwrap(),
        Some(winner.clone())
    );
    a.pool.close().await;
    b.pool.close().await;
    let reopened = ConversionStore::new(&url).await.unwrap();
    assert_eq!(
        reopened.get_by_id(&original.submission_id).await.unwrap(),
        Some(winner.clone())
    );
    assert!(sign(&reopened, &original).await.is_err());
    let (duplicate, created) = reopened
        .reserve_live(
            &contract_tests::submission(),
            &contract_tests::quote(),
            2_000,
        )
        .await
        .unwrap();
    assert!(!created);
    assert_eq!(duplicate, winner);
    reopened.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn wsol_signed_write_cancelled_during_sqlite_lock_contention_keeps_original_reservation() {
    let dir = std::env::temp_dir().join(format!("wsol-write-cancel-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let store = Arc::new(ConversionStore::new(&url).await.unwrap());
    let reserved = reserve(&store).await;
    let blocker = ConversionStore::new(&url).await.unwrap();
    let transaction = blocker.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let task = tokio::spawn({
        let store = store.clone();
        let reserved = reserved.clone();
        let entered = entered.clone();
        async move {
            entered.notify_one();
            sign(&store, &reserved).await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    transaction.rollback().await.unwrap();
    store.pool.close().await;
    blocker.pool.close().await;
    let reopened = ConversionStore::new(&url).await.unwrap();
    assert_eq!(
        reopened.get_by_id(&reserved.submission_id).await.unwrap(),
        Some(reserved)
    );
    reopened.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
