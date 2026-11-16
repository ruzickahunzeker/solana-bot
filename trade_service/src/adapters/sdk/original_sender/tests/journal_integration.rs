use super::*;
use crate::{
    adapters::{
        persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
        sdk::damm_sell_message::{candidate_fixture_for_wallet, DammSellCandidate},
    },
    application::{
        ordinary_claims::test_support::{admit_request_for_pubkey, fixture},
        ordinary_sdk_submission::{self, SdkSigningPermit},
        ports::{
            ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
            ordinary_sdk_submission::OrdinarySdkCandidateBackend,
        },
    },
    config::TradePersistenceMode,
    domain::{sdk_prepared_record::SdkPreparedRecord, trade::TradeStatus},
};
use sol_trade_sdk::common::TradeTransactionVersion;
use sqlx::{Row, SqlitePool};

struct JournalRpc {
    pool: SqlitePool,
    request_id: String,
    trace: Arc<Trace>,
    reply: String,
    block: bool,
}

impl RpcSender for JournalRpc {
    fn send<'a, 'f>(
        &'a self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ClientError>> + Send + 'f>>
    where
        'a: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            assert_eq!(request, RpcRequest::SendTransaction);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(params[0].as_str().unwrap())
                .unwrap();
            // This is the real RpcClient egress, after Application consumed its journal permit.
            let row = sqlx::query("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
                .bind(&self.request_id).fetch_one(&self.pool).await.unwrap();
            assert_eq!(row.get::<&str, _>("submission_stage"), "broadcasting");
            assert_eq!(row.get::<Vec<u8>, _>("signed_transaction"), bytes);
            let tx: VersionedTransaction = bincode::deserialize(&bytes).unwrap();
            assert_eq!(
                row.get::<&str, _>("signed_signature"),
                tx.signatures[0].to_string()
            );
            self.trace.rpc.lock().unwrap().push((request, params));
            self.trace.packets.lock().unwrap().push(bytes);
            if self.block {
                let _flag = DropFlag(self.trace.clone());
                std::future::pending::<()>().await;
            }
            Ok(json!(self.reply))
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://sdk-journal-egress".into()
    }
}

struct JournalBackend {
    execution: Arc<AdmittedTrade>,
    signer: Arc<Keypair>,
    keys: Arc<AtomicUsize>,
    sender: OriginalSdkSender<Observer>,
}
impl JournalBackend {
    fn context(&self) -> DammSellContext {
        DammSellContext {
            wallet: self.signer.pubkey(),
            provider: SwqosType::Default,
            version: TradeTransactionVersion::V0,
            seed_optimized: false,
        }
    }
}
impl OrdinarySdkCandidateBackend for JournalBackend {
    type Candidate = DammSellCandidate;
    type Signed = DammSignedCandidate;
    fn ensure_live_allowed(&self) -> Result<()> {
        Ok(())
    }
    fn metadata<'a>(&self, c: &'a DammSellCandidate) -> &'a SdkPreparedRecord {
        c.metadata()
    }
    fn unsigned_message(&self, c: &DammSellCandidate) -> Result<Vec<u8>> {
        Ok(c.message_bytes())
    }
    fn authorize(&self, e: &AdmittedTrade, c: &DammSellCandidate) -> Result<()> {
        c.authorize_live(e, self.context())
    }
    fn sign(
        &self,
        c: DammSellCandidate,
        permit: SdkSigningPermit<'_>,
    ) -> Result<DammSignedCandidate> {
        c.sign_original(permit, self.context(), || {
            self.keys.fetch_add(1, Ordering::SeqCst);
            Ok(self.signer.clone())
        })
    }
    fn signed_submission<'a>(&self, s: &'a DammSignedCandidate) -> &'a SignedSubmission {
        s.submission()
    }
    fn verify_signed(
        &self,
        s: &DammSignedCandidate,
        m: &SdkPreparedRecord,
        b: &[u8],
    ) -> Result<()> {
        crate::adapters::sdk::damm_sell_message::validate_signed_candidate(m, b, s.submission())
    }
    fn authorize_broadcast(&self, s: &DammSignedCandidate) -> Result<()> {
        s.recheck(&self.execution, self.context())
    }
    async fn broadcast(
        &self,
        artifact: &DammSignedCandidate,
        s: &SignedSubmission,
    ) -> Result<String> {
        self.sender
            .broadcast(&self.execution, self.context(), artifact, s, false, 0)
            .await
    }
}

struct IntegrationCase {
    fixture: crate::application::ordinary_claims::test_support::Fixture,
    owner: crate::application::ports::ordinary_claims::ExecutionOwner,
    candidate: DammSellCandidate,
    journal: SqliteOrdinaryExecutionClaims,
    backend: JournalBackend,
    expected: SignedSubmission,
    trace: Arc<Trace>,
}

async fn setup(mismatch: bool, block: bool) -> IntegrationCase {
    let state = fixture(TradePersistenceMode::Durable).await.state;
    let signer = Arc::new(Keypair::new());
    let candidate = candidate_fixture_for_wallet(signer.pubkey());
    let d = candidate.metadata().sell().unwrap().details();
    let address = |bytes| Pubkey::new_from_array(bytes).to_string();
    let request = serde_json::from_value(json!({
        "wallet_id":"fixture-wallet", "side":"sell", "pool":"meteora-damm-v2",
        "mint":address(d.mint), "quote_mint":address(d.quote_mint),
        "pool_id":address(d.pool.pool), "sell_percent":d.sell_percent,
        "slippage_bps":d.slippage_bps, "tip_sol":f64::from_bits(d.tip_sol_bits),
        "use_mev":d.use_mev, "mev_provider":d.requested_mev_provider,
        "simulate":false, "execution_method":"sdk", "idempotency_key":" journal-egress-key "
    }))
    .unwrap();
    let f = admit_request_for_pubkey(state, request, signer.pubkey().to_string()).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original SDK execution owner is required")
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let tx = VersionedTransaction::try_new(
        bincode::deserialize::<VersionedMessage>(&candidate.message_bytes()).unwrap(),
        &[signer.as_ref()],
    )
    .unwrap();
    let expected = SignedSubmission {
        signature: tx.signatures[0].to_string(),
        bytes: bincode::serialize(&tx).unwrap(),
    };
    let (mut sender, trace) =
        super::sender(SwqosType::Default, &expected, vec![None], false, false);
    let pool = f.state.persistence.pool_for_tests();
    sender.rpc = Arc::new(RpcClient::new_sender(
        JournalRpc {
            pool: pool.clone(),
            request_id: f.record.request_id.clone(),
            trace: trace.clone(),
            reply: if mismatch {
                Signature::new_unique().to_string()
            } else {
                expected.signature.clone()
            },
            block,
        },
        RpcClientConfig::default(),
    ));
    IntegrationCase {
        backend: JournalBackend {
            execution: f.execution.clone(),
            signer,
            keys: Arc::new(AtomicUsize::new(0)),
            sender,
        },
        journal: SqliteOrdinaryExecutionClaims::new(pool),
        fixture: f,
        owner,
        candidate,
        expected,
        trace,
    }
}

#[tokio::test]
async fn sdk_original_sender_journal_integration_signs_once_persists_before_real_rpc_egress() {
    let c = setup(false, false).await;
    let outcome = ordinary_sdk_submission::execute(
        &c.journal,
        &c.backend,
        &c.fixture.execution,
        c.owner,
        c.candidate,
    )
    .await;
    assert_eq!(outcome.record.unwrap().status, TradeStatus::Submitted);
    assert!(outcome.error.is_none());
    assert_eq!(
        outcome.signature.as_deref(),
        Some(c.expected.signature.as_str())
    );
    assert_eq!(*c.trace.packets.lock().unwrap(), vec![c.expected.bytes]);
    assert_eq!(c.backend.keys.load(Ordering::SeqCst), 1);
    assert_eq!(
        c.fixture
            .claims()
            .claim(&c.fixture.execution)
            .await
            .unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(c.trace.rpc.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn sdk_original_sender_journal_integration_wrong_rpc_receipt_keeps_original_signature_unknown(
) {
    let c = setup(true, false).await;
    let outcome = ordinary_sdk_submission::execute(
        &c.journal,
        &c.backend,
        &c.fixture.execution,
        c.owner,
        c.candidate,
    )
    .await;
    let record = outcome.record.unwrap();
    assert_eq!(record.status, TradeStatus::Unknown);
    assert_eq!(
        record.signature.as_deref(),
        Some(c.expected.signature.as_str())
    );
    assert!(record.error.is_some());
    assert_eq!(*c.trace.packets.lock().unwrap(), vec![c.expected.bytes]);
    assert_eq!(c.backend.keys.load(Ordering::SeqCst), 1);
    assert_eq!(
        c.fixture
            .claims()
            .claim(&c.fixture.execution)
            .await
            .unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(c.trace.rpc.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn sdk_original_sender_journal_integration_lost_receipt_write_does_not_resign_or_resend() {
    let c = setup(false, false).await;
    let pool = c.fixture.state.persistence.pool_for_tests();
    sqlx::raw_sql("CREATE TRIGGER reject_sdk_receipt BEFORE UPDATE OF submission_stage ON ordinary_execution_claims WHEN NEW.submission_stage='submitted' BEGIN SELECT RAISE(ABORT, 'synthetic receipt write failure'); END;")
        .execute(&pool).await.unwrap();
    let outcome = ordinary_sdk_submission::execute(
        &c.journal,
        &c.backend,
        &c.fixture.execution,
        c.owner,
        c.candidate,
    )
    .await;
    assert!(outcome.record.is_none());
    assert_eq!(
        outcome.signature.as_deref(),
        Some(c.expected.signature.as_str())
    );
    assert!(outcome
        .error
        .unwrap()
        .contains("synthetic receipt write failure"));
    let parent = c
        .fixture
        .state
        .persistence
        .get(&c.fixture.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent.status, TradeStatus::Submitting);
    assert_eq!(
        parent.signature.as_deref(),
        Some(c.expected.signature.as_str())
    );
    let row = sqlx::query("SELECT submission_stage,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&parent.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<&str, _>("submission_stage"), "broadcasting");
    assert_eq!(
        row.get::<Vec<u8>, _>("signed_transaction"),
        c.expected.bytes
    );
    assert_eq!(
        c.fixture
            .claims()
            .claim(&c.fixture.execution)
            .await
            .unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(c.backend.keys.load(Ordering::SeqCst), 1);
    assert_eq!(c.trace.rpc.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn sdk_original_sender_journal_integration_cancellation_retains_consumed_original_packet() {
    let c = setup(false, true).await;
    let trace = c.trace.clone();
    let keys = c.backend.keys.clone();
    let f = c.fixture;
    let task_execution = f.execution.clone();
    let task = tokio::spawn(async move {
        ordinary_sdk_submission::execute(
            &c.journal,
            &c.backend,
            &task_execution,
            c.owner,
            c.candidate,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while trace.packets.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert_eq!(keys.load(Ordering::SeqCst), 1);
    let pool = f.state.persistence.pool_for_tests();
    let row = sqlx::query("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&f.record.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<&str, _>("submission_stage"), "broadcasting");
    assert_eq!(row.get::<&str, _>("signed_signature"), c.expected.signature);
    assert_eq!(
        row.get::<Vec<u8>, _>("signed_transaction"),
        c.expected.bytes
    );
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(trace.rpc.lock().unwrap().len(), 1);
}
