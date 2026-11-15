use super::*;
use crate::{
    application::{
        ports::{
            ordinary_execution::{OrdinaryExecutionWallet, TradeExecutionFuture},
            ordinary_journal::{SnapshotWrite, TradeReservation},
            ordinary_sol_buy::{OrdinarySolBuyWallet, SolBuyExecutionFuture},
            trade_admission::TradeRecordIndex,
        },
        trade_command::test_support::command,
        trade_execution::UncertainSdkSubmission,
        trade_persistence_input::{RejectedTradeInput, ValidatedTradeInput},
    },
    domain::{blockhash::Blockhash, execution_policy::BroadcastPolicy},
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Trace(Mutex<Vec<String>>);
impl Trace {
    fn add(&self, value: impl Into<String>) {
        self.0.lock().unwrap().push(value.into());
    }
    fn values(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}
struct Journal {
    trace: Arc<Trace>,
    mode: TradeJournalMode,
    fail: bool,
}
impl OrdinaryTradeJournal for Journal {
    fn mode(&self) -> TradeJournalMode {
        self.mode
    }
    fn ensure_snapshot_storage(&self, _: SnapshotWrite) -> anyhow::Result<()> {
        panic!("execution cannot reserve")
    }
    async fn reserve_record(
        &self,
        _: &TradeRecord,
        _: &ValidatedTradeInput,
    ) -> anyhow::Result<TradeReservation> {
        panic!("execution cannot reserve")
    }
    async fn mirror_accepted(&self, _: &TradeRecord, _: ValidatedTradeInput) -> anyhow::Result<()> {
        panic!("execution cannot create another order")
    }
    async fn mirror_rejection(&self, _: &TradeRecord, _: RejectedTradeInput) -> anyhow::Result<()> {
        panic!("execution cannot create another order")
    }
    async fn update_record(&self, record: &TradeRecord) -> anyhow::Result<()> {
        self.trace.add(format!("persist:{:?}", record.status));
        anyhow::ensure!(!self.fail, "synthetic update failure");
        Ok(())
    }
    async fn read_record(&self, _: &str) -> anyhow::Result<Option<TradeRecord>> {
        panic!("no hidden recovery query")
    }
    async fn recent_records(&self, _: usize) -> anyhow::Result<Vec<TradeRecord>> {
        panic!("no hidden query")
    }
}
impl OrdinaryExecutionStorage for Journal {
    // No instance is constructed: direct/aggregate fixtures must never request SDK storage.
    type SdkJournal = crate::adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims;
    fn sdk_journal(&self) -> Option<Self::SdkJournal> {
        panic!("not a journaled SDK fixture")
    }
}
struct Index {
    record: Mutex<TradeRecord>,
    trace: Arc<Trace>,
}
impl TradeRecordIndex for Index {
    fn insert_record(&self, record: &TradeRecord) {
        self.trace.add(format!("insert:{:?}", record.status));
        *self.record.lock().unwrap() = record.clone();
    }
    fn get_record(&self, id: &str) -> Option<TradeRecord> {
        let record = self.record.lock().unwrap();
        (record.request_id == id).then(|| record.clone())
    }
    fn remove_record(&self, _: &str) {
        panic!("execution cannot remove the original record")
    }
}
impl TradeExecutionIndex for Index {
    fn update_status(
        &self,
        id: &str,
        status: TradeStatus,
        signature: Option<String>,
        error: Option<String>,
    ) -> Option<TradeRecord> {
        let mut record = self.record.lock().unwrap();
        assert_eq!(record.request_id, id);
        self.trace.add(format!("index:{status:?}"));
        record.status = status;
        if signature.is_some() {
            record.signature = signature;
        }
        record.error = error;
        Some(record.clone())
    }
}
struct HashRead {
    trace: Arc<Trace>,
    available: bool,
}
impl CachedBlockhashRead for HashRead {
    fn fresh_hash(&self, age: u64) -> Option<Blockhash> {
        assert_eq!(age, 17);
        self.trace.add("hash");
        self.available.then(|| Blockhash::from_bytes([7; 32]))
    }
}
struct Params(Arc<Trace>);
impl ParamCacheWriter for Params {
    fn record(&self, hint: &PoolRouteHint) -> String {
        assert_eq!(hint.mint, command().route.mint);
        self.0.add("cache");
        "original-cache-key".into()
    }
}
struct Claims(Arc<Trace>);
impl OrdinaryExecutionClaimer for Claims {
    async fn claim(&self, execution: &AdmittedTrade) -> anyhow::Result<ExecutionClaim> {
        self.0.add("claim");
        Ok(ExecutionClaim::Acquired(ExecutionOwner::fixture(execution)))
    }
}
#[derive(Clone, Copy)]
enum Report {
    Success,
    Unknown,
    Pending,
}
#[derive(Clone)]
struct Backend {
    trace: Arc<Trace>,
    report: Report,
    available: bool,
    journaled: bool,
}
impl OrdinaryExecutionBackend for Backend {
    fn resolve(&self, wallet: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        assert_eq!(wallet, "fixture-wallet");
        self.trace.add("resolve-sdk");
        self.available
            .then(|| Box::new(self.clone()) as Box<dyn OrdinaryExecutionWallet>)
    }
}
impl OrdinaryExecutionWallet for Backend {
    fn sdk<'a>(
        &'a self,
        input: &'a TradeCommand,
        context: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(async move {
            assert_eq!(input.wallet_id, "fixture-wallet");
            assert_eq!(context.blockhash.to_bytes(), [7; 32]);
            assert!(context.wait_tx_confirmed);
            assert_eq!(context.mev_submit_verify_ms, 19);
            self.trace.add("sdk");
            match self.report {
                Report::Success => Ok(ExecuteResult {
                    success: true,
                    signature: Some("original-result".into()),
                    error: None,
                }),
                Report::Unknown => Err(UncertainSdkSubmission::new(
                    vec!["original-result".into()],
                    "uncertain fixture".into(),
                )
                .into()),
                Report::Pending => {
                    std::future::pending::<()>().await;
                    unreachable!()
                }
            }
        })
    }
    fn pumpapi<'a>(&'a self, _: &'a TradeCommand, _: bool) -> TradeExecutionFuture<'a> {
        Box::pin(async { panic!("no PumpAPI development in this fixture") })
    }
}
impl OrdinarySolBuyBackend for Backend {
    fn resolve(&self, _: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        self.trace.add("resolve-sol-buy");
        Some(Box::new(self.clone()))
    }
}
impl OrdinarySolBuyWallet for Backend {
    fn buy<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        owner: Option<&'a ExecutionOwner>,
    ) -> SolBuyExecutionFuture<'a> {
        Box::pin(async move {
            assert!(owner.is_some());
            self.trace.add("sol-buy");
            if self.journaled {
                let mut record = record();
                record.request_id = execution.reservation().request_id().into();
                record.status = TradeStatus::Submitted;
                record.signature = Some("original-result".into());
                Ok(SolBuyExecutionOutcome::Journaled(JournaledSolBuyOutcome {
                    record: Some(record),
                    signature: None,
                    error: None,
                }))
            } else {
                Ok(SolBuyExecutionOutcome::Legacy(ExecuteResult {
                    success: true,
                    signature: Some("original-result".into()),
                    error: None,
                }))
            }
        })
    }
}
fn record() -> TradeRecord {
    let command = command();
    TradeRecord {
        request_id: "tr_runtime_fixture".into(),
        wallet_id: command.wallet_id,
        wallet_pubkey: command.route.mint.to_string(),
        side: command.amount.side(),
        pool: command.route.pool,
        mint: command.route.mint.to_string(),
        pool_id: None,
        execution_method: ExecutionMethod::Sdk,
        status: TradeStatus::Accepted,
        signature: None,
        error: None,
        duplicate: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}
struct Fixture {
    trace: Arc<Trace>,
    journal: Journal,
    index: Index,
    hash: HashRead,
    backend: Backend,
    params: Params,
    claims: Claims,
    settings: OrdinaryExecutionSettings,
}
impl Fixture {
    fn new() -> Self {
        let trace = Arc::new(Trace::default());
        Self {
            journal: Journal {
                trace: trace.clone(),
                mode: TradeJournalMode::Memory,
                fail: false,
            },
            index: Index {
                trace: trace.clone(),
                record: Mutex::new(record()),
            },
            hash: HashRead {
                trace: trace.clone(),
                available: true,
            },
            backend: Backend {
                trace: trace.clone(),
                report: Report::Success,
                available: true,
                journaled: false,
            },
            params: Params(trace.clone()),
            claims: Claims(trace.clone()),
            trace,
            settings: OrdinaryExecutionSettings {
                broadcast: BroadcastPolicy::Enabled,
                blockhash_max_age_ms: 17,
                wait_tx_confirmed: true,
                mev_submit_verify_ms: 19,
            },
        }
    }
    fn service(&self) -> OrdinaryExecutionService<'_, Journal, Index> {
        OrdinaryExecutionService::new(
            &self.journal,
            &self.index,
            &self.hash,
            &self.backend,
            &self.backend,
            &self.params,
            self.settings,
        )
    }
    fn execution(&self, aggregate: bool) -> AdmittedTrade {
        let mut command = command();
        command.execution.method = ExecutionMethod::Sdk;
        command.aggregator_sol = aggregate;
        AdmittedTrade::unreserved_fixture(&self.index.record.lock().unwrap(), command)
    }
    fn status(&self) -> TradeStatus {
        self.index.record.lock().unwrap().status
    }
}

#[tokio::test]
async fn ordinary_runtime_project_ports_keep_sdk_order_context_and_original_cache() {
    let f = Fixture::new();
    f.service().execute(&f.execution(false), &f.claims).await;
    assert_eq!(
        f.trace.values(),
        [
            "index:BuildingTx",
            "persist:BuildingTx",
            "resolve-sdk",
            "hash",
            "index:Submitting",
            "persist:Submitting",
            "sdk",
            "index:Submitted",
            "persist:Submitted",
            "cache"
        ]
    );
    assert_eq!(f.status(), TradeStatus::Submitted);
}

#[tokio::test]
async fn ordinary_runtime_project_ports_preserve_aggregate_priority_and_journal_parent_authority() {
    for journaled in [false, true] {
        let mut f = Fixture::new();
        f.backend.journaled = journaled;
        f.hash.available = false;
        f.service().execute(&f.execution(true), &f.claims).await;
        let mut expected = vec![
            "claim",
            "index:BuildingTx",
            "persist:BuildingTx",
            "resolve-sol-buy",
            "sol-buy",
        ];
        if journaled {
            expected.push("insert:Submitted");
        } else {
            expected.extend(["index:Submitted", "persist:Submitted"]);
        }
        expected.push("cache");
        assert_eq!(f.trace.values(), expected);
        assert_eq!(f.status(), TradeStatus::Submitted);
    }
}

#[tokio::test]
async fn ordinary_runtime_project_ports_stop_before_backend_on_policy_storage_or_hash_failure() {
    for failure in ["policy", "storage", "hash", "wallet"] {
        let mut f = Fixture::new();
        let (expected, status) = match failure {
            "policy" => {
                f.settings.broadcast = BroadcastPolicy::Disabled;
                (
                    vec!["index:Rejected", "persist:Rejected"],
                    TradeStatus::Rejected,
                )
            }
            "storage" => {
                f.journal.mode = TradeJournalMode::Durable;
                f.journal.fail = true;
                (
                    vec!["index:BuildingTx", "persist:BuildingTx"],
                    TradeStatus::BuildingTx,
                )
            }
            "hash" => {
                f.hash.available = false;
                (
                    vec![
                        "index:BuildingTx",
                        "persist:BuildingTx",
                        "resolve-sdk",
                        "hash",
                        "index:Rejected",
                        "persist:Rejected",
                    ],
                    TradeStatus::Rejected,
                )
            }
            _ => {
                f.backend.available = false;
                (
                    vec![
                        "index:BuildingTx",
                        "persist:BuildingTx",
                        "resolve-sdk",
                        "index:Failed",
                        "persist:Failed",
                    ],
                    TradeStatus::Failed,
                )
            }
        };
        f.service().execute(&f.execution(false), &f.claims).await;
        assert_eq!(f.trace.values(), expected, "{failure}");
        assert_eq!(f.status(), status);
    }
}

#[tokio::test]
async fn ordinary_runtime_project_ports_keep_uncertainty_and_do_not_retry_or_change_backend() {
    let mut f = Fixture::new();
    f.backend.report = Report::Unknown;
    f.service().execute(&f.execution(false), &f.claims).await;
    assert_eq!(f.status(), TradeStatus::Unknown);
    assert_eq!(
        f.index.record.lock().unwrap().signature.as_deref(),
        Some("original-result")
    );
    assert_eq!(
        f.trace
            .values()
            .iter()
            .filter(|step| step.as_str() == "sdk")
            .count(),
        1
    );
    assert_eq!(
        &f.trace.values()[7..],
        ["index:Unknown", "persist:Unknown", "cache"]
    );
}

#[tokio::test]
async fn ordinary_runtime_project_ports_cancellation_does_not_invent_a_result_or_retry() {
    let mut f = Fixture::new();
    f.backend.report = Report::Pending;
    let service = f.service();
    let execution = f.execution(false);
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(20),
        service.execute(&execution, &f.claims)
    )
    .await
    .is_err());
    assert_eq!(f.status(), TradeStatus::Submitting);
    assert_eq!(
        f.trace.values(),
        [
            "index:BuildingTx",
            "persist:BuildingTx",
            "resolve-sdk",
            "hash",
            "index:Submitting",
            "persist:Submitting",
            "sdk"
        ]
    );
}

#[tokio::test]
async fn ordinary_runtime_settings_copy_only_four_original_values_without_validation() {
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.security.broadcast = BroadcastPolicy::Disabled;
    config.blockhash_max_age_ms = u64::MAX;
    config.wait_tx_confirmed = false;
    config.mev_submit_verify_ms = 0;
    config.wallets.clear();
    config.rpc_urls.clear();
    config.default_tip_sol = f64::NAN;
    let settings = config.ordinary_execution_settings();
    config.blockhash_max_age_ms = 0;
    config.wait_tx_confirmed = true;
    assert_eq!(settings.broadcast, BroadcastPolicy::Disabled);
    assert_eq!(settings.blockhash_max_age_ms, u64::MAX);
    assert!(!settings.wait_tx_confirmed);
    assert_eq!(settings.mev_submit_verify_ms, 0);
}
