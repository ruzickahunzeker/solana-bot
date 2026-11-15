use super::*;
use crate::{
    adapters::persistence::submission_journal::SqliteSubmissionJournal,
    api::dto::quote_submissions::QuoteSubmissionStatusResponse,
    application::ports::submission_journal::SubmissionJournal,
    domain::{
        reconciliation::ChainObservation,
        submission::{Reservation, SignedSubmission, SubmissionIntent, SubmissionStatus},
    },
};
use anyhow::Result;
use serde_json::json;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;

const SIGNATURE: &str = "test-only-signature";

fn intent() -> SubmissionIntent {
    SubmissionIntent {
        key: SubmissionKey::new("wallet", "key").unwrap(),
        wallet_pubkey: "pubkey".into(),
        quote_id: "quote".into(),
        quote: json!({"amount_lamports":1000}),
    }
}

async fn setup(signed: bool) -> (SqlitePool, SqliteSubmissionJournal, String) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let journal = SqliteSubmissionJournal::new(pool.clone());
    journal.initialize().await.unwrap();
    let Reservation::Created(record) = journal.reserve(&intent(), &json!({})).await.unwrap() else {
        panic!("expected new reservation")
    };
    if signed {
        journal
            .record_signed(
                &record.submission_id,
                &SignedSubmission {
                    signature: SIGNATURE.into(),
                    bytes: vec![1, 2, 3],
                },
            )
            .await
            .unwrap();
    }
    (pool, journal, record.submission_id)
}

fn observation(commitment: ChainCommitment, error: Option<&str>) -> ChainObservation {
    ChainObservation {
        signature: SIGNATURE.into(),
        commitment,
        slot: 100 + commitment.rank() as u64,
        error: error.map(str::to_owned),
    }
}

enum Answer {
    Found(ChainObservation),
    Missing,
    Error,
    Pending,
}
struct Reader {
    answer: Answer,
    calls: AtomicUsize,
    entered: Option<Arc<Notify>>,
    release: Option<Arc<Notify>>,
}
impl Reader {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            calls: AtomicUsize::new(0),
            entered: None,
            release: None,
        }
    }
    fn found(commitment: ChainCommitment, error: Option<&str>) -> Self {
        Self::new(Answer::Found(observation(commitment, error)))
    }
}
impl SignatureStatusReader for Reader {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        assert_eq!(signature, SIGNATURE);
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(notify) = &self.entered {
            notify.notify_one();
        }
        if let Some(notify) = &self.release {
            notify.notified().await;
        }
        match &self.answer {
            Answer::Found(observation) => Ok(Some(observation.clone())),
            Answer::Missing => Ok(None),
            Answer::Error => anyhow::bail!(
                "RPC unavailable at https://rpc.example.invalid/?api-key=fixture-secret"
            ),
            Answer::Pending => std::future::pending().await,
        }
    }
}

async fn query<J: ReconciliationJournal>(journal: &J, reader: &Reader) -> ReconciliationResult {
    reconcile(
        journal,
        reader,
        &intent().key,
        "pubkey",
        "quote",
        Duration::from_millis(20),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn confirmation_advances_to_finality_without_losing_original_bytes() {
    let (pool, journal, _) = setup(true).await;
    for (commitment, expected) in [
        (ChainCommitment::Processed, SubmissionStatus::Submitted),
        (ChainCommitment::Confirmed, SubmissionStatus::Confirmed),
        (ChainCommitment::Finalized, SubmissionStatus::Finalized),
    ] {
        let result = query(&journal, &Reader::found(commitment, None)).await;
        assert_eq!(result.record.status, expected);
        assert_eq!(result.record.commitment, Some(commitment));
        assert_eq!(result.outcome, LookupOutcome::Updated);
        assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
        assert!(result.record.error.is_none());
    }
    let reader = Reader::new(Answer::Pending);
    assert_eq!(
        query(&journal, &reader).await.outcome,
        LookupOutcome::NotRequired
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    let bytes: Vec<u8> = sqlx::query_scalar("SELECT signed_transaction FROM quote_submissions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
}

#[tokio::test]
async fn processed_error_is_not_terminal_and_stronger_confirmation_can_succeed() {
    let (_, journal, _) = setup(true).await;
    let pending = query(
        &journal,
        &Reader::found(
            ChainCommitment::Processed,
            Some("processed execution error"),
        ),
    )
    .await;
    assert_eq!(pending.record.status, SubmissionStatus::Unknown);
    assert_eq!(pending.record.commitment, Some(ChainCommitment::Processed));
    let confirmed = query(&journal, &Reader::found(ChainCommitment::Confirmed, None)).await;
    assert_eq!(confirmed.record.status, SubmissionStatus::Confirmed);
    assert!(confirmed.record.error.is_none());
}

#[tokio::test]
async fn confirmed_failure_can_be_superseded_by_finality_and_final_failure_is_retained() {
    let (_, journal, _) = setup(true).await;
    let confirmed = query(
        &journal,
        &Reader::found(ChainCommitment::Confirmed, Some("confirmed error")),
    )
    .await;
    assert_eq!(confirmed.record.status, SubmissionStatus::Failed);
    let final_result = query(&journal, &Reader::found(ChainCommitment::Finalized, None)).await;
    assert_eq!(final_result.record.status, SubmissionStatus::Finalized);

    let (_, journal, _) = setup(true).await;
    let failed = query(
        &journal,
        &Reader::found(ChainCommitment::Finalized, Some("final execution error")),
    )
    .await;
    assert_eq!(failed.record.status, SubmissionStatus::Failed);
    let reader = Reader::found(ChainCommitment::Finalized, None);
    let repeated = query(&journal, &reader).await;
    assert_eq!(repeated.record.status, SubmissionStatus::Failed);
    assert_eq!(
        repeated.record.error.as_deref(),
        Some("final execution error")
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_timeout_and_rpc_error_preserve_status_and_original_signature() {
    for answer in [Answer::Missing, Answer::Pending, Answer::Error] {
        let (_, journal, id) = setup(true).await;
        journal
            .record_broadcast(
                &id,
                SubmissionStatus::Unknown,
                Some("original broadcast timeout"),
            )
            .await
            .unwrap();
        let reader = Reader::new(answer);
        let result = query(&journal, &reader).await;
        assert!(matches!(
            result.outcome,
            LookupOutcome::NotFound | LookupOutcome::Unavailable
        ));
        assert_eq!(result.record.status, SubmissionStatus::Unknown);
        assert_eq!(result.record.signature.as_deref(), Some(SIGNATURE));
        assert_eq!(
            result.record.error.as_deref(),
            Some("original broadcast timeout")
        );
        assert!(result.record.commitment.is_none());
        if let Some(error) = result.lookup_error {
            assert!(!error.contains("fixture-secret"));
        }
        assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn unsigned_records_identity_conflicts_and_missing_orders_do_not_query_rpc() {
    let (pool, journal, _) = setup(false).await;
    let reader = Reader::found(ChainCommitment::Finalized, None);
    assert_eq!(
        query(&journal, &reader).await.outcome,
        LookupOutcome::NotRequired
    );
    for (pubkey, quote) in [("wrong-pubkey", "quote"), ("pubkey", "wrong-quote")] {
        let err = reconcile(
            &journal,
            &reader,
            &intent().key,
            pubkey,
            quote,
            Duration::from_secs(1),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(err.kind, FailureKind::Conflict);
    }
    let err = reconcile(
        &journal,
        &reader,
        &SubmissionKey::new("wallet", "missing").unwrap(),
        "pubkey",
        "quote",
        Duration::from_secs(1),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.kind, FailureKind::NotFound);
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_submissions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn weaker_or_same_rank_observations_cannot_overwrite_confirmation() {
    let (_, journal, _) = setup(true).await;
    let first = query(&journal, &Reader::found(ChainCommitment::Confirmed, None)).await;
    for commitment in [ChainCommitment::Processed, ChainCommitment::Confirmed] {
        let result = query(&journal, &Reader::found(commitment, Some("stale error"))).await;
        assert_eq!(result.outcome, LookupOutcome::Unchanged);
        assert_eq!(result.record.status, SubmissionStatus::Confirmed);
        assert_eq!(result.record.observed_slot, first.record.observed_slot);
        assert!(result.record.error.is_none());
    }
}

#[tokio::test]
async fn concurrent_late_confirmed_response_cannot_downgrade_finalized() {
    let (_, journal, _) = setup(true).await;
    let journal = Arc::new(journal);
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let reader = Arc::new(Reader {
        answer: Answer::Found(observation(ChainCommitment::Confirmed, None)),
        calls: AtomicUsize::new(0),
        entered: Some(entered.clone()),
        release: Some(release.clone()),
    });
    let task = {
        let journal = journal.clone();
        let reader = reader.clone();
        tokio::spawn(async move {
            reconcile(
                journal.as_ref(),
                reader.as_ref(),
                &intent().key,
                "pubkey",
                "quote",
                Duration::from_secs(5),
            )
            .await
            .unwrap()
        })
    };
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert_eq!(
        query(
            journal.as_ref(),
            &Reader::found(ChainCommitment::Finalized, None)
        )
        .await
        .record
        .status,
        SubmissionStatus::Finalized
    );
    release.notify_one();
    let delayed = task.await.unwrap();
    assert_eq!(delayed.record.status, SubmissionStatus::Finalized);
    assert_eq!(delayed.outcome, LookupOutcome::Unchanged);
}

#[tokio::test]
async fn concurrent_rpc_miss_or_error_returns_current_finalized_record() {
    for answer in [Answer::Missing, Answer::Error] {
        let (_, journal, _) = setup(true).await;
        let journal = Arc::new(journal);
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let reader = Arc::new(Reader {
            answer,
            calls: AtomicUsize::new(0),
            entered: Some(entered.clone()),
            release: Some(release.clone()),
        });
        let task = {
            let journal = journal.clone();
            let reader = reader.clone();
            tokio::spawn(async move {
                reconcile(
                    journal.as_ref(),
                    reader.as_ref(),
                    &intent().key,
                    "pubkey",
                    "quote",
                    Duration::from_secs(5),
                )
                .await
                .unwrap()
            })
        };
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        query(
            journal.as_ref(),
            &Reader::found(ChainCommitment::Finalized, None),
        )
        .await;
        release.notify_one();
        let result = task.await.unwrap();
        assert_eq!(result.record.status, SubmissionStatus::Finalized);
        assert!(matches!(
            result.outcome,
            LookupOutcome::NotFound | LookupOutcome::Unavailable
        ));
    }
}

#[tokio::test]
async fn file_restart_preserves_finalized_observation_without_rpc_or_replay() {
    use crate::{
        config::{TradePersistenceConfig, TradePersistenceMode},
        persistence::TradePersistence,
    };
    let dir = std::env::temp_dir().join(format!("reconciliation-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("trades.sqlite").display()),
        busy_timeout_ms: 1000,
    };
    let persistence = TradePersistence::new(&config).await.unwrap();
    let journal = persistence.submission_journal().unwrap();
    let Reservation::Created(record) = journal.reserve(&intent(), &json!({})).await.unwrap() else {
        panic!("expected new order")
    };
    journal
        .record_signed(
            &record.submission_id,
            &SignedSubmission {
                signature: SIGNATURE.into(),
                bytes: vec![1, 2, 3],
            },
        )
        .await
        .unwrap();
    query(&journal, &Reader::found(ChainCommitment::Finalized, None)).await;
    drop(journal);
    drop(persistence);
    let persistence = TradePersistence::new(&config).await.unwrap();
    let journal = persistence.submission_journal().unwrap();
    let reader = Reader::new(Answer::Pending);
    let restored = query(&journal, &reader).await;
    assert_eq!(restored.record.status, SubmissionStatus::Finalized);
    assert_eq!(restored.record.observed_slot, Some(103));
    assert_eq!(restored.record.signature.as_deref(), Some(SIGNATURE));
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    drop(journal);
    drop(persistence);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn late_broadcast_result_preserves_observed_confirmation() {
    let (_, journal, id) = setup(true).await;
    query(&journal, &Reader::found(ChainCommitment::Confirmed, None)).await;
    journal
        .record_broadcast(&id, SubmissionStatus::Unknown, Some("late timeout"))
        .await
        .unwrap();
    let record = journal.get(&intent().key).await.unwrap().unwrap();
    assert_eq!(record.status, SubmissionStatus::Confirmed);
    assert!(record.error.is_none());
}

#[tokio::test]
async fn wrong_signature_observation_never_updates_order() {
    let (_, journal, id) = setup(true).await;
    let mut wrong = observation(ChainCommitment::Finalized, None);
    wrong.signature = "different-signature".into();
    let result = query(&journal, &Reader::new(Answer::Found(wrong.clone()))).await;
    assert_eq!(result.outcome, LookupOutcome::Unavailable);
    assert!(result.record.commitment.is_none());
    assert!(!journal.record_observation(&id, &wrong).await.unwrap());
}

struct WriteFailure<'a>(&'a SqliteSubmissionJournal);
impl ReconciliationJournal for WriteFailure<'_> {
    async fn lookup(&self, key: &SubmissionKey) -> Result<Option<SubmissionRecord>> {
        self.0.lookup(key).await
    }
    async fn record_observation(&self, _: &str, _: &ChainObservation) -> Result<bool> {
        anyhow::bail!("test write failure")
    }
}

#[tokio::test]
async fn observation_write_failure_returns_unavailable_without_claiming_confirmation() {
    let (_, journal, _) = setup(true).await;
    let err = reconcile(
        &WriteFailure(&journal),
        &Reader::found(ChainCommitment::Finalized, None),
        &intent().key,
        "pubkey",
        "quote",
        Duration::from_secs(1),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.kind, FailureKind::Unavailable);
    assert_eq!(
        journal.get(&intent().key).await.unwrap().unwrap().status,
        SubmissionStatus::Signed
    );
}

#[tokio::test]
async fn query_response_excludes_signed_bytes_and_preserves_separate_lookup_errors() {
    let (_, journal, _) = setup(true).await;
    let response =
        QuoteSubmissionStatusResponse::from(query(&journal, &Reader::new(Answer::Error)).await);
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["status"], "unknown");
    assert_eq!(value["reconciliation_status"], "unavailable");
    assert_eq!(value["signature"], SIGNATURE);
    for key in [
        "signed_transaction",
        "bytes",
        "private_key",
        "intent",
        "idempotency_key",
    ] {
        assert!(value.get(key).is_none());
    }
    assert!(!value.to_string().contains("fixture-secret"));
}

#[tokio::test]
async fn legacy_schema_migrates_idempotently_without_losing_identity_or_signed_bytes() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/002_quote_submissions.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let intent = intent();
    sqlx::query("INSERT INTO quote_submissions(submission_id,wallet_id,idempotency_key,fingerprint,intent_json,preflight_json,status,signature,signed_transaction,created_at_ms,updated_at_ms) VALUES('legacy','wallet','key',?,?,'{}','submitted',?,X'010203',1,1)")
        .bind(intent.fingerprint().unwrap()).bind(serde_json::to_string(&intent).unwrap()).bind(SIGNATURE).execute(&pool).await.unwrap();
    let journal = SqliteSubmissionJournal::new(pool.clone());
    journal.initialize().await.unwrap();
    let old = journal.get(&intent.key).await.unwrap().unwrap();
    assert_eq!(old.submission_id, "legacy");
    assert_eq!(old.status, SubmissionStatus::Submitted);
    assert_eq!(old.signature.as_deref(), Some(SIGNATURE));
    assert!(old.commitment.is_none());
    query(&journal, &Reader::found(ChainCommitment::Finalized, None)).await;
    journal.initialize().await.unwrap();
    assert_eq!(
        journal.get(&intent.key).await.unwrap().unwrap().status,
        SubmissionStatus::Finalized
    );
    let bytes: Vec<u8> = sqlx::query_scalar("SELECT signed_transaction FROM quote_submissions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_submission_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn migration_failure_rolls_back_and_does_not_mark_migration_complete() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/002_quote_submissions.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("CREATE TABLE quote_submissions_v1(block_migration INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    let journal = SqliteSubmissionJournal::new(pool.clone());
    assert!(journal.initialize().await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='quote_submission_migrations'").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
    sqlx::query("DROP TABLE quote_submissions_v1")
        .execute(&pool)
        .await
        .unwrap();
    journal.initialize().await.unwrap();
}
