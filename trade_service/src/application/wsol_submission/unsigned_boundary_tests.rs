//! Unsigned previews must never acquire a live key or call sign/send ports.
use super::{
    tests::{harness, harness_with_store, input, Stage as Gate},
    *,
};
use crate::{
    application::wsol_failure::WsolFailureKind,
    conversion::{ConversionStore, WsolSubmission},
};
use std::sync::Arc;

pub(crate) fn simulation_input() -> WsolSubmissionInput {
    let mut request = input();
    request.simulate = true;
    request.confirmed = false;
    request
}

#[tokio::test]
async fn wsol_unsigned_boundary_faults_are_typed_and_never_create_live_evidence_or_error_context() {
    for (mode, kind, stage) in [
        (1, WsolFailureKind::Rejected, Stage::UnsignedMetadata),
        (2, WsolFailureKind::Rejected, Stage::UnsignedWallet),
        (3, WsolFailureKind::Rejected, Stage::UnsignedWallet),
        (4, WsolFailureKind::Integrity, Stage::UnsignedWallet),
        (6, WsolFailureKind::Unavailable, Stage::UnsignedPrepare),
        (13, WsolFailureKind::Unavailable, Stage::UnsignedPrepare),
        (14, WsolFailureKind::Unavailable, Stage::UnsignedRpc),
        (15, WsolFailureKind::Rejected, Stage::UnsignedRpc),
        (16, WsolFailureKind::Integrity, Stage::UnsignedRpc),
        (21, WsolFailureKind::Unavailable, Stage::UnsignedMetadata),
        (22, WsolFailureKind::Integrity, Stage::UnsignedWallet),
    ] {
        let (service, shared) = harness().await;
        shared.set_runtime_mode(mode);
        let error = service.simulate(simulation_input()).await.err().unwrap();
        let failure = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!((failure.kind, failure.stage), (kind, stage), "mode {mode}");
        assert!(failure.submission_id.is_none() && failure.signature.is_none());
        assert!(!format!("{failure:?}").contains("private-api-key"));
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
        assert!(shared
            .events()
            .iter()
            .all(|event| !event.starts_with("sign:")
                && !event.starts_with("broadcast:")
                && event != "reserve"));
    }
}

#[tokio::test]
async fn wsol_unsigned_boundary_preserves_flag_key_and_owner_validation_before_runtime() {
    for field in 0..5 {
        let (service, shared) = harness().await;
        let mut request = simulation_input();
        match field {
            0 => request.simulate = false,
            1 => request.quote_id = "missing".into(),
            2 => request.idempotency_key.clear(),
            3 => request.owner_id = "foreign".into(),
            _ => request.wallet_id = "foreign".into(),
        }
        let error = service.simulate(request).await.err().unwrap();
        let failure = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!(failure.kind, WsolFailureKind::Rejected);
        assert_eq!(
            failure.stage,
            match field {
                0 => Stage::UnsignedAdmission,
                1 => Stage::UnsignedQuote,
                _ => Stage::UnsignedInput,
            }
        );
        assert!(shared.events().iter().all(|event| event == "quote"));
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
    }
}

#[tokio::test]
async fn wsol_unsigned_boundary_simulation_does_not_overwrite_an_existing_live_reservation() {
    let (service, shared) = harness().await;
    let quote = shared.store.get_quote("q1").await.unwrap().unwrap();
    let (original, _) = shared
        .store
        .reserve_live(&WsolSubmission::from(&input()), &quote, 1)
        .await
        .unwrap();
    for confirmed in [false, true] {
        let mut request = simulation_input();
        request.confirmed = confirmed;
        let result = service.simulate(request).await.unwrap();
        assert_eq!(result.quote_id, "q1");
        assert_eq!(result.network_fee_lamports, 10_000);
        assert_eq!(result.units_consumed, Some(999));
        assert_eq!(
            shared.store.get_by_key("key").await.unwrap(),
            Some(original.clone())
        );
    }
    assert_eq!(
        shared.events(),
        [
            "quote",
            "metadata",
            "resolve:Some(1)",
            "prepare:1",
            "unsigned:1",
            "quote",
            "metadata",
            "resolve:Some(2)",
            "prepare:2",
            "unsigned:2"
        ]
    );
}

#[tokio::test]
async fn wsol_unsigned_boundary_cancellation_and_file_reopen_never_reserve_sign_or_replay() {
    for at in [Gate::Prepared, Gate::UnsignedSimulation] {
        let dir =
            std::env::temp_dir().join(format!("wsol-unsigned-cancel-{}", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", dir.join("test.sqlite").display());
        let (service, shared) =
            harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
        let gate = shared.pause_stage(at);
        let task = tokio::spawn({
            let service = service.clone();
            async move { service.simulate(simulation_input()).await }
        });
        gate.wait_until_entered().await;
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
        assert!(shared
            .events()
            .iter()
            .all(|event| !event.starts_with("sign:")
                && !event.starts_with("broadcast:")
                && event != "reserve"));
        drop(service);
        drop(shared);
        let (service, shared) =
            harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
        assert!(shared.events().is_empty());
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
        drop(service);
        drop(shared);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
