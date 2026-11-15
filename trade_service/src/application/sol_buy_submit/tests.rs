use super::{test_support::*, *};
use crate::{
    application::ports::quote_cache::{QuoteBinding, QuoteCache},
    application::ports::submission_journal::SubmissionJournal,
    application::quote_submission_types::QuoteSubmissionOutcome,
    domain::{submission::SubmissionKey, trade::Address},
};
use std::sync::Arc;
use tokio::sync::Barrier;

#[tokio::test]
async fn submit_service_simulation_never_reserves_signs_or_broadcasts_and_live_key_remains_free() {
    let h = harness().await;
    let simulated = h
        .service
        .submit(&h.bound.quote_id, &h.command(true))
        .await
        .unwrap();
    assert_eq!(simulated.status, QuoteSubmissionOutcome::Simulated);
    assert!(simulated.signature.is_none());
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "cache", "prepare", "drop"]
    );
    assert!(h
        .journal()
        .get(&SubmissionKey::new(&h.bound.wallet_id, "fixture-key").unwrap())
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        serde_json::to_value(&h.trace.prepared.lock().unwrap()[0]).unwrap(),
        serde_json::to_value(&h.bound).unwrap()
    );
    h.trace.clear();
    let live = h
        .service
        .submit(&h.bound.quote_id, &h.command(false))
        .await
        .unwrap();
    assert_eq!(live.status, QuoteSubmissionOutcome::Submitted);
    assert_eq!(
        h.trace.events(),
        [
            "resolve",
            "journal",
            "pubkey",
            "get",
            "pubkey",
            "cache",
            "get",
            "live_allowed",
            "prepare",
            "reserve",
            "sign",
            "record_signed",
            "broadcast",
            "record_broadcast",
            "get",
            "drop"
        ]
    );
}

#[tokio::test]
async fn submit_service_replays_durable_result_before_expired_cache_and_keeps_simulation_separate()
{
    let h = harness().await;
    let request = h.command(false);
    let first = h.service.submit(&h.bound.quote_id, &request).await.unwrap();
    h.trace.faults.lock().unwrap().cache = true;
    h.trace.clear();
    let replay = h.service.submit(&h.bound.quote_id, &request).await.unwrap();
    assert_eq!(replay.signature, first.signature);
    assert_eq!(replay.status, first.status);
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
    h.trace.clear();
    let error = h
        .service
        .submit(&h.bound.quote_id, &h.command(true))
        .await
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Expired);
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "cache", "drop"]
    );
    h.trace.clear();
    let error = h
        .service
        .submit("different-quote", &request)
        .await
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Conflict);
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
}

#[tokio::test]
async fn submit_service_preserves_wallet_validation_durability_and_history_failure_priority() {
    let h = harness().await;
    let mut request = h.command(false);
    request.wallet_id = "missing".into();
    request.idempotency_key.clear();
    let error = h.service.submit("missing", &request).await.unwrap_err();
    assert_eq!(error.message, "wallet_id not found");
    assert_eq!(h.trace.events(), ["resolve"]);
    h.trace.clear();
    request.wallet_id = h.bound.wallet_id.clone();
    let error = h.service.submit("missing", &request).await.unwrap_err();
    assert_eq!(error.kind, FailureKind::InvalidRequest);
    assert!(error.message.starts_with("idempotency_key"));
    assert_eq!(h.trace.events(), ["resolve", "drop"]);
    h.trace.clear();
    request.idempotency_key = "fixture-key".into();
    request.confirmed = false;
    assert_eq!(
        h.service
            .submit("missing", &request)
            .await
            .unwrap_err()
            .message,
        "live SOL buy requires confirmed=true"
    );
    assert_eq!(h.trace.events(), ["resolve", "drop"]);
    h.trace.clear();
    request.confirmed = true;
    h.trace.faults.lock().unwrap().no_journal = true;
    let error = h.service.submit("missing", &request).await.unwrap_err();
    assert_eq!(error.kind, FailureKind::Unavailable);
    assert_eq!(h.trace.events(), ["resolve", "journal", "drop"]);
    h.trace.clear();
    h.trace.faults.lock().unwrap().no_journal = false;
    h.trace.faults.lock().unwrap().get_at = Some(1);
    assert_eq!(
        h.service
            .submit("missing", &request)
            .await
            .unwrap_err()
            .kind,
        FailureKind::Unavailable
    );
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "drop"]
    );
    h.trace.clear();
    h.trace.faults.lock().unwrap().get_at = Some(2);
    assert_eq!(
        h.service
            .submit(&h.bound.quote_id, &request)
            .await
            .unwrap_err()
            .kind,
        FailureKind::Unavailable
    );
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "get", "pubkey", "cache", "get", "drop"]
    );
}

#[tokio::test]
async fn submit_service_simulates_without_journal_and_retains_wallet_across_registry_replacement() {
    let h = harness().await;
    h.trace.faults.lock().unwrap().no_journal = true;
    h.trace.faults.lock().unwrap().replace_wallet = true;
    let result = h
        .service
        .submit(&h.bound.quote_id, &h.command(true))
        .await
        .unwrap();
    assert_eq!(result.status, QuoteSubmissionOutcome::Simulated);
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "pubkey", "cache", "prepare", "drop"]
    );
    assert_eq!(h.trace.reads.lock().unwrap()[0].2, h.bound.wallet_pubkey);
    assert_eq!(
        h.trace.prepared.lock().unwrap()[0].wallet_pubkey,
        h.bound.wallet_pubkey
    );
    assert_eq!(
        h.trace.wallets.lock().unwrap()[&h.bound.wallet_id],
        Address::from_bytes([9; 32])
    );
    h.trace.clear();
    assert_eq!(
        h.service
            .submit(&h.bound.quote_id, &h.command(true))
            .await
            .unwrap_err()
            .message,
        "quote wallet mismatch"
    );
}

#[tokio::test]
async fn submit_service_blocks_expiry_policy_and_prepare_failure_before_reserve_or_sign() {
    for mode in 0..3 {
        let h = harness().await;
        {
            let mut f = h.trace.faults.lock().unwrap();
            f.expire_after_cache = mode == 0;
            f.forbidden = mode == 1;
            f.prepare = mode == 2;
        }
        let error = h
            .service
            .submit(&h.bound.quote_id, &h.command(false))
            .await
            .unwrap_err();
        assert_eq!(
            error.kind,
            [
                FailureKind::Expired,
                FailureKind::Forbidden,
                FailureKind::Backend
            ][mode]
        );
        assert!(!error.message.contains("https://"));
        for forbidden in ["reserve", "sign", "record_signed", "broadcast"] {
            assert!(!h.trace.events().contains(&forbidden));
        }
    }
}

#[tokio::test]
async fn submit_service_write_failures_keep_core_no_broadcast_and_no_resign_guards() {
    for mode in 0..3 {
        let h = harness().await;
        {
            let mut f = h.trace.faults.lock().unwrap();
            f.reserve = mode == 0;
            f.record_signed = mode == 1;
            f.record_broadcast = mode == 2;
        }
        assert_eq!(
            h.service
                .submit(&h.bound.quote_id, &h.command(false))
                .await
                .unwrap_err()
                .kind,
            FailureKind::Unavailable
        );
        let events = h.trace.events();
        assert_eq!(events.contains(&"sign"), mode != 0);
        assert_eq!(events.contains(&"broadcast"), mode == 2);
        if mode > 0 {
            h.trace.clear();
            h.trace.faults.lock().unwrap().cache = true;
            let replay = h
                .service
                .submit(&h.bound.quote_id, &h.command(false))
                .await
                .unwrap();
            assert_eq!(
                replay.status,
                if mode == 1 {
                    QuoteSubmissionOutcome::Pending
                } else {
                    QuoteSubmissionOutcome::Unknown
                }
            );
            assert_eq!(
                h.trace.events(),
                ["resolve", "journal", "pubkey", "get", "drop"]
            );
        }
    }
}

#[tokio::test]
async fn submit_service_concurrent_same_key_reserves_one_signature_and_one_synthetic_broadcast() {
    let h = harness().await;
    *h.trace.barrier.lock().unwrap() = Some(Arc::new(Barrier::new(8)));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let service = h.service.clone();
        let id = h.bound.quote_id.clone();
        let command = h.command(false);
        tasks.push(tokio::spawn(
            async move { service.submit(&id, &command).await },
        ));
    }
    for task in tasks {
        let result = task.await.unwrap().unwrap();
        assert!(matches!(
            result.status,
            QuoteSubmissionOutcome::Submitted
                | QuoteSubmissionOutcome::Pending
                | QuoteSubmissionOutcome::Unknown
        ));
    }
    let events = h.trace.events();
    assert_eq!(events.iter().filter(|&&e| e == "prepare").count(), 8);
    assert_eq!(events.iter().filter(|&&e| e == "sign").count(), 1);
    assert_eq!(events.iter().filter(|&&e| e == "broadcast").count(), 1);
}

#[tokio::test]
async fn submit_service_preserves_literal_wallet_and_key_without_trimming_or_default_resolution() {
    let h = harness().await;
    let wallet = " fixture-wallet ";
    h.trace
        .wallets
        .lock()
        .unwrap()
        .insert(wallet.into(), Address::from_bytes([1; 32]));
    let bound = h
        .cache
        .bind(
            QuoteBinding {
                wallet_id: wallet,
                wallet_pubkey: h.bound.wallet_pubkey.clone(),
                slippage_bps: h.bound.quote_slippage_bps,
            },
            h.bound.quote.clone(),
        )
        .unwrap();
    let mut command = h.command(false);
    command.wallet_id = wallet.into();
    command.idempotency_key = " fixture-key ".into();
    let result = h.service.submit(&bound.quote_id, &command).await.unwrap();
    assert_eq!(result.wallet_id, wallet);
    assert_eq!(h.trace.reads.lock().unwrap()[0].1, wallet);
    let key = SubmissionKey::new(wallet, " fixture-key ").unwrap();
    let stored = h.journal().get(&key).await.unwrap().unwrap();
    assert_eq!(stored.intent.key, key);
    assert!(h
        .journal()
        .get(&SubmissionKey::new(wallet, "fixture-key").unwrap())
        .await
        .unwrap()
        .is_none());
    assert!(h
        .journal()
        .get(&SubmissionKey::new("fixture-wallet", " fixture-key ").unwrap())
        .await
        .unwrap()
        .is_none());
}
