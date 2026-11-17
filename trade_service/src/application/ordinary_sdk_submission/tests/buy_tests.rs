use super::*;
use crate::adapters::sdk::{
    damm_probe::{
        test_support::buy_candidate_fixture, DammBuyCandidate, DammBuyContext,
        DammBuySignedCandidate,
    },
    prepared_candidate::validate_signed_candidate,
};
use sol_trade_sdk::swqos::SwqosType;

struct BuyBackend {
    engine: Engine,
    context: DammBuyContext,
}
impl OrdinarySdkCandidateBackend for BuyBackend {
    type Candidate = DammBuyCandidate;
    type Signed = DammBuySignedCandidate;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.engine.live()
    }
    fn metadata<'a>(&self, candidate: &'a Self::Candidate) -> &'a SdkPreparedRecord {
        candidate.metadata()
    }
    fn unsigned_message(&self, candidate: &Self::Candidate) -> Result<Vec<u8>> {
        self.engine.control.event("unsigned")?;
        Ok(candidate.message_bytes())
    }
    fn authorize(&self, e: &AdmittedTrade, candidate: &Self::Candidate) -> Result<()> {
        self.engine
            .authorize(e, candidate.metadata(), &candidate.message_bytes())?;
        candidate.authorize_live(e, self.context)
    }
    fn sign(
        &self,
        mut candidate: Self::Candidate,
        permit: SdkSigningPermit<'_>,
    ) -> Result<Self::Signed> {
        self.engine.control.signs.fetch_add(1, Ordering::SeqCst);
        self.engine.control.event("sign")?;
        if self.engine.control.changed("expire-before-sign") {
            candidate.expire_for_test();
        }
        let mut context = self.context;
        if self.engine.control.changed("sign-context") {
            context.seed_optimized = true;
        }
        let mut signed = candidate.sign_original(permit, context, || {
            self.engine.control.event("acquire-signer")?;
            self.engine.control.keys.fetch_add(1, Ordering::SeqCst);
            if self.engine.control.changed("sign-key") {
                return Ok(Arc::new(Keypair::new()));
            }
            Ok(self.engine.signer.clone())
        })?;
        if self.engine.control.changed("expire-after-sign") {
            signed.expire_for_test();
        }
        Ok(signed)
    }
    fn signed_submission<'a>(&self, signed: &'a Self::Signed) -> &'a SignedSubmission {
        signed.submission()
    }
    fn verify_signed(&self, signed: &Self::Signed, m: &SdkPreparedRecord, b: &[u8]) -> Result<()> {
        self.engine.control.event("verify-signed")?;
        validate_signed_candidate(m, b, signed.submission())
    }
    fn authorize_broadcast(&self, signed: &Self::Signed) -> Result<()> {
        self.engine.authorize_broadcast()?;
        signed.recheck(
            self.engine
                .alternate_execution
                .as_deref()
                .unwrap_or(self.engine.execution.as_ref()),
            self.context,
        )
    }
    async fn broadcast(&self, signed: &Self::Signed, bytes: &SignedSubmission) -> Result<String> {
        signed.check_submission(&self.engine.execution, self.context, bytes)?;
        self.engine.send(bytes).await
    }
}

struct BuyCase {
    journal: Journal,
    backend: BuyBackend,
    execution: Arc<AdmittedTrade>,
    owner: ExecutionOwner,
    candidate: DammBuyCandidate,
}
async fn setup_buy(native: bool) -> BuyCase {
    let signer = Arc::new(Keypair::new());
    let (candidate, context) =
        buy_candidate_fixture(signer.pubkey(), native, SwqosType::Default).await;
    let d = candidate.metadata().buy().unwrap().details();
    let mut request = serde_json::json!({
        "wallet_id":"fixture-wallet", "side":"buy", "pool":"meteora-damm-v2", "mint":Address::from_bytes(d.mint).to_string(), "quote_mint":Address::from_bytes(d.quote_mint).to_string(), "pool_id":d.pool_hint.map(|v| Address::from_bytes(v).to_string()),
        "simulate":false,"execution_method":"sdk","slippage_bps":d.slippage_bps,"tip_sol":f64::from_bits(d.tip_sol_bits),"use_mev":d.use_mev,"mev_provider":d.requested_mev_provider,"idempotency_key":" controlled-buy-original "
    });
    request[if native {
        "amount_lamports"
    } else {
        "amount_quote_units"
    }] = serde_json::json!(d.input_units);
    let f = admit_request_for_pubkey(
        fixture(TradePersistenceMode::Durable).await.state,
        serde_json::from_value(request).unwrap(),
        signer.pubkey().to_string(),
    )
    .await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original buyer owner required")
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let pool = f.state.persistence.pool_for_tests();
    let control = Arc::new(Control::default());
    BuyCase {
        journal: Journal {
            store: SqliteOrdinaryExecutionClaims::new(pool.clone()),
            pool: pool.clone(),
            control: control.clone(),
        },
        backend: BuyBackend {
            engine: Engine {
                signer,
                control,
                pool,
                request_id: parent.request_id,
                execution: f.execution.clone(),
                alternate_execution: None,
            },
            context,
        },
        execution: f.execution,
        owner,
        candidate,
    }
}

#[tokio::test]
async fn sdk_controlled_buy_original_owner_persists_and_reads_exact_signatures_before_one_send_for_both_assets(
) {
    for native in [false, true] {
        let h = setup_buy(native).await;
        let message = h.candidate.message_bytes();
        let outcome = execute(&h.journal, &h.backend, &h.execution, h.owner, h.candidate).await;
        assert_eq!(outcome.record.unwrap().status, TradeStatus::Submitted);
        assert!(outcome.error.is_none());
        assert_eq!(h.backend.engine.control.keys.load(Ordering::SeqCst), 1);
        assert_eq!(h.backend.engine.control.sends.load(Ordering::SeqCst), 1);
        let captured = h
            .journal
            .store
            .capture_observation(h.execution.reservation().request_id())
            .await
            .unwrap();
        assert_eq!(
            Some(captured.context.signature()),
            outcome.signature.as_deref()
        );
        let row = sqlx::query("SELECT unsigned_message,signed_transaction,sdk_prepared_json FROM ordinary_execution_claims WHERE request_id=?").bind(h.execution.reservation().request_id()).fetch_one(&h.journal.pool).await.unwrap();
        assert_eq!(row.get::<Vec<u8>, _>("unsigned_message"), message);
        let packet: VersionedTransaction =
            bincode::deserialize(&row.get::<Vec<u8>, _>("signed_transaction")).unwrap();
        assert_eq!(packet.message.serialize(), message);
        assert!(matches!(
            SdkPreparedRecord::decode(row.get("sdk_prepared_json"), &message).unwrap(),
            SdkPreparedRecord::DammBuy(_)
        ));
        assert_eq!(
            h.journal.store.claim(&h.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_controlled_buy_each_journal_or_authorization_failure_never_retries_a_signer_or_sender()
{
    for failure in [
        "prepare",
        "prepare-after",
        "read-prepared",
        "begin-signing",
        "begin-signing-after",
        "live-3",
        "authorize-3",
        "sign",
        "acquire-signer",
        "record-signed",
        "record-signed-after",
        "read-signed",
        "authorize-broadcast-1",
        "take-broadcast",
        "take-broadcast-after",
        "authorize-broadcast-2",
        "record-broadcast",
        "record-broadcast-after",
    ] {
        let h = setup_buy(true).await;
        *h.backend.engine.control.fail.lock().unwrap() = Some(failure);
        let result = execute(&h.journal, &h.backend, &h.execution, h.owner, h.candidate).await;
        assert!(result.error.is_some(), "{failure}");
        let signed = matches!(
            failure,
            "record-signed"
                | "record-signed-after"
                | "read-signed"
                | "authorize-broadcast-1"
                | "take-broadcast"
                | "take-broadcast-after"
                | "authorize-broadcast-2"
                | "record-broadcast"
                | "record-broadcast-after"
        );
        let sent = matches!(failure, "record-broadcast" | "record-broadcast-after");
        assert_eq!(
            h.backend.engine.control.keys.load(Ordering::SeqCst),
            usize::from(signed),
            "{failure}"
        );
        assert_eq!(
            h.backend.engine.control.sends.load(Ordering::SeqCst),
            usize::from(sent),
            "{failure}"
        );
        assert_eq!(result.signature.is_some(), signed, "{failure}");
        assert_eq!(
            h.journal.store.claim(&h.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld,
            "{failure}"
        );
    }
}

#[tokio::test]
async fn sdk_controlled_buy_expired_or_changed_context_rejects_before_key_and_postsign_expiry_keeps_signature(
) {
    for changed in [
        "expire-before-sign",
        "sign-context",
        "sign-key",
        "expire-after-sign",
    ] {
        let h = setup_buy(false).await;
        *h.backend.engine.control.tamper.lock().unwrap() = Some(changed);
        let result = execute(&h.journal, &h.backend, &h.execution, h.owner, h.candidate).await;
        assert!(result.error.is_some(), "{changed}");
        assert_eq!(
            h.backend.engine.control.sends.load(Ordering::SeqCst),
            0,
            "{changed}"
        );
        if changed == "expire-before-sign" || changed == "sign-context" {
            assert_eq!(h.backend.engine.control.keys.load(Ordering::SeqCst), 0);
        }
        if changed == "expire-after-sign" {
            assert!(result.signature.is_some());
        }
        assert_eq!(
            h.journal.store.claim(&h.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_controlled_buy_cancellation_at_original_stages_never_releases_owner_or_replays() {
    for point in [
        "prepare-after",
        "begin-signing-after",
        "record-signed-after",
        "broadcast",
    ] {
        let h = setup_buy(true).await;
        let control = h.backend.engine.control.clone();
        *control.block.lock().unwrap() = Some(point);
        let pool = h.journal.pool.clone();
        let execution = h.execution.clone();
        let task = tokio::spawn(async move {
            execute(&h.journal, &h.backend, &h.execution, h.owner, h.candidate).await
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            control.reached.notified(),
        )
        .await
        .unwrap();
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        let before = (
            control.keys.load(Ordering::SeqCst),
            control.sends.load(Ordering::SeqCst),
        );
        assert_eq!(
            SqliteOrdinaryExecutionClaims::new(pool)
                .claim(&execution)
                .await
                .unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        assert_eq!(
            (
                control.keys.load(Ordering::SeqCst),
                control.sends.load(Ordering::SeqCst)
            ),
            before
        );
    }
}

#[tokio::test]
async fn sdk_controlled_buy_changed_readbacks_or_foreign_owner_order_and_provider_never_send() {
    for change in ["prepared", "signed", "taken", "owner", "order", "provider"] {
        let mut h = setup_buy(true).await;
        match change {
            "owner" => h.owner = ExecutionOwner::fixture(&h.execution),
            "order" => h.backend.engine.alternate_execution = Some(setup_buy(true).await.execution),
            "provider" => h.backend.context.provider = SwqosType::Temporal,
            _ => *h.backend.engine.control.tamper.lock().unwrap() = Some(change),
        }
        let result = execute(&h.journal, &h.backend, &h.execution, h.owner, h.candidate).await;
        assert!(result.error.is_some(), "{change}");
        assert_eq!(
            h.backend.engine.control.sends.load(Ordering::SeqCst),
            0,
            "{change}"
        );
        let signed = matches!(change, "signed" | "taken" | "order");
        assert_eq!(
            h.backend.engine.control.keys.load(Ordering::SeqCst),
            usize::from(signed),
            "{change}"
        );
        assert_eq!(result.signature.is_some(), signed, "{change}");
        assert_eq!(
            h.journal.store.claim(&h.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_controlled_buy_original_signature_unknown_and_finality_are_never_rewritten_or_retried()
{
    for response in 1..=4 {
        let h = setup_buy(false).await;
        h.backend
            .engine
            .control
            .response
            .store(response, Ordering::SeqCst);
        let result = execute(&h.journal, &h.backend, &h.execution, h.owner, h.candidate).await;
        let record = result.record.unwrap();
        assert_eq!(
            record.status,
            match response {
                1 | 2 => TradeStatus::Unknown,
                3 => TradeStatus::Confirmed,
                _ => TradeStatus::Failed,
            }
        );
        assert_eq!(record.signature, result.signature);
        assert!(result.signature.is_some());
        assert_eq!(h.backend.engine.control.keys.load(Ordering::SeqCst), 1);
        assert_eq!(h.backend.engine.control.sends.load(Ordering::SeqCst), 1);
        assert_eq!(
            h.journal.store.claim(&h.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        let row=sqlx::query("SELECT signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?").bind(h.execution.reservation().request_id()).fetch_one(&h.journal.pool).await.unwrap();
        assert_eq!(
            Some(row.get::<String, _>("signed_signature")),
            result.signature
        );
        let tx: VersionedTransaction =
            bincode::deserialize(&row.get::<Vec<u8>, _>("signed_transaction")).unwrap();
        assert_eq!(Some(tx.signatures[0].to_string()), result.signature);
    }
}
