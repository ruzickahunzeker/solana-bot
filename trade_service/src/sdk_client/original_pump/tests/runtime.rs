use super::*;
use crate::{
    adapters::test_support::read_rpc_server::{DammEvidenceKind, DammEvidenceOracle},
    application::{
        ordinary_claims::test_support::Fixture,
        ordinary_runtime::OrdinaryExecutionService,
        ports::{
            blockhash::CachedBlockhashRead,
            ordinary_execution::{
                OrdinaryExecutionBackend, OrdinaryExecutionWallet, PreparedSdkFuture,
                TradeExecutionFuture,
            },
            ordinary_reconciliation::RecoveredOrdinaryObservations,
            ordinary_sol_buy::{OrdinarySolBuyBackend, OrdinarySolBuyWallet},
            pool_queries::ParamCacheWriter,
        },
        trade_command::TradeCommand,
        trade_settings::OrdinaryExecutionSettings,
    },
    domain::{route::PoolRouteHint, sdk_prepared_record::SdkPreparedRecord, trade::TradeStatus},
    persistence::TradePersistence,
};
use base64::Engine;
use serde_json::json;
use sol_trade_sdk::swqos::SwqosClientTrait;
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};
use sqlx::{Row, SqlitePool};
use std::sync::{atomic::AtomicBool, Mutex};
use tokio::sync::Notify;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Good,
    BadBank,
    BadQuote,
    BadFinal,
    WrongReceipt,
    ReceiptWriteFailure,
    HoldQuote,
    HoldFinal,
    HoldSend,
    SlowQuoteHoldFinal,
}

struct Trace {
    expected: [Value; 4],
    results: [Value; 4],
    frozen: VersionedTransaction,
    mode: Mode,
    phases: Mutex<Vec<usize>>,
    sends: Mutex<Vec<Vec<u8>>>,
    binding: Mutex<Option<(SqlitePool, String)>>,
    reached: Notify,
    dropped: AtomicBool,
    cancelled: Notify,
}
struct Dropped<'a>(&'a Trace);
impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
        self.0.cancelled.notify_one();
    }
}

impl DammEvidenceOracle for Trace {
    fn evidence(
        &self,
        kind: DammEvidenceKind,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Value> + Send + '_>> {
        Box::pin(async move {
            let phase = {
                let mut phases = self.phases.lock().unwrap();
                let phase = phases.len() + 1;
                assert!(phase <= 3, "unexpected additional account/probe read");
                match kind {
                    DammEvidenceKind::Accounts => assert_eq!(phase, 1),
                    DammEvidenceKind::Probe => assert!(phase >= 2),
                }
                assert_eq!(params, self.expected[phase]["params"]);
                phases.push(phase);
                phase
            };
            let mut result = self.results[phase].clone();
            match (self.mode, phase) {
                (Mode::BadBank, 1) => {
                    result["value"][1]["owner"] = json!("11111111111111111111111111111111")
                }
                (Mode::BadQuote, 2) | (Mode::BadFinal, 3) => {
                    result["value"]["err"] = json!("fixture bank simulation rejected")
                }
                _ => {}
            }
            if self.mode == Mode::SlowQuoteHoldFinal && phase == 2 {
                tokio::time::sleep(Duration::from_millis(1_250)).await;
            }
            if matches!(
                (self.mode, phase),
                (Mode::HoldQuote, 2) | (Mode::HoldFinal | Mode::SlowQuoteHoldFinal, 3)
            ) {
                let _drop = Dropped(self);
                self.reached.notify_one();
                std::future::pending::<()>().await;
            }
            result
        })
    }
}

struct MetadataOnly;
impl SwqosClientTrait for MetadataOnly {
    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Default
    }
    fn get_tip_account(&self) -> Result<String> {
        panic!("Default cannot acquire relay tip")
    }
    fn send_transaction<'a, 'b, 'f>(
        &'a self,
        _: TradeType,
        _: &'b VersionedTransaction,
        _: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { panic!("must use fixed original RPC, not SDK/provider send worker") })
    }
    fn send_transactions<'a, 'b, 'f>(
        &'a self,
        _: TradeType,
        _: &'b Vec<VersionedTransaction>,
        _: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { panic!("must not use an SDK batch worker") })
    }
}

struct MemoryRpc {
    url: String,
    trace: Arc<Trace>,
}
impl RpcSender for MemoryRpc {
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
            assert!(
                matches!(request, RpcRequest::SendTransaction),
                "all reads/simulations must use original shared HTTP runtime: {request}"
            );
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(params[0].as_str().unwrap())
                .unwrap();
            let tx: VersionedTransaction = bincode::deserialize(&bytes).unwrap();
            assert_eq!(tx.message, self.trace.frozen.message);
            assert_eq!(tx.signatures.len(), 1);
            assert!(tx.signatures[0].verify(
                tx.message.static_account_keys()[0].as_ref(),
                &tx.message.serialize()
            ));
            assert_eq!(
                params[1],
                json!({"encoding":"base64","skipPreflight":true,"preflightCommitment":"processed","maxRetries":3,"minContextSlot":0})
            );
            let (pool, id) = self.trace.binding.lock().unwrap().as_ref().unwrap().clone();
            let row = sqlx::query("SELECT submission_stage,signed_signature,signed_transaction,unsigned_message FROM ordinary_execution_claims WHERE request_id=?").bind(&id).fetch_one(&pool).await.unwrap();
            assert_eq!(row.get::<&str, _>("submission_stage"), "broadcasting");
            assert_eq!(row.get::<Vec<u8>, _>("signed_transaction"), bytes);
            assert_eq!(
                row.get::<Vec<u8>, _>("unsigned_message"),
                tx.message.serialize()
            );
            assert_eq!(
                row.get::<&str, _>("signed_signature"),
                tx.signatures[0].to_string()
            );
            self.trace.sends.lock().unwrap().push(bytes);
            if self.trace.mode == Mode::HoldSend {
                let _drop = Dropped(self.trace.as_ref());
                self.trace.reached.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(json!(if self.trace.mode == Mode::WrongReceipt {
                Signature::new_unique().to_string()
            } else {
                tx.signatures[0].to_string()
            }))
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        self.url.clone()
    }
}

#[derive(Clone)]
struct Backend {
    wallet: Arc<SdkTradeClient>,
    runtime: Arc<SharedReadRuntime>,
    journal: Arc<TradePersistence>,
}
impl OrdinaryExecutionBackend for Backend {
    fn supports_prepared_sdk(&self) -> bool {
        true
    }
    fn resolve(&self, id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        assert_eq!(id, "fixture-wallet");
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
            self.wallet
                .prepare_original_pump(
                    execution,
                    context,
                    self.runtime.clone(),
                    self.journal.ordinary_execution_claims().unwrap(),
                )
                .await
        })
    }
    fn sdk<'a>(&'a self, _: &'a TradeCommand, _: SdkExecutionContext) -> TradeExecutionFuture<'a> {
        Box::pin(async { panic!("controlled Pump runtime must not fall back to opaque SDK") })
    }
    fn pumpapi<'a>(&'a self, _: &'a TradeCommand, _: bool) -> TradeExecutionFuture<'a> {
        Box::pin(async { panic!("controlled Pump must not enter PumpAPI") })
    }
}
impl OrdinarySolBuyBackend for Backend {
    fn resolve(&self, _: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        panic!("native Pump cannot enter aggregator")
    }
}
struct HashRead;
impl CachedBlockhashRead for HashRead {
    fn fresh_hash(&self, _: u64) -> Option<Blockhash> {
        Some(context().blockhash)
    }
}
struct Params;
impl ParamCacheWriter for Params {
    fn record(&self, _: &PoolRouteHint) -> String {
        "public-fixture-hint".into()
    }
}

struct Case {
    f: Fixture,
    backend: Backend,
    server: LocalServer,
    trace: Arc<Trace>,
    original: (String, String, String),
}
impl Case {
    async fn new(side: TradeType, mode: Mode) -> Self {
        let (f, _, _) = PumpNativeCandidateRecord::history_fixture(side, SwqosType::Default).await;
        let (results, mut expected, frozen, buyback, tip) =
            PumpNativeCandidateRecord::original_http_fixture(side).await;
        assert_eq!(buyback, global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS[0]);
        assert!(tip.is_none());
        expected[0]["params"][1]["minContextSlot"] = Value::Null;
        let frozen: VersionedTransaction = bincode::deserialize(&frozen).unwrap();
        assert_eq!(frozen.signatures, [Signature::default()]);
        let trace = Arc::new(Trace {
            expected,
            results,
            frozen,
            mode,
            phases: Mutex::default(),
            sends: Mutex::default(),
            binding: Mutex::new(Some((
                f.state.persistence.pool_for_tests(),
                f.record.request_id.clone(),
            ))),
            reached: Notify::new(),
            dropped: AtomicBool::new(false),
            cancelled: Notify::new(),
        });
        let server = LocalServer::new(Reply::DammEvidence {
            oracle: trace.clone(),
            other: Box::new(Reply::PumpNativeDiscovery {
                result: trace.results[0].clone(),
                // Delay before the funding timestamp so its freshness limit cannot tie
                // the factory's earlier whole-operation deadline.
                mode: Box::new(if mode == Mode::SlowQuoteHoldFinal {
                    Reply::DelayedHeaders(1_000)
                } else {
                    Reply::Success
                }),
            }),
        })
        .await;
        let mut wallet = client(&server.url);
        let config = wallet.lanes[0].client.infrastructure.config.clone();
        wallet.lanes[0].client.infrastructure = Arc::new(TradingInfrastructure {
            rpc: Arc::new(RpcClient::new_sender(
                MemoryRpc {
                    url: server.url.clone(),
                    trace: trace.clone(),
                },
                RpcClientConfig::default(),
            )),
            swqos_clients: Arc::new(vec![Arc::new(MetadataOnly)]),
            config,
            max_sender_concurrency: 1,
            effective_core_ids: Arc::new(vec![]),
        });
        let original: (String, String, String) = sqlx::query_as(
            "SELECT request_hash,idempotency_key,request_json FROM trades WHERE request_id=?",
        )
        .bind(&f.record.request_id)
        .fetch_one(&f.state.persistence.pool_for_tests())
        .await
        .unwrap();
        if mode == Mode::ReceiptWriteFailure {
            sqlx::raw_sql("CREATE TRIGGER deny_pump_receipt BEFORE UPDATE OF submission_stage ON ordinary_execution_claims WHEN NEW.submission_stage='submitted' BEGIN SELECT RAISE(ABORT,'fixture receipt write failure'); END;").execute(&f.state.persistence.pool_for_tests()).await.unwrap();
        }
        let backend = Backend {
            wallet: Arc::new(wallet),
            runtime: Arc::new(SharedReadRuntime::new(&[server.url.clone()]).unwrap()),
            journal: f.state.persistence.clone(),
        };
        Self {
            f,
            backend,
            server,
            trace,
            original,
        }
    }
    async fn run(&self) {
        self.run_with_policy(BroadcastPolicy::Enabled).await;
    }
    async fn run_with_policy(&self, broadcast: BroadcastPolicy) {
        OrdinaryExecutionService::new(
            self.f.state.persistence.as_ref(),
            self.f.state.trades.as_ref(),
            &HashRead,
            &self.backend,
            &self.backend,
            &Params,
            OrdinaryExecutionSettings {
                broadcast,
                blockhash_max_age_ms: 17,
                wait_tx_confirmed: false,
                mev_submit_verify_ms: 0,
            },
        )
        .execute(&self.f.execution, &self.f.claims())
        .await;
    }
    async fn unchanged(&self) {
        let after: (String, String, String) = sqlx::query_as(
            "SELECT request_hash,idempotency_key,request_json FROM trades WHERE request_id=?",
        )
        .bind(&self.f.record.request_id)
        .fetch_one(&self.f.state.persistence.pool_for_tests())
        .await
        .unwrap();
        assert_eq!(after, self.original);
    }
    async fn finish(self) {
        self.server.finish().await;
        self.f.state.persistence.pool_for_tests().close().await;
    }
}

#[tokio::test]
async fn pump_runtime_package_actual_factory_http_claim_controller_and_fixed_sender_buy_and_sell() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let c = Case::new(side, Mode::Good).await;
        c.run().await;
        let saved =
            c.f.state
                .persistence
                .get(&c.f.record.request_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(saved.status, TradeStatus::Submitted);
        assert!(saved.signature.is_some());
        assert_eq!(*c.server.requests.lock().unwrap(), c.trace.expected);
        assert_eq!(*c.trace.phases.lock().unwrap(), [1, 2, 3]);
        assert_eq!(c.trace.sends.lock().unwrap().len(), 1);
        let row = sqlx::query("SELECT sdk_prepared_json,unsigned_message,signed_transaction,operation FROM ordinary_execution_claims WHERE request_id=?").bind(&c.f.record.request_id).fetch_one(&c.f.state.persistence.pool_for_tests()).await.unwrap();
        assert_eq!(row.get::<&str, _>("operation"), "ordinary-sdk-trade");
        let message: Vec<u8> = row.get("unsigned_message");
        let metadata = SdkPreparedRecord::decode(row.get("sdk_prepared_json"), &message).unwrap();
        assert!(matches!(metadata, SdkPreparedRecord::PumpPrepared(_)));
        assert_eq!(message, c.trace.frozen.message.serialize());
        assert_eq!(
            row.get::<Vec<u8>, _>("signed_transaction"),
            c.trace.sends.lock().unwrap()[0]
        );
        c.unchanged().await;
        c.run().await;
        assert_eq!(c.server.requests.lock().unwrap().len(), 4);
        assert_eq!(c.trace.sends.lock().unwrap().len(), 1);
        c.finish().await;
    }
}

#[tokio::test]
async fn pump_runtime_package_bad_bank_quote_or_final_stops_without_claim_signature_or_legacy() {
    for mode in [Mode::BadBank, Mode::BadQuote, Mode::BadFinal] {
        for side in [TradeType::Buy, TradeType::Sell] {
            let c = Case::new(side, mode).await;
            c.run().await;
            let saved =
                c.f.state
                    .persistence
                    .get(&c.f.record.request_id)
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(saved.status, TradeStatus::Failed, "{mode:?}");
            assert!(saved.signature.is_none() && saved.error.is_some());
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
                    .fetch_one(&c.f.state.persistence.pool_for_tests())
                    .await
                    .unwrap(),
                0
            );
            assert!(c.trace.sends.lock().unwrap().is_empty());
            let expected = match mode {
                Mode::BadBank => 2,
                Mode::BadQuote => 3,
                _ => 4,
            };
            assert_eq!(c.server.requests.lock().unwrap().len(), expected);
            c.unchanged().await;
            c.run().await;
            assert_eq!(c.server.requests.lock().unwrap().len(), expected);
            c.finish().await;
        }
    }
}

#[tokio::test]
async fn pump_runtime_package_receipt_mismatch_or_write_failure_keeps_original_signature_without_retry(
) {
    for mode in [Mode::WrongReceipt, Mode::ReceiptWriteFailure] {
        let c = Case::new(TradeType::Sell, mode).await;
        c.run().await;
        let saved =
            c.f.state
                .persistence
                .get(&c.f.record.request_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            saved.status,
            if mode == Mode::WrongReceipt {
                TradeStatus::Unknown
            } else {
                TradeStatus::Submitting
            }
        );
        let cached = c.f.state.trades.get(&c.f.record.request_id).unwrap();
        assert_eq!(cached.status, TradeStatus::Unknown);
        assert_eq!(cached.signature, saved.signature);
        assert!(saved.signature.is_some());
        let signed: VersionedTransaction =
            bincode::deserialize(&c.trace.sends.lock().unwrap()[0]).unwrap();
        assert_eq!(saved.signature.unwrap(), signed.signatures[0].to_string());
        c.run().await;
        assert_eq!(c.server.requests.lock().unwrap().len(), 4);
        assert_eq!(c.trace.sends.lock().unwrap().len(), 1);
        c.unchanged().await;
        c.finish().await;
    }
}

#[tokio::test]
async fn pump_runtime_package_cancellation_releases_reads_and_preserves_signed_send_without_retry()
{
    for mode in [Mode::HoldQuote, Mode::HoldFinal, Mode::HoldSend] {
        let c = Arc::new(Case::new(TradeType::Buy, mode).await);
        let trace = c.trace.clone();
        let id = c.f.record.request_id.clone();
        let persistence = c.f.state.persistence.clone();
        let runtime = c.backend.runtime.clone();
        let url = c.server.url.clone();
        let active = c.clone();
        let task = tokio::spawn(async move {
            active.run().await;
        });
        tokio::time::timeout(Duration::from_secs(3), trace.reached.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        // The server remains alive: only client disconnection may cancel its held oracle.
        tokio::time::timeout(Duration::from_secs(1), async {
            while !trace.dropped.load(Ordering::SeqCst) {
                trace.cancelled.notified().await;
            }
        })
        .await
        .unwrap();
        let parent = persistence.get(&id).await.unwrap().unwrap();
        if mode == Mode::HoldSend {
            assert_eq!(parent.status, TradeStatus::Submitting);
            assert!(parent.signature.is_some());
            assert!(trace.dropped.load(Ordering::SeqCst));
            let store = persistence.ordinary_execution_claims().unwrap();
            let captured = store.capture_observation(&id).await.unwrap();
            assert_eq!(captured.record.signature, parent.signature);
            assert_eq!(trace.sends.lock().unwrap().len(), 1);
            let reads = c.server.requests.lock().unwrap().len();
            c.run().await;
            assert_eq!(c.server.requests.lock().unwrap().len(), reads);
            assert_eq!(trace.sends.lock().unwrap().len(), 1);
        } else {
            assert_eq!(parent.status, TradeStatus::Accepted);
            assert!(parent.signature.is_none());
            assert!(trace.sends.lock().unwrap().is_empty());
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
                    .fetch_one(&persistence.pool_for_tests())
                    .await
                    .unwrap(),
                0
            );
        }
        let before = trace.sends.lock().unwrap().len();
        tokio::task::yield_now().await;
        assert_eq!(trace.sends.lock().unwrap().len(), before);
        assert!(runtime.parameter_slot().is_ok());
        assert!(runtime.aggregator_simulation_slot(&url).is_ok());
        Arc::try_unwrap(c).ok().unwrap().finish().await;
    }
}

#[tokio::test]
async fn pump_runtime_package_nonaccepted_missing_or_mismatched_parent_stops_before_wallet_or_hash()
{
    struct NoResolve;
    impl OrdinaryExecutionBackend for NoResolve {
        fn supports_prepared_sdk(&self) -> bool {
            true
        }
        fn resolve(&self, _: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
            panic!("closed parent cannot resolve wallet or fall back")
        }
    }
    impl OrdinarySolBuyBackend for NoResolve {
        fn resolve(&self, _: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
            panic!("closed parent cannot route")
        }
    }
    struct NoHash;
    impl CachedBlockhashRead for NoHash {
        fn fresh_hash(&self, _: u64) -> Option<Blockhash> {
            panic!("closed parent cannot select blockhash")
        }
    }
    let states = [
        "warming_params",
        "building_tx",
        "submitting",
        "simulated",
        "submitted",
        "unknown",
        "confirmed",
        "failed",
        "rejected",
    ];
    for broadcast in [BroadcastPolicy::Enabled, BroadcastPolicy::Disabled] {
        for (state, signed) in states
            .into_iter()
            .flat_map(|state| [(state, false), (state, true)])
            .chain([
                ("accepted-signed", true),
                ("missing", false),
                ("identity", false),
            ])
        {
            let c = Case::new(TradeType::Buy, Mode::Good).await;
            let pool = c.f.state.persistence.pool_for_tests();
            match state {
                "missing" => {
                    sqlx::query("DELETE FROM trades WHERE request_id=?")
                        .bind(&c.f.record.request_id)
                        .execute(&pool)
                        .await
                        .unwrap();
                }
                "identity" => {
                    sqlx::query(
                    "UPDATE trades SET wallet_pubkey='foreign-fixture-wallet' WHERE request_id=?",
                )
                .bind(&c.f.record.request_id)
                .execute(&pool)
                .await
                .unwrap();
                }
                "accepted-signed" => {
                    sqlx::query(
                    "UPDATE trades SET signature='original-fixture-signature' WHERE request_id=?",
                )
                .bind(&c.f.record.request_id)
                .execute(&pool)
                .await
                .unwrap();
                }
                status => {
                    sqlx::query("UPDATE trades SET status=?,signature=?,error='original-result' WHERE request_id=?")
                    .bind(status)
                    .bind(signed.then_some("original-fixture-signature"))
                    .bind(&c.f.record.request_id).execute(&pool).await.unwrap();
                }
            }
            let before =
                c.f.state
                    .persistence
                    .get(&c.f.record.request_id)
                    .await
                    .unwrap();
            let index_before = c.f.state.trades.get(&c.f.record.request_id);
            OrdinaryExecutionService::new(
                c.f.state.persistence.as_ref(),
                c.f.state.trades.as_ref(),
                &NoHash,
                &NoResolve,
                &NoResolve,
                &Params,
                OrdinaryExecutionSettings {
                    broadcast,
                    blockhash_max_age_ms: 17,
                    wait_tx_confirmed: false,
                    mev_submit_verify_ms: 0,
                },
            )
            .execute(&c.f.execution, &c.f.claims())
            .await;
            assert_eq!(
                serde_json::to_value(
                    c.f.state
                        .persistence
                        .get(&c.f.record.request_id)
                        .await
                        .unwrap()
                )
                .unwrap(),
                serde_json::to_value(before).unwrap(),
                "{state}/{signed}/{broadcast:?}"
            );
            assert_eq!(
                serde_json::to_value(c.f.state.trades.get(&c.f.record.request_id)).unwrap(),
                serde_json::to_value(index_before).unwrap(),
                "{state}/{signed}/{broadcast:?}"
            );
            assert_eq!(c.server.connections.load(Ordering::SeqCst), 0);
            assert!(c.trace.sends.lock().unwrap().is_empty());
            c.finish().await;
        }
    }
}

#[tokio::test]
async fn pump_runtime_package_step_and_whole_timeouts_fail_original_parent_without_signing() {
    for mode in [Mode::HoldQuote, Mode::HoldFinal, Mode::SlowQuoteHoldFinal] {
        let c = Case::new(TradeType::Buy, mode).await;
        tokio::time::timeout(Duration::from_secs(5), c.run())
            .await
            .unwrap();
        let saved =
            c.f.state
                .persistence
                .get(&c.f.record.request_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(saved.status, TradeStatus::Failed);
        assert!(saved.signature.is_none());
        let error = saved.error.unwrap();
        match mode {
            Mode::HoldQuote => assert!(
                matches!(
                    error.as_str(),
                    "native Pump probe timed out" | "native Pump probe RPC unavailable"
                ),
                "{error}"
            ),
            Mode::HoldFinal => assert!(
                matches!(
                    error.as_str(),
                    "native Pump final timed out" | "native Pump final RPC unavailable"
                ),
                "{error}"
            ),
            Mode::SlowQuoteHoldFinal => {
                assert_eq!(error, "native Pump complete evidence deadline expired")
            }
            _ => unreachable!(),
        }
        assert_eq!(
            *c.trace.phases.lock().unwrap(),
            if mode == Mode::HoldQuote {
                vec![1, 2]
            } else {
                vec![1, 2, 3]
            }
        );
        assert!(c.trace.sends.lock().unwrap().is_empty());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
                .fetch_one(&c.f.state.persistence.pool_for_tests())
                .await
                .unwrap(),
            0
        );
        c.unchanged().await;
        c.finish().await;
    }
}

#[tokio::test]
async fn pump_runtime_package_disabled_policy_rejects_only_unsigned_accepted_parent_before_reads() {
    let c = Case::new(TradeType::Buy, Mode::Good).await;
    c.run_with_policy(BroadcastPolicy::Disabled).await;
    let saved =
        c.f.state
            .persistence
            .get(&c.f.record.request_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.status, TradeStatus::Rejected);
    assert_eq!(
        saved.error.as_deref(),
        Some(crate::domain::execution_policy::BROADCAST_DISABLED_ERROR)
    );
    assert!(saved.signature.is_none());
    assert_eq!(c.server.connections.load(Ordering::SeqCst), 0);
    assert!(c.trace.sends.lock().unwrap().is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&c.f.state.persistence.pool_for_tests())
            .await
            .unwrap(),
        0
    );
    c.unchanged().await;
    c.finish().await;
}

#[tokio::test]
async fn pump_runtime_package_unreadable_parent_does_not_overwrite_index_or_enter_sdk() {
    for policy in [BroadcastPolicy::Enabled, BroadcastPolicy::Disabled] {
        let c = Case::new(TradeType::Buy, Mode::Good).await;
        let before = serde_json::to_value(c.f.state.trades.get(&c.f.record.request_id)).unwrap();
        c.f.state.persistence.pool_for_tests().close().await;
        c.run_with_policy(policy).await;
        assert_eq!(
            serde_json::to_value(c.f.state.trades.get(&c.f.record.request_id)).unwrap(),
            before
        );
        assert_eq!(c.server.connections.load(Ordering::SeqCst), 0);
        assert!(c.trace.sends.lock().unwrap().is_empty());
        c.finish().await;
    }
}
