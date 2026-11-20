use super::*;
use crate::{
    application::{
        ordinary_claims::test_support::NominalPumpPreparation,
        ports::{
            ordinary_execution::{
                JournaledSdkFuture, OrdinaryExecutionWallet, PreparedSdkExecution,
                PreparedSdkFuture, TradeExecutionFuture,
            },
            ordinary_sol_buy::OrdinarySolBuyWallet,
        },
    },
    domain::{
        blockhash::Blockhash, execution_policy::BroadcastPolicy,
        pump_native_candidate::PumpNativeCandidateRecord, sdk_prepared_record::SdkPreparedRecord,
    },
};
use sol_trade_sdk::swqos::{SwqosType, TradeType};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Preparation {
    Ready,
    Unsupported,
    Failure,
    Stale,
    Pending,
    SignedUnknown,
}

#[derive(Clone)]
struct Backend {
    trace: Arc<Mutex<Vec<&'static str>>>,
    preparation: Preparation,
    json: String,
    message: Vec<u8>,
    journal: Arc<crate::persistence::TradePersistence>,
}

impl Backend {
    fn note(&self, step: &'static str) {
        self.trace.lock().unwrap().push(step);
    }
}

impl OrdinaryExecutionBackend for Backend {
    fn supports_prepared_sdk(&self) -> bool {
        true
    }
    fn resolve(&self, _: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        self.note("resolve");
        Some(Box::new(self.clone()))
    }
}

impl OrdinaryExecutionWallet for Backend {
    fn prepare_original_sdk<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        context: SdkExecutionContext,
    ) -> PreparedSdkFuture<'a> {
        Box::pin(async move {
            self.note("prepare");
            assert_eq!(context.blockhash.to_bytes(), [7; 32]);
            assert_eq!(
                self.journal
                    .get(execution.reservation().request_id())
                    .await?
                    .unwrap()
                    .status,
                TradeStatus::Accepted
            );
            match self.preparation {
                Preparation::Unsupported => Ok(None),
                Preparation::Failure => anyhow::bail!("fixture bank failed"),
                Preparation::Pending => std::future::pending().await,
                _ => Ok(Some(Box::new(Prepared {
                    backend: self.clone(),
                    nominal: NominalPumpPreparation {
                        metadata: SdkPreparedRecord::decode(&self.json, &self.message)?,
                        message: self.message.clone(),
                    },
                }) as Box<dyn PreparedSdkExecution>)),
            }
        })
    }
    fn sdk<'a>(&'a self, _: &'a TradeCommand, _: SdkExecutionContext) -> TradeExecutionFuture<'a> {
        Box::pin(async move {
            assert_eq!(self.preparation as u8, Preparation::Unsupported as u8);
            self.note("legacy-sdk");
            Ok(ExecuteResult {
                success: true,
                signature: Some("legacy-fixture".into()),
                error: None,
            })
        })
    }
    fn pumpapi<'a>(&'a self, _: &'a TradeCommand, _: bool) -> TradeExecutionFuture<'a> {
        Box::pin(async { panic!("native Pump runtime cannot use PumpAPI") })
    }
}

impl OrdinarySolBuyBackend for Backend {
    fn resolve(&self, _: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        panic!("native Pump runtime cannot use SOL aggregator")
    }
}

struct Prepared {
    backend: Backend,
    nominal: NominalPumpPreparation,
}
impl PreparedSdkExecution for Prepared {
    fn metadata(&self) -> &SdkPreparedRecord {
        &self.nominal.metadata
    }
    fn message(&self) -> &[u8] {
        &self.nominal.message
    }
    fn validate_current(&self, execution: &AdmittedTrade) -> anyhow::Result<()> {
        self.backend.note("validate");
        anyhow::ensure!(
            self.backend.preparation != Preparation::Stale,
            "fixture bank expired"
        );
        self.nominal.validate_current(execution)
    }
    fn execute<'a>(
        self: Box<Self>,
        execution: &'a AdmittedTrade,
        owner: ExecutionOwner,
    ) -> JournaledSdkFuture<'a>
    where
        Self: 'a,
    {
        Box::pin(async move {
            owner.ensure_binding(execution).unwrap();
            self.backend.note("execute-prepared");
            let mut parent = self
                .backend
                .journal
                .get(execution.reservation().request_id())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(parent.status, TradeStatus::BuildingTx);
            let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ordinary_execution_claims WHERE request_id=? AND owner_nonce=?")
                .bind(execution.reservation().request_id()).bind(owner.nonce())
                .fetch_one(&self.backend.journal.pool_for_tests()).await.unwrap();
            assert_eq!(held, 1);
            if self.backend.preparation == Preparation::SignedUnknown {
                return JournaledSolBuyOutcome {
                    record: None,
                    signature: Some("fixture-uncertain-signature".into()),
                    error: Some("fixture lost acknowledgement".into()),
                };
            }
            // Runtime port oracle only: no signer, packet, RPC or sender exists in this double.
            parent.status = TradeStatus::Submitted;
            parent.signature = Some("fixture-original-result".into());
            self.backend.journal.update(&parent).await.unwrap();
            JournaledSolBuyOutcome {
                record: Some(parent),
                signature: None,
                error: None,
            }
        })
    }
}

struct HashRead;
impl CachedBlockhashRead for HashRead {
    fn fresh_hash(&self, _: u64) -> Option<Blockhash> {
        Some(Blockhash::from_bytes([7; 32]))
    }
}
struct Params;
impl ParamCacheWriter for Params {
    fn record(&self, _: &PoolRouteHint) -> String {
        "fixture-pump-key".into()
    }
}

#[derive(Clone, Copy)]
enum ClaimResponse {
    Real,
    Held,
    Uncertain,
}
struct Claims {
    backend: Backend,
    real: crate::adapters::persistence::ordinary_claims::RuntimeExecutionClaims,
    response: ClaimResponse,
}
impl OrdinaryExecutionClaimer for Claims {
    async fn claim(&self, _: &AdmittedTrade) -> anyhow::Result<ExecutionClaim> {
        panic!("Pump must not widen the original Pool-only SDK selector")
    }
    async fn claim_prepared_sdk(
        &self,
        execution: &AdmittedTrade,
        prepared: &dyn PreparedSdkExecution,
    ) -> anyhow::Result<ExecutionClaim> {
        self.backend.note("claim-prepared");
        match self.response {
            ClaimResponse::Real => self.real.claim_prepared_sdk(execution, prepared).await,
            ClaimResponse::Held => Ok(ExecutionClaim::AlreadyHeld),
            ClaimResponse::Uncertain => anyhow::bail!("fixture claim acknowledgement unavailable"),
        }
    }
}

async fn exercise(
    preparation: Preparation,
    response: ClaimResponse,
) -> (
    crate::application::ordinary_claims::test_support::Fixture,
    Backend,
) {
    let (f, record, message) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let nominal = NominalPumpPreparation::new(record, message);
    let backend = Backend {
        trace: Arc::default(),
        preparation,
        json: nominal.metadata.encode(&nominal.message).unwrap(),
        message: nominal.message,
        journal: f.state.persistence.clone(),
    };
    let claims = Claims {
        backend: backend.clone(),
        real: f.claims(),
        response,
    };
    let service = OrdinaryExecutionService::new(
        f.state.persistence.as_ref(),
        f.state.trades.as_ref(),
        &HashRead,
        &backend,
        &backend,
        &Params,
        OrdinaryExecutionSettings {
            broadcast: BroadcastPolicy::Enabled,
            blockhash_max_age_ms: 17,
            wait_tx_confirmed: false,
            mev_submit_verify_ms: 19,
        },
    );
    if preparation == Preparation::Pending {
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(30),
            service.execute(&f.execution, &claims)
        )
        .await
        .is_err());
    } else {
        service.execute(&f.execution, &claims).await;
    }
    (f, backend)
}

#[tokio::test]
async fn pump_prepared_runtime_prepares_accepted_parent_then_uses_original_claim_before_execution()
{
    let (f, backend) = exercise(Preparation::Ready, ClaimResponse::Real).await;
    let trace = backend.trace.lock().unwrap();
    assert_eq!(trace[0..3], ["resolve", "prepare", "validate"]);
    assert!(
        trace.iter().position(|s| *s == "claim-prepared").unwrap()
            < trace.iter().position(|s| *s == "execute-prepared").unwrap()
    );
    assert_eq!(
        trace.iter().filter(|s| **s == "execute-prepared").count(),
        1
    );
    assert!(!trace.contains(&"legacy-sdk"));
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Submitted
    );
    assert_eq!(
        f.state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .signature
            .as_deref(),
        Some("fixture-original-result")
    );
}

#[tokio::test]
async fn pump_prepared_runtime_only_explicit_unsupported_can_use_legacy_before_claim() {
    let (f, backend) = exercise(Preparation::Unsupported, ClaimResponse::Real).await;
    assert_eq!(
        *backend.trace.lock().unwrap(),
        ["resolve", "prepare", "resolve", "legacy-sdk"]
    );
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Submitted
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn pump_prepared_runtime_bank_errors_and_expiry_fail_without_claim_signing_or_fallback() {
    for prep in [Preparation::Failure, Preparation::Stale] {
        let (f, backend) = exercise(prep, ClaimResponse::Real).await;
        let trace = backend.trace.lock().unwrap();
        assert!(
            !trace.contains(&"claim-prepared")
                && !trace.contains(&"execute-prepared")
                && !trace.contains(&"legacy-sdk")
        );
        assert_eq!(
            f.state.trades.get(&f.record.request_id).unwrap().status,
            TradeStatus::Failed
        );
        assert_eq!(
            f.state
                .persistence
                .get(&f.record.request_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TradeStatus::Failed
        );
    }
}

#[tokio::test]
async fn pump_prepared_runtime_claim_conflict_or_uncertain_commit_never_overwrites_parent_or_retries(
) {
    for response in [ClaimResponse::Held, ClaimResponse::Uncertain] {
        let (f, backend) = exercise(Preparation::Ready, response).await;
        let trace = backend.trace.lock().unwrap();
        assert_eq!(trace.last(), Some(&"claim-prepared"));
        assert!(!trace.contains(&"execute-prepared") && !trace.contains(&"legacy-sdk"));
        assert_eq!(
            f.state.trades.get(&f.record.request_id).unwrap().status,
            TradeStatus::Accepted
        );
        assert_eq!(
            f.state
                .persistence
                .get(&f.record.request_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TradeStatus::Accepted
        );
    }
}

#[tokio::test]
async fn pump_prepared_runtime_cancellation_before_claim_does_not_invent_failure_or_owner() {
    let (f, backend) = exercise(Preparation::Pending, ClaimResponse::Real).await;
    assert_eq!(*backend.trace.lock().unwrap(), ["resolve", "prepare"]);
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Accepted
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn pump_prepared_runtime_preserves_uncertain_original_signature_and_never_uses_legacy() {
    let (f, backend) = exercise(Preparation::SignedUnknown, ClaimResponse::Real).await;
    let parent = f.state.trades.get(&f.record.request_id).unwrap();
    assert_eq!(parent.status, TradeStatus::Unknown);
    assert_eq!(
        parent.signature.as_deref(),
        Some("fixture-uncertain-signature")
    );
    assert_eq!(
        backend.trace.lock().unwrap().last(),
        Some(&"execute-prepared")
    );
}
