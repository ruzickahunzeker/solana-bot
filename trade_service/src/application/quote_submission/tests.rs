use super::*;
use crate::adapters::persistence::submission_journal::SqliteSubmissionJournal;
use crate::config::{TradePersistenceConfig, TradePersistenceMode};
use crate::domain::quote::{QuoteEstimate, QuotePreflight, QuoteRouteLeg};
use crate::domain::submission::SignedSubmission;
use crate::persistence::TradePersistence;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{Barrier, Notify};

fn bound() -> BoundQuote {
    BoundQuote {
        quote_id: "fixture-quote".into(),
        wallet_id: "fixture-wallet".into(),
        wallet_pubkey: "fixture-pubkey".into(),
        quote_slippage_bps: 100,
        expires_at_ms: u64::MAX,
        quote: QuoteEstimate {
            input_mint: "fixture-wsol".into(),
            output_mint: "fixture-target".into(),
            amount_lamports: 1_000_000,
            expected_output_units: 1000,
            minimum_output_units: 990,
            price_impact_percent: "0.01".into(),
            route: vec![QuoteRouteLeg {
                label: "fixture".into(),
                input_mint: "fixture-wsol".into(),
                output_mint: "fixture-target".into(),
            }],
            executable: false,
        },
    }
}

fn request() -> QuoteSubmissionCommand {
    QuoteSubmissionCommand {
        wallet_id: "fixture-wallet".into(),
        confirmed: true,
        simulate: false,
        idempotency_key: "fixture-key".into(),
    }
}

fn preflight() -> QuotePreflight {
    QuotePreflight {
        quote_id: "fixture-quote".into(),
        wallet_id: "fixture-wallet".into(),
        quote_still_valid: true,
        unsigned_transaction_built: true,
        aggregator_simulation_ok: true,
        local_simulation_ok: true,
        instruction_count: 1,
        address_lookup_table_count: 0,
        resolved_account_count: 2,
        instruction_audit_passed: true,
        executable: true,
        reason: "test-only fake preflight".into(),
        prioritization_fee_lamports: 1,
        compute_unit_limit: 200_000,
        calculated_priority_fee_lamports: 1,
        explicit_system_spend_lamports: 1_000_000,
        writable_account_count: 2,
        target_ata: "fixture-ata".into(),
        top_level_programs: vec![],
        simulated_fee_lamports: Some(5000),
        simulated_units_consumed: Some(100),
        simulated_payer_debit_lamports: Some(1_005_000),
        simulated_inner_instruction_groups: Some(0),
        simulated_target_token_delta_units: Some(1000),
        simulated_target_token_post_units: Some(1000),
        baseline_slot: 1,
        simulation_slot: 1,
        observed_same_slot_payer_debit_lamports: Some(1_005_000),
        observed_same_slot_target_token_delta_units: Some(1000),
        simulated_inner_instruction_count: Some(0),
        simulated_inner_programs: Some(vec![]),
        simulated_inner_payer_system_spend_lamports: Some(1_000_000),
    }
}

async fn journal() -> SqliteSubmissionJournal {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let journal = SqliteSubmissionJournal::new(pool);
    journal.initialize().await.unwrap();
    journal
}

#[derive(Default)]
struct FakeBackend {
    prepared: AtomicUsize,
    signed: AtomicUsize,
    sent: AtomicUsize,
    prepare_barrier: Option<Arc<Barrier>>,
    broadcast_entered: Option<Arc<Notify>>,
    fail_prepare: bool,
    fail_sign: bool,
    broadcast_disabled: bool,
    invalid_preflight: bool,
    timeout: bool,
    different_signature: bool,
}

impl FakeBackend {
    fn counts(&self) -> (usize, usize, usize) {
        (
            self.prepared.load(Ordering::SeqCst),
            self.signed.load(Ordering::SeqCst),
            self.sent.load(Ordering::SeqCst),
        )
    }
}

impl QuoteSubmissionBackend for FakeBackend {
    type Prepared = ();
    fn ensure_live_allowed(&self) -> Result<()> {
        if self.broadcast_disabled {
            anyhow::bail!(crate::domain::execution_policy::BROADCAST_DISABLED_ERROR);
        }
        Ok(())
    }

    async fn prepare(&self, bound: &BoundQuote) -> Result<((), QuotePreflight)> {
        self.prepared.fetch_add(1, Ordering::SeqCst);
        if self.fail_prepare {
            anyhow::bail!("fake preflight failed");
        }
        if let Some(barrier) = &self.prepare_barrier {
            barrier.wait().await;
        }
        let mut preflight = preflight();
        preflight.quote_id = bound.quote_id.clone();
        preflight.wallet_id = bound.wallet_id.clone();
        if self.invalid_preflight {
            preflight.wallet_id = "wrong-wallet".into();
        }
        Ok(((), preflight))
    }
    fn sign(&self, _: ()) -> Result<SignedSubmission> {
        self.signed.fetch_add(1, Ordering::SeqCst);
        if self.fail_sign {
            anyhow::bail!("fake signer unavailable");
        }
        Ok(SignedSubmission {
            signature: "fixture-signature".into(),
            bytes: vec![1, 2, 3],
        })
    }
    async fn broadcast(&self, _: &SignedSubmission) -> Result<String> {
        self.sent.fetch_add(1, Ordering::SeqCst);
        if let Some(notify) = &self.broadcast_entered {
            notify.notify_one();
            std::future::pending::<()>().await;
        }
        if self.timeout {
            anyhow::bail!("fake RPC timeout");
        }
        Ok(if self.different_signature {
            "wrong-signature"
        } else {
            "fixture-signature"
        }
        .into())
    }
}

struct FaultJournal<'a> {
    inner: &'a SqliteSubmissionJournal,
    reserve: bool,
    signed: bool,
    result: bool,
}

impl SubmissionJournal for FaultJournal<'_> {
    async fn get(&self, key: &SubmissionKey) -> Result<Option<SubmissionRecord>> {
        self.inner.get(key).await
    }
    async fn reserve(&self, intent: &SubmissionIntent, preflight: &Value) -> Result<Reservation> {
        if self.reserve {
            anyhow::bail!("fake reserve write failure");
        }
        self.inner.reserve(intent, preflight).await
    }
    async fn record_signed(&self, id: &str, signed: &SignedSubmission) -> Result<()> {
        if self.signed {
            anyhow::bail!("fake signature write failure");
        }
        self.inner.record_signed(id, signed).await
    }
    async fn fail_before_sign(&self, id: &str, error: &str) -> Result<()> {
        self.inner.fail_before_sign(id, error).await
    }
    async fn record_broadcast(
        &self,
        id: &str,
        status: SubmissionStatus,
        error: Option<&str>,
    ) -> Result<()> {
        if self.result {
            anyhow::bail!("fake result write failure");
        }
        self.inner.record_broadcast(id, status, error).await
    }
}

#[tokio::test]
async fn concurrent_same_key_prepares_many_but_signs_and_broadcasts_once() {
    let journal = Arc::new(journal().await);
    let backend = Arc::new(FakeBackend {
        prepare_barrier: Some(Arc::new(Barrier::new(16))),
        ..Default::default()
    });
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let journal = journal.clone();
        let backend = backend.clone();
        tasks.push(tokio::spawn(async move {
            execute(
                Some(journal.as_ref()),
                backend.as_ref(),
                &bound(),
                &request(),
            )
            .await
            .unwrap()
        }));
    }
    for task in tasks {
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .status,
            QuoteSubmissionOutcome::Pending
                | QuoteSubmissionOutcome::Unknown
                | QuoteSubmissionOutcome::Submitted
        ));
    }
    assert_eq!(backend.counts(), (16, 1, 1));
    let replay = existing(
        journal.as_ref(),
        &validate(&request()).unwrap(),
        "fixture-pubkey",
        "fixture-quote",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(replay.status, QuoteSubmissionOutcome::Submitted);
    assert_eq!(replay.signature.as_deref(), Some("fixture-signature"));
}

#[tokio::test]
async fn simulation_does_not_reserve_sign_or_broadcast_or_replay_live_result() {
    let journal = journal().await;
    let backend = FakeBackend::default();
    let mut simulation = request();
    simulation.simulate = true;
    simulation.confirmed = false;
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &simulation)
            .await
            .unwrap()
            .status,
        QuoteSubmissionOutcome::Simulated
    );
    assert!(journal
        .get(&validate(&request()).unwrap())
        .await
        .unwrap()
        .is_none());
    assert_eq!(backend.counts(), (1, 0, 0));
    execute(Some(&journal), &backend, &bound(), &request())
        .await
        .unwrap();
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &simulation)
            .await
            .unwrap()
            .status,
        QuoteSubmissionOutcome::Simulated
    );
    assert_eq!(backend.counts(), (3, 1, 1));
}

#[tokio::test]
async fn live_requires_confirmation_and_durable_storage_before_preparation() {
    let journal = journal().await;
    let backend = FakeBackend::default();
    let mut req = request();
    req.confirmed = false;
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &req)
            .await
            .unwrap_err()
            .kind,
        FailureKind::InvalidRequest
    );
    assert_eq!(
        execute::<SqliteSubmissionJournal, _>(None, &backend, &bound(), &request())
            .await
            .unwrap_err()
            .kind,
        FailureKind::Unavailable
    );
    assert_eq!(backend.counts(), (0, 0, 0));
    req.simulate = true;
    assert_eq!(
        execute::<SqliteSubmissionJournal, _>(None, &backend, &bound(), &req)
            .await
            .unwrap()
            .status,
        QuoteSubmissionOutcome::Simulated
    );
}

#[tokio::test]
async fn broadcast_policy_blocks_live_before_prepare_reserve_or_sign_but_allows_unsigned_simulation(
) {
    let journal = journal().await;
    let backend = FakeBackend {
        broadcast_disabled: true,
        ..Default::default()
    };
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &request())
            .await
            .unwrap_err()
            .kind,
        FailureKind::Forbidden
    );
    assert_eq!(backend.counts(), (0, 0, 0));
    assert!(journal
        .get(&validate(&request()).unwrap())
        .await
        .unwrap()
        .is_none());
    let mut simulation = request();
    simulation.simulate = true;
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &simulation)
            .await
            .unwrap()
            .status,
        QuoteSubmissionOutcome::Simulated
    );
    assert_eq!(backend.counts(), (1, 0, 0));
}

#[tokio::test]
async fn disabling_broadcast_does_not_prevent_existing_order_lookup() {
    let journal = journal().await;
    execute(
        Some(&journal),
        &FakeBackend::default(),
        &bound(),
        &request(),
    )
    .await
    .unwrap();
    let disabled = FakeBackend {
        broadcast_disabled: true,
        ..Default::default()
    };
    assert_eq!(
        execute(Some(&journal), &disabled, &bound(), &request())
            .await
            .unwrap()
            .status,
        QuoteSubmissionOutcome::Submitted
    );
    assert_eq!(disabled.counts(), (0, 0, 0));
}

#[tokio::test]
async fn same_key_on_different_wallets_is_not_a_collision() {
    let journal = journal().await;
    let backend = FakeBackend::default();
    execute(Some(&journal), &backend, &bound(), &request())
        .await
        .unwrap();
    let mut other = bound();
    other.wallet_id = "other-wallet".into();
    other.wallet_pubkey = "other-pubkey".into();
    let mut req = request();
    req.wallet_id = "other-wallet".into();
    execute(Some(&journal), &backend, &other, &req)
        .await
        .unwrap();
    assert_eq!(backend.counts(), (2, 2, 2));
    assert!(journal
        .get(&SubmissionKey::new("fixture-wallet", "fixture-key").unwrap())
        .await
        .unwrap()
        .is_some());
    assert!(journal
        .get(&SubmissionKey::new("other-wallet", "fixture-key").unwrap())
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn confirmation_during_broadcast_wins_in_both_journal_and_submit_response() {
    use crate::application::ports::reconciliation::ReconciliationJournal;
    use crate::domain::reconciliation::{ChainCommitment, ChainObservation};
    struct ConfirmingBackend<'a> {
        inner: FakeBackend,
        journal: &'a SqliteSubmissionJournal,
    }
    impl QuoteSubmissionBackend for ConfirmingBackend<'_> {
        type Prepared = ();
        fn ensure_live_allowed(&self) -> Result<()> {
            self.inner.ensure_live_allowed()
        }
        async fn prepare(&self, bound: &BoundQuote) -> Result<((), QuotePreflight)> {
            self.inner.prepare(bound).await
        }
        fn sign(&self, prepared: ()) -> Result<SignedSubmission> {
            self.inner.sign(prepared)
        }
        async fn broadcast(&self, signed: &SignedSubmission) -> Result<String> {
            let record = self
                .journal
                .get(&validate(&request()).unwrap())
                .await?
                .unwrap();
            assert_eq!(record.status, SubmissionStatus::Signed);
            assert!(
                self.journal
                    .record_observation(
                        &record.submission_id,
                        &ChainObservation {
                            signature: signed.signature.clone(),
                            commitment: ChainCommitment::Confirmed,
                            slot: 100,
                            error: None,
                        }
                    )
                    .await?
            );
            self.inner.broadcast(signed).await
        }
    }
    let journal = journal().await;
    let backend = ConfirmingBackend {
        inner: FakeBackend::default(),
        journal: &journal,
    };
    let response = execute(Some(&journal), &backend, &bound(), &request())
        .await
        .unwrap();
    assert_eq!(response.status, QuoteSubmissionOutcome::Confirmed);
    assert_eq!(response.signature.as_deref(), Some("fixture-signature"));
    assert_eq!(
        journal
            .get(&validate(&request()).unwrap())
            .await
            .unwrap()
            .unwrap()
            .status,
        SubmissionStatus::Confirmed
    );
    assert_eq!(backend.inner.counts(), (1, 1, 1));
}

#[tokio::test]
async fn independent_connection_pools_reserve_same_key_only_once() {
    let dir = std::env::temp_dir().join(format!("quote-journal-race-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("trades.sqlite").display()),
        busy_timeout_ms: 1000,
    };
    let persistence = TradePersistence::new(&config).await.unwrap();
    let first = Arc::new(persistence.submission_journal().unwrap());
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect(&config.database_url)
        .await
        .unwrap();
    let second = Arc::new(SqliteSubmissionJournal::new(pool.clone()));
    let barrier = Arc::new(Barrier::new(16));
    let mut tasks = vec![];
    for index in 0..16 {
        let journal = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            let intent = SubmissionIntent {
                key: validate(&request()).unwrap(),
                wallet_pubkey: "fixture-pubkey".into(),
                quote_id: "fixture-quote".into(),
                quote: serde_json::to_value(bound()).unwrap(),
            };
            barrier.wait().await;
            match journal
                .reserve(&intent, &serde_json::to_value(preflight()).unwrap())
                .await
                .unwrap()
            {
                Reservation::Created(record) => (1, record.submission_id),
                Reservation::Existing(record) => (0, record.submission_id),
                Reservation::Conflict => panic!("same intent must not conflict"),
            }
        }));
    }
    let mut created = 0;
    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        let (count, id) = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        created += count;
        ids.insert(id);
    }
    assert_eq!(created, 1);
    assert_eq!(ids.len(), 1);
    pool.close().await;
    drop(first);
    drop(second);
    drop(persistence);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn preflight_for_another_wallet_never_reserves_or_signs() {
    let journal = journal().await;
    let backend = FakeBackend {
        invalid_preflight: true,
        ..Default::default()
    };
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &request())
            .await
            .unwrap_err()
            .kind,
        FailureKind::Backend
    );
    assert_eq!(backend.counts(), (1, 0, 0));
    assert!(journal
        .get(&validate(&request()).unwrap())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn reserve_and_signature_write_failures_never_broadcast() {
    for signed in [false, true] {
        let journal = journal().await;
        let backend = FakeBackend::default();
        let faults = FaultJournal {
            inner: &journal,
            reserve: !signed,
            signed,
            result: false,
        };
        assert_eq!(
            execute(Some(&faults), &backend, &bound(), &request())
                .await
                .unwrap_err()
                .kind,
            FailureKind::Unavailable
        );
        assert_eq!(backend.counts(), (1, usize::from(signed), 0));
        if signed {
            let replay = execute(Some(&journal), &backend, &bound(), &request())
                .await
                .unwrap();
            assert_eq!(replay.status, QuoteSubmissionOutcome::Pending);
            assert!(replay.signature.is_none());
            assert_eq!(backend.counts(), (1, 1, 0));
        }
    }
}

#[tokio::test]
async fn timeout_and_wrong_rpc_signature_keep_original_signature_without_resubmission() {
    for timeout in [false, true] {
        let journal = journal().await;
        let backend = FakeBackend {
            timeout,
            different_signature: !timeout,
            ..Default::default()
        };
        let first = execute(Some(&journal), &backend, &bound(), &request())
            .await
            .unwrap();
        assert_eq!(first.status, QuoteSubmissionOutcome::Unknown);
        assert_eq!(first.signature.as_deref(), Some("fixture-signature"));
        assert!(first.error.is_some());
        let repeated = execute(Some(&journal), &backend, &bound(), &request())
            .await
            .unwrap();
        assert_eq!(repeated.signature, first.signature);
        assert_eq!(backend.counts(), (1, 1, 1));
    }
}

#[tokio::test]
async fn result_write_failure_keeps_saved_signature_and_never_resends() {
    let journal = journal().await;
    let backend = FakeBackend::default();
    let faults = FaultJournal {
        inner: &journal,
        reserve: false,
        signed: false,
        result: true,
    };
    assert_eq!(
        execute(Some(&faults), &backend, &bound(), &request())
            .await
            .unwrap_err()
            .kind,
        FailureKind::Unavailable
    );
    let repeated = execute(Some(&journal), &backend, &bound(), &request())
        .await
        .unwrap();
    assert_eq!(repeated.status, QuoteSubmissionOutcome::Unknown);
    assert_eq!(repeated.signature.as_deref(), Some("fixture-signature"));
    assert_eq!(backend.counts(), (1, 1, 1));
}

#[tokio::test]
async fn cancellation_during_broadcast_keeps_signature_and_never_resends() {
    let journal = Arc::new(journal().await);
    let notify = Arc::new(Notify::new());
    let backend = Arc::new(FakeBackend {
        broadcast_entered: Some(notify.clone()),
        ..Default::default()
    });
    let task = {
        let journal = journal.clone();
        let backend = backend.clone();
        tokio::spawn(async move {
            execute(
                Some(journal.as_ref()),
                backend.as_ref(),
                &bound(),
                &request(),
            )
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
        .await
        .unwrap();
    let saved = journal
        .get(&validate(&request()).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, SubmissionStatus::Signed);
    assert_eq!(saved.signature.as_deref(), Some("fixture-signature"));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let repeated = execute(
        Some(journal.as_ref()),
        backend.as_ref(),
        &bound(),
        &request(),
    )
    .await
    .unwrap();
    assert_eq!(repeated.status, QuoteSubmissionOutcome::Unknown);
    assert_eq!(backend.counts(), (1, 1, 1));
}

#[tokio::test]
async fn conflicts_and_expired_replays_do_not_access_backend() {
    let journal = journal().await;
    let backend = FakeBackend::default();
    execute(Some(&journal), &backend, &bound(), &request())
        .await
        .unwrap();
    for (pubkey, quote) in [
        ("other-pubkey", "fixture-quote"),
        ("fixture-pubkey", "other-quote"),
    ] {
        assert_eq!(
            existing(&journal, &validate(&request()).unwrap(), pubkey, quote)
                .await
                .unwrap_err()
                .kind,
            FailureKind::Conflict
        );
    }
    let mut expired = bound();
    expired.expires_at_ms = 0;
    assert_eq!(
        execute(Some(&journal), &backend, &expired, &request())
            .await
            .unwrap()
            .status,
        QuoteSubmissionOutcome::Submitted
    );
    assert_eq!(backend.counts(), (1, 1, 1));
}

#[tokio::test]
async fn expired_quote_or_failed_prepare_never_signs() {
    let journal = journal().await;
    for fail_prepare in [false, true] {
        let backend = FakeBackend {
            fail_prepare,
            ..Default::default()
        };
        let mut expired = bound();
        expired.expires_at_ms = 0;
        let error = execute(Some(&journal), &backend, &expired, &request())
            .await
            .unwrap_err();
        assert_eq!(
            error.kind,
            if fail_prepare {
                FailureKind::Backend
            } else {
                FailureKind::Expired
            }
        );
        assert_eq!(backend.counts(), (1, 0, 0));
        assert!(journal
            .get(&validate(&request()).unwrap())
            .await
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn signer_failure_is_terminal_for_the_reserved_key() {
    let journal = journal().await;
    let backend = FakeBackend {
        fail_sign: true,
        ..Default::default()
    };
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &request())
            .await
            .unwrap_err()
            .kind,
        FailureKind::Backend
    );
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &request())
            .await
            .unwrap()
            .status,
        QuoteSubmissionOutcome::Failed
    );
    assert_eq!(backend.counts(), (1, 1, 0));
}

#[tokio::test]
async fn fingerprint_binds_all_quote_content_and_reservation_rejects_changes() {
    let journal = journal().await;
    let intent = SubmissionIntent {
        key: validate(&request()).unwrap(),
        wallet_pubkey: "fixture-pubkey".into(),
        quote_id: "fixture-quote".into(),
        quote: serde_json::to_value(bound()).unwrap(),
    };
    assert!(matches!(
        journal.reserve(&intent, &json!({})).await.unwrap(),
        Reservation::Created(_)
    ));
    for field in [
        "amount_lamports",
        "minimum_output_units",
        "quote_slippage_bps",
        "expires_at_ms",
        "output_mint",
        "route",
    ] {
        let mut changed = intent.clone();
        changed.quote[field] = json!("changed");
        assert_ne!(
            intent.fingerprint().unwrap(),
            changed.fingerprint().unwrap()
        );
        assert!(matches!(
            journal.reserve(&changed, &json!({})).await.unwrap(),
            Reservation::Conflict
        ));
    }
    let mut other_wallet = intent.clone();
    other_wallet.wallet_pubkey = "other".into();
    assert!(matches!(
        journal.reserve(&other_wallet, &json!({})).await.unwrap(),
        Reservation::Conflict
    ));
}

#[tokio::test]
async fn journal_state_transitions_cannot_overwrite_persisted_signature() {
    let journal = journal().await;
    let intent = SubmissionIntent {
        key: validate(&request()).unwrap(),
        wallet_pubkey: "fixture-pubkey".into(),
        quote_id: "fixture-quote".into(),
        quote: json!({}),
    };
    let Reservation::Created(record) = journal.reserve(&intent, &json!({})).await.unwrap() else {
        panic!("new reservation required")
    };
    assert!(journal
        .record_broadcast(&record.submission_id, SubmissionStatus::Submitted, None)
        .await
        .is_err());
    assert!(journal
        .record_signed(
            &record.submission_id,
            &SignedSubmission {
                signature: String::new(),
                bytes: vec![]
            }
        )
        .await
        .is_err());
    let signed = SignedSubmission {
        signature: "original".into(),
        bytes: vec![1],
    };
    journal
        .record_signed(&record.submission_id, &signed)
        .await
        .unwrap();
    assert!(journal
        .record_signed(&record.submission_id, &signed)
        .await
        .is_err());
    assert!(journal
        .fail_before_sign(&record.submission_id, "failure")
        .await
        .is_err());
    journal
        .record_broadcast(
            &record.submission_id,
            SubmissionStatus::Unknown,
            Some("timeout"),
        )
        .await
        .unwrap();
    assert!(journal
        .record_broadcast(&record.submission_id, SubmissionStatus::Submitted, None)
        .await
        .is_err());
    assert_eq!(
        journal
            .get(&intent.key)
            .await
            .unwrap()
            .unwrap()
            .signature
            .as_deref(),
        Some("original")
    );
}

#[tokio::test]
async fn actual_database_failure_stops_before_preparation_or_signing() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let journal = SqliteSubmissionJournal::new(pool.clone());
    journal.initialize().await.unwrap();
    pool.close().await;
    let backend = FakeBackend::default();
    assert_eq!(
        execute(Some(&journal), &backend, &bound(), &request())
            .await
            .unwrap_err()
            .kind,
        FailureKind::Unavailable
    );
    assert_eq!(backend.counts(), (0, 0, 0));
}

#[tokio::test]
async fn real_file_restart_preserves_bytes_and_replays_without_quote_cache() {
    let dir = std::env::temp_dir().join(format!("quote-journal-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("trades.sqlite").display()),
        busy_timeout_ms: 1000,
    };
    let persistence = TradePersistence::new(&config).await.unwrap();
    let journal = persistence.submission_journal().unwrap();
    let backend = FakeBackend::default();
    execute(Some(&journal), &backend, &bound(), &request())
        .await
        .unwrap();
    let mut interrupted = SubmissionIntent {
        key: SubmissionKey::new("fixture-wallet", "interrupted").unwrap(),
        wallet_pubkey: "fixture-pubkey".into(),
        quote_id: "interrupted-quote".into(),
        quote: json!({}),
    };
    let Reservation::Created(record) = journal
        .reserve(&interrupted, &serde_json::to_value(preflight()).unwrap())
        .await
        .unwrap()
    else {
        panic!("new reservation required")
    };
    journal
        .record_signed(
            &record.submission_id,
            &SignedSubmission {
                signature: "saved-before-crash".into(),
                bytes: vec![7, 8, 9],
            },
        )
        .await
        .unwrap();
    interrupted.key.idempotency_key = "unsigned-interrupted".into();
    journal
        .reserve(&interrupted, &serde_json::to_value(preflight()).unwrap())
        .await
        .unwrap();
    drop(journal);
    drop(persistence);
    let persistence = TradePersistence::new(&config).await.unwrap();
    let journal = persistence.submission_journal().unwrap();
    let replay = existing(
        &journal,
        &validate(&request()).unwrap(),
        "fixture-pubkey",
        "fixture-quote",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(replay.status, QuoteSubmissionOutcome::Submitted);
    let interrupted = journal
        .get(&SubmissionKey::new("fixture-wallet", "interrupted").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(interrupted.status, SubmissionStatus::Unknown);
    assert_eq!(interrupted.signature.as_deref(), Some("saved-before-crash"));
    let unsigned = journal
        .get(&SubmissionKey::new("fixture-wallet", "unsigned-interrupted").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unsigned.status, SubmissionStatus::Unknown);
    assert!(unsigned.signature.is_none());
    let pool = SqlitePoolOptions::new()
        .connect(&config.database_url)
        .await
        .unwrap();
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT signed_transaction FROM quote_submissions WHERE idempotency_key='interrupted'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bytes, vec![7, 8, 9]);
    assert_eq!(backend.counts(), (1, 1, 1));
    pool.close().await;
    drop(journal);
    drop(persistence);
    std::fs::remove_dir_all(dir).unwrap();
}
