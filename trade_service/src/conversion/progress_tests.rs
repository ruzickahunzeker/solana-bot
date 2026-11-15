//! Temporary journals and synthetic, invalid signing evidence only; no wallets or RPC.
use super::*;
use crate::domain::wsol_progress::{WsolBroadcastOutcome as B, WsolObservedOutcome as O};
use std::sync::Arc;

fn broadcast(value: B) -> WsolUpdate {
    WsolUpdate::Broadcast(value)
}
fn observed(value: O) -> WsolUpdate {
    WsolUpdate::Observation(value)
}

async fn seed(store: &ConversionStore, state: WsolState) -> ConversionRecord {
    let (reserved, _) = store
        .reserve_live(&contract_tests::submission(), &contract_tests::quote(), 1)
        .await
        .unwrap();
    store
        .record_signed(
            &reserved.submission_id,
            "synthetic-signature",
            &[1, 2, 3],
            2,
        )
        .await
        .unwrap();
    let signed = store
        .get_by_id(&reserved.submission_id)
        .await
        .unwrap()
        .unwrap();
    let update = match state {
        WsolState::Signed => return signed,
        WsolState::Unknown => broadcast(B::Unknown),
        WsolState::Submitted => observed(O::Submitted),
        WsolState::Confirmed => observed(O::Confirmed),
        WsolState::Failed => observed(O::Failed),
        WsolState::Reserved => unreachable!(),
    };
    store
        .apply_signed_update(
            WsolSignedOperation::from_record(&signed).unwrap(),
            update,
            None,
            3,
        )
        .await
        .unwrap();
    signed
}
async fn stamp(store: &ConversionStore) -> i64 {
    sqlx::query_scalar(
        "SELECT updated_at_ms FROM conversion_submissions WHERE idempotency_key='fixture-key'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap()
}
async fn apply(
    store: &ConversionStore,
    expected: &ConversionRecord,
    update: WsolUpdate,
    error: Option<&str>,
    now: u64,
) -> Result<()> {
    store
        .apply_signed_update(
            WsolSignedOperation::from_record(expected)?,
            update,
            error,
            now,
        )
        .await
}

#[tokio::test]
async fn wsol_progress_sql_matrix_matches_policy_without_rewriting_weaker_results() {
    for state in [
        WsolState::Signed,
        WsolState::Unknown,
        WsolState::Submitted,
        WsolState::Confirmed,
        WsolState::Failed,
    ] {
        for update in [
            WsolUpdate::SimulationUncertain,
            broadcast(B::Unknown),
            broadcast(B::Submitted),
            observed(O::Submitted),
            observed(O::Confirmed),
            observed(O::Failed),
        ] {
            let store = ConversionStore::new("sqlite::memory:").await.unwrap();
            let original = seed(&store, state).await;
            let before = store
                .get_by_id(&original.submission_id)
                .await
                .unwrap()
                .unwrap();
            let before_stamp = stamp(&store).await;
            let result = apply(&store, &original, update, None, 100).await;
            let after = store
                .get_by_id(&original.submission_id)
                .await
                .unwrap()
                .unwrap();
            match should_advance(state, update) {
                Ok(true) => {
                    result.unwrap();
                    assert_eq!(after.status, update.target().as_str());
                    assert_eq!(stamp(&store).await, 100);
                }
                Ok(false) => {
                    result.unwrap();
                    assert_eq!(after, before);
                    assert_eq!(stamp(&store).await, before_stamp);
                }
                Err(_) => {
                    assert!(result.is_err());
                    assert_eq!(after, before);
                    assert_eq!(stamp(&store).await, before_stamp);
                }
            }
            assert_eq!(after.signature, original.signature);
            assert_eq!(after.signed_transaction, original.signed_transaction);
        }
    }
}

#[tokio::test]
async fn wsol_progress_binds_all_identity_fields_signature_and_exact_persisted_bytes() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let original = seed(&store, WsolState::Signed).await;
    for field in 0..10 {
        let mut wrong = original.clone();
        match field {
            0 => wrong.submission_id.push('x'),
            1 => wrong.idempotency_key.push('x'),
            2 => wrong.fingerprint.push('x'),
            3 => wrong.quote_id.push('x'),
            4 => wrong.wallet_id.push('x'),
            5 => wrong.wallet_pubkey.push('x'),
            6 => wrong.owner_type.push('x'),
            7 => wrong.owner_id.push('x'),
            8 => wrong.signature = Some("other-signature".into()),
            _ => wrong.signed_transaction = Some(vec![4, 5, 6]),
        }
        assert!(
            apply(&store, &wrong, broadcast(B::Submitted), None, 100)
                .await
                .is_err(),
            "field {field}"
        );
        assert_eq!(
            store.get_by_id(&original.submission_id).await.unwrap(),
            Some(original.clone())
        );
        assert_eq!(stamp(&store).await, 2);
    }
    for (signature, bytes) in [
        ("", &[1, 2, 3][..]),
        ("synthetic-signature", &[][..]),
        ("other-signature", &[1, 2, 3][..]),
        ("synthetic-signature", &[4, 5, 6][..]),
    ] {
        assert!(store
            .apply_signed_update(
                WsolSignedOperation {
                    record: &original,
                    signature,
                    signed_bytes: bytes
                },
                broadcast(B::Submitted),
                None,
                100
            )
            .await
            .is_err());
    }
    assert_eq!(
        store.get_by_id(&original.submission_id).await.unwrap(),
        Some(original)
    );
}

#[tokio::test]
async fn wsol_progress_noop_does_not_execute_update_triggers_or_clear_original_evidence() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let original = seed(&store, WsolState::Submitted).await;
    let before = store
        .get_by_id(&original.submission_id)
        .await
        .unwrap()
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_progress BEFORE UPDATE ON conversion_submissions BEGIN SELECT RAISE(FAIL,'no update allowed'); END;").execute(&store.pool).await.unwrap();
    for update in [
        broadcast(B::Unknown),
        broadcast(B::Submitted),
        observed(O::Submitted),
    ] {
        apply(
            &store,
            &original,
            update,
            Some("late uncertain result"),
            100,
        )
        .await
        .unwrap();
        assert_eq!(
            store.get_by_id(&original.submission_id).await.unwrap(),
            Some(before.clone())
        );
        assert_eq!(stamp(&store).await, 3);
    }
}

#[tokio::test]
async fn wsol_progress_write_and_post_update_corruption_roll_back_all_fields() {
    for body in ["SELECT RAISE(FAIL,'synthetic write failure');","UPDATE conversion_submissions SET wallet_pubkey='corrupt' WHERE submission_id=NEW.submission_id;","UPDATE conversion_submissions SET signed_transaction=X'0405' WHERE submission_id=NEW.submission_id;"] {
        let store=ConversionStore::new("sqlite::memory:").await.unwrap();let original=seed(&store,WsolState::Signed).await;
        let timing=if body.starts_with("SELECT") {"BEFORE"}else{"AFTER"};
        sqlx::raw_sql(&format!("CREATE TRIGGER reject_progress {timing} UPDATE ON conversion_submissions WHEN OLD.status='signed' AND NEW.status='submitted' BEGIN {body} END;")).execute(&store.pool).await.unwrap();
        assert!(apply(&store,&original,broadcast(B::Submitted),None,100).await.is_err());
        assert_eq!(store.get_by_id(&original.submission_id).await.unwrap(),Some(original));assert_eq!(stamp(&store).await,2);
    }
}

#[tokio::test]
async fn wsol_progress_unsigned_corrupt_missing_evidence_and_out_of_range_time_fail_closed() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let (reserved, _) = store
        .reserve_live(&contract_tests::submission(), &contract_tests::quote(), 1)
        .await
        .unwrap();
    assert!(WsolSignedOperation::from_record(&reserved).is_err());
    assert!(store
        .apply_signed_update(
            WsolSignedOperation {
                record: &reserved,
                signature: "synthetic-signature",
                signed_bytes: &[1, 2, 3]
            },
            broadcast(B::Submitted),
            None,
            2
        )
        .await
        .is_err());
    store
        .record_signed(
            &reserved.submission_id,
            "synthetic-signature",
            &[1, 2, 3],
            2,
        )
        .await
        .unwrap();
    let original = store
        .get_by_id(&reserved.submission_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        apply(&store, &original, broadcast(B::Submitted), None, u64::MAX)
            .await
            .is_err()
    );
    for sql in [
        "UPDATE conversion_submissions SET status='corrupt'",
        "UPDATE conversion_submissions SET signature=NULL",
        "UPDATE conversion_submissions SET signed_transaction=NULL",
        "UPDATE conversion_submissions SET signature=''",
        "UPDATE conversion_submissions SET signed_transaction=X''",
    ] {
        sqlx::query("UPDATE conversion_submissions SET status='signed',signature='synthetic-signature',signed_transaction=X'010203'").execute(&store.pool).await.unwrap();
        sqlx::query(sql).execute(&store.pool).await.unwrap();
        let before = store
            .get_by_id(&original.submission_id)
            .await
            .unwrap()
            .unwrap();
        assert!(apply(&store, &original, broadcast(B::Submitted), None, 100)
            .await
            .is_err());
        assert_eq!(
            store.get_by_id(&original.submission_id).await.unwrap(),
            Some(before)
        );
        assert_eq!(stamp(&store).await, 2);
    }
}

#[tokio::test]
async fn wsol_progress_terminal_record_never_reopens_when_another_key_owns_pending_wallet() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let original = seed(&store, WsolState::Confirmed).await;
    let mut other = contract_tests::submission();
    other.idempotency_key = "another-key".into();
    let (pending, created) = store
        .reserve_live(&other, &contract_tests::quote(), 4)
        .await
        .unwrap();
    assert!(created);
    for update in [
        broadcast(B::Unknown),
        broadcast(B::Submitted),
        observed(O::Submitted),
        observed(O::Confirmed),
    ] {
        apply(&store, &original, update, None, 100).await.unwrap();
    }
    assert!(apply(
        &store,
        &original,
        observed(O::Failed),
        Some("contradictory terminal"),
        100
    )
    .await
    .is_err());
    assert_eq!(
        store
            .get_by_id(&original.submission_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "confirmed"
    );
    assert_eq!(
        store.get_by_id(&pending.submission_id).await.unwrap(),
        Some(pending)
    );
}

#[tokio::test]
async fn wsol_progress_two_pools_concurrent_results_preserve_terminal_signature_and_survive_reopen()
{
    let dir = std::env::temp_dir().join(format!("wsol-progress-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let a = Arc::new(ConversionStore::new(&url).await.unwrap());
    let original = Arc::new(seed(&a, WsolState::Signed).await);
    let b = Arc::new(ConversionStore::new(&url).await.unwrap());
    let barrier = Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = Vec::new();
    for index in 0..16 {
        let store = if index % 2 == 0 { a.clone() } else { b.clone() };
        let barrier = barrier.clone();
        let original = original.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            let update = match index % 3 {
                0 => broadcast(B::Unknown),
                1 => broadcast(B::Submitted),
                _ => observed(O::Confirmed),
            };
            apply(&store, &original, update, None, 100 + index)
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let terminal = a.get_by_id(&original.submission_id).await.unwrap().unwrap();
    assert_eq!(terminal.status, "confirmed");
    assert_eq!(terminal.signature, original.signature);
    assert_eq!(terminal.signed_transaction, original.signed_transaction);
    a.pool.close().await;
    b.pool.close().await;
    let reopened = ConversionStore::new(&url).await.unwrap();
    apply(
        &reopened,
        &original,
        broadcast(B::Unknown),
        Some("late after reopen"),
        200,
    )
    .await
    .unwrap();
    assert_eq!(
        reopened.get_by_id(&original.submission_id).await.unwrap(),
        Some(terminal)
    );
    let (duplicate, created) = reopened
        .reserve_live(
            &contract_tests::submission(),
            &contract_tests::quote(),
            2_000,
        )
        .await
        .unwrap();
    assert!(!created);
    assert_eq!(duplicate.status, "confirmed");
    reopened.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
