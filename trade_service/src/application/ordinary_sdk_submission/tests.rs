use super::*;
use crate::{
    adapters::{
        persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
        sdk::damm_sell_message::{
            candidate_fixture_for_wallet, candidate_fixture_for_wallet_with_mev,
            validate_signed_candidate, DammSellCandidate, DammSellContext, DammSignedCandidate,
        },
    },
    application::{
        ordinary_claims::test_support::{admit_request_for_pubkey, fixture},
        ports::ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
        sdk_preparation::StoredSdkCandidate,
        trade_record::TradeRecordV1,
    },
    config::TradePersistenceMode,
    domain::{sdk_candidate::SdkCandidateMetadata, submission::SignedSubmission, trade::Address},
};
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use sqlx::{Row, SqlitePool};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::sync::Notify;

#[derive(Default)]
struct Control {
    events: Mutex<Vec<String>>,
    fail: Mutex<Option<&'static str>>,
    block: Mutex<Option<&'static str>>,
    tamper: Mutex<Option<&'static str>>,
    reached: Notify,
    release: Notify,
    live: AtomicUsize,
    authorize: AtomicUsize,
    broadcast_authorize: AtomicUsize,
    signs: AtomicUsize,
    keys: AtomicUsize,
    sends: AtomicUsize,
    response: AtomicUsize,
}
impl Control {
    fn event(&self, name: &str) -> Result<()> {
        self.events.lock().unwrap().push(name.into());
        ensure!(
            self.fail.lock().unwrap().as_deref() != Some(name),
            "synthetic {name} failure"
        );
        Ok(())
    }
    async fn gate(&self, name: &str) -> Result<()> {
        self.event(name)?;
        let blocked = self.block.lock().unwrap().as_deref() == Some(name);
        if blocked {
            self.reached.notify_one();
            self.release.notified().await;
        }
        Ok(())
    }
    fn changed(&self, label: &str) -> bool {
        self.tamper.lock().unwrap().as_deref() == Some(label)
    }
}

struct Journal {
    store: SqliteOrdinaryExecutionClaims,
    pool: SqlitePool,
    control: Arc<Control>,
}
impl OrdinarySdkSubmissionJournal for Journal {
    async fn prepare(
        &self,
        e: &AdmittedTrade,
        o: &ExecutionOwner,
        m: &SdkCandidateMetadata,
        b: &[u8],
    ) -> Result<()> {
        self.control.gate("prepare").await?;
        self.store.prepare_sdk_candidate(e, o, m, b).await?;
        self.control.gate("prepare-after").await
    }
    async fn read_prepared(
        &self,
        e: &AdmittedTrade,
        o: &ExecutionOwner,
    ) -> Result<StoredSdkCandidate> {
        self.control.gate("read-prepared").await?;
        let mut value = self.store.read_sdk_candidate(e, o).await?;
        if self.control.changed("prepared") {
            value.message[0] ^= 1;
        }
        Ok(value)
    }
    async fn begin_signing(
        &self,
        e: &AdmittedTrade,
        o: &ExecutionOwner,
        m: &SdkCandidateMetadata,
        b: &[u8],
    ) -> Result<()> {
        self.control.gate("begin-signing").await?;
        self.store.begin_sdk_signing(e, o, m, b).await?;
        self.control.gate("begin-signing-after").await
    }
    async fn record_signed(
        &self,
        e: &AdmittedTrade,
        o: &ExecutionOwner,
        s: &SignedSubmission,
    ) -> Result<()> {
        self.control.gate("record-signed").await?;
        self.store.record_sdk_signed(e, o, s).await?;
        self.control.gate("record-signed-after").await
    }
    async fn read_signed(&self, e: &AdmittedTrade, o: &ExecutionOwner) -> Result<SignedSubmission> {
        self.control.gate("read-signed").await?;
        let mut value = self.store.read_sdk_signed(e, o).await?;
        if self.control.changed("signed") {
            value.bytes[0] ^= 1;
        }
        Ok(value)
    }
    async fn take_broadcast(
        &self,
        e: &AdmittedTrade,
        o: &ExecutionOwner,
        s: &SignedSubmission,
    ) -> Result<SignedSubmission> {
        self.control.gate("take-broadcast").await?;
        let mut value = self.store.take_sdk_broadcast(e, o, s).await?;
        self.control.gate("take-broadcast-after").await?;
        if self.control.changed("taken") {
            value.bytes[0] ^= 1;
        }
        Ok(value)
    }
    async fn record_broadcast(
        &self,
        e: &AdmittedTrade,
        o: &ExecutionOwner,
        s: &SignedSubmission,
        ok: bool,
        error: Option<&str>,
    ) -> Result<()> {
        self.control.gate("record-broadcast").await?;
        self.store.record_sdk_broadcast(e, o, s, ok, error).await?;
        self.control.gate("record-broadcast-after").await
    }
    async fn fail_before_sign(
        &self,
        e: &AdmittedTrade,
        o: &ExecutionOwner,
        error: &str,
    ) -> Result<()> {
        self.control.gate("fail-before-sign").await?;
        self.store.fail_sdk_before_sign(e, o, error).await
    }
    async fn observe(&self, e: &AdmittedTrade, o: &ExecutionOwner) -> Result<TradeRecordV1> {
        self.control.gate("observe").await?;
        let mut value = self.store.observe_sdk_submission(e, o).await?;
        if self.control.changed("parent") {
            value.request_id = "other-parent".into();
        }
        if self.control.changed("failed-parent") {
            value.status = TradeStatus::Failed;
        }
        Ok(value)
    }
}

struct Engine {
    signer: Arc<Keypair>,
    control: Arc<Control>,
    pool: SqlitePool,
    request_id: String,
    execution: Arc<AdmittedTrade>,
    alternate_execution: Option<Arc<AdmittedTrade>>,
}
impl Engine {
    fn live(&self) -> Result<()> {
        let n = self.control.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.control.event(&format!("live-{n}"))
    }
    fn authorize(&self, e: &AdmittedTrade, m: &SdkCandidateMetadata, message: &[u8]) -> Result<()> {
        let n = self.control.authorize.fetch_add(1, Ordering::SeqCst) + 1;
        self.control.event(&format!("authorize-{n}"))?;
        validate_origin(e, m, message)
    }
    fn sign(&self, message: &[u8]) -> Result<SignedSubmission> {
        self.control.signs.fetch_add(1, Ordering::SeqCst);
        self.control.event("sign")?;
        self.control.event("acquire-signer")?;
        self.control.keys.fetch_add(1, Ordering::SeqCst);
        let signature = self.signer.sign_message(message);
        let transaction = VersionedTransaction {
            signatures: vec![signature],
            message: bincode::deserialize(message)?,
        };
        Ok(SignedSubmission {
            signature: signature.to_string(),
            bytes: bincode::serialize(&transaction)?,
        })
    }
    fn verify(&self, s: &SignedSubmission, m: &SdkCandidateMetadata, message: &[u8]) -> Result<()> {
        self.control.event("verify-signed")?;
        validate_signed_candidate(m, message, s)
    }
    fn authorize_broadcast(&self) -> Result<()> {
        let n = self
            .control
            .broadcast_authorize
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        self.control.event(&format!("authorize-broadcast-{n}"))
    }
    async fn send(&self, s: &SignedSubmission) -> Result<String> {
        self.control.sends.fetch_add(1, Ordering::SeqCst);
        self.control.gate("broadcast").await?;
        // The fake sender checks the actual isolated SQLite row at the send boundary.
        let row = sqlx::query("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&self.request_id).fetch_one(&self.pool).await?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "broadcasting"
                && row.try_get::<&str, _>("signed_signature")? == s.signature
                && row.try_get::<Vec<u8>, _>("signed_transaction")? == s.bytes,
            "fake sender did not receive the already-persisted original bytes"
        );
        match self.control.response.load(Ordering::SeqCst) {
            0 => Ok(s.signature.clone()),
            1 => Err(anyhow::anyhow!(
                "sendTransaction failed at https://rpc.invalid/private-token"
            )),
            2 => Ok("different-signature".into()),
            _ => unreachable!(),
        }
    }
}
struct First(Engine);
impl First {
    fn context(&self) -> DammSellContext {
        DammSellContext {
            wallet: self.0.signer.pubkey(),
            provider: sol_trade_sdk::swqos::SwqosType::Default,
            version: sol_trade_sdk::common::TradeTransactionVersion::V0,
            seed_optimized: false,
        }
    }
}
impl OrdinarySdkCandidateBackend for First {
    type Candidate = DammSellCandidate;
    type Signed = DammSignedCandidate;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.0.live()
    }
    fn metadata<'a>(&self, c: &'a DammSellCandidate) -> &'a SdkCandidateMetadata {
        c.metadata()
    }
    fn unsigned_message(&self, c: &DammSellCandidate) -> Result<Vec<u8>> {
        self.0.control.event("unsigned")?;
        Ok(c.message_bytes())
    }
    fn authorize(&self, e: &AdmittedTrade, c: &DammSellCandidate) -> Result<()> {
        self.0.authorize(e, c.metadata(), &c.message_bytes())?;
        c.authorize_live(e, self.context())
    }
    fn sign(
        &self,
        c: DammSellCandidate,
        permit: SdkSigningPermit<'_>,
    ) -> Result<DammSignedCandidate> {
        self.0.control.signs.fetch_add(1, Ordering::SeqCst);
        self.0.control.event("sign")?;
        let mut c = if self.0.control.changed("sign-candidate") {
            candidate_fixture_for_wallet(Keypair::new().pubkey())
        } else {
            c
        };
        if self.0.control.changed("expire-before-sign") {
            c.expire_holding_for_test();
        }
        let mut context = self.context();
        if self.0.control.changed("sign-context") {
            context.seed_optimized = true;
        }
        let mut signed = c.sign_original(permit, context, || {
            self.0.control.event("acquire-signer")?;
            self.0.control.keys.fetch_add(1, Ordering::SeqCst);
            if self.0.control.changed("sign-key") {
                return Ok(Arc::new(Keypair::new()));
            }
            Ok(self.0.signer.clone())
        })?;
        if self.0.control.changed("expire-after-sign") {
            signed.expire_holding_for_test();
        }
        if self.0.control.changed("sign-error-with-signature") {
            return Err(
                crate::application::trade_execution::UncertainSdkSubmission::new(
                    vec![signed.submission().signature.clone()],
                    "synthetic post-sign artifact failure".into(),
                )
                .into(),
            );
        }
        Ok(signed)
    }
    fn signed_submission<'a>(&self, s: &'a DammSignedCandidate) -> &'a SignedSubmission {
        s.submission()
    }
    fn verify_signed(
        &self,
        s: &DammSignedCandidate,
        m: &SdkCandidateMetadata,
        message: &[u8],
    ) -> Result<()> {
        self.0.verify(s.submission(), m, message)
    }
    fn authorize_broadcast(&self, s: &DammSignedCandidate) -> Result<()> {
        self.0.authorize_broadcast()?;
        s.recheck(
            self.0
                .alternate_execution
                .as_deref()
                .unwrap_or(self.0.execution.as_ref()),
            self.context(),
        )
    }
    async fn broadcast(&self, s: &SignedSubmission) -> Result<String> {
        self.0.send(s).await
    }
}

// The second backend uses a different project-only candidate representation.
struct PlainCandidate {
    metadata: SdkCandidateMetadata,
    message: Vec<u8>,
}
struct Second(Engine);
struct SecondSigned {
    candidate: PlainCandidate,
    signed: SignedSubmission,
}
impl OrdinarySdkCandidateBackend for Second {
    type Candidate = PlainCandidate;
    type Signed = SecondSigned;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.0.live()
    }
    fn metadata<'a>(&self, c: &'a PlainCandidate) -> &'a SdkCandidateMetadata {
        &c.metadata
    }
    fn unsigned_message(&self, c: &PlainCandidate) -> Result<Vec<u8>> {
        self.0.control.event("unsigned")?;
        Ok(c.message.clone())
    }
    fn authorize(&self, e: &AdmittedTrade, c: &PlainCandidate) -> Result<()> {
        self.0.authorize(e, &c.metadata, &c.message)
    }
    fn sign(&self, c: PlainCandidate, permit: SdkSigningPermit<'_>) -> Result<SecondSigned> {
        permit.validate(&c.metadata, &c.message)?;
        let signed = self.0.sign(&c.message)?;
        Ok(SecondSigned {
            candidate: c,
            signed,
        })
    }
    fn signed_submission<'a>(&self, s: &'a SecondSigned) -> &'a SignedSubmission {
        &s.signed
    }
    fn verify_signed(
        &self,
        s: &SecondSigned,
        m: &SdkCandidateMetadata,
        message: &[u8],
    ) -> Result<()> {
        self.0.verify(&s.signed, m, message)
    }
    fn authorize_broadcast(&self, s: &SecondSigned) -> Result<()> {
        self.0.authorize_broadcast()?;
        validate_signed_candidate(&s.candidate.metadata, &s.candidate.message, &s.signed)
    }
    async fn broadcast(&self, s: &SignedSubmission) -> Result<String> {
        self.0.send(s).await
    }
}

struct Harness {
    execution: Arc<AdmittedTrade>,
    owner: ExecutionOwner,
    candidate: DammSellCandidate,
    journal: Journal,
    backend: First,
}
struct Run {
    outcome: JournaledTradeOutcome,
    execution: Arc<AdmittedTrade>,
    journal: Journal,
    control: Arc<Control>,
}
impl Harness {
    async fn run(self) -> Run {
        let control = self.backend.0.control.clone();
        let outcome = execute(
            &self.journal,
            &self.backend,
            &self.execution,
            self.owner,
            self.candidate,
        )
        .await;
        Run {
            outcome,
            execution: self.execution,
            journal: self.journal,
            control,
        }
    }
}
async fn setup() -> Harness {
    setup_mev(Some(false), None).await
}

#[tokio::test]
async fn sdk_controlled_signing_enforces_requested_provider_with_existing_mode_precedence() {
    use crate::domain::trade::MevProvider::{Auto, BlockRazor, Default, Jito, Temporal};
    for (use_mev, requested, allowed) in [
        (Some(false), None, true),
        (Some(false), Some(Default), true),
        (Some(false), Some(Auto), true),
        (None, Some(Auto), true),
        (None, None, true),
        (Some(true), Some(BlockRazor), false),
        (Some(true), None, false),
        (Some(true), Some(Jito), false),
        (Some(true), Some(Temporal), false),
        (Some(true), Some(Auto), false),
    ] {
        let r = setup_mev(use_mev, requested).await.run().await;
        assert_eq!(
            r.control.keys.load(Ordering::SeqCst),
            usize::from(allowed),
            "{use_mev:?}/{requested:?}"
        );
        assert_eq!(r.control.sends.load(Ordering::SeqCst), usize::from(allowed));
        assert_eq!(
            stage(&r.journal, &r.execution).await,
            if allowed { "submitted" } else { "failed" }
        );
        if !allowed {
            assert!(r.outcome.error.unwrap().contains("MEV policy"));
        }
    }
    let state = crate::http_contract_tests::state().await;
    for (use_mev, requested) in [(false, Jito), (true, Default)] {
        let request: crate::models::TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id":"fixture-wallet", "side":"sell", "pool":"meteora-damm-v2",
            "mint":Address::from_bytes([2;32]).to_string(), "quote_mint":crate::domain::trade::WSOL_MINT,
            "pool_id":Address::from_bytes([3;32]).to_string(), "sell_percent":100,
            "simulate":false, "execution_method":"sdk", "idempotency_key":"invalid-mode",
            "use_mev":use_mev, "mev_provider":requested,
        })).unwrap();
        assert!(crate::validation::validate_trade(&request, &state.config).is_err());
    }
}

async fn setup_mev(
    use_mev: Option<bool>,
    requested: Option<crate::domain::trade::MevProvider>,
) -> Harness {
    let signer = Keypair::new();
    let candidate = candidate_fixture_for_wallet_with_mev(signer.pubkey(), use_mev, requested);
    let d = candidate.metadata().details();
    let state = fixture(TradePersistenceMode::Durable).await.state;
    let request = serde_json::from_value(serde_json::json!({
        "wallet_id":"fixture-wallet", "side":"sell", "pool":"meteora-damm-v2",
        "mint":Address::from_bytes(d.mint).to_string(), "quote_mint":Address::from_bytes(d.quote_mint).to_string(),
        "pool_id":Address::from_bytes(d.pool.pool).to_string(), "sell_percent":d.sell_percent,
        "slippage_bps":d.slippage_bps, "tip_sol":f64::from_bits(d.tip_sol_bits),
        "use_mev":d.use_mev, "mev_provider":d.requested_mev_provider,
        "simulate":false, "execution_method":"sdk", "idempotency_key":" application-sdk-literal "
    })).unwrap();
    let f = admit_request_for_pubkey(state, request, signer.pubkey().to_string()).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original owner required")
    };
    let record = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&record).await.unwrap();
    let pool = f.state.persistence.pool_for_tests();
    let control = Arc::new(Control::default());
    Harness {
        execution: f.execution.clone(),
        owner,
        candidate,
        journal: Journal {
            store: SqliteOrdinaryExecutionClaims::new(pool.clone()),
            pool: pool.clone(),
            control: control.clone(),
        },
        backend: First(Engine {
            signer: Arc::new(signer),
            control,
            pool,
            request_id: record.request_id,
            execution: f.execution,
            alternate_execution: None,
        }),
    }
}
async fn stage(j: &Journal, e: &AdmittedTrade) -> String {
    sqlx::query_scalar("SELECT submission_stage FROM ordinary_execution_claims WHERE request_id=?")
        .bind(e.reservation().request_id())
        .fetch_one(&j.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn sdk_application_submission_two_candidate_backends_obey_the_same_original_journal_order() {
    let mut traces = Vec::new();
    for second in [false, true] {
        let h = setup().await;
        let control = h.backend.0.control.clone();
        let outcome = if second {
            let message = h.candidate.message_bytes();
            let candidate = PlainCandidate {
                metadata: SdkCandidateMetadata::decode(
                    &h.candidate.metadata().encode(&message).unwrap(),
                    &message,
                )
                .unwrap(),
                message,
            };
            execute(
                &h.journal,
                &Second(h.backend.0),
                &h.execution,
                h.owner,
                candidate,
            )
            .await
        } else {
            execute(&h.journal, &h.backend, &h.execution, h.owner, h.candidate).await
        };
        let record = outcome.record.unwrap();
        assert_eq!(record.status, TradeStatus::Submitted);
        assert_eq!(record.signature, outcome.signature);
        assert!(record.error.is_none() && outcome.error.is_none());
        assert_eq!(control.signs.load(Ordering::SeqCst), 1);
        assert_eq!(control.sends.load(Ordering::SeqCst), 1);
        assert_eq!(stage(&h.journal, &h.execution).await, "submitted");
        assert_eq!(
            h.journal.store.claim(&h.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        traces.push(control.events.lock().unwrap().clone());
    }
    assert_eq!(traces[0], traces[1]);
    assert_eq!(
        traces[0],
        [
            "live-1",
            "unsigned",
            "authorize-1",
            "prepare",
            "prepare-after",
            "read-prepared",
            "live-2",
            "authorize-2",
            "begin-signing",
            "begin-signing-after",
            "live-3",
            "authorize-3",
            "sign",
            "acquire-signer",
            "verify-signed",
            "record-signed",
            "record-signed-after",
            "read-signed",
            "live-4",
            "authorize-broadcast-1",
            "take-broadcast",
            "take-broadcast-after",
            "live-5",
            "authorize-broadcast-2",
            "broadcast",
            "record-broadcast",
            "record-broadcast-after",
            "observe"
        ]
    );
}

#[tokio::test]
async fn sdk_application_submission_each_proven_presign_failure_stops_before_signer_and_sender() {
    for point in [
        "live-1",
        "unsigned",
        "authorize-1",
        "prepare",
        "prepare-after",
        "read-prepared",
        "live-2",
        "authorize-2",
    ] {
        let h = setup().await;
        *h.backend.0.control.fail.lock().unwrap() = Some(point);
        let r = h.run().await;
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 0, "{point}");
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 0, "{point}");
        assert_eq!(
            r.outcome.record.unwrap().status,
            TradeStatus::Failed,
            "{point}"
        );
        assert!(r.outcome.signature.is_none());
        assert_eq!(stage(&r.journal, &r.execution).await, "failed");
        assert_eq!(
            r.journal.store.claim(&r.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_application_submission_signing_cas_or_postawait_authorization_failure_is_never_presign_failed(
) {
    for point in [
        "begin-signing",
        "begin-signing-after",
        "live-3",
        "authorize-3",
        "sign",
    ] {
        let h = setup().await;
        *h.backend.0.control.fail.lock().unwrap() = Some(point);
        let r = h.run().await;
        assert!(r.outcome.record.is_none(), "{point}");
        assert!(r.outcome.signature.is_none(), "{point}");
        assert!(r.outcome.error.unwrap().contains(point));
        assert_eq!(
            r.control.signs.load(Ordering::SeqCst),
            usize::from(point == "sign")
        );
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
        assert!(!r
            .control
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|s| s == "fail-before-sign"));
        assert_eq!(
            stage(&r.journal, &r.execution).await,
            if point == "begin-signing" {
                "prepared"
            } else {
                "signing"
            }
        );
    }
}

#[tokio::test]
async fn sdk_application_submission_every_postsign_storage_gate_or_policy_failure_preserves_original_signature(
) {
    for point in [
        "verify-signed",
        "record-signed",
        "record-signed-after",
        "read-signed",
        "live-4",
        "authorize-broadcast-1",
        "take-broadcast",
        "take-broadcast-after",
        "live-5",
        "authorize-broadcast-2",
        "record-broadcast",
        "record-broadcast-after",
        "observe",
    ] {
        let h = setup().await;
        *h.backend.0.control.fail.lock().unwrap() = Some(point);
        let r = h.run().await;
        assert!(r.outcome.signature.is_some(), "{point}");
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 1, "{point}");
        assert_eq!(
            r.control.sends.load(Ordering::SeqCst),
            usize::from(["record-broadcast", "record-broadcast-after", "observe"].contains(&point)),
            "{point}"
        );
        assert!(!r
            .control
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|s| s == "fail-before-sign"));
        assert!(
            r.outcome
                .record
                .as_ref()
                .is_none_or(|v| v.status == TradeStatus::Submitted),
            "{point}"
        );
        assert_ne!(stage(&r.journal, &r.execution).await, "failed");
    }
}

#[tokio::test]
async fn sdk_application_submission_tampered_independent_readbacks_never_reach_sender() {
    for point in ["prepared", "signed", "taken"] {
        let h = setup().await;
        *h.backend.0.control.tamper.lock().unwrap() = Some(point);
        let r = h.run().await;
        assert_eq!(
            r.control.signs.load(Ordering::SeqCst),
            usize::from(point != "prepared")
        );
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
        assert_eq!(r.outcome.signature.is_some(), point != "prepared");
        assert_eq!(
            stage(&r.journal, &r.execution).await,
            match point {
                "prepared" => "failed",
                "signed" => "signed",
                "taken" => "broadcasting",
                _ => unreachable!(),
            }
        );
    }
}

#[tokio::test]
async fn sdk_application_submission_sender_failure_or_different_receipt_is_unknown_not_failed_or_retryable(
) {
    for response in [1, 2] {
        let h = setup().await;
        h.backend
            .0
            .control
            .response
            .store(response, Ordering::SeqCst);
        let r = h.run().await;
        let record = r.outcome.record.unwrap();
        assert_eq!(record.status, TradeStatus::Unknown);
        assert_eq!(record.signature, r.outcome.signature);
        assert!(record.signature.is_some());
        assert!(!record.error.unwrap().contains("private-token"));
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 1);
        assert_eq!(stage(&r.journal, &r.execution).await, "unknown");
        assert_eq!(
            r.journal.store.claim(&r.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_application_submission_cancellation_at_each_committed_boundary_never_continues_work() {
    for (point, expected_stage, signs, sends) in [
        ("prepare-after", "prepared", 0, 0),
        ("begin-signing-after", "signing", 0, 0),
        ("record-signed-after", "signed", 1, 0),
        ("take-broadcast-after", "broadcasting", 1, 0),
        ("record-broadcast", "broadcasting", 1, 1),
    ] {
        let h = setup().await;
        let control = h.backend.0.control.clone();
        *control.block.lock().unwrap() = Some(point);
        let pool = h.journal.pool.clone();
        let ex = h.execution.clone();
        let task = tokio::spawn(h.run());
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            control.reached.notified(),
        )
        .await
        .unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        let actual: String = sqlx::query_scalar(
            "SELECT submission_stage FROM ordinary_execution_claims WHERE request_id=?",
        )
        .bind(ex.reservation().request_id())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(actual, expected_stage);
        assert_eq!(control.signs.load(Ordering::SeqCst), signs);
        assert_eq!(control.sends.load(Ordering::SeqCst), sends);
        assert_eq!(
            SqliteOrdinaryExecutionClaims::new(pool)
                .claim(&ex)
                .await
                .unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        let before = control.events.lock().unwrap().clone();
        control.release.notify_one();
        tokio::task::yield_now().await;
        assert_eq!(*control.events.lock().unwrap(), before);
    }
}

#[tokio::test]
async fn sdk_application_submission_mismatched_or_failed_observation_never_overwrites_signed_outcome(
) {
    for point in ["parent", "failed-parent"] {
        let h = setup().await;
        *h.backend.0.control.tamper.lock().unwrap() = Some(point);
        let r = h.run().await;
        assert!(r.outcome.record.is_none());
        assert!(r.outcome.signature.is_some());
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 1);
        assert_eq!(stage(&r.journal, &r.execution).await, "submitted");
    }
}

#[tokio::test]
async fn sdk_application_submission_wrong_owner_or_candidate_stops_before_any_signature() {
    for wrong_owner in [true, false] {
        let mut h = setup().await;
        if wrong_owner {
            h.owner = ExecutionOwner::fixture(&h.execution);
        } else {
            h.candidate = candidate_fixture_for_wallet(Keypair::new().pubkey());
        }
        let r = h.run().await;
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 0);
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
        assert!(r.outcome.signature.is_none());
        assert!(!r
            .control
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|v| v == "begin-signing"));
    }
}

#[tokio::test]
async fn sdk_application_submission_journal_cannot_downgrade_any_postsigning_stage_to_failed() {
    for target in ["signing", "signed", "broadcasting", "submitted", "unknown"] {
        let h = setup().await;
        let message = h.candidate.message_bytes();
        h.journal
            .store
            .prepare_sdk_candidate(&h.execution, &h.owner, h.candidate.metadata(), &message)
            .await
            .unwrap();
        h.journal
            .store
            .begin_sdk_signing(&h.execution, &h.owner, h.candidate.metadata(), &message)
            .await
            .unwrap();
        let signed = h.backend.0.sign(&message).unwrap();
        if target != "signing" {
            h.journal
                .store
                .record_sdk_signed(&h.execution, &h.owner, &signed)
                .await
                .unwrap();
        }
        if ["broadcasting", "submitted", "unknown"].contains(&target) {
            h.journal
                .store
                .take_sdk_broadcast(&h.execution, &h.owner, &signed)
                .await
                .unwrap();
        }
        if ["submitted", "unknown"].contains(&target) {
            h.journal
                .store
                .record_sdk_broadcast(&h.execution, &h.owner, &signed, target == "submitted", None)
                .await
                .unwrap();
        }
        let before = h
            .journal
            .store
            .observe_sdk_submission(&h.execution, &h.owner)
            .await
            .unwrap();
        assert!(
            h.journal
                .store
                .fail_sdk_before_sign(&h.execution, &h.owner, "cannot downgrade")
                .await
                .is_err(),
            "{target}"
        );
        let after = h
            .journal
            .store
            .observe_sdk_submission(&h.execution, &h.owner)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        assert_eq!(stage(&h.journal, &h.execution).await, target);
        assert_eq!(
            h.journal.store.claim(&h.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        assert!(h
            .journal
            .store
            .read_sdk_candidate(&h.execution, &h.owner)
            .await
            .is_err());
    }
}

#[tokio::test]
async fn sdk_controlled_signing_rejects_changed_candidate_context_and_key_before_releasing_bytes() {
    for point in ["sign-candidate", "sign-context", "sign-key"] {
        let h = setup().await;
        *h.backend.0.control.tamper.lock().unwrap() = Some(point);
        let r = h.run().await;
        assert!(r.outcome.record.is_none(), "{point}");
        assert!(r.outcome.signature.is_none(), "{point}");
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
        assert_eq!(
            r.control.keys.load(Ordering::SeqCst),
            usize::from(point == "sign-key")
        );
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
        assert_eq!(stage(&r.journal, &r.execution).await, "signing");
    }
}

#[tokio::test]
async fn sdk_controlled_signing_preserves_real_signature_when_artifact_release_reports_an_error() {
    let h = setup().await;
    *h.backend.0.control.tamper.lock().unwrap() = Some("sign-error-with-signature");
    let r = h.run().await;
    assert!(r.outcome.signature.is_some());
    assert!(r.outcome.record.is_none());
    assert!(r
        .outcome
        .error
        .unwrap()
        .contains("post-sign artifact failure"));
    assert_eq!(r.control.keys.load(Ordering::SeqCst), 1);
    assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
    assert_eq!(stage(&r.journal, &r.execution).await, "signing");
}

#[tokio::test]
async fn sdk_controlled_signing_signed_artifact_cannot_be_rebound_to_another_original_order() {
    let mut h = setup().await;
    h.backend.0.alternate_execution = Some(setup().await.execution);
    let r = h.run().await;
    assert!(r.outcome.record.is_none());
    assert!(r.outcome.signature.is_some());
    assert!(r
        .outcome
        .error
        .unwrap()
        .contains("different original order"));
    assert_eq!(r.control.keys.load(Ordering::SeqCst), 1);
    assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
    assert_eq!(stage(&r.journal, &r.execution).await, "signed");
}

#[tokio::test]
async fn sdk_controlled_signing_original_evidence_expiration_is_not_renewed_by_signing_or_storage()
{
    for point in ["initial", "expire-before-sign", "expire-after-sign"] {
        let mut h = setup().await;
        if point == "initial" {
            h.candidate.expire_holding_for_test();
        } else {
            *h.backend.0.control.tamper.lock().unwrap() = Some(point);
        }
        let r = h.run().await;
        assert_eq!(
            r.control.keys.load(Ordering::SeqCst),
            usize::from(point == "expire-after-sign"),
            "{point}"
        );
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 0, "{point}");
        assert_eq!(
            r.outcome.signature.is_some(),
            point == "expire-after-sign",
            "{point}"
        );
        assert!(r.outcome.error.unwrap().contains("expired"), "{point}");
        assert_eq!(
            stage(&r.journal, &r.execution).await,
            match point {
                "initial" => "failed",
                "expire-before-sign" => "signing",
                "expire-after-sign" => "signed",
                _ => unreachable!(),
            }
        );
    }
}
