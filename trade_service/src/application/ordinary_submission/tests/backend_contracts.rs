//! NP-04: independent fake construction, the existing controller and original SQLite authority.
//! Receipts below are deliberately not Solana transactions; no signer or RPC is available.
use super::*;
use crate::domain::reconciliation::ChainObservation;
use std::time::Duration;

mod record_backend;
mod tape_backend;
use record_backend::RecordBackend;
use tape_backend::TapeBackend;

#[derive(Debug, PartialEq, Eq)]
struct IntentView {
    wallet_id: String,
    payer: String,
    input: String,
    output: String,
    amount: u64,
    expected: u64,
    minimum: u64,
    slippage: u16,
    expires: u64,
    legs: Vec<(String, String, String)>,
}

impl IntentView {
    fn expected(bound: &BoundQuote) -> Self {
        Self {
            wallet_id: bound.wallet_id.clone(),
            payer: bound.wallet_pubkey.clone(),
            input: bound.quote.input_mint.clone(),
            output: bound.quote.output_mint.clone(),
            amount: bound.quote.amount_lamports,
            expected: bound.quote.expected_output_units,
            minimum: bound.quote.minimum_output_units,
            slippage: bound.quote_slippage_bps,
            expires: bound.expires_at_ms,
            legs: bound
                .quote
                .route
                .iter()
                .map(|leg| {
                    (
                        leg.label.clone(),
                        leg.input_mint.clone(),
                        leg.output_mint.clone(),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    Sign,
    SignedMessage,
    Send,
    Receipt,
}

// Shared fault observation only, never construction, candidate conversion or a second journal.
struct Probe {
    pool: SqlitePool,
    events: Events,
    report: QuotePreflight,
    fault: Fault,
    halt: Option<&'static str>,
    reached: Arc<Notify>,
}

impl Probe {
    fn event(&self, label: &'static str) {
        self.events.lock().unwrap().push(label);
    }
    fn count(&self, label: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|s| **s == label)
            .count()
    }
    async fn at(&self, label: &'static str) {
        self.event(label);
        if self.halt == Some(label) {
            self.reached.notify_one();
            std::future::pending::<()>().await;
        }
    }
    async fn assert_persisted(&self, signed: &SignedSubmission) -> Result<()> {
        let row: (String, Vec<u8>, String, String) = sqlx::query_as(
            "SELECT c.signed_signature,c.signed_transaction,c.submission_stage,t.status FROM ordinary_execution_claims c JOIN trades t USING(request_id)"
        ).fetch_one(&self.pool).await?;
        ensure!(
            row == (
                signed.signature.clone(),
                signed.bytes.clone(),
                "signed".into(),
                "submitting".into()
            ),
            "fake sender must receive the original persisted receipt bytes"
        );
        Ok(())
    }
}

trait FakeConstruction: OrdinaryQuoteBackend + SignatureStatusReader + Send + Sync + 'static {
    const SIGNATURE: &'static str;
    fn new(probe: Arc<Probe>) -> Self;
    fn inspect(prepared: &Self::Prepared) -> Result<IntentView>;
}

fn probe(pipeline: &Backend, fault: Fault, halt: Option<&'static str>) -> Arc<Probe> {
    Arc::new(Probe {
        pool: pipeline.pool.clone(),
        events: pipeline.events.clone(),
        report: pipeline.report.clone(),
        fault,
        halt,
        reached: Arc::new(Notify::new()),
    })
}

async fn row(pool: &SqlitePool) -> (String, Option<String>, Option<Vec<u8>>) {
    sqlx::query_as("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims")
        .fetch_one(pool).await.unwrap()
}

async fn amounts<B: FakeConstruction>() {
    let (_, _, pipeline, _) = setup().await;
    let io = probe(&pipeline, Fault::None, None);
    let builder = B::new(io.clone());
    for amount in [1, 1_000_000, 9_007_199_254_740_993, u64::MAX] {
        let mut bound = pipeline.bound.clone();
        bound.quote.amount_lamports = amount;
        bound.quote.expected_output_units = u64::MAX;
        bound.quote.minimum_output_units = 15_679_732_462_653_119_872;
        let (prepared, _) = builder.prepare(&bound).await.unwrap();
        assert_eq!(B::inspect(&prepared).unwrap(), IntentView::expected(&bound));
        assert!(!builder.prepared_message(&prepared).unwrap().is_empty());
    }
    assert_eq!(io.count("sign"), 0);
    assert_eq!(io.count("broadcast"), 0);
}

#[tokio::test]
async fn backend_contract_raw_units_and_minimums_never_pass_through_float() {
    amounts::<RecordBackend>().await;
    amounts::<TapeBackend>().await;
}

#[tokio::test]
async fn backend_contract_construction_encodings_differ_but_retain_identical_intent() {
    let (_, _, pipeline, _) = setup().await;
    let record = RecordBackend::new(probe(&pipeline, Fault::None, None));
    let tape = TapeBackend::new(probe(&pipeline, Fault::None, None));
    let (record_plan, _) = record.prepare(&pipeline.bound).await.unwrap();
    let (tape_plan, _) = tape.prepare(&pipeline.bound).await.unwrap();
    assert_eq!(
        RecordBackend::inspect(&record_plan).unwrap(),
        TapeBackend::inspect(&tape_plan).unwrap()
    );
    assert_ne!(
        record.prepared_message(&record_plan).unwrap(),
        tape.prepared_message(&tape_plan).unwrap()
    );
}

async fn success<B: FakeConstruction>() -> Vec<&'static str> {
    let (f, owner, pipeline, journal) = setup().await;
    let io = probe(&pipeline, Fault::None, None);
    let builder = B::new(io.clone());
    let outcome = journaled(execute(&journal, &pipeline, &builder, &f.execution, &owner).await);
    let record = outcome.record.unwrap();
    assert_eq!(record.status, TradeStatus::Submitted);
    assert_eq!(record.signature.as_deref(), Some(B::SIGNATURE));
    assert_eq!(row(&io.pool).await.0, "submitted");
    let saved = journal
        .inner
        .capture_observation(&record.request_id)
        .await
        .unwrap();
    assert_eq!(saved.context.signature(), B::SIGNATURE);
    let original = serde_json::to_value(&record).unwrap();
    let again = journaled(execute(&journal, &pipeline, &builder, &f.execution, &owner).await);
    assert_eq!(
        serde_json::to_value(again.record.unwrap()).unwrap(),
        original
    );
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(io.count("prepare"), 1);
    assert_eq!(io.count("sign"), 1);
    assert_eq!(io.count("broadcast"), 1);
    assert_eq!(io.count("lookup_signature"), 1);
    let events = io.events.lock().unwrap().clone();
    events
}

#[tokio::test]
async fn backend_contract_independent_candidates_use_the_same_journal_order_and_no_retry() {
    let record = success::<RecordBackend>().await;
    let tape = success::<TapeBackend>().await;
    assert_eq!(record, tape);
    let position = |label| record.iter().position(|v| *v == label).unwrap();
    assert!(position("constructed") < position("persist_prepared"));
    assert!(position("authorize") < position("sign"));
    assert!(position("persist_signed") < position("read_signed"));
    assert!(position("read_signed") < position("broadcast"));
}

async fn reject<B: FakeConstruction>() {
    for mutation in 0..9 {
        let (f, owner, mut pipeline, journal) = setup().await;
        match mutation {
            0 => pipeline.bound.quote.amount_lamports += 1,
            1 => pipeline.bound.quote.input_mint = "not-wsol".into(),
            2 => pipeline.bound.wallet_pubkey = "another-wallet".into(),
            3 => pipeline.bound.quote.minimum_output_units = 0,
            4 => pipeline.bound.quote.minimum_output_units = 849,
            5 => pipeline.bound.quote.route[0].input_mint = "discontinuous".into(),
            6 => pipeline.bound.quote.route[0].label = "unsupported-protocol".into(),
            7 => pipeline.report.executable = false,
            8 => pipeline.bound.expires_at_ms = 1,
            _ => unreachable!(),
        }
        let io = probe(&pipeline, Fault::None, None);
        let builder = B::new(io.clone());
        let outcome = journaled(execute(&journal, &pipeline, &builder, &f.execution, &owner).await);
        assert_eq!(
            outcome.record.unwrap().status,
            TradeStatus::Failed,
            "{mutation}"
        );
        assert!(outcome.signature.is_none(), "{mutation}");
        assert_eq!(row(&io.pool).await, ("failed".into(), None, None));
        assert_eq!(io.count("sign"), 0, "{mutation}");
        assert_eq!(io.count("broadcast"), 0, "{mutation}");
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn backend_contract_amount_asset_wallet_capability_expiry_and_preflight_rejections() {
    reject::<RecordBackend>().await;
    reject::<TapeBackend>().await;
}

async fn storage_failures<B: FakeConstruction>() {
    for (fault, signing) in [
        ("persist_prepared", false),
        ("prepare_ack", false),
        ("corrupt_prepared", false),
        ("authorize", false),
        ("authorize_ack", false),
        ("persist_signed", true),
        ("signed_ack", true),
        ("different_readback", true),
    ] {
        let (f, owner, pipeline, mut journal) = setup().await;
        journal.fault = Some(fault);
        let io = probe(&pipeline, Fault::None, None);
        let builder = B::new(io.clone());
        let outcome = journaled(execute(&journal, &pipeline, &builder, &f.execution, &owner).await);
        assert_eq!(io.count("sign"), usize::from(signing), "{fault}");
        assert_eq!(io.count("broadcast"), 0, "{fault}");
        if signing {
            assert_eq!(outcome.signature.as_deref(), Some(B::SIGNATURE), "{fault}");
            assert!(outcome.record.is_none(), "{fault}");
        }
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld,
            "{fault}"
        );
    }
}

#[tokio::test]
async fn backend_contract_journal_failure_never_bypasses_original_authority() {
    storage_failures::<RecordBackend>().await;
    storage_failures::<TapeBackend>().await;
}

async fn uncertain<B: FakeConstruction>() {
    for fault in [
        Fault::Sign,
        Fault::SignedMessage,
        Fault::Send,
        Fault::Receipt,
    ] {
        let (f, owner, pipeline, journal) = setup().await;
        let io = probe(&pipeline, fault, None);
        let builder = B::new(io.clone());
        let outcome = journaled(execute(&journal, &pipeline, &builder, &f.execution, &owner).await);
        let sent = matches!(fault, Fault::Send | Fault::Receipt);
        assert_eq!(io.count("prepare"), 1);
        assert_eq!(io.count("sign"), 1);
        assert_eq!(io.count("broadcast"), usize::from(sent));
        let stored = row(&io.pool).await;
        assert_eq!(
            stored.0,
            if fault == Fault::Sign {
                "failed"
            } else if sent {
                "unknown"
            } else {
                "signing"
            }
        );
        assert_eq!(
            outcome.signature.as_deref(),
            (fault != Fault::Sign).then_some(B::SIGNATURE)
        );
        if sent {
            assert_eq!(outcome.record.unwrap().status, TradeStatus::Unknown);
            assert_eq!(stored.1.as_deref(), Some(B::SIGNATURE));
            assert!(!outcome
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("private-token"));
        } else if fault == Fault::Sign {
            // This fake definitively refuses before producing any receipt, matching the V1 oracle.
            assert_eq!(outcome.record.unwrap().status, TradeStatus::Failed);
            assert!(stored.1.is_none() && stored.2.is_none());
        } else {
            assert!(outcome.record.is_none());
        }
        let again = journaled(execute(&journal, &pipeline, &builder, &f.execution, &owner).await);
        assert!(again.record.is_none_or(|record| record.status
            == if fault == Fault::Sign {
                TradeStatus::Failed
            } else {
                TradeStatus::Unknown
            }));
        assert_eq!(row(&io.pool).await, stored);
        assert_eq!(io.count("prepare"), 1);
        assert_eq!(io.count("sign"), 1);
        assert_eq!(io.count("broadcast"), usize::from(sent));
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn backend_contract_unknown_and_altered_receipts_never_rebuild_or_resign() {
    uncertain::<RecordBackend>().await;
    uncertain::<TapeBackend>().await;
}

async fn cancelled<B: FakeConstruction>() {
    for halt in ["constructed", "persist_signed", "broadcast"] {
        let (f, owner, pipeline, mut journal) = setup().await;
        let execution = f.execution.clone();
        let io = probe(&pipeline, Fault::None, Some(halt));
        if halt == "persist_signed" {
            journal.after_signed = Some(io.reached.clone());
        }
        let builder = B::new(io.clone());
        let task = tokio::spawn(async move {
            execute(&journal, &pipeline, &builder, &f.execution, &owner).await
        });
        tokio::time::timeout(Duration::from_secs(3), io.reached.notified())
            .await
            .unwrap();
        task.abort();
        let Err(cancelled) = task.await else {
            panic!("blocked contract task must be cancelled")
        };
        assert!(cancelled.is_cancelled());
        let signed = halt != "constructed";
        let sent = usize::from(halt == "broadcast");
        let saved = row(&io.pool).await;
        assert_eq!(saved.0, if signed { "signed" } else { "preparing" });
        assert_eq!(saved.1.as_deref(), signed.then_some(B::SIGNATURE));
        assert_eq!(io.count("prepare"), 1);
        assert_eq!(io.count("sign"), usize::from(signed));
        assert_eq!(io.count("broadcast"), sent);
        let store = SqliteOrdinaryExecutionClaims::new(io.pool.clone());
        assert_eq!(
            store.claim(&execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        if signed {
            let captured = store
                .capture_observation(execution.reservation().request_id())
                .await
                .unwrap();
            assert_eq!(captured.context.signature(), B::SIGNATURE);
            let result = crate::application::ordinary_reconciliation::reconcile_captured(
                &store,
                &B::new(io.clone()),
                &captured.context,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(result.record.status, TradeStatus::Submitting);
            assert_eq!(row(&io.pool).await, saved);
            assert_eq!(io.count("broadcast"), sent);
            assert_eq!(io.count("sign"), 1);
        }
    }
}

#[tokio::test]
async fn backend_contract_cancellation_retains_the_original_claim_and_signed_evidence() {
    cancelled::<RecordBackend>().await;
    cancelled::<TapeBackend>().await;
}

async fn cannot_switch<First: FakeConstruction, Other: FakeConstruction>() {
    let (f, owner, pipeline, journal) = setup().await;
    let io = probe(&pipeline, Fault::Send, None);
    let first = First::new(io.clone());
    let original = journaled(execute(&journal, &pipeline, &first, &f.execution, &owner).await);
    assert_eq!(original.record.unwrap().status, TradeStatus::Unknown);
    let saved = row(&io.pool).await;
    let alternative_io = probe(&pipeline, Fault::None, None);
    let other = Other::new(alternative_io.clone());
    let repeated = journaled(execute(&journal, &pipeline, &other, &f.execution, &owner).await);
    assert_eq!(
        repeated.record.unwrap().signature.as_deref(),
        Some(First::SIGNATURE)
    );
    assert_eq!(row(&io.pool).await, saved);
    // Both probes share event observation; a second prepare/sign/send would increment these.
    assert_eq!(alternative_io.count("prepare"), 1);
    assert_eq!(alternative_io.count("sign"), 1);
    assert_eq!(alternative_io.count("broadcast"), 1);
}

#[tokio::test]
async fn backend_contract_switching_backend_after_unknown_cannot_create_another_transaction() {
    cannot_switch::<RecordBackend, TapeBackend>().await;
    cannot_switch::<TapeBackend, RecordBackend>().await;
}
