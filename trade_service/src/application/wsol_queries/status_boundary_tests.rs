//! Status snapshots, inert synthetic artifacts and temporary SQLite only.
use super::{
    tests::{harness, harness_with_store, seed, signature_observation},
    *,
};
use crate::application::wsol_failure::{WsolFailure, WsolFailureKind, WsolFailureStage};
use crate::application::wsol_submission_types::WsolSubmissionRecord;
use crate::conversion::ConversionStore;
use std::sync::Arc;

pub(crate) fn invalid_record(
    original: &WsolSubmissionRecord,
    field: usize,
) -> WsolSubmissionRecord {
    let mut record = original.clone();
    match field {
        0 => record.submission_id.push('x'),
        1 => record.submission_id.clear(),
        2 => record.idempotency_key.clear(),
        3 => record.idempotency_key = " ".into(),
        4 => record.idempotency_key = "x".repeat(129),
        5 => record.fingerprint.clear(),
        6 => record.quote_id.clear(),
        7 => record.wallet_id.clear(),
        8 => record.wallet_pubkey.clear(),
        9 => record.owner_type.clear(),
        10 => record.owner_id.clear(),
        11 => record.status = "unsupported".into(),
        12 => record.error = Some("dirty reserved".into()),
        13 => record.signature = Some("unexpected".into()),
        14 => record.signed_transaction = Some(vec![1]),
        15 => record.status = "signed".into(),
        16 => record.status = "confirmed".into(),
        17 => {
            record.status = "failed".into();
            record.signature = Some("orphan".into());
        }
        18 => {
            record.status = "submitted".into();
            record.signature = Some(String::new());
            record.signed_transaction = Some(vec![1]);
        }
        19 => {
            record.status = "failed".into();
            record.signed_transaction = Some(vec![1]);
        }
        20 => {
            record.status = "signed".into();
            record.signature = Some("synthetic-signature".into());
            record.signed_transaction = Some(vec![]);
        }
        _ => unreachable!(),
    }
    record
}

#[tokio::test]
async fn wsol_status_boundary_terminal_evidence_and_error_are_immutable_including_unsigned_failure()
{
    for (status, add_signature) in [("failed", true), ("failed", false), ("confirmed", false)] {
        let (service, shared) = harness().await;
        let original = seed(&shared, status).await;
        let mut current = original.clone();
        if add_signature {
            current.signature = Some("late-synthetic-signature".into());
            current.signed_transaction = Some(vec![4, 5, 6]);
        } else {
            current.error = Some("replacement terminal error".into());
        }
        shared.set_status_records(vec![Some(original.clone()), Some(current)]);
        assert!(
            service
                .status(&original.submission_id, "telegram", "42")
                .await
                .is_err(),
            "{status}, add_signature={add_signature}"
        );
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
async fn wsol_status_boundary_rejects_wrong_ids_and_invalid_initial_shape_before_runtime() {
    for field in 0..=20 {
        let (service, shared) = harness().await;
        let original = seed(&shared, "reserved").await;
        let record = invalid_record(&original, field);
        shared.set_status_records(vec![Some(record.clone()), Some(record.clone())]);
        let error = service
            .status(
                &original.submission_id,
                &record.owner_type,
                &record.owner_id,
            )
            .await
            .err()
            .unwrap();
        let failure = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!(failure.kind, WsolFailureKind::Integrity, "field {field}");
        assert_eq!(failure.stage, WsolFailureStage::StatusRead);
        assert!(failure.submission_id.is_none() && failure.signature.is_none());
        assert_eq!(shared.events(), ["submission"], "field {field}");
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
async fn wsol_status_boundary_keeps_legal_concurrent_progress_including_signed_terminal_failure() {
    for (before, after) in [
        ("reserved", "signed"),
        ("reserved", "confirmed"),
        ("signed", "unknown"),
        ("signed", "confirmed"),
        ("submitted", "failed"),
        ("unknown", "confirmed"),
    ] {
        let (service, shared) = harness().await;
        let original = seed(&shared, before).await;
        let mut current = original.clone();
        current.status = after.into();
        current.signature = Some("synthetic-signature".into());
        current.signed_transaction = Some(vec![1, 2, 3]);
        current.error = (after == "failed").then_some("synthetic chain failure".into());
        shared.mode(9);
        shared.set_status_records(vec![Some(original.clone()), Some(current.clone())]);
        let result = service
            .status(&original.submission_id, "telegram", "42")
            .await
            .unwrap();
        assert_eq!(result.record, current, "{before} -> {after}");
        assert!(!result.duplicate);
        assert!(shared
            .events()
            .iter()
            .all(|event| !event.starts_with("record:")));
    }
}

#[tokio::test]
async fn wsol_status_boundary_classifies_read_constructor_and_observation_faults_without_retry() {
    for (mode, kind, stage, status) in [
        (
            17,
            WsolFailureKind::Unavailable,
            WsolFailureStage::StatusRead,
            "signed",
        ),
        (
            6,
            WsolFailureKind::Unavailable,
            WsolFailureStage::StatusLookup,
            "signed",
        ),
        (
            13,
            WsolFailureKind::OutcomeUnknown,
            WsolFailureStage::StatusObservation,
            "signed",
        ),
        (
            19,
            WsolFailureKind::OutcomeUnknown,
            WsolFailureStage::StatusObservation,
            "submitted",
        ),
        (
            18,
            WsolFailureKind::Unavailable,
            WsolFailureStage::StatusFinalRead,
            "submitted",
        ),
    ] {
        let (service, shared) = harness().await;
        let original = seed(&shared, "signed").await;
        shared.mode(mode);
        let error = service
            .status(&original.submission_id, "telegram", "42")
            .await
            .err()
            .unwrap();
        let failure = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!((failure.kind, failure.stage), (kind, stage));
        assert!(!format!("{failure:?}").contains("private-api-key"));
        assert!(failure
            .client_message()
            .contains("do not automatically retry"));
        assert_eq!(
            failure.submission_id.as_deref(),
            (mode != 17).then_some(original.submission_id.as_str())
        );
        assert_eq!(
            failure.signature.as_deref(),
            (mode != 17).then_some("synthetic-signature")
        );
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
async fn wsol_status_boundary_missing_and_foreign_records_never_publish_unverified_context() {
    for final_read in [false, true] {
        let (service, shared) = harness().await;
        let original = seed(&shared, "reserved").await;
        shared.set_status_records(if final_read {
            vec![Some(original.clone()), None]
        } else {
            vec![None]
        });
        let error = service
            .status(&original.submission_id, "telegram", "42")
            .await
            .err()
            .unwrap();
        let failure = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!(
            failure.kind,
            if final_read {
                WsolFailureKind::Unavailable
            } else {
                WsolFailureKind::Rejected
            }
        );
        assert_eq!(
            failure.submission_id.as_deref(),
            final_read.then_some(original.submission_id.as_str())
        );
        assert!(failure.signature.is_none());
    }
    let (service, shared) = harness().await;
    let original = seed(&shared, "signed").await;
    let mut foreign = original.clone();
    foreign.owner_id = "foreign".into();
    foreign.submission_id.push('x');
    shared.set_status_records(vec![Some(foreign)]);
    let error = service
        .status(&original.submission_id, "telegram", "42")
        .await
        .err()
        .unwrap();
    let failure = error.downcast_ref::<WsolFailure>().unwrap();
    assert_eq!(failure.kind, WsolFailureKind::Rejected);
    assert_eq!(
        failure.to_string(),
        "conversion submission does not belong to this owner"
    );
    assert!(failure.submission_id.is_none() && failure.signature.is_none());
    assert_eq!(shared.events(), ["submission"]);
}

#[tokio::test]
async fn wsol_status_boundary_redacts_historical_error_only_in_returned_snapshot() {
    let (service, shared) = harness().await;
    let original = seed(&shared, "failed").await;
    let mut history = original.clone();
    history.error = Some("old failure https://fixture.invalid/private-api-key".into());
    shared.set_status_records(vec![Some(history.clone()), Some(history.clone())]);
    let result = service
        .status(&original.submission_id, "telegram", "42")
        .await
        .unwrap();
    assert!(!result.record.error.unwrap().contains("private-api-key"));
    assert!(history.error.unwrap().contains("private-api-key"));
    assert_eq!(
        shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(original)
    );
    assert_eq!(shared.events(), ["submission", "submission"]);
}

#[tokio::test]
async fn wsol_status_boundary_lost_observation_ack_reopens_original_finalized_record_without_rpc() {
    let dir = std::env::temp_dir().join(format!("wsol-status-ack-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let (service, shared) =
        harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
    let original = seed(&shared, "signed").await;
    shared.mode(19);
    shared.set_observation(signature_observation(ChainCommitment::Finalized, None));
    let error = service
        .status(&original.submission_id, "telegram", "42")
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<WsolFailure>().unwrap().kind,
        WsolFailureKind::OutcomeUnknown
    );
    assert_eq!(
        shared.events(),
        ["submission", "status_reader", "lookup", "record:confirmed"]
    );
    drop(service);
    drop(shared);
    let (service, shared) =
        harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
    let current = service
        .status(&original.submission_id, "telegram", "42")
        .await
        .unwrap();
    assert_eq!(current.record.status, "confirmed");
    assert_eq!(current.record.signature, original.signature);
    assert_eq!(
        current.record.signed_transaction,
        original.signed_transaction
    );
    assert_eq!(shared.events(), ["submission", "submission"]);
    drop(service);
    drop(shared);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[tokio::test]
async fn wsol_status_boundary_rejects_coherent_regressions_and_invalid_final_shape() {
    for (before, after) in [
        ("unknown", "signed"),
        ("submitted", "unknown"),
        ("confirmed", "signed"),
        ("confirmed", "failed"),
        ("failed", "reserved"),
    ] {
        let (service, shared) = harness().await;
        let original = seed(&shared, before).await;
        let mut current = original.clone();
        current.status = after.into();
        current.error = None;
        shared.mode(9);
        shared.set_status_records(vec![Some(original.clone()), Some(current)]);
        assert!(
            service
                .status(&original.submission_id, "telegram", "42")
                .await
                .is_err(),
            "{before} -> {after}"
        );
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
        assert!(shared
            .events()
            .iter()
            .all(|event| !event.starts_with("record:")));
    }
    for field in 11..=20 {
        let (service, shared) = harness().await;
        let original = seed(&shared, "reserved").await;
        shared.set_status_records(vec![
            Some(original.clone()),
            Some(invalid_record(&original, field)),
        ]);
        assert!(
            service
                .status(&original.submission_id, "telegram", "42")
                .await
                .is_err(),
            "field {field}"
        );
        assert_eq!(shared.events(), ["submission", "submission"]);
    }
}
