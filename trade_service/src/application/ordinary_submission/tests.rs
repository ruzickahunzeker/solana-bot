use super::*;
use crate::{
    adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
    application::{
        ordinary_claims::test_support::{fixture, Fixture},
        ports::{
            ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
            quote_execution::QuoteSubmissionBackend,
        },
    },
    config::TradePersistenceMode,
    domain::{
        quote::{QuoteEstimate, QuoteRouteLeg},
        submission::SignedSubmission,
        trade::TradeStatus,
    },
};
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

mod backend_contracts;
#[path = "reconciliation_tests.rs"]
mod reconciliation_tests;

const MESSAGE: &[u8] = &[3, 4, 5];
const SIGNATURE: &str = "fake-original-ordinary-signature";
type Events = Arc<Mutex<Vec<&'static str>>>;

struct Backend {
    pool: SqlitePool,
    events: Events,
    bound: BoundQuote,
    report: QuotePreflight,
    failure: Option<&'static str>,
    observation: Option<crate::domain::reconciliation::ChainObservation>,
}

impl Backend {
    fn event(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }
}

impl OrdinarySolBuyPipeline for Backend {
    type Request = crate::application::ports::ordinary_sol_buy::SolBuyIntent;
    fn map_request(&self, input: Self::Request) -> Self::Request {
        self.event("map");
        input
    }
    fn validate_request(&self, request: &Self::Request) -> Result<()> {
        self.event("validate");
        ensure!(
            request.amount_lamports == self.bound.quote.amount_lamports,
            "fake request amount mismatch"
        );
        Ok(())
    }
    async fn quote(&self, _: &Self::Request) -> Result<QuoteEstimate> {
        self.event("quote");
        let stage: String =
            sqlx::query_scalar("SELECT submission_stage FROM ordinary_execution_claims")
                .fetch_one(&self.pool)
                .await?;
        assert_eq!(stage, "preparing");
        if self.failure == Some("quote") {
            anyhow::bail!("synthetic quote failure");
        }
        Ok(self.bound.quote.clone())
    }
    fn bind(&self, _: &Self::Request, quote: QuoteEstimate) -> Result<BoundQuote> {
        self.event("bind");
        let mut bound = self.bound.clone();
        bound.quote = quote;
        Ok(bound)
    }
    async fn submit(
        &self,
        _: &BoundQuote,
        _: &QuoteSubmissionCommand,
    ) -> Result<QuoteSubmissionResult> {
        panic!("durable controller must not fall back to legacy submission")
    }
}

impl QuoteSubmissionBackend for Backend {
    type Prepared = Vec<u8>;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.event("policy");
        ensure!(
            self.failure != Some("policy")
                && !(self.failure == Some("policy_after") && count(self, "policy") > 1),
            "synthetic policy denial"
        );
        Ok(())
    }
    async fn prepare(&self, _: &BoundQuote) -> Result<(Self::Prepared, QuotePreflight)> {
        self.event("prepare");
        if self.failure == Some("prepare") {
            anyhow::bail!("synthetic prepare failure");
        }
        Ok((
            if self.failure == Some("oversized_unsigned") {
                vec![3; 1233]
            } else {
                MESSAGE.to_vec()
            },
            self.report.clone(),
        ))
    }
    fn sign(&self, prepared: Self::Prepared) -> Result<SignedSubmission> {
        self.event("sign");
        if self.failure == Some("sign") {
            anyhow::bail!("synthetic sign failure");
        }
        // Fixed non-executable bytes stand in for a signer receipt; no key or crypto signing.
        Ok(SignedSubmission {
            signature: SIGNATURE.into(),
            bytes: [vec![99], prepared].concat(),
        })
    }
    async fn broadcast(&self, signed: &SignedSubmission) -> Result<String> {
        self.event("broadcast");
        let saved: (String, Vec<u8>, String, String) = sqlx::query_as("SELECT c.signed_signature,c.signed_transaction,c.submission_stage,t.status FROM ordinary_execution_claims c JOIN trades t USING(request_id)").fetch_one(&self.pool).await?;
        assert_eq!(
            saved,
            (
                SIGNATURE.into(),
                signed.bytes.clone(),
                "signed".into(),
                "submitting".into()
            )
        );
        if self.failure == Some("broadcast") {
            anyhow::bail!("sendTransaction failed: https://rpc.example.invalid/private-token");
        }
        if self.failure == Some("receipt") {
            return Ok("different-receipt".into());
        }
        Ok(SIGNATURE.into())
    }
}

impl OrdinaryQuoteBackend for Backend {
    fn prepared_message(&self, prepared: &Self::Prepared) -> Result<Vec<u8>> {
        self.event("inspect_unsigned");
        Ok(prepared.clone())
    }
    fn signed_message(&self, signed: &SignedSubmission) -> Result<Vec<u8>> {
        self.event("inspect_signed");
        if self.failure == Some("changed_message") {
            return Ok(vec![8]);
        }
        Ok(signed.bytes[1..].to_vec())
    }
}

impl SignatureStatusReader for Backend {
    async fn lookup_signature(
        &self,
        signature: &str,
    ) -> Result<Option<crate::domain::reconciliation::ChainObservation>> {
        self.event("lookup_signature");
        assert_eq!(signature, SIGNATURE);
        if self.failure == Some("lookup") {
            anyhow::bail!("RPC failed: https://rpc.example.invalid/private-lookup-token");
        }
        Ok(self.observation.clone())
    }
}

struct Journal {
    inner: SqliteOrdinaryExecutionClaims,
    pool: SqlitePool,
    events: Events,
    fault: Option<&'static str>,
    after_signed: Option<Arc<Notify>>,
    after_final: Option<Arc<Notify>>,
}

impl Journal {
    fn event(&self, event: &'static str) -> Result<()> {
        self.events.lock().unwrap().push(event);
        ensure!(
            self.fault != Some(event),
            "synthetic journal {event} failure"
        );
        Ok(())
    }
}

impl OrdinarySubmissionJournal for Journal {
    async fn begin(&self, ex: &AdmittedTrade, owner: &ExecutionOwner) -> Result<()> {
        self.event("begin")?;
        self.inner.begin(ex, owner).await
    }
    async fn prepare(
        &self,
        ex: &AdmittedTrade,
        owner: &ExecutionOwner,
        bound: &BoundQuote,
        report: &QuotePreflight,
        message: &[u8],
    ) -> Result<()> {
        self.event("persist_prepared")?;
        self.inner
            .prepare(ex, owner, bound, report, message)
            .await?;
        match self.fault {
            Some("corrupt_prepared") => {
                sqlx::query("UPDATE ordinary_execution_claims SET unsigned_message=x'01'")
                    .execute(&self.pool)
                    .await?;
            }
            Some("corrupt_quote") => {
                sqlx::query("UPDATE ordinary_execution_claims SET bound_quote_json='{}'")
                    .execute(&self.pool)
                    .await?;
            }
            Some("corrupt_preflight") => {
                sqlx::query("UPDATE ordinary_execution_claims SET preflight_json='{}'")
                    .execute(&self.pool)
                    .await?;
            }
            Some("corrupt_unsigned_hash") => {
                sqlx::query("UPDATE ordinary_execution_claims SET unsigned_sha256='corrupt'")
                    .execute(&self.pool)
                    .await?;
            }
            Some("prepare_ack") => anyhow::bail!("synthetic lost prepared acknowledgement"),
            _ => {}
        }
        Ok(())
    }
    async fn authorize_sign(
        &self,
        ex: &AdmittedTrade,
        owner: &ExecutionOwner,
        bound: &BoundQuote,
        report: &QuotePreflight,
        message: &[u8],
    ) -> Result<()> {
        self.event("authorize")?;
        self.inner
            .authorize_sign(ex, owner, bound, report, message)
            .await?;
        ensure!(
            self.fault != Some("authorize_ack"),
            "synthetic lost authorization acknowledgement"
        );
        Ok(())
    }
    async fn record_signed(
        &self,
        ex: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
    ) -> Result<()> {
        self.event("persist_signed")?;
        self.inner.record_signed(ex, owner, signed).await?;
        if let Some(gate) = &self.after_signed {
            gate.notify_one();
            std::future::pending::<()>().await;
        }
        ensure!(
            self.fault != Some("signed_ack"),
            "synthetic lost signed acknowledgement"
        );
        Ok(())
    }
    async fn read_signed(
        &self,
        ex: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<SignedSubmission> {
        self.event("read_signed")?;
        if self.fault == Some("corrupt_signed") {
            sqlx::query("UPDATE ordinary_execution_claims SET signed_transaction=x'01'")
                .execute(&self.pool)
                .await?;
        }
        let mut signed = self.inner.read_signed(ex, owner).await?;
        if self.fault == Some("different_readback") {
            signed.bytes.push(7);
        }
        Ok(signed)
    }
    async fn record_broadcast(
        &self,
        ex: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
        submitted: bool,
        error: Option<&str>,
    ) -> Result<()> {
        self.event("persist_broadcast")?;
        self.inner
            .record_broadcast(ex, owner, signed, submitted, error)
            .await?;
        ensure!(
            self.fault != Some("broadcast_ack"),
            "synthetic lost broadcast acknowledgement"
        );
        Ok(())
    }
    async fn fail_before_sign(
        &self,
        ex: &AdmittedTrade,
        owner: &ExecutionOwner,
        error: &str,
    ) -> Result<()> {
        self.event("fail_before_sign")?;
        self.inner.fail_before_sign(ex, owner, error).await
    }
    async fn observe(
        &self,
        ex: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<crate::application::trade_record::TradeRecordV1> {
        self.event("observe")?;
        self.inner.observe(ex, owner).await
    }
}

impl OrdinaryReconciliationJournal for Journal {
    async fn capture_live_observation(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
    ) -> Result<crate::application::ordinary_observation_context::CapturedOrdinaryObservation> {
        self.inner
            .capture_live_observation(execution, owner, signature)
            .await
    }
    async fn observation_record(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
    ) -> Result<crate::application::trade_record::TradeRecordV1> {
        self.inner
            .observation_record(execution, owner, signature)
            .await
    }

    async fn record_final_observation(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
        observation: &crate::domain::reconciliation::ChainObservation,
    ) -> Result<crate::application::trade_record::TradeRecordV1> {
        let record = self
            .inner
            .record_final_observation(execution, owner, signature, observation)
            .await?;
        if let Some(entered) = &self.after_final {
            entered.notify_one();
            std::future::pending::<()>().await;
        }
        ensure!(
            self.fault != Some("final_ack"),
            "synthetic lost final observation acknowledgement"
        );
        Ok(record)
    }
}

impl RecoveredOrdinaryObservations for Journal {
    async fn capture_observation(
        &self,
        request_id: &str,
    ) -> Result<crate::application::ordinary_observation_context::CapturedOrdinaryObservation> {
        self.inner.capture_observation(request_id).await
    }
    async fn read_observation(
        &self,
        context: &crate::application::ordinary_observation_context::OrdinaryObservationContext,
    ) -> Result<crate::application::trade_record::TradeRecordV1> {
        self.inner.read_observation(context).await
    }
    async fn record_captured_finality(
        &self,
        context: &crate::application::ordinary_observation_context::OrdinaryObservationContext,
        observation: &crate::domain::reconciliation::ChainObservation,
    ) -> Result<crate::application::trade_record::TradeRecordV1> {
        let record = self
            .inner
            .record_captured_finality(context, observation)
            .await?;
        if let Some(entered) = &self.after_final {
            entered.notify_one();
            std::future::pending::<()>().await;
        }
        ensure!(
            self.fault != Some("final_ack"),
            "synthetic lost final observation acknowledgement"
        );
        Ok(record)
    }
}

async fn setup() -> (Fixture, ExecutionOwner, Backend, Journal) {
    let f = fixture(TradePersistenceMode::Durable).await;
    setup_fixture(f).await
}

async fn setup_fixture(f: Fixture) -> (Fixture, ExecutionOwner, Backend, Journal) {
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original owner required")
    };
    let building = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&building).await.unwrap();
    let pool = f.state.persistence.pool_for_tests();
    let events = Arc::new(Mutex::new(vec![]));
    let command = f.execution.command();
    let bound = BoundQuote {
        quote_id: "public-ordinary-quote".into(),
        wallet_id: f.record.wallet_id.clone(),
        wallet_pubkey: f.record.wallet_pubkey.clone(),
        quote_slippage_bps: u16::try_from(command.execution.slippage_bps).unwrap(),
        expires_at_ms: u64::MAX,
        quote: QuoteEstimate {
            input_mint: WSOL_MINT.into(),
            output_mint: command.route.mint.to_string(),
            amount_lamports: command.amount.buy_units().unwrap(),
            expected_output_units: 1000,
            minimum_output_units: 850,
            price_impact_percent: "0.01".into(),
            executable: false,
            route: vec![QuoteRouteLeg {
                label: "fake-amm".into(),
                input_mint: WSOL_MINT.into(),
                output_mint: command.route.mint.to_string(),
            }],
        },
    };
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    let mut report: QuotePreflight = serde_json::from_value(value["preflight"].clone()).unwrap();
    report.quote_id = bound.quote_id.clone();
    report.wallet_id = bound.wallet_id.clone();
    let backend = Backend {
        pool: pool.clone(),
        events: events.clone(),
        bound,
        report,
        failure: None,
        observation: None,
    };
    let journal = Journal {
        inner: SqliteOrdinaryExecutionClaims::new(pool.clone()),
        pool,
        events,
        fault: None,
        after_signed: None,
        after_final: None,
    };
    (f, owner, backend, journal)
}

fn journaled(outcome: SolBuyExecutionOutcome) -> JournaledSolBuyOutcome {
    match outcome {
        SolBuyExecutionOutcome::Journaled(value) => value,
        SolBuyExecutionOutcome::Legacy(_) => panic!("must not downgrade to legacy"),
    }
}
fn count(backend: &Backend, event: &str) -> usize {
    backend
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|value| **value == event)
        .count()
}

#[tokio::test]
async fn durable_controller_sends_only_persisted_readback_and_repeated_owner_is_read_only() {
    let (f, owner, backend, journal) = setup().await;
    let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
    let record = outcome.record.unwrap();
    assert_eq!(record.status, TradeStatus::Submitted);
    assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
    assert_eq!(
        *backend.events.lock().unwrap(),
        [
            "begin",
            "map",
            "validate",
            "quote",
            "bind",
            "policy",
            "prepare",
            "inspect_unsigned",
            "persist_prepared",
            "authorize",
            "policy",
            "sign",
            "inspect_signed",
            "persist_signed",
            "read_signed",
            "broadcast",
            "persist_broadcast",
            "lookup_signature",
            "observe"
        ]
    );
    let before = serde_json::to_value(&record).unwrap();
    let again = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
    assert_eq!(serde_json::to_value(again.record.unwrap()).unwrap(), before);
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn quote_prepare_policy_and_journal_gate_failures_cannot_reach_signing_or_broadcast() {
    for (backend_failure, journal_fault) in [
        (Some("quote"), None),
        (Some("prepare"), None),
        (Some("policy"), None),
        (Some("policy_after"), None),
        (Some("oversized_unsigned"), None),
        (None, Some("begin")),
        (None, Some("persist_prepared")),
        (None, Some("prepare_ack")),
        (None, Some("authorize")),
        (None, Some("authorize_ack")),
    ] {
        let (f, owner, mut backend, mut journal) = setup().await;
        backend.failure = backend_failure;
        journal.fault = journal_fault;
        let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        assert_eq!(count(&backend, "sign"), 0);
        assert_eq!(count(&backend, "broadcast"), 0);
        if journal_fault != Some("begin") {
            assert_eq!(outcome.record.unwrap().status, TradeStatus::Failed);
        }
    }
}

#[tokio::test]
async fn signed_write_or_readback_failure_keeps_original_signature_and_never_sends() {
    for fault in [
        "persist_signed",
        "signed_ack",
        "read_signed",
        "different_readback",
        "corrupt_signed",
    ] {
        let (f, owner, backend, mut journal) = setup().await;
        journal.fault = Some(fault);
        let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        assert!(outcome.record.is_none());
        assert_eq!(outcome.signature.as_deref(), Some(SIGNATURE));
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 0);
    }
}

#[tokio::test]
async fn broadcast_error_or_different_receipt_persists_unknown_with_original_signature() {
    for failure in ["broadcast", "receipt"] {
        let (f, owner, mut backend, journal) = setup().await;
        backend.failure = Some(failure);
        let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        let record = outcome.record.unwrap();
        assert_eq!(record.status, TradeStatus::Unknown);
        assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
        assert!(!record.error.unwrap().contains("private-token"));
        assert_eq!(count(&backend, "broadcast"), 1);
    }
}

#[tokio::test]
async fn signed_message_cannot_differ_from_authorized_unsigned_plan() {
    let (f, owner, mut backend, journal) = setup().await;
    backend.failure = Some("changed_message");
    let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
    assert_eq!(outcome.signature.as_deref(), Some(SIGNATURE));
    assert!(outcome.record.is_none());
    assert_eq!(count(&backend, "broadcast"), 0);
    let stored: Option<String> =
        sqlx::query_scalar("SELECT signed_signature FROM ordinary_execution_claims")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
    assert!(stored.is_none());
}

#[tokio::test]
async fn broadcast_write_ack_loss_does_not_downgrade_authoritative_submitted_snapshot() {
    let (f, owner, backend, mut journal) = setup().await;
    journal.fault = Some("broadcast_ack");
    let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
    assert_eq!(outcome.record.unwrap().status, TradeStatus::Submitted);
    assert_eq!(count(&backend, "broadcast"), 1);
}

#[tokio::test]
async fn cancellation_after_signed_commit_preserves_bytes_and_never_replays() {
    let (f, owner, backend, mut journal) = setup().await;
    let gate = Arc::new(Notify::new());
    journal.after_signed = Some(gate.clone());
    let backend = Arc::new(backend);
    let journal = Arc::new(journal);
    let owner = Arc::new(owner);
    let execution = f.execution.clone();
    let b = backend.clone();
    let j = journal.clone();
    let o = owner.clone();
    let task = tokio::spawn(async move {
        execute(j.as_ref(), b.as_ref(), b.as_ref(), &execution, &o).await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let stored = journal
        .inner
        .read_signed(&f.execution, &owner)
        .await
        .unwrap();
    assert_eq!(stored.signature, SIGNATURE);
    let again = journaled(
        execute(
            journal.as_ref(),
            backend.as_ref(),
            backend.as_ref(),
            &f.execution,
            &owner,
        )
        .await,
    );
    assert!(again.record.is_none());
    assert_eq!(again.signature.as_deref(), Some(SIGNATURE));
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 0);
}

#[tokio::test]
async fn every_quote_intent_dimension_and_preflight_permission_is_bound_before_signing() {
    for field in [
        "wallet",
        "pubkey",
        "mint",
        "input",
        "amount",
        "slippage",
        "min",
        "route",
        "expired",
        "preflight",
    ] {
        let (f, owner, mut backend, journal) = setup().await;
        match field {
            "wallet" => backend.bound.wallet_id = "other".into(),
            "pubkey" => backend.bound.wallet_pubkey = "other".into(),
            "mint" => backend.bound.quote.output_mint = "other".into(),
            "input" => backend.bound.quote.input_mint = "other".into(),
            "amount" => backend.bound.quote.amount_lamports += 1,
            "slippage" => backend.bound.quote_slippage_bps += 1,
            "min" => backend.bound.quote.minimum_output_units = 1,
            "route" => backend.bound.quote.route.clear(),
            "expired" => backend.bound.expires_at_ms = 0,
            "preflight" => backend.report.instruction_audit_passed = false,
            _ => unreachable!(),
        }
        let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        assert_eq!(
            outcome.record.unwrap().status,
            TradeStatus::Failed,
            "{field}"
        );
        assert_eq!(count(&backend, "sign"), 0, "{field}");
    }
}

#[tokio::test]
async fn owner_nonce_cannot_be_replaced_or_borrowed_from_another_reservation() {
    let (f, owner, backend, journal) = setup().await;
    let wrong = ExecutionOwner::fixture(&f.execution);
    assert!(!format!("{wrong:?}").contains(wrong.nonce()));
    assert!(
        journaled(execute(&journal, &backend, &backend, &f.execution, &wrong).await)
            .record
            .is_none()
    );
    assert_eq!(count(&backend, "quote"), 0);
    assert_eq!(count(&backend, "sign"), 0);
    let other = fixture(TradePersistenceMode::Durable).await;
    assert!(owner.ensure_binding(&other.execution).is_err());
    assert!(
        journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await)
            .record
            .is_some()
    );
    assert_eq!(count(&backend, "sign"), 1);
}

#[tokio::test]
async fn prepared_message_or_metadata_corruption_blocks_signing_without_rewriting_bad_evidence() {
    for fault in [
        "corrupt_prepared",
        "corrupt_quote",
        "corrupt_preflight",
        "corrupt_unsigned_hash",
    ] {
        let (f, owner, backend, mut journal) = setup().await;
        journal.fault = Some(fault);
        let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        assert!(outcome.record.is_none());
        assert_eq!(count(&backend, "sign"), 0);
        assert_eq!(count(&backend, "broadcast"), 0);
        let stage: String =
            sqlx::query_scalar("SELECT submission_stage FROM ordinary_execution_claims")
                .fetch_one(&journal.pool)
                .await
                .unwrap();
        assert_eq!(stage, "prepared");
    }
}

#[tokio::test]
async fn signer_failure_has_no_signature_and_a_final_pre_sign_failure_receipt() {
    let (f, owner, mut backend, journal) = setup().await;
    backend.failure = Some("sign");
    let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
    let record = outcome.record.unwrap();
    assert_eq!(record.status, TradeStatus::Failed);
    assert!(record.signature.is_none());
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 0);
}

#[tokio::test]
async fn final_write_or_observation_failure_preserves_signed_evidence_without_resend() {
    for fault in ["persist_broadcast", "observe"] {
        let (f, owner, backend, mut journal) = setup().await;
        journal.fault = Some(fault);
        let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
        assert!(outcome.record.is_none());
        assert_eq!(outcome.signature.as_deref(), Some(SIGNATURE));
        let stored = f
            .state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.signature.as_deref(), Some(SIGNATURE));
        assert_eq!(
            stored.status,
            if fault == "observe" {
                TradeStatus::Submitted
            } else {
                TradeStatus::Submitting
            }
        );
        assert_eq!(count(&backend, "sign"), 1);
        assert_eq!(count(&backend, "broadcast"), 1);
    }
}

#[tokio::test]
async fn concurrent_reuse_of_one_owner_only_prepares_signs_and_sends_once() {
    let (f, owner, backend, journal) = setup().await;
    let owner = Arc::new(owner);
    let backend = Arc::new(backend);
    let journal = Arc::new(journal);
    let mut tasks = vec![];
    for _ in 0..8 {
        let ex = f.execution.clone();
        let o = owner.clone();
        let b = backend.clone();
        let j = journal.clone();
        tasks.push(tokio::spawn(async move {
            execute(j.as_ref(), b.as_ref(), b.as_ref(), &ex, &o).await;
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(count(&backend, "quote"), 1);
    assert_eq!(count(&backend, "sign"), 1);
    assert_eq!(count(&backend, "broadcast"), 1);
    assert_eq!(
        f.state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TradeStatus::Submitted
    );
}

#[tokio::test]
async fn old_unowned_claim_rows_are_not_given_a_new_owner_or_reexecuted() {
    let (f, owner, backend, journal) = setup().await;
    sqlx::query("UPDATE ordinary_execution_claims SET owner_nonce=NULL")
        .execute(&journal.pool)
        .await
        .unwrap();
    journal.inner.initialize().await.unwrap();
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    let outcome = journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await);
    assert!(outcome.record.is_none());
    assert_eq!(count(&backend, "quote"), 0);
    assert_eq!(count(&backend, "sign"), 0);
    let nonce: Option<String> =
        sqlx::query_scalar("SELECT owner_nonce FROM ordinary_execution_claims")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
    assert!(nonce.is_none());
}

#[tokio::test]
async fn original_004_schema_migrates_idempotently_without_inventing_an_owner() {
    let (f, owner, backend, journal) = setup().await;
    let before = f.state.persistence.get(&f.record.request_id).await.unwrap();
    sqlx::raw_sql(
        "DROP TABLE ordinary_execution_claims; DROP TABLE ordinary_submission_migrations;",
    )
    .execute(&journal.pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/004_ordinary_execution_claims.sql"
    ))
    .execute(&journal.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms) VALUES(?,1,'ordinary-sol-buy',?,123)")
        .bind(&f.record.request_id)
        .bind(f.execution.reservation().request_hash())
        .execute(&journal.pool)
        .await
        .unwrap();
    for _ in 0..2 {
        journal.inner.initialize().await.unwrap();
        let evidence: (Option<String>, String, Option<Vec<u8>>, Option<String>) = sqlx::query_as(
            "SELECT owner_nonce,submission_stage,signed_transaction,signed_signature FROM ordinary_execution_claims",
        )
        .fetch_one(&journal.pool)
        .await
        .unwrap();
        assert_eq!(evidence, (None, "claimed".into(), None, None));
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        assert_eq!(
            serde_json::to_value(f.state.persistence.get(&f.record.request_id).await.unwrap())
                .unwrap(),
            serde_json::to_value(&before).unwrap(),
        );
    }
    assert!(
        journaled(execute(&journal, &backend, &backend, &f.execution, &owner).await)
            .record
            .is_none()
    );
    assert_eq!(count(&backend, "quote"), 0);
    assert_eq!(count(&backend, "sign"), 0);
    assert_eq!(count(&backend, "broadcast"), 0);
}

#[tokio::test]
async fn signed_file_reopen_retains_original_bytes_signature_and_unknown_parent_without_rebroadcast(
) {
    use crate::application::ordinary_claims::test_support::admit;
    use crate::config::TradePersistenceConfig;
    use crate::persistence::TradePersistence;
    let dir = std::env::temp_dir().join(format!("ordinary-signed-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 100,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let file = admit(state).await;
    let ExecutionClaim::Acquired(owner) = file.claims().claim(&file.execution).await.unwrap()
    else {
        panic!("new owner required")
    };
    let building = file
        .state
        .trades
        .update(&file.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    file.state.persistence.update(&building).await.unwrap();
    let (_, _, template, _) = setup().await;
    let pool = file.state.persistence.pool_for_tests();
    let inner = SqliteOrdinaryExecutionClaims::new(pool.clone());
    inner.begin(&file.execution, &owner).await.unwrap();
    inner
        .prepare(
            &file.execution,
            &owner,
            &template.bound,
            &template.report,
            MESSAGE,
        )
        .await
        .unwrap();
    inner
        .authorize_sign(
            &file.execution,
            &owner,
            &template.bound,
            &template.report,
            MESSAGE,
        )
        .await
        .unwrap();
    let signed = SignedSubmission {
        signature: SIGNATURE.into(),
        bytes: [vec![99], MESSAGE.to_vec()].concat(),
    };
    inner
        .record_signed(&file.execution, &owner, &signed)
        .await
        .unwrap();
    pool.close().await;
    let restored = TradePersistence::new(&config).await.unwrap();
    let restored_pool = restored.pool_for_tests();
    let journal = restored.ordinary_execution_claims().unwrap();
    let record = journal.observe(&file.execution, &owner).await.unwrap();
    assert_eq!(record.status, TradeStatus::Unknown);
    assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
    assert!(journal.read_signed(&file.execution, &owner).await.is_err());
    assert_eq!(
        journal.claim(&file.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    let raw: Vec<u8> =
        sqlx::query_scalar("SELECT signed_transaction FROM ordinary_execution_claims")
            .fetch_one(&restored_pool)
            .await
            .unwrap();
    assert_eq!(raw, signed.bytes);
    restored_pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
