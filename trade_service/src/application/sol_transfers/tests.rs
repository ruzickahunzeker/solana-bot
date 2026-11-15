use super::{test_support::*, *};
use crate::{domain::trade::Address, wallet_repository::SolTransferRecord};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn transfer_service_persists_before_send_and_returns_original_confirmed_record() {
    let h = harness().await;
    let out = h.submit().await.unwrap();
    assert_eq!(out.record.status, "confirmed");
    assert!(!out.duplicate);
    assert_eq!(out.record.signature.as_deref(), Some(SIG));
    assert_eq!(out.record.actual_amount_lamports, Some(1_000_000));
    assert_eq!(
        h.trace.events(),
        [
            "metadata",
            "journal",
            "reserve",
            "resolve",
            "pubkey",
            "prepare",
            "sign",
            "record_signed",
            "broadcast",
            "record_broadcast",
            "confirm",
            "record_confirmed",
            "lookup",
            "drop"
        ]
    );
    h.trace.clear();
    let replay = h.submit().await.unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.record.transfer_id, out.record.transfer_id);
    assert_eq!(h.trace.events(), ["metadata", "journal", "reserve"]);
}

#[tokio::test]
async fn transfer_service_prebroadcast_failures_are_terminal_and_never_reenter_execution() {
    for mode in 0..4 {
        let h = harness().await;
        {
            let mut f = h.trace.faults.lock().unwrap();
            f.prepare = mode == 0;
            f.sign = mode == 1;
            f.bad_snapshot = mode == 2;
            f.changed_signed_snapshot = mode == 3;
        }
        let out = h.submit().await.unwrap();
        assert_eq!(out.record.status, "failed");
        assert!(out.record.signature.is_none());
        assert!(!out.record.error.unwrap().contains("https://"));
        assert_eq!(h.trace.count("broadcast"), 0);
        h.trace.clear();
        assert!(h.submit().await.unwrap().duplicate);
        assert_eq!(h.trace.count("prepare"), 0);
    }
}

#[tokio::test]
async fn transfer_service_write_failures_never_broadcast_without_signed_bytes_or_restart_work() {
    for mode in 0..3 {
        let h = harness().await;
        let condition = [
            "NEW.signature IS NOT NULL AND NEW.status='unknown'",
            "NEW.status='submitted'",
            "NEW.status='confirmed'",
        ][mode];
        sqlx::raw_sql(&format!("CREATE TRIGGER reject_stage BEFORE UPDATE ON wallet_sol_transfers WHEN {condition} BEGIN SELECT RAISE(FAIL,'synthetic write failure'); END;")).execute(&h.pool).await.unwrap();
        let error = h.submit().await.unwrap_err();
        assert_eq!(error.kind, TransferFailureKind::Unavailable);
        assert!(error.message.contains("do not resubmit"));
        assert_eq!(h.trace.count("sign"), 1);
        assert_eq!(h.trace.count("broadcast"), usize::from(mode > 0));
        let entry = h.journal().lookup(KEY).await.unwrap().unwrap();
        assert_eq!(
            entry.state,
            Some([State::Reserved, State::Signed, State::Submitted][mode])
        );
        assert_eq!(entry.record.signature.is_some(), mode > 0);
        h.trace.clear();
        let replay = h.submit().await.unwrap();
        assert!(replay.duplicate);
        assert_eq!(
            replay.record.status,
            ["processing", "unknown", "submitted"][mode]
        );
        assert_eq!(h.trace.count("sign"), 0);
        assert_eq!(h.trace.count("broadcast"), 0);
    }
}

#[tokio::test]
async fn transfer_service_send_failure_and_wrong_receipt_preserve_original_signature_as_unknown() {
    for wrong in [false, true] {
        let h = harness().await;
        h.trace.faults.lock().unwrap().broadcast = !wrong;
        h.trace.faults.lock().unwrap().different_receipt = wrong;
        let out = h.submit().await.unwrap();
        assert_eq!(out.record.status, "unknown");
        assert_eq!(out.record.signature.as_deref(), Some(SIG));
        assert!(!out.record.error.unwrap().contains("https://"));
        assert_eq!(h.trace.count("confirm"), 0);
        h.trace.clear();
        assert!(h.submit().await.unwrap().duplicate);
        assert_eq!(h.trace.count("broadcast"), 0);
    }
}

#[tokio::test]
async fn transfer_service_policy_key_storage_runtime_and_wallet_identity_checks_are_fail_closed() {
    let h = harness().await;
    let mut policy = h.policy();
    policy.broadcast = BroadcastPolicy::Disabled;
    assert_eq!(
        h.service
            .submit("source", h.input(), policy)
            .await
            .unwrap_err()
            .kind,
        TransferFailureKind::Forbidden
    );
    assert!(h.trace.events().is_empty());
    let mut input = h.input();
    input.idempotency_key = "";
    assert_eq!(
        h.service
            .submit("missing", input, h.policy())
            .await
            .unwrap_err()
            .kind,
        TransferFailureKind::BadRequest
    );
    assert!(h.trace.events().is_empty());
    for mode in 0..3 {
        let h = harness().await;
        match mode {
            0 => h.trace.faults.lock().unwrap().no_journal = true,
            1 => h.trace.wallets.lock().unwrap().clear(),
            2 => {
                h.trace
                    .wallets
                    .lock()
                    .unwrap()
                    .insert("source".into(), Address::from_bytes([9; 32]));
            }
            _ => unreachable!(),
        }
        assert_eq!(
            h.submit().await.unwrap_err().kind,
            TransferFailureKind::Unavailable
        );
        assert_eq!(h.trace.count("prepare"), 0);
        assert_eq!(h.trace.count("sign"), 0);
        if mode > 0 {
            assert_eq!(
                h.journal().lookup(KEY).await.unwrap().unwrap().state,
                Some(State::Failed)
            );
        }
    }
}

#[tokio::test]
async fn transfer_service_duplicate_rejects_changed_target_amount_wallet_and_owner() {
    let h = harness().await;
    h.submit().await.unwrap();
    h.trace.clear();
    let other = Address::from_bytes([9; 32]).to_string();
    for mode in 0..4 {
        let mut input = h.input();
        let mut wallet = "source";
        match mode {
            0 => input.to_address = Some(&other),
            1 => input.amount.fixed_lamports = Some(2),
            2 => wallet = "target",
            3 => {
                wallet = "foreign";
                input.owner_id = "7";
            }
            _ => unreachable!(),
        }
        let error = h
            .service
            .submit(wallet, input, h.policy())
            .await
            .unwrap_err();
        assert_eq!(error.kind, TransferFailureKind::Conflict);
        assert_eq!(h.trace.count("resolve"), 0);
    }
}

#[tokio::test]
async fn transfer_service_only_replays_legacy_terminal_records_with_known_signature() {
    for known in [false, true] {
        let h = harness().await;
        let row = SolTransferRecord {
            transfer_id: "legacy-transfer".into(),
            idempotency_key: KEY.into(),
            owner_type: "telegram".into(),
            owner_id: "42".into(),
            wallet_id: "source".into(),
            from_pubkey: Address::from_bytes([1; 32]).to_string(),
            to_pubkey: Address::from_bytes([2; 32]).to_string(),
            amount_mode: "fixed".into(),
            requested_amount_lamports: Some(1_000_000),
            requested_percent: None,
            actual_amount_lamports: None,
            fee_lamports: None,
            balance_before_lamports: None,
            status: "processing".into(),
            signature: None,
            error: None,
        };
        h.repo.reserve_sol_transfer(&row).await.unwrap();
        if known {
            h.repo
                .finish_sol_transfer(
                    &row.transfer_id,
                    "confirmed",
                    Some(1_000_000),
                    Some(5000),
                    Some(2_000_000),
                    Some("legacy-signature"),
                    None,
                )
                .await
                .unwrap();
        }
        if known {
            let out = h.submit().await.unwrap();
            assert!(out.duplicate);
            assert_eq!(out.record.signature.as_deref(), Some("legacy-signature"));
        } else {
            assert!(h
                .submit()
                .await
                .unwrap_err()
                .message
                .contains("legacy SOL transfer outcome cannot be verified"));
        }
        assert_eq!(h.trace.count("resolve"), 0);
        assert_eq!(h.trace.count("sign"), 0);
    }
}

#[tokio::test]
async fn transfer_service_retains_original_wallet_across_registry_replacement() {
    let h = harness().await;
    h.trace.faults.lock().unwrap().replace_wallet = true;
    let out = h.submit().await.unwrap();
    assert_eq!(out.record.status, "confirmed");
    assert_eq!(
        out.record.from_pubkey,
        Address::from_bytes([1; 32]).to_string()
    );
    assert_eq!(h.trace.count("resolve"), 1);
    h.trace.clear();
    assert!(h.submit().await.unwrap().duplicate);
    assert_eq!(h.trace.count("resolve"), 0);
}

#[tokio::test]
async fn transfer_service_cancellation_at_each_durable_or_execution_boundary_never_retries() {
    for stage in [
        Stage::Reserved,
        Stage::Prepare,
        Stage::RecordedSigned,
        Stage::Broadcast,
        Stage::Confirm,
    ] {
        let h = Arc::new(harness().await);
        let (entered, _release) = h.trace.gate(stage);
        let worker = h.clone();
        let task = tokio::spawn(async move { worker.submit().await });
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let before = h.journal().lookup(KEY).await.unwrap().unwrap();
        let signature_saved = matches!(
            stage,
            Stage::RecordedSigned | Stage::Broadcast | Stage::Confirm
        );
        assert_eq!(before.record.signature.is_some(), signature_saved);
        h.journal().initialize().await.unwrap();
        h.trace.clear();
        let out = h.submit().await.unwrap();
        assert!(out.duplicate);
        assert_eq!(out.record.signature.is_some(), signature_saved);
        assert_eq!(h.trace.count("sign"), 0);
        assert_eq!(h.trace.count("broadcast"), 0);
        assert_eq!(
            out.record.status,
            if stage == Stage::Confirm {
                "submitted"
            } else {
                "unknown"
            }
        );
    }
}

#[tokio::test]
async fn transfer_service_concurrent_same_key_has_one_signer_and_one_synthetic_send() {
    let h = Arc::new(harness().await);
    let (entered, release) = h.trace.gate(Stage::Prepare);
    let first = h.clone();
    let task = tokio::spawn(async move { first.submit().await });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let mut workers = Vec::new();
    for _ in 0..16 {
        let h = h.clone();
        workers.push(tokio::spawn(async move { h.submit().await }));
    }
    for worker in workers {
        let out = worker.await.unwrap().unwrap();
        assert!(out.duplicate);
        assert_eq!(out.record.status, "processing");
    }
    release.notify_one();
    assert_eq!(task.await.unwrap().unwrap().record.status, "confirmed");
    assert_eq!(h.trace.count("prepare"), 1);
    assert_eq!(h.trace.count("sign"), 1);
    assert_eq!(h.trace.count("broadcast"), 1);
}

#[tokio::test]
async fn transfer_service_final_read_failure_keeps_confirmed_signature_and_replays_without_execution(
) {
    let h = harness().await;
    h.trace.faults.lock().unwrap().lookup = true;
    let error = h.submit().await.unwrap_err();
    assert_eq!(error.kind, TransferFailureKind::Unavailable);
    assert!(error.message.contains("do not resubmit"));
    let saved = h.journal().lookup(KEY).await.unwrap().unwrap();
    assert_eq!(saved.state, Some(State::Confirmed));
    assert_eq!(saved.record.signature.as_deref(), Some(SIG));
    h.trace.clear();
    let replay = h.submit().await.unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.record.signature.as_deref(), Some(SIG));
    assert_eq!(h.trace.count("sign"), 0);
    assert_eq!(h.trace.count("broadcast"), 0);
}
