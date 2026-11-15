use super::{test_support::*, *};
use crate::{
    application::{
        ports::{reconciliation::ReconciliationJournal, submission_journal::SubmissionJournal},
        submission_reconciler::LookupOutcome,
    },
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        submission::{SubmissionIntent, SubmissionStatus},
        trade::Address,
    },
};
use std::sync::Arc;
use tokio::sync::Barrier;

#[tokio::test]
async fn reconciliation_service_keeps_key_wallet_and_durable_error_priority() {
    let h = harness(true).await;
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
    ] {
        let error = h.service.reconcile(QUOTE, wallet, key).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::InvalidRequest);
        assert_eq!(error.message, message);
        assert!(h.trace.events().is_empty());
    }
    let error = h
        .service
        .reconcile(QUOTE, "missing", KEY)
        .await
        .err()
        .unwrap();
    assert_eq!(error.message, "wallet_id not found");
    assert_eq!(h.trace.events(), ["resolve"]);
    h.trace.clear();
    h.trace.faults.lock().unwrap().no_journal = true;
    let error = h.query().await.err().unwrap();
    assert_eq!(error.kind, FailureKind::Unavailable);
    assert_eq!(
        error.message,
        "quote submission status requires durable trade persistence"
    );
    assert_eq!(h.trace.events(), ["resolve", "journal", "drop"]);
}

#[tokio::test]
async fn reconciliation_service_snapshots_reader_then_pubkey_and_preserves_record_and_signed_bytes()
{
    let h = harness(true).await;
    let result = h.query().await.unwrap();
    assert_eq!(result.outcome, LookupOutcome::Updated);
    assert_eq!(result.record.submission_id, h.submission_id);
    assert_eq!(result.record.status, SubmissionStatus::Confirmed);
    assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
    assert_eq!(result.record.observed_slot, Some(102));
    assert_eq!(
        result.record.intent.quote["amount_lamports"],
        serde_json::json!(9_007_199_254_740_993_u64)
    );
    assert_eq!(
        h.trace.events(),
        ["resolve", "journal", "reader", "pubkey", "lookup", "rpc", "observe", "lookup", "drop"]
    );
    let bytes: Vec<u8> = sqlx::query_scalar("SELECT signed_transaction FROM quote_submissions")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
    assert_eq!(*h.trace.signatures.lock().unwrap(), [SIGNATURE]);
}

#[tokio::test]
async fn reconciliation_service_unsigned_and_finalized_orders_never_query_status_reader() {
    for signed in [false, true] {
        let h = harness(signed).await;
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
        let result = h.query().await.unwrap();
        assert_eq!(result.outcome, LookupOutcome::NotRequired);
        assert_eq!(
            h.trace.events(),
            ["resolve", "journal", "reader", "pubkey", "lookup", "drop"]
        );
        assert!(h.trace.signatures.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn reconciliation_service_missing_and_conflicting_records_do_not_query_chain() {
    for mode in 0..3 {
        let h = harness(true).await;
        if mode == 2 {
            h.trace
                .wallets
                .lock()
                .unwrap()
                .insert(WALLET.into(), Address::from_bytes([9; 32]));
        }
        let error = h
            .service
            .reconcile(
                if mode == 1 { "different-quote" } else { QUOTE },
                WALLET,
                if mode == 0 { "missing-key" } else { KEY },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.kind,
            if mode == 0 {
                FailureKind::NotFound
            } else {
                FailureKind::Conflict
            }
        );
        assert_eq!(
            h.trace.events(),
            ["resolve", "journal", "reader", "pubkey", "lookup", "drop"]
        );
    }
}

#[tokio::test]
async fn reconciliation_service_storage_failures_remain_unavailable_without_retries() {
    for mode in 0..3 {
        let h = harness(true).await;
        {
            let mut faults = h.trace.faults.lock().unwrap();
            if mode < 2 {
                faults.lookup_at = Some(mode + 1);
            } else {
                faults.observe = true;
            }
        }
        let error = h.query().await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Unavailable);
        assert!(!error.message.contains("https://"));
        assert_eq!(
            h.trace.signatures.lock().unwrap().len(),
            usize::from(mode != 0)
        );
        let record = h
            .journal()
            .get(&SubmissionKey::new(WALLET, KEY).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            if mode == 1 {
                SubmissionStatus::Confirmed
            } else {
                SubmissionStatus::Signed
            }
        );
        assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
    }
}

#[tokio::test]
async fn reconciliation_service_miss_failure_and_wrong_signature_preserve_original_record() {
    for (answer, outcome) in [
        (Answer::Missing, LookupOutcome::NotFound),
        (Answer::Error, LookupOutcome::Unavailable),
        (Answer::Mismatch, LookupOutcome::Unavailable),
    ] {
        let h = harness(true).await;
        *h.trace.answer.lock().unwrap() = answer;
        let result = h.query().await.unwrap();
        assert_eq!(result.outcome, outcome);
        assert_eq!(result.record.status, SubmissionStatus::Signed);
        assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
        assert!(!result.lookup_error.unwrap_or_default().contains("https://"));
        assert_eq!(
            h.trace.events(),
            ["resolve", "journal", "reader", "pubkey", "lookup", "rpc", "lookup", "drop"]
        );
    }
}

#[tokio::test]
async fn reconciliation_service_keeps_original_three_second_deadline_for_pending_reader() {
    assert_eq!(STATUS_LOOKUP_TIMEOUT, Duration::from_secs(3));
    let h = harness(true).await;
    *h.trace.answer.lock().unwrap() = Answer::Pending;
    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(6), h.query())
        .await
        .unwrap()
        .unwrap();
    assert!(started.elapsed() >= STATUS_LOOKUP_TIMEOUT);
    assert_eq!(result.outcome, LookupOutcome::Unavailable);
    assert_eq!(
        result.lookup_error.as_deref(),
        Some("signature status lookup timed out; original transaction was not retried")
    );
    assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
    assert_eq!(*h.trace.signatures.lock().unwrap(), [SIGNATURE]);
}

#[tokio::test]
async fn reconciliation_service_retains_original_identity_across_wallet_removal_and_replacement() {
    for remove in [false, true] {
        let h = harness(true).await;
        h.trace.faults.lock().unwrap().replace_wallet = !remove;
        h.trace.faults.lock().unwrap().remove_wallet = remove;
        let result = h.query().await.unwrap();
        assert_eq!(result.record.status, SubmissionStatus::Confirmed);
        assert_eq!(*h.trace.resolved_ids.lock().unwrap(), [WALLET]);
        assert_eq!(
            result.record.intent.wallet_pubkey,
            Address::from_bytes([1; 32]).to_string()
        );
        h.trace.clear();
        let error = h.query().await.err().unwrap();
        assert_eq!(
            error.kind,
            if remove {
                FailureKind::InvalidRequest
            } else {
                FailureKind::Conflict
            }
        );
        assert!(!h.trace.events().contains(&"rpc"));
    }
}

#[tokio::test]
async fn reconciliation_service_concurrent_observations_cas_once_without_mutating_original_signature(
) {
    let h = harness(true).await;
    *h.trace.answer.lock().unwrap() = Answer::Found(ChainCommitment::Finalized, None);
    *h.trace.barrier.lock().unwrap() = Some(Arc::new(Barrier::new(8)));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let service = h.service.clone();
        tasks.push(tokio::spawn(async move {
            service.reconcile(QUOTE, WALLET, KEY).await
        }));
    }
    let mut updated = 0;
    for task in tasks {
        let result = task.await.unwrap().unwrap();
        updated += usize::from(result.outcome == LookupOutcome::Updated);
        assert_eq!(result.record.status, SubmissionStatus::Finalized);
        assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
    }
    assert_eq!(updated, 1);
    assert_eq!(h.trace.signatures.lock().unwrap().len(), 8);
}

#[tokio::test]
async fn reconciliation_service_preserves_literal_ids_and_keys_in_read_only_queries() {
    let h = harness(false).await;
    let wallet = " fixture-wallet ";
    let key = " fixture-key ";
    let quote = " fixture-quote ";
    let address = Address::from_bytes([1; 32]);
    h.trace
        .wallets
        .lock()
        .unwrap()
        .insert(wallet.into(), address);
    let intent = SubmissionIntent {
        key: SubmissionKey::new(wallet, key).unwrap(),
        wallet_pubkey: address.to_string(),
        quote_id: quote.into(),
        quote: serde_json::json!({}),
    };
    h.journal()
        .reserve(&intent, &serde_json::json!({}))
        .await
        .unwrap();
    let result = h.service.reconcile(quote, wallet, key).await.unwrap();
    assert_eq!(result.record.intent.key, intent.key);
    assert_eq!(result.record.intent.quote_id, quote);
    assert_eq!(*h.trace.resolved_ids.lock().unwrap(), [wallet]);
    assert_eq!(*h.trace.lookup_keys.lock().unwrap(), [intent.key]);
    assert!(h.trace.signatures.lock().unwrap().is_empty());
}
