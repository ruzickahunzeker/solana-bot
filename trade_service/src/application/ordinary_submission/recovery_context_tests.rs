use super::*;
use crate::application::ordinary_reconciliation::reconcile_captured;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[test]
fn ordinary_recovery_context_v1_amount_and_explicit_slippage_keep_frozen_rounding() {
    let mut request: crate::models::TradeRequest = serde_json::from_value(json!({
        "side":"buy","pool":"pump-amm","mint":crate::domain::trade::WSOL_MINT,
        "quote_mint":crate::domain::trade::USDC_MINT,"amount_sol":0.001,
        "simulate":false,"idempotency_key":" original literal key "
    }))
    .unwrap();
    for (sol, expected) in [
        (0.0000000005, 1),
        (0.00000000149, 1),
        (0.0000000015, 2),
        (0.0012345675, 1_234_568),
        (0.1, 100_000_000),
        (9_000_000_000.0, 9_000_000_000_000_000_000),
    ] {
        request.amount_sol = Some(sol);
        assert_eq!(
            crate::validation::replay_sol_observation_amount(&request).unwrap(),
            (expected, None)
        );
    }
    request.amount_sol = Some(0.001);
    for (percent, bps, expected) in [
        (None, None, None),
        (Some(0.0049), None, Some(0)),
        (Some(0.005), None, Some(1)),
        (Some(15.0), Some(9999), Some(1500)),
        (None, Some(1234), Some(1234)),
    ] {
        request.slippage_percent = percent;
        request.slippage_bps = bps;
        assert_eq!(
            crate::validation::replay_sol_observation_amount(&request).unwrap(),
            (1_000_000, expected)
        );
    }
    for bad in [0.0, 0.00000000049, -1.0, f64::INFINITY, f64::NAN, f64::MAX] {
        request.amount_sol = Some(bad);
        assert!(crate::validation::replay_sol_observation_amount(&request).is_err());
    }
    request.amount_sol = Some(0.001);
    request.simulate = Some(true);
    assert!(crate::validation::replay_sol_observation_amount(&request).is_err());
    request.simulate = None;
    assert!(crate::validation::replay_sol_observation_amount(&request).is_ok());
    request.quote_mint = Some(crate::domain::trade::WSOL_MINT.into());
    assert!(crate::validation::replay_sol_observation_amount(&request).is_err());
}

async fn edit_json(pool: &SqlitePool, column: &str, field: &str, value: Value) {
    let table = match column {
        "request_json" => "trades",
        "bound_quote_json" | "preflight_json" => "ordinary_execution_claims",
        _ => panic!("unexpected fixture column"),
    };
    let old: String = sqlx::query_scalar(&format!("SELECT {column} FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap();
    let mut parsed: Value = serde_json::from_str(&old).unwrap();
    parsed[field] = value;
    sqlx::query(&format!("UPDATE {table} SET {column}=?"))
        .bind(serde_json::to_string(&parsed).unwrap())
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn ordinary_recovery_context_reopens_without_admitted_trade_owner_or_wallet_runtime() {
    for stage in ["signed", "submitted", "unknown"] {
        let (f, owner, mut backend, journal, config, dir) = file_setup().await;
        if stage == "signed" {
            stage_signed(&f, &owner, &backend, &journal, SIGNATURE).await;
        } else {
            if stage == "unknown" {
                backend.failure = Some("broadcast");
            }
            execute(&journal, &backend, &backend, &f.execution, &owner).await;
        }
        let request_id = f.record.request_id.clone();
        let before: (String, String, Vec<u8>) = sqlx::query_as(
            "SELECT owner_nonce,signed_signature,signed_transaction FROM ordinary_execution_claims",
        )
        .fetch_one(&journal.pool)
        .await
        .unwrap();
        let events = backend.events.clone();
        journal.pool.close().await;
        drop(owner);
        drop(f);
        drop(backend);
        drop(journal);

        let restored = TradePersistence::new(&config).await.unwrap();
        sqlx::raw_sql("CREATE TRIGGER deny_recovery_owner_write BEFORE UPDATE OF owner_nonce ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'recovery cannot issue owner'); END; CREATE TRIGGER deny_recovery_artifact_write BEFORE UPDATE OF signed_signature,signed_transaction,unsigned_message ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'recovery cannot replace signed artifact'); END; CREATE TRIGGER deny_recovery_claim_insert BEFORE INSERT ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'recovery cannot create execution claim'); END;")
            .execute(&restored.pool_for_tests()).await.unwrap();
        let store = restored.ordinary_execution_claims().unwrap();
        let captured = store.capture_observation(&request_id).await.unwrap();
        assert_eq!(
            captured.record.status,
            if stage == "submitted" {
                TradeStatus::Submitted
            } else {
                TradeStatus::Unknown
            }
        );
        let missing = Reader::new(Answer::Missing);
        let unchanged = reconcile_captured(
            &store,
            &missing,
            &captured.context,
            Duration::from_millis(20),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(unchanged.record).unwrap(),
            serde_json::to_value(&captured.record).unwrap()
        );
        let reader = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
        let result = reconcile_captured(
            &store,
            &reader,
            &captured.context,
            Duration::from_millis(20),
        )
        .await
        .unwrap();
        assert_eq!(result.record.status, TradeStatus::Confirmed);
        assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
        let after: (String, String, Vec<u8>) = sqlx::query_as(
            "SELECT owner_nonce,signed_signature,signed_transaction FROM ordinary_execution_claims",
        )
        .fetch_one(&restored.pool_for_tests())
        .await
        .unwrap();
        assert_eq!(after, before);
        let saved_stage: String =
            sqlx::query_scalar("SELECT submission_stage FROM ordinary_execution_claims")
                .fetch_one(&restored.pool_for_tests())
                .await
                .unwrap();
        assert_eq!(saved_stage, stage);
        let claim_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&restored.pool_for_tests())
            .await
            .unwrap();
        assert_eq!(claim_rows, 1);
        let parent_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trades")
            .fetch_one(&restored.pool_for_tests())
            .await
            .unwrap();
        assert_eq!(parent_rows, 1);
        for event in ["sign", "broadcast"] {
            assert_eq!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|value| **value == event)
                    .count(),
                if stage == "signed" { 0 } else { 1 }
            );
        }
        restored.pool_for_tests().close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn ordinary_recovery_context_rejects_changed_original_evidence_before_lookup() {
    for changed in [
        "request_metadata",
        "owner",
        "bound",
        "report",
        "unsigned",
        "signed",
        "parent_time",
        "claim_time",
        "hash",
        "key",
    ] {
        let (f, owner, backend, journal) = setup().await;
        execute(&journal, &backend, &backend, &f.execution, &owner).await;
        let captured = journal
            .capture_observation(&f.record.request_id)
            .await
            .unwrap();
        match changed {
            "request_metadata" => {
                edit_json(
                    &journal.pool,
                    "request_json",
                    "source",
                    json!("changed-but-unhashed-source"),
                )
                .await
            }
            "owner" => {
                sqlx::query("UPDATE ordinary_execution_claims SET owner_nonce=?")
                    .bind("a".repeat(32))
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "bound" => {
                edit_json(
                    &journal.pool,
                    "bound_quote_json",
                    "expires_at_ms",
                    json!(123),
                )
                .await
            }
            "report" => {
                edit_json(
                    &journal.pool,
                    "preflight_json",
                    "reason",
                    json!("new audit metadata"),
                )
                .await
            }
            "unsigned" | "signed" => {
                let (bytes, hash) = if changed == "unsigned" {
                    ("unsigned_message", "unsigned_sha256")
                } else {
                    ("signed_transaction", "signed_sha256")
                };
                let replacement: Vec<u8> = vec![77, 88];
                sqlx::query(&format!(
                    "UPDATE ordinary_execution_claims SET {bytes}=?,{hash}=?"
                ))
                .bind(&replacement)
                .bind(format!("{:x}", Sha256::digest(&replacement)))
                .execute(&journal.pool)
                .await
                .unwrap();
            }
            "parent_time" => {
                sqlx::query("UPDATE trades SET created_at_ms=created_at_ms+1")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "claim_time" => {
                sqlx::query("UPDATE ordinary_execution_claims SET created_at_ms=created_at_ms+1")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "hash" => {
                for table in ["trades", "ordinary_execution_claims"] {
                    sqlx::query(&format!("UPDATE {table} SET request_hash=?"))
                        .bind("b".repeat(64))
                        .execute(&journal.pool)
                        .await
                        .unwrap();
                }
            }
            "key" => {
                edit_json(
                    &journal.pool,
                    "request_json",
                    "idempotency_key",
                    json!("different literal key"),
                )
                .await;
                sqlx::query("UPDATE trades SET idempotency_key='different literal key'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = snapshot(&f, &journal.pool).await;
        let reader = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
        assert!(
            reconcile_captured(
                &journal,
                &reader,
                &captured.context,
                Duration::from_millis(20)
            )
            .await
            .is_err(),
            "{changed}"
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0, "{changed}");
        assert!(
            journal
                .record_captured_finality(
                    &captured.context,
                    &observation(ChainCommitment::Finalized, None)
                )
                .await
                .is_err(),
            "{changed}"
        );
        assert_eq!(snapshot(&f, &journal.pool).await, before);
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 1);
    }
}

#[tokio::test]
async fn ordinary_recovery_context_rejects_incomplete_or_contradictory_ledger_rows() {
    for changed in [
        "unowned",
        "bad_owner",
        "wrong_operation",
        "wrong_version",
        "wrong_state",
        "partial_bytes",
        "partial_prepared",
        "bad_final",
        "amount",
        "simulate",
        "wallet",
        "pool_id",
        "method",
    ] {
        let (f, owner, backend, journal) = setup().await;
        execute(&journal, &backend, &backend, &f.execution, &owner).await;
        // Forge damaged-file samples that normal SQLite CHECK constraints already prevent.
        if matches!(changed, "wrong_operation" | "wrong_version") {
            sqlx::query("PRAGMA ignore_check_constraints=ON")
                .execute(&journal.pool)
                .await
                .unwrap();
        }
        match changed {
            "unowned" => {
                sqlx::query("UPDATE ordinary_execution_claims SET owner_nonce=NULL")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "bad_owner" => {
                sqlx::query(
                    "UPDATE ordinary_execution_claims SET owner_nonce='not-a-valid-original-owner'",
                )
                .execute(&journal.pool)
                .await
                .unwrap();
            }
            "wrong_operation" => {
                sqlx::query("UPDATE ordinary_execution_claims SET operation='other-operation'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "wrong_version" => {
                sqlx::query("UPDATE ordinary_execution_claims SET schema_version=2")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "wrong_state" => {
                sqlx::query("UPDATE trades SET status='rejected'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "partial_bytes" => {
                sqlx::query("UPDATE ordinary_execution_claims SET signed_sha256=NULL")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "partial_prepared" => {
                sqlx::query("UPDATE ordinary_execution_claims SET preflight_json=NULL")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "bad_final" => {
                sqlx::query("UPDATE ordinary_execution_claims SET final_observation_json='{}'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "amount" => edit_json(&journal.pool, "request_json", "amount_sol", json!(0.002)).await,
            "simulate" => edit_json(&journal.pool, "request_json", "simulate", json!(true)).await,
            "wallet" => {
                sqlx::query("UPDATE trades SET wallet_pubkey='invalid-0OIl'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "pool_id" => {
                edit_json(
                    &journal.pool,
                    "request_json",
                    "pool_id",
                    json!(crate::domain::trade::WSOL_MINT),
                )
                .await
            }
            "method" => {
                edit_json(
                    &journal.pool,
                    "request_json",
                    "execution_method",
                    json!("pumpapi"),
                )
                .await
            }
            _ => unreachable!(),
        }
        sqlx::query("PRAGMA ignore_check_constraints=OFF")
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(
            journal
                .capture_observation(&f.record.request_id)
                .await
                .is_err(),
            "{changed}"
        );
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 1);
    }
}

#[tokio::test]
async fn ordinary_recovery_context_allows_state_progress_but_never_rewrites_signed_artifacts() {
    let (f, owner, backend, journal) = setup().await;
    stage_signed(&f, &owner, &backend, &journal, SIGNATURE).await;
    let captured = journal
        .capture_observation(&f.record.request_id)
        .await
        .unwrap();
    assert_eq!(captured.record.status, TradeStatus::Submitting);
    journal
        .inner
        .record_broadcast(
            &f.execution,
            &owner,
            &SignedSubmission {
                signature: SIGNATURE.into(),
                bytes: [vec![99], MESSAGE.to_vec()].concat(),
            },
            true,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        journal
            .read_observation(&captured.context)
            .await
            .unwrap()
            .status,
        TradeStatus::Submitted
    );
    let later = journal
        .capture_observation(&f.record.request_id)
        .await
        .unwrap();
    captured.context.ensure_same(&later.context).unwrap();
    let result = reconcile_captured(
        &journal,
        &Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None))),
        &captured.context,
        Duration::from_millis(20),
    )
    .await
    .unwrap();
    assert_eq!(result.record.status, TradeStatus::Confirmed);
    assert_eq!(
        journal
            .read_observation(&captured.context)
            .await
            .unwrap()
            .signature
            .as_deref(),
        Some(SIGNATURE)
    );
    assert_eq!(count(&backend, "sign"), 0);
    assert_eq!(count(&backend, "broadcast"), 0);
}

#[tokio::test]
async fn ordinary_recovery_context_old_quote_expiry_is_not_new_admission_or_signing_permission() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    edit_json(&journal.pool, "bound_quote_json", "expires_at_ms", json!(0)).await;
    let captured = journal
        .capture_observation(&f.record.request_id)
        .await
        .unwrap();
    let reader = Reader::new(Answer::Found(observation(
        ChainCommitment::Finalized,
        Some("finalized transaction error"),
    )));
    let result = reconcile_captured(
        &journal,
        &reader,
        &captured.context,
        Duration::from_millis(20),
    )
    .await
    .unwrap();
    assert_eq!(result.record.status, TradeStatus::Failed);
    assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
    assert_eq!(count(&backend, "quote"), 1);
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
}

#[tokio::test]
async fn ordinary_recovery_context_is_redacted_and_rejects_cross_order_identity() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let captured = journal
        .capture_observation(&f.record.request_id)
        .await
        .unwrap();
    assert_eq!(
        format!("{:?}", captured.context),
        "OrdinaryObservationContext(<redacted>)"
    );
    let other = fixture(TradePersistenceMode::Durable).await;
    assert!(captured.context.ensure_execution(&other.execution).is_err());
    let record = captured.record.clone();
    for field in ["request_id", "wallet_id", "wallet_pubkey", "signature"] {
        let mut value = serde_json::to_value(&record).unwrap();
        value[field] = json!("different-public-identity");
        let changed = serde_json::from_value(value).unwrap();
        assert!(captured.context.ensure_record(&changed).is_err(), "{field}");
    }
}

struct FinalAfterGate {
    entered: Notify,
    release: Notify,
}
impl SignatureStatusReader for FinalAfterGate {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        assert_eq!(signature, SIGNATURE);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Some(observation(ChainCommitment::Finalized, None)))
    }
}

#[tokio::test]
async fn ordinary_recovery_context_refuses_evidence_changed_while_rpc_was_pending() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let captured = journal
        .capture_observation(&f.record.request_id)
        .await
        .unwrap();
    let journal = Arc::new(journal);
    let reader = Arc::new(FinalAfterGate {
        entered: Notify::new(),
        release: Notify::new(),
    });
    let task = {
        let (j, r) = (journal.clone(), reader.clone());
        tokio::spawn(async move {
            reconcile_captured(
                j.as_ref(),
                r.as_ref(),
                &captured.context,
                Duration::from_secs(2),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), reader.entered.notified())
        .await
        .unwrap();
    edit_json(
        &journal.pool,
        "request_json",
        "source",
        json!("changed during lookup"),
    )
    .await;
    let before = snapshot(&f, &journal.pool).await;
    reader.release.notify_one();
    assert!(task.await.unwrap().is_err());
    assert_eq!(snapshot(&f, &journal.pool).await, before);
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn ordinary_recovery_context_two_pools_update_without_any_execution_owner() {
    let (f, owner, backend, journal, config, dir) = file_setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let id = f.record.request_id.clone();
    let second = TradePersistence::new(&config).await.unwrap();
    let first = Arc::new(SqliteOrdinaryExecutionClaims::new(journal.pool.clone()));
    let other = Arc::new(second.ordinary_execution_claims().unwrap());
    let c1 = Arc::new(first.capture_observation(&id).await.unwrap().context);
    let c2 = Arc::new(other.capture_observation(&id).await.unwrap().context);
    c1.ensure_same(&c2).unwrap();
    drop(owner);
    drop(f);
    let mut tasks = vec![];
    for index in 0..16 {
        let (store, context) = if index % 2 == 0 {
            (first.clone(), c1.clone())
        } else {
            (other.clone(), c2.clone())
        };
        tasks.push(tokio::spawn(async move {
            store
                .record_captured_finality(&context, &observation(ChainCommitment::Finalized, None))
                .await
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().unwrap().status, TradeStatus::Confirmed);
    }
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
    journal.pool.close().await;
    second.pool_for_tests().close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
