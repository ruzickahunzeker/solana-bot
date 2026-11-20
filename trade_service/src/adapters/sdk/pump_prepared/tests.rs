use super::*;
use crate::{
    adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
    application::{
        ordinary_claims::test_support::Fixture,
        ordinary_sdk_submission,
        ports::{
            ordinary_claims::{ExecutionClaim, ExecutionOwner, OrdinaryExecutionClaimer},
            ordinary_execution::{JournaledSdkFuture, PreparedSdkExecution},
            ordinary_sdk_submission::OrdinarySdkCandidateBackend,
        },
    },
    domain::{preparation_balance::MAX_BALANCE_AGE, trade::TradeStatus},
};
use sol_trade_sdk::swqos::TradeType;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn fixture(side: TradeType, provider: SwqosType) -> Fixture {
    PumpNativeCandidateRecord::history_fixture(side, provider)
        .await
        .0
}

async fn prepare_candidate<'a>(
    f: &'a Fixture,
    side: TradeType,
    provider: SwqosType,
) -> (PumpPreparedCandidate<'a>, PumpPreparedContext) {
    let evidence = PumpNativeBankEvidence::original_fixture(&f.execution, side, provider)
        .await
        .unwrap();
    let record = evidence.record().unwrap();
    let d = record.details();
    let context = PumpPreparedContext {
        wallet: Pubkey::new_from_array(d.wallet),
        provider,
        version: TradeTransactionVersion::V0,
        seed_optimized: false,
        blockhash: Hash::new_from_array(d.recent_blockhash),
        tip_destination: d.tip_destination.map(Pubkey::new_from_array),
    };
    (
        PumpPreparedCandidate::new(evidence, context, &f.execution).unwrap(),
        context,
    )
}

struct ClaimEvidence<'a, 'b> {
    candidate: &'a PumpPreparedCandidate<'b>,
    context: PumpPreparedContext,
}
impl PreparedSdkExecution for ClaimEvidence<'_, '_> {
    fn metadata(&self) -> &SdkPreparedRecord {
        self.candidate.metadata()
    }
    fn message(&self) -> &[u8] {
        self.candidate.message()
    }
    fn validate_current(&self, e: &AdmittedTrade) -> Result<()> {
        self.candidate.authorize_live(e, self.context)
    }
    fn execute<'a>(
        self: Box<Self>,
        _: &'a AdmittedTrade,
        _: ExecutionOwner,
    ) -> JournaledSdkFuture<'a>
    where
        Self: 'a,
    {
        Box::pin(async { panic!("claim evidence double must not execute") })
    }
}

struct Backend<'a> {
    execution: &'a AdmittedTrade,
    context: PumpPreparedContext,
    authorize_calls: AtomicUsize,
    key_calls: AtomicUsize,
    broadcast_calls: AtomicUsize,
    fail_authorize_at: Option<usize>,
    fail_broadcast: bool,
    wrong_key: bool,
}
impl<'a> Backend<'a> {
    fn new(context: PumpPreparedContext, execution: &'a AdmittedTrade) -> Self {
        Self {
            execution,
            context,
            authorize_calls: AtomicUsize::new(0),
            key_calls: AtomicUsize::new(0),
            broadcast_calls: AtomicUsize::new(0),
            fail_authorize_at: None,
            fail_broadcast: false,
            wrong_key: false,
        }
    }
}
impl<'a> OrdinarySdkCandidateBackend for Backend<'a> {
    type Candidate = PumpPreparedCandidate<'a>;
    type Signed = PumpSignedCandidate<'a>;
    fn ensure_live_allowed(&self) -> Result<()> {
        Ok(())
    }
    fn metadata<'b>(&self, c: &'b Self::Candidate) -> &'b SdkPreparedRecord {
        c.metadata()
    }
    fn unsigned_message(&self, c: &Self::Candidate) -> Result<Vec<u8>> {
        Ok(c.message().to_vec())
    }
    fn authorize(&self, e: &AdmittedTrade, c: &Self::Candidate) -> Result<()> {
        let call = self.authorize_calls.fetch_add(1, Ordering::SeqCst) + 1;
        ensure!(
            self.fail_authorize_at != Some(call),
            "fixture current authorization rejected"
        );
        c.authorize_live(e, self.context)
    }
    fn sign(&self, c: Self::Candidate, permit: SdkSigningPermit<'_>) -> Result<Self::Signed> {
        c.sign_original(permit, self.context, || {
            self.key_calls.fetch_add(1, Ordering::SeqCst);
            // Existing public, unfunded fixture identity only, never a loaded wallet.
            Ok(Arc::new(Keypair::new_from_array(if self.wrong_key {
                [12; 32]
            } else {
                [11; 32]
            })))
        })
    }
    fn signed_submission<'b>(&self, signed: &'b Self::Signed) -> &'b SignedSubmission {
        signed.submission()
    }
    fn verify_signed(&self, s: &Self::Signed, m: &SdkPreparedRecord, message: &[u8]) -> Result<()> {
        validate_signed_candidate(m, message, s.submission())
    }
    fn authorize_broadcast(&self, s: &Self::Signed) -> Result<()> {
        s.recheck(self.execution, self.context)
    }
    async fn broadcast(&self, _: &Self::Signed, signed: &SignedSubmission) -> Result<String> {
        self.broadcast_calls.fetch_add(1, Ordering::SeqCst);
        ensure!(
            !self.fail_broadcast,
            "fixture transport acknowledgement lost"
        );
        Ok(signed.signature.clone())
    }
}

async fn owner(
    f: &Fixture,
    candidate: &PumpPreparedCandidate<'_>,
    context: PumpPreparedContext,
) -> ExecutionOwner {
    let claim = ClaimEvidence { candidate, context };
    let ExecutionClaim::Acquired(owner) = f
        .claims()
        .claim_prepared_sdk(&f.execution, &claim)
        .await
        .unwrap()
    else {
        panic!("original prepared owner required")
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    owner
}

#[tokio::test]
async fn pump_original_candidate_keeps_bank_admission_context_and_frozen_bytes() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let f = fixture(side, provider).await;
            let (candidate, context) = prepare_candidate(&f, side, provider).await;
            candidate.authorize_live(&f.execution, context).unwrap();
            let message = candidate.message().to_vec();
            let SdkPreparedRecord::PumpPrepared(record) = candidate.metadata() else {
                panic!()
            };
            assert!(record.is_prepared());
            assert_eq!(
                record.details().request_hash,
                f.execution.reservation().request_hash()
            );
            let alternate = f
                .execution
                .with_command_for_test(f.execution.command().clone());
            assert!(candidate.authorize_live(&alternate, context).is_err());
            for field in ["wallet", "provider", "version", "seed", "hash", "tip"] {
                let mut changed = context;
                match field {
                    "wallet" => changed.wallet = Pubkey::new_from_array([91; 32]),
                    "provider" => {
                        changed.provider = if provider == SwqosType::Default {
                            SwqosType::Temporal
                        } else {
                            SwqosType::Default
                        }
                    }
                    "version" => changed.version = TradeTransactionVersion::V1,
                    "seed" => changed.seed_optimized = true,
                    "hash" => changed.blockhash = Hash::new_from_array([91; 32]),
                    _ => changed.tip_destination = Some(Pubkey::new_from_array([91; 32])),
                }
                assert!(
                    candidate.authorize_live(&f.execution, changed).is_err(),
                    "{field}"
                );
            }
            assert_eq!(candidate.message(), message);
            assert!(f.claims().claim(&f.execution).await.is_err());
        }
    }
}

#[tokio::test]
async fn pump_original_controller_uses_one_claim_cas_exact_signature_and_original_journal() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let f = fixture(side, provider).await;
            let (candidate, context) = prepare_candidate(&f, side, provider).await;
            let message = candidate.message().to_vec();
            let owner = owner(&f, &candidate, context).await;
            let backend = Backend::new(context, &f.execution);
            let store = SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests());
            let outcome =
                ordinary_sdk_submission::execute(&store, &backend, &f.execution, owner, candidate)
                    .await;
            let record = outcome.record.unwrap();
            assert_eq!(record.status, TradeStatus::Submitted);
            assert_eq!(backend.key_calls.load(Ordering::SeqCst), 1);
            assert_eq!(backend.broadcast_calls.load(Ordering::SeqCst), 1);
            let (bytes, signature, stage): (Vec<u8>, String, String) = sqlx::query_as("SELECT signed_transaction,signed_signature,submission_stage FROM ordinary_execution_claims WHERE request_id=?")
                .bind(&record.request_id).fetch_one(&f.state.persistence.pool_for_tests()).await.unwrap();
            let tx: VersionedTransaction = bincode::deserialize(&bytes).unwrap();
            assert_eq!(tx.message.serialize(), message);
            assert_eq!(signature, record.signature.unwrap());
            assert_eq!(stage, "submitted");
            assert!(tx.signatures[0].verify(&context.wallet.to_bytes(), &message));
        }
    }
}

#[tokio::test]
async fn pump_original_controller_rechecks_after_readback_and_signing_cas_before_key_acquisition() {
    for failed_call in [2, 3] {
        let f = fixture(TradeType::Buy, SwqosType::Default).await;
        let (candidate, context) = prepare_candidate(&f, TradeType::Buy, SwqosType::Default).await;
        let owner = owner(&f, &candidate, context).await;
        let mut backend = Backend::new(context, &f.execution);
        backend.fail_authorize_at = Some(failed_call);
        let store = SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests());
        let outcome =
            ordinary_sdk_submission::execute(&store, &backend, &f.execution, owner, candidate)
                .await;
        assert!(outcome
            .error
            .unwrap()
            .contains("fixture current authorization rejected"));
        assert_eq!(backend.key_calls.load(Ordering::SeqCst), 0);
        assert_eq!(backend.broadcast_calls.load(Ordering::SeqCst), 0);
        let row = sqlx::query("SELECT signed_signature,signed_transaction,submission_stage FROM ordinary_execution_claims WHERE request_id=?").bind(&f.record.request_id).fetch_one(&f.state.persistence.pool_for_tests()).await.unwrap();
        use sqlx::Row;
        assert!(row
            .try_get::<Option<String>, _>("signed_signature")
            .unwrap()
            .is_none());
        assert!(row
            .try_get::<Option<Vec<u8>>, _>("signed_transaction")
            .unwrap()
            .is_none());
        assert_eq!(
            row.try_get::<String, _>("submission_stage").unwrap(),
            if failed_call == 2 {
                "failed"
            } else {
                "signing"
            }
        );
    }
}

#[tokio::test]
async fn pump_original_controller_lost_broadcast_ack_preserves_only_original_signed_packet() {
    let f = fixture(TradeType::Sell, SwqosType::Temporal).await;
    let (candidate, context) = prepare_candidate(&f, TradeType::Sell, SwqosType::Temporal).await;
    let owner = owner(&f, &candidate, context).await;
    let mut backend = Backend::new(context, &f.execution);
    backend.fail_broadcast = true;
    let store = SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests());
    let outcome =
        ordinary_sdk_submission::execute(&store, &backend, &f.execution, owner, candidate).await;
    let record = outcome.record.unwrap();
    assert_eq!(record.status, TradeStatus::Unknown);
    assert!(record.signature.is_some());
    assert_eq!(backend.key_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.broadcast_calls.load(Ordering::SeqCst), 1);
    let (fresh, _) = prepare_candidate(&f, TradeType::Sell, SwqosType::Temporal).await;
    assert!(matches!(
        f.claims()
            .claim_prepared_sdk(
                &f.execution,
                &ClaimEvidence {
                    candidate: &fresh,
                    context
                }
            )
            .await
            .unwrap(),
        ExecutionClaim::AlreadyHeld
    ));
}

#[tokio::test]
async fn pump_original_candidate_expiry_survives_nominal_roundtrip_and_never_acquires_key() {
    let f = fixture(TradeType::Buy, SwqosType::Default).await;
    let (candidate, context) = prepare_candidate(&f, TradeType::Buy, SwqosType::Default).await;
    let json = candidate.metadata().encode(candidate.message()).unwrap();
    let message = candidate.message().to_vec();
    let owner = owner(&f, &candidate, context).await;
    tokio::time::sleep(MAX_BALANCE_AGE + std::time::Duration::from_millis(20)).await;
    SdkPreparedRecord::decode(&json, &message).unwrap();
    assert!(candidate.authorize_live(&f.execution, context).is_err());
    let store = SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests());
    let backend = Backend::new(context, &f.execution);
    let outcome =
        ordinary_sdk_submission::execute(&store, &backend, &f.execution, owner, candidate).await;
    assert_eq!(outcome.record.unwrap().status, TradeStatus::Failed);
    assert_eq!(backend.key_calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.broadcast_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pump_original_signer_rejects_different_key_without_packet_or_broadcast() {
    let f = fixture(TradeType::Buy, SwqosType::Default).await;
    let (candidate, context) = prepare_candidate(&f, TradeType::Buy, SwqosType::Default).await;
    let owner = owner(&f, &candidate, context).await;
    let mut backend = Backend::new(context, &f.execution);
    backend.wrong_key = true;
    let store = SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests());
    let outcome =
        ordinary_sdk_submission::execute(&store, &backend, &f.execution, owner, candidate).await;
    assert!(outcome.signature.is_none());
    assert!(outcome.error.unwrap().contains("signer differs"));
    assert_eq!(backend.key_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.broadcast_calls.load(Ordering::SeqCst), 0);
    let (signature, bytes): (Option<String>, Option<Vec<u8>>) = sqlx::query_as("SELECT signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&f.record.request_id).fetch_one(&f.state.persistence.pool_for_tests()).await.unwrap();
    assert!(signature.is_none() && bytes.is_none());
}

#[tokio::test]
async fn pump_original_file_reopen_only_recovers_original_signature_observation_not_execution() {
    use crate::{
        application::{
            ordinary_claims::test_support::admit_request_for_pubkey,
            ports::ordinary_reconciliation::RecoveredOrdinaryObservations,
        },
        config::{TradePersistenceConfig, TradePersistenceMode},
        domain::reconciliation::{ChainCommitment, ChainObservation},
        persistence::TradePersistence,
    };
    for side in [TradeType::Buy, TradeType::Sell] {
        let dir =
            std::env::temp_dir().join(format!("pump-original-observe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let config = TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: format!("sqlite://{}", dir.join("public-fixture.sqlite").display()),
            busy_timeout_ms: 2000,
        };
        let (id, signature, immutable) = {
            let seed = fixture(side, SwqosType::Temporal).await;
            let json: String =
                sqlx::query_scalar("SELECT request_json FROM trades WHERE request_id=?")
                    .bind(&seed.record.request_id)
                    .fetch_one(&seed.state.persistence.pool_for_tests())
                    .await
                    .unwrap();
            let request = serde_json::from_str(&json).unwrap();
            let mut state = crate::http_contract_tests::state().await;
            state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
            let f =
                admit_request_for_pubkey(state, request, seed.record.wallet_pubkey.clone()).await;
            drop(seed);
            let (candidate, context) = prepare_candidate(&f, side, SwqosType::Temporal).await;
            let owner = owner(&f, &candidate, context).await;
            let backend = Backend::new(context, &f.execution);
            let pool = f.state.persistence.pool_for_tests();
            let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
            let outcome =
                ordinary_sdk_submission::execute(&store, &backend, &f.execution, owner, candidate)
                    .await;
            let signature = outcome.record.unwrap().signature.unwrap();
            let immutable: (String, String, Vec<u8>) = sqlx::query_as("SELECT owner_nonce,sdk_prepared_json,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
                .bind(&f.record.request_id).fetch_one(&pool).await.unwrap();
            pool.close().await;
            (f.record.request_id.clone(), signature, immutable)
        };
        let restored = TradePersistence::new(&config).await.unwrap();
        let pool = restored.pool_for_tests();
        sqlx::raw_sql("CREATE TRIGGER no_pump_recovery_owner BEFORE UPDATE OF owner_nonce ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no restored owner'); END; CREATE TRIGGER no_pump_recovery_artifact BEFORE UPDATE OF signed_signature,signed_transaction,unsigned_message ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no restored artifacts'); END; CREATE TRIGGER no_pump_recovery_claim BEFORE INSERT ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no restored claim'); END;")
            .execute(&pool).await.unwrap();
        let store = restored.ordinary_execution_claims().unwrap();
        let captured = store.capture_observation(&id).await.unwrap();
        assert_eq!(
            captured.record.signature.as_deref(),
            Some(signature.as_str())
        );
        let finalized = store
            .record_captured_finality(
                &captured.context,
                &ChainObservation {
                    signature: signature.clone(),
                    commitment: ChainCommitment::Finalized,
                    slot: 200,
                    error: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(finalized.status, TradeStatus::Confirmed);
        let after: (String, String, Vec<u8>) = sqlx::query_as("SELECT owner_nonce,sdk_prepared_json,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&id).fetch_one(&pool).await.unwrap();
        assert_eq!(after, immutable);
        let mut json: serde_json::Value = serde_json::from_str(
            &sqlx::query_scalar::<_, String>("SELECT request_json FROM trades WHERE request_id=?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap(),
        )
        .unwrap();
        json["slippage_bps"] = serde_json::json!(1234);
        sqlx::query("UPDATE trades SET request_json=? WHERE request_id=?")
            .bind(json.to_string())
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(store.capture_observation(&id).await.is_err());
        pool.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
