use super::*;
#[path = "recovery_context_tests.rs"]
mod recovery_context_tests;
#[path = "scan_recovery_tests.rs"]
mod scan_recovery_tests;
#[path = "sdk_mixed_scan_tests.rs"]
mod sdk_mixed_scan_tests;
use crate::{
    application::{
        ordinary_claims::test_support::admit,
        ordinary_reconciliation::{reconcile, OrdinaryReconciliationResult},
    },
    config::TradePersistenceConfig,
    domain::reconciliation::{ChainCommitment, ChainObservation},
    persistence::TradePersistence,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

fn observation(commitment: ChainCommitment, error: Option<&str>) -> ChainObservation {
    ChainObservation {
        signature: SIGNATURE.into(),
        commitment,
        slot: 123,
        error: error.map(str::to_owned),
    }
}

enum Answer {
    Found(ChainObservation),
    Missing,
    Error,
    Pending,
    GatedMissing(Arc<Notify>),
}

struct Reader {
    answer: Answer,
    calls: AtomicUsize,
    entered: Notify,
    dropped: Arc<AtomicBool>,
}

impl Reader {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            dropped: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl SignatureStatusReader for Reader {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        assert_eq!(signature, SIGNATURE);
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _guard = DropFlag(self.dropped.clone());
        self.entered.notify_one();
        match &self.answer {
            Answer::Found(value) => Ok(Some(value.clone())),
            Answer::Missing => Ok(None),
            Answer::Error => anyhow::bail!(
                "lookup failed: https://rpc.example.invalid/private-observation-token"
            ),
            Answer::Pending => std::future::pending().await,
            Answer::GatedMissing(release) => {
                release.notified().await;
                Ok(None)
            }
        }
    }
}

async fn query<J: OrdinaryReconciliationJournal + RecoveredOrdinaryObservations>(
    journal: &J,
    reader: &Reader,
    f: &Fixture,
    owner: &ExecutionOwner,
) -> Result<OrdinaryReconciliationResult> {
    reconcile(
        journal,
        reader,
        &f.execution,
        owner,
        SIGNATURE,
        Duration::from_millis(20),
    )
    .await
}

async fn snapshot(f: &Fixture, pool: &SqlitePool) -> serde_json::Value {
    let parent = f.state.persistence.get(&f.record.request_id).await.unwrap();
    let evidence: (Option<String>, Option<String>, Option<Vec<u8>>, String, Option<String>) =
        sqlx::query_as("SELECT final_observation_json,signed_signature,signed_transaction,submission_stage,submission_error FROM ordinary_execution_claims")
            .fetch_one(pool).await.unwrap();
    serde_json::json!({"parent":parent,"evidence":evidence})
}

async fn stage_signed(
    f: &Fixture,
    owner: &ExecutionOwner,
    backend: &Backend,
    journal: &Journal,
    signature: &str,
) {
    journal.inner.begin(&f.execution, owner).await.unwrap();
    journal
        .inner
        .prepare(
            &f.execution,
            owner,
            &backend.bound,
            &backend.report,
            MESSAGE,
        )
        .await
        .unwrap();
    journal
        .inner
        .authorize_sign(
            &f.execution,
            owner,
            &backend.bound,
            &backend.report,
            MESSAGE,
        )
        .await
        .unwrap();
    journal
        .inner
        .record_signed(
            &f.execution,
            owner,
            &SignedSubmission {
                signature: signature.into(),
                bytes: [vec![99], MESSAGE.to_vec()].concat(),
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn ordinary_observation_pre_broadcast_stages_do_not_authorize_a_lookup() {
    for signed in [false, true] {
        let (f, owner, backend, journal) = setup().await;
        if signed {
            stage_signed(&f, &owner, &backend, &journal, SIGNATURE).await;
        }
        let before = snapshot(&f, &journal.pool).await;
        let reader = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
        assert!(query(&journal, &reader, &f, &owner).await.is_err());
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot(&f, &journal.pool).await, before);
        assert_eq!(count(&backend, "sign"), 0);
        assert_eq!(count(&backend, "broadcast"), 0);
    }
}

#[tokio::test]
async fn ordinary_observation_real_rpc_parser_requires_explicit_finality_and_consistent_errors() {
    use crate::adapters::rpc::signature_status::RpcSignatureStatusReader;
    use serde_json::json;
    use solana_client::{nonblocking::rpc_client::RpcClient, rpc_request::RpcRequest};
    use solana_sdk::signature::Signature;
    use std::collections::HashMap;

    for (case, expected) in [
        ("processed", TradeStatus::Submitted),
        ("confirmed", TradeStatus::Submitted),
        ("finalized", TradeStatus::Confirmed),
        ("confirmed_error", TradeStatus::Submitted),
        ("finalized_error", TradeStatus::Failed),
        ("missing_level", TradeStatus::Submitted),
        ("inconsistent_error", TradeStatus::Submitted),
        ("empty_result", TradeStatus::Submitted),
        ("zero_slot", TradeStatus::Submitted),
    ] {
        let (f, owner, backend, journal) = setup().await;
        // A public, non-cryptographic signature/byte fixture; no signer is invoked.
        let signature = Signature::from([1; 64]).to_string();
        stage_signed(&f, &owner, &backend, &journal, &signature).await;
        let persisted = SignedSubmission {
            signature: signature.clone(),
            bytes: [vec![99], MESSAGE.to_vec()].concat(),
        };
        journal
            .inner
            .record_broadcast(&f.execution, &owner, &persisted, true, None)
            .await
            .unwrap();
        let mut status = json!({"slot":123,"confirmations":1,"status":{"Ok":null},"err":null,"confirmationStatus":"finalized"});
        match case {
            "processed" | "confirmed" => status["confirmationStatus"] = json!(case),
            "confirmed_error" | "finalized_error" | "inconsistent_error" => {
                status["err"] = json!({"InstructionError":[0,{"Custom":42}]});
                if case != "inconsistent_error" {
                    status["status"] = json!({"Err":status["err"]});
                }
                if case == "confirmed_error" {
                    status["confirmationStatus"] = json!("confirmed");
                }
            }
            "missing_level" => {
                status.as_object_mut().unwrap().remove("confirmationStatus");
            }
            "zero_slot" => status["slot"] = json!(0),
            _ => {}
        }
        let values = if case == "empty_result" {
            json!([])
        } else {
            json!([status])
        };
        let mocks = HashMap::from([(
            RpcRequest::GetSignatureStatuses,
            json!({"context":{"slot":123},"value":values}),
        )]);
        let reader = RpcSignatureStatusReader {
            rpcs: vec![Arc::new(RpcClient::new_mock_with_mocks(
                "succeeds".into(),
                mocks,
            ))],
        };
        let result = reconcile(
            &journal,
            &reader,
            &f.execution,
            &owner,
            &signature,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(result.record.status, expected, "{case}");
        assert_eq!(result.record.signature.as_deref(), Some(signature.as_str()));
        if matches!(
            case,
            "missing_level" | "inconsistent_error" | "empty_result" | "zero_slot"
        ) {
            assert!(result.lookup_error.is_some(), "{case}");
        }
        assert_eq!(count(&backend, "sign"), 0);
        assert_eq!(count(&backend, "broadcast"), 0);
    }
}

#[tokio::test]
async fn ordinary_observation_controller_finalizes_original_signature_after_all_send_receipts() {
    for failure in [None, Some("broadcast"), Some("receipt")] {
        let (f, owner, mut backend, journal) = setup().await;
        backend.failure = failure;
        backend.observation = Some(observation(ChainCommitment::Finalized, None));
        let result = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        let record = result.record.unwrap();
        assert_eq!(record.status, TradeStatus::Confirmed);
        assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
        assert!(record.error.is_none());
        let evidence: (String, Vec<u8>, String) = sqlx::query_as("SELECT final_observation_json,signed_transaction,submission_stage FROM ordinary_execution_claims")
            .fetch_one(&journal.pool).await.unwrap();
        let saved: serde_json::Value = serde_json::from_str(&evidence.0).unwrap();
        assert_eq!(saved["signature"], SIGNATURE);
        assert_eq!(saved["commitment"], "finalized");
        assert_eq!(saved["slot"], 123);
        assert_eq!(evidence.1, [vec![99], MESSAGE.to_vec()].concat());
        assert_eq!(
            evidence.2,
            if failure.is_none() {
                "submitted"
            } else {
                "unknown"
            }
        );
        let repeated = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        assert_eq!(repeated.record.unwrap().status, TradeStatus::Confirmed);
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 1);
        assert_eq!(count(&backend, "lookup_signature"), 1);
    }
}

#[tokio::test]
async fn ordinary_observation_finalized_chain_failure_preserves_signature_and_redacts_error() {
    let (f, owner, mut backend, journal) = setup().await;
    backend.observation = Some(observation(
        ChainCommitment::Finalized,
        Some("execution failed: https://rpc.example.invalid/private-final-token"),
    ));
    let result = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
    let record = result.record.unwrap();
    assert_eq!(record.status, TradeStatus::Failed);
    assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
    assert_eq!(
        record.error.as_deref(),
        Some("execution failed: <redacted-url>")
    );
    let stored = snapshot(&f, &journal.pool).await;
    assert!(!stored.to_string().contains("private-final-token"));
    let reader = Reader::new(Answer::Pending);
    assert_eq!(
        query(&journal, &reader, &f, &owner)
            .await
            .unwrap()
            .record
            .status,
        TradeStatus::Failed
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn ordinary_observation_weak_or_missing_evidence_never_writes_or_changes_parent() {
    for failure in [None, Some("broadcast")] {
        let (f, owner, mut backend, journal) = setup().await;
        backend.failure = failure;
        execute(&journal, &backend, &backend, &f.execution, &owner).await;
        let before = snapshot(&f, &journal.pool).await;
        for answer in [
            Answer::Missing,
            Answer::Found(observation(ChainCommitment::Processed, None)),
            Answer::Found(observation(
                ChainCommitment::Processed,
                Some("temporary failure"),
            )),
            Answer::Found(observation(ChainCommitment::Confirmed, None)),
            Answer::Found(observation(
                ChainCommitment::Confirmed,
                Some("not finalized failure"),
            )),
        ] {
            let reader = Reader::new(answer);
            query(&journal, &reader, &f, &owner).await.unwrap();
            assert_eq!(snapshot(&f, &journal.pool).await, before);
            assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
        }
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 1);
    }
}

#[tokio::test]
async fn ordinary_observation_wrong_signature_zero_slot_and_rpc_errors_preserve_original() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let before = snapshot(&f, &journal.pool).await;
    let mut wrong = observation(ChainCommitment::Finalized, None);
    wrong.signature = "another-signature".into();
    let mut zero = observation(ChainCommitment::Finalized, None);
    zero.slot = 0;
    for answer in [Answer::Found(wrong), Answer::Found(zero), Answer::Error] {
        let result = query(&journal, &Reader::new(answer), &f, &owner)
            .await
            .unwrap();
        assert!(result.lookup_error.is_some());
        assert!(!result
            .lookup_error
            .unwrap()
            .contains("private-observation-token"));
        assert_eq!(snapshot(&f, &journal.pool).await, before);
    }
}

#[tokio::test]
async fn ordinary_observation_deadline_drops_lookup_without_losing_signed_evidence() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let before = snapshot(&f, &journal.pool).await;
    let reader = Reader::new(Answer::Pending);
    let result = query(&journal, &reader, &f, &owner).await.unwrap();
    assert!(result.lookup_error.unwrap().contains("timed out"));
    assert!(reader.dropped.load(Ordering::SeqCst));
    assert_eq!(snapshot(&f, &journal.pool).await, before);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn ordinary_observation_cancellation_drops_reader_without_new_execution_permission() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let before = snapshot(&f, &journal.pool).await;
    let journal = Arc::new(journal);
    let owner = Arc::new(owner);
    let reader = Arc::new(Reader::new(Answer::Pending));
    let task = {
        let (j, o, r, ex) = (
            journal.clone(),
            owner.clone(),
            reader.clone(),
            f.execution.clone(),
        );
        tokio::spawn(async move {
            reconcile(
                j.as_ref(),
                r.as_ref(),
                &ex,
                &o,
                SIGNATURE,
                Duration::from_secs(30),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), reader.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(reader.dropped.load(Ordering::SeqCst));
    assert_eq!(snapshot(&f, &journal.pool).await, before);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn ordinary_observation_terminal_evidence_is_immutable_and_weak_results_cannot_overwrite_it()
{
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let finality = observation(ChainCommitment::Finalized, None);
    journal
        .inner
        .record_final_observation(&f.execution, &owner, SIGNATURE, &finality)
        .await
        .unwrap();
    let before = snapshot(&f, &journal.pool).await;
    sqlx::raw_sql("CREATE TABLE terminal_writes(n INTEGER); CREATE TRIGGER audit_terminal_writes AFTER UPDATE ON trades WHEN OLD.status IN ('confirmed','failed') BEGIN INSERT INTO terminal_writes VALUES(1); END;")
        .execute(&journal.pool).await.unwrap();
    journal
        .inner
        .record_final_observation(&f.execution, &owner, SIGNATURE, &finality)
        .await
        .unwrap();
    for changed in [
        observation(ChainCommitment::Confirmed, None),
        observation(ChainCommitment::Finalized, Some("conflicting final error")),
        ChainObservation {
            slot: 124,
            ..finality.clone()
        },
    ] {
        assert!(journal
            .inner
            .record_final_observation(&f.execution, &owner, SIGNATURE, &changed)
            .await
            .is_err());
    }
    let reader = Reader::new(Answer::Pending);
    query(&journal, &reader, &f, &owner).await.unwrap();
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    assert_eq!(snapshot(&f, &journal.pool).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM terminal_writes")
            .fetch_one(&journal.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn ordinary_observation_bad_owner_parent_or_signed_evidence_blocks_lookup() {
    for corruption in [
        "owner",
        "signature",
        "wallet",
        "request_hash",
        "signed_bytes",
        "final_json",
    ] {
        let (f, owner, backend, journal) = setup().await;
        execute(&journal, &backend, &backend, &f.execution, &owner).await;
        let alternate_owner = ExecutionOwner::fixture(&f.execution);
        match corruption {
            "wallet" => {
                sqlx::query("UPDATE trades SET wallet_pubkey='different-wallet'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "request_hash" => {
                sqlx::query("UPDATE trades SET request_hash='different-hash'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "signed_bytes" => {
                sqlx::query("UPDATE ordinary_execution_claims SET signed_transaction=x'01'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            "final_json" => {
                sqlx::query("UPDATE ordinary_execution_claims SET final_observation_json='{}'")
                    .execute(&journal.pool)
                    .await
                    .unwrap();
            }
            _ => {}
        }
        let reader = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
        let result = reconcile(
            &journal,
            &reader,
            &f.execution,
            if corruption == "owner" {
                &alternate_owner
            } else {
                &owner
            },
            if corruption == "signature" {
                "different-signature"
            } else {
                SIGNATURE
            },
            Duration::from_millis(20),
        )
        .await;
        assert!(result.is_err(), "{corruption}");
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0, "{corruption}");
    }
}

#[tokio::test]
async fn ordinary_observation_write_and_readback_faults_roll_back_without_false_finality() {
    for trigger in [
        "CREATE TRIGGER deny_final BEFORE UPDATE OF final_observation_json ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'synthetic observation write failure'); END;",
        "CREATE TRIGGER deny_parent BEFORE UPDATE ON trades WHEN NEW.status='confirmed' BEGIN SELECT RAISE(ABORT,'synthetic final parent write failure'); END;",
        "CREATE TRIGGER corrupt_final AFTER UPDATE OF final_observation_json ON ordinary_execution_claims BEGIN UPDATE ordinary_execution_claims SET final_observation_json='{}'; END;",
        "CREATE TRIGGER corrupt_parent AFTER UPDATE ON trades WHEN NEW.status='confirmed' BEGIN UPDATE trades SET error='corrupt-final-error' WHERE request_id=NEW.request_id; END;",
    ] {
        let (f, owner, backend, journal) = setup().await;
        execute(&journal, &backend, &backend, &f.execution, &owner).await;
        let before = snapshot(&f, &journal.pool).await;
        sqlx::raw_sql(trigger).execute(&journal.pool).await.unwrap();
        let reader = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
        assert!(query(&journal, &reader, &f, &owner).await.is_err());
        assert_eq!(snapshot(&f, &journal.pool).await, before);
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 1);
    }
}

#[tokio::test]
async fn ordinary_observation_lost_commit_acknowledgement_preserves_final_parent_and_original_key()
{
    let (f, owner, backend, mut journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    journal.fault = Some("final_ack");
    let reader = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
    assert!(query(&journal, &reader, &f, &owner).await.is_err());
    let pending = Reader::new(Answer::Pending);
    assert_eq!(
        query(&journal, &pending, &f, &owner)
            .await
            .unwrap()
            .record
            .status,
        TradeStatus::Confirmed
    );
    assert_eq!(pending.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn ordinary_observation_cancellation_after_final_commit_keeps_final_evidence() {
    let (f, owner, backend, mut journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let committed = Arc::new(Notify::new());
    journal.after_final = Some(committed.clone());
    let journal = Arc::new(journal);
    let owner = Arc::new(owner);
    let task = {
        let (j, o, ex) = (journal.clone(), owner.clone(), f.execution.clone());
        tokio::spawn(async move {
            let reader = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
            reconcile(
                j.as_ref(),
                &reader,
                &ex,
                &o,
                SIGNATURE,
                Duration::from_secs(2),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), committed.notified())
        .await
        .unwrap();
    assert_eq!(
        journal
            .inner
            .observation_record(&f.execution, &owner, SIGNATURE)
            .await
            .unwrap()
            .status,
        TradeStatus::Confirmed
    );
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let pending = Reader::new(Answer::Pending);
    assert_eq!(
        query(journal.as_ref(), &pending, &f, &owner)
            .await
            .unwrap()
            .record
            .status,
        TradeStatus::Confirmed
    );
    assert_eq!(pending.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn ordinary_observation_refreshes_concurrent_finality_after_a_missing_rpc_result() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let journal = Arc::new(journal);
    let owner = Arc::new(owner);
    let release = Arc::new(Notify::new());
    let reader = Arc::new(Reader::new(Answer::GatedMissing(release.clone())));
    let task = {
        let (j, o, r, ex) = (
            journal.clone(),
            owner.clone(),
            reader.clone(),
            f.execution.clone(),
        );
        tokio::spawn(async move {
            reconcile(
                j.as_ref(),
                r.as_ref(),
                &ex,
                &o,
                SIGNATURE,
                Duration::from_secs(2),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), reader.entered.notified())
        .await
        .unwrap();
    journal
        .inner
        .record_final_observation(
            &f.execution,
            &owner,
            SIGNATURE,
            &observation(ChainCommitment::Finalized, None),
        )
        .await
        .unwrap();
    release.notify_one();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.record.status, TradeStatus::Confirmed);
    assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
    assert!(result.lookup_error.is_none());
}

async fn file_setup() -> (
    Fixture,
    ExecutionOwner,
    Backend,
    Journal,
    TradePersistenceConfig,
    PathBuf,
) {
    let dir = std::env::temp_dir().join(format!("ordinary-observation-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2_000,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let (f, owner, backend, journal) = setup_fixture(admit(state).await).await;
    (f, owner, backend, journal, config, dir)
}

#[tokio::test]
async fn ordinary_observation_file_reopen_retains_evidence_without_rebroadcast_or_new_owner() {
    for finalized in [false, true] {
        let (f, owner, mut backend, journal, config, dir) = file_setup().await;
        if finalized {
            backend.observation = Some(observation(ChainCommitment::Finalized, None));
        }
        execute(&journal, &backend, &backend, &f.execution, &owner).await;
        let before = snapshot(&f, &journal.pool).await;
        journal.pool.close().await;
        let restored = TradePersistence::new(&config).await.unwrap();
        let inner = restored.ordinary_execution_claims().unwrap();
        let record = inner
            .observation_record(&f.execution, &owner, SIGNATURE)
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(&record).unwrap(), before["parent"]);
        let after: Option<String> =
            sqlx::query_scalar("SELECT final_observation_json FROM ordinary_execution_claims")
                .fetch_one(&restored.pool_for_tests())
                .await
                .unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), before["evidence"][0]);
        assert_eq!(
            inner.claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        let repeated = journaled(execute(&inner, &backend, &backend, &f.execution, &owner).await);
        assert_eq!(repeated.record.unwrap().status, record.status);
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 1);
        assert_eq!(count(&backend, "lookup_signature"), 1);
        restored.pool_for_tests().close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn ordinary_observation_missing_lookup_works_on_read_only_sqlite_without_initialization() {
    use std::str::FromStr;
    let (f, owner, backend, journal, config, dir) = file_setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let before = snapshot(&f, &journal.pool).await;
    let options = sqlx::sqlite::SqliteConnectOptions::from_str(&config.database_url)
        .unwrap()
        .read_only(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let read_only = SqliteOrdinaryExecutionClaims::new(pool.clone());
    let reader = Reader::new(Answer::Missing);
    assert_eq!(
        query(&read_only, &reader, &f, &owner)
            .await
            .unwrap()
            .record
            .status,
        TradeStatus::Submitted
    );
    let finality = Reader::new(Answer::Found(observation(ChainCommitment::Finalized, None)));
    assert!(query(&read_only, &finality, &f, &owner).await.is_err());
    assert_eq!(snapshot(&f, &journal.pool).await, before);
    pool.close().await;
    journal.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn ordinary_observation_two_sqlite_pools_converge_without_replacing_final_evidence() {
    let (f, owner, backend, journal, config, dir) = file_setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let second = TradePersistence::new(&config).await.unwrap();
    let first = Arc::new(journal.inner);
    let other = Arc::new(second.ordinary_execution_claims().unwrap());
    let owner = Arc::new(owner);
    let mut tasks = vec![];
    for index in 0..16 {
        let j = if index % 2 == 0 {
            first.clone()
        } else {
            other.clone()
        };
        let (o, ex) = (owner.clone(), f.execution.clone());
        tasks.push(tokio::spawn(async move {
            j.record_final_observation(
                &ex,
                &o,
                SIGNATURE,
                &observation(ChainCommitment::Finalized, None),
            )
            .await
        }));
    }
    for task in tasks {
        let record = task.await.unwrap().unwrap();
        assert_eq!(record.status, TradeStatus::Confirmed);
        assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
    }
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
    journal.pool.close().await;
    second.pool_for_tests().close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn ordinary_observation_005_schema_migrates_once_preserving_signed_rows_and_parent() {
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    let before = snapshot(&f, &journal.pool).await;
    sqlx::raw_sql("DROP INDEX idx_ordinary_signed_observation_scan; ALTER TABLE ordinary_execution_claims DROP COLUMN final_observation_json; DELETE FROM ordinary_submission_migrations WHERE name IN ('006_ordinary_final_observations','007_ordinary_observation_scan');")
        .execute(&journal.pool).await.unwrap();
    for _ in 0..2 {
        journal.inner.initialize().await.unwrap();
        assert_eq!(snapshot(&f, &journal.pool).await, before);
        assert_eq!(
            journal.inner.claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name='006_ordinary_final_observations'").fetch_one(&journal.pool).await.unwrap(), 1);
    }
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn ordinary_scan_real_journal_finalizes_without_a_new_claim_signature_or_send() {
    use crate::application::ordinary_observation_scan::OrdinaryObservationScanner;
    let (f, owner, backend, journal) = setup().await;
    execute(&journal, &backend, &backend, &f.execution, &owner).await;
    sqlx::raw_sql("CREATE TRIGGER deny_scan_owner BEFORE UPDATE OF owner_nonce ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no new owner'); END; CREATE TRIGGER deny_scan_claim BEFORE INSERT ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no new claim'); END; CREATE TRIGGER deny_scan_signature BEFORE UPDATE OF signed_signature,signed_transaction ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no new signature'); END;")
        .execute(&journal.pool).await.unwrap();
    drop(owner);
    let reader = Arc::new(Reader::new(Answer::Found(observation(
        ChainCommitment::Finalized,
        None,
    ))));
    let mut scanner = OrdinaryObservationScanner::new(journal.inner, reader.clone());
    let result = scanner.step(i64::MAX, 8, Duration::ZERO).await.unwrap();
    assert_eq!(
        (result.examined, result.finalized, result.rejected),
        (1, 1, 0)
    );
    assert_eq!(
        f.state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TradeStatus::Confirmed
    );
    assert_eq!(
        scanner
            .step(i64::MAX, 8, Duration::ZERO)
            .await
            .unwrap()
            .examined,
        0
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}
