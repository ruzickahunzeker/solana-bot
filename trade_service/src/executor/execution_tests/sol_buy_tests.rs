use super::*;
use crate::{
    application::{
        ordinary_sol_buy,
        ports::ordinary_sol_buy::{
            OrdinarySolBuyBackend, OrdinarySolBuyPipeline, OrdinarySolBuyWallet, SolBuyIntent,
        },
        quote_submission_types::{
            QuoteSubmissionCommand, QuoteSubmissionOutcome, QuoteSubmissionResult,
        },
    },
    domain::{
        quote::{BoundQuote, QuoteEstimate, QuotePreflight},
        trade::{Address, TradeAmount, WSOL_MINT},
    },
};

struct AggregateTrace {
    events: Mutex<Vec<&'static str>>,
    commands: Mutex<Vec<TradeCommand>>,
    requests: Mutex<Vec<SolBuyIntent>>,
    submissions: Mutex<Vec<QuoteSubmissionCommand>>,
    available: AtomicBool,
    remove_after_lookup: AtomicBool,
    close_after_submit: AtomicBool,
    failure: Mutex<Option<&'static str>>,
    outcome: Mutex<QuoteSubmissionOutcome>,
    signature: Mutex<Option<&'static str>>,
}
struct AggregateBackend {
    trace: Arc<AggregateTrace>,
    trades: Arc<TradeStore>,
    persistence: Arc<TradePersistence>,
    request_id: String,
}
struct AggregateWallet {
    trace: Arc<AggregateTrace>,
    trades: Arc<TradeStore>,
    persistence: Arc<TradePersistence>,
    request_id: String,
}

impl OrdinarySolBuyBackend for AggregateBackend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        assert_eq!(wallet_id, "fixture-wallet");
        assert_eq!(
            self.trades.get(&self.request_id).unwrap().status,
            TradeStatus::BuildingTx
        );
        self.trace.events.lock().unwrap().push("resolve");
        if !self.trace.available.load(Ordering::SeqCst) {
            return None;
        }
        if self.trace.remove_after_lookup.load(Ordering::SeqCst) {
            self.trace.available.store(false, Ordering::SeqCst);
        }
        Some(Box::new(AggregateWallet {
            trace: self.trace.clone(),
            trades: self.trades.clone(),
            persistence: self.persistence.clone(),
            request_id: self.request_id.clone(),
        }))
    }
}
impl OrdinarySolBuyWallet for AggregateWallet {
    fn buy<'a>(
        &'a self,
        execution: &'a crate::application::trade_admission::AdmittedTrade,
        _owner: Option<&'a crate::application::ports::ordinary_claims::ExecutionOwner>,
    ) -> crate::application::ports::ordinary_sol_buy::SolBuyExecutionFuture<'a> {
        let command = execution.command();
        self.trace.events.lock().unwrap().push("buy");
        self.trace.commands.lock().unwrap().push(command.clone());
        Box::pin(async move {
            ordinary_sol_buy::execute(self, command)
                .await
                .map(crate::application::ports::ordinary_sol_buy::SolBuyExecutionOutcome::Legacy)
        })
    }
}
impl AggregateWallet {
    fn event(&self, stage: &'static str) -> anyhow::Result<()> {
        self.trace.events.lock().unwrap().push(stage);
        assert_eq!(
            self.trades.get(&self.request_id).unwrap().status,
            TradeStatus::BuildingTx
        );
        if *self.trace.failure.lock().unwrap() == Some(stage) {
            anyhow::bail!("synthetic {stage} failure");
        }
        Ok(())
    }
}
impl OrdinarySolBuyPipeline for AggregateWallet {
    type Request = SolBuyIntent;
    fn map_request(&self, input: SolBuyIntent) -> Self::Request {
        self.trace.events.lock().unwrap().push("map");
        self.trace.requests.lock().unwrap().push(input.clone());
        input
    }
    fn validate_request(&self, request: &Self::Request) -> anyhow::Result<()> {
        self.event("validate")?;
        crate::aggregator_quote::validate_request(
            &crate::adapters::sdk::ordinary_sol_buy::quote_request(request.clone()),
        )
    }
    async fn quote(&self, request: &Self::Request) -> anyhow::Result<QuoteEstimate> {
        self.event("quote")?;
        if self.persistence.is_durable() {
            assert_eq!(
                self.persistence
                    .get(&self.request_id)
                    .await?
                    .unwrap()
                    .status,
                TradeStatus::BuildingTx
            );
        }
        Ok(QuoteEstimate {
            input_mint: WSOL_MINT.into(),
            output_mint: request.mint.to_string(),
            amount_lamports: request.amount_lamports,
            expected_output_units: 1_000,
            minimum_output_units: 850,
            price_impact_percent: "0.01".into(),
            route: vec![],
            executable: false,
        })
    }
    fn bind(&self, request: &Self::Request, quote: QuoteEstimate) -> anyhow::Result<BoundQuote> {
        self.event("bind")?;
        assert_eq!(quote.amount_lamports, request.amount_lamports);
        Ok(BoundQuote {
            quote_id: "synthetic-bound-id".into(),
            wallet_id: request.wallet_id.clone(),
            wallet_pubkey: Address::from_bytes([0; 32]).to_string(),
            quote_slippage_bps: request.slippage_bps,
            expires_at_ms: u64::MAX,
            quote,
        })
    }
    async fn submit(
        &self,
        bound: &BoundQuote,
        request: &QuoteSubmissionCommand,
    ) -> anyhow::Result<QuoteSubmissionResult> {
        self.event("submit")?;
        self.trace.submissions.lock().unwrap().push(request.clone());
        if self.trace.close_after_submit.load(Ordering::SeqCst) {
            self.persistence.pool_for_tests().close().await;
        }
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/contracts/sol_buy_v1.json"
        ))
        .unwrap();
        let preflight: QuotePreflight =
            serde_json::from_value(fixture["preflight"].clone()).unwrap();
        Ok(QuoteSubmissionResult {
            quote_id: bound.quote_id.clone(),
            wallet_id: request.wallet_id.clone(),
            status: *self.trace.outcome.lock().unwrap(),
            signature: self.trace.signature.lock().unwrap().map(str::to_owned),
            error: None,
            preflight,
        })
    }
}

struct AggregateFixture {
    f: Fixture,
    trace: Arc<AggregateTrace>,
}
impl AggregateFixture {
    fn install(&mut self) {
        self.f.state.ordinary_sol_buy = Arc::new(AggregateBackend {
            trace: self.trace.clone(),
            trades: self.f.state.trades.clone(),
            persistence: self.f.state.persistence.clone(),
            request_id: self.f.request_id.clone(),
        });
    }
    fn events(&self) -> Vec<&'static str> {
        self.trace.events.lock().unwrap().clone()
    }
}
async fn aggregate_fixture(
    method: ExecutionMethod,
    simulate: bool,
    mode: TradePersistenceMode,
) -> AggregateFixture {
    let mut f = fixture(method, Side::Buy, simulate, mode).await;
    f.command.aggregator_sol = true;
    f.state.blockhash_cache = Arc::default();
    let trace = Arc::new(AggregateTrace {
        events: Mutex::default(),
        commands: Mutex::default(),
        requests: Mutex::default(),
        submissions: Mutex::default(),
        available: AtomicBool::new(true),
        remove_after_lookup: AtomicBool::new(false),
        close_after_submit: AtomicBool::new(false),
        failure: Mutex::new(None),
        outcome: Mutex::new(QuoteSubmissionOutcome::Submitted),
        signature: Mutex::new(Some("synthetic-aggregate-signature")),
    });
    let mut result = AggregateFixture { f, trace };
    result.install();
    result
}

#[tokio::test]
async fn ordinary_sol_buy_real_executor_preserves_branch_order_without_blockhash_or_submitting() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for simulate in [false, true] {
            for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
                let a = aggregate_fixture(method, simulate, mode).await;
                a.f.execute().await;
                let expected = if simulate {
                    TradeStatus::Simulated
                } else {
                    TradeStatus::Submitted
                };
                assert_eq!(a.f.status(), expected);
                assert_eq!(
                    a.events(),
                    ["resolve", "buy", "map", "validate", "quote", "bind", "submit"]
                );
                assert_eq!(a.f.calls(), 0);
                assert_eq!(a.f.resolves(), 0);
                assert_eq!(a.trace.commands.lock().unwrap().len(), 1);
                {
                    let intents = a.trace.requests.lock().unwrap();
                    assert_eq!(intents.len(), 1);
                    assert_eq!(intents[0].amount_lamports, 1_000_000);
                    assert_eq!(intents[0].slippage_bps, 1234);
                    let submissions = a.trace.submissions.lock().unwrap();
                    assert_eq!(submissions.len(), 1);
                    assert_eq!(submissions[0].simulate, simulate);
                    assert_eq!(submissions[0].confirmed, !simulate);
                    assert_eq!(submissions[0].idempotency_key, a.f.command.idempotency_key);
                }
                if mode == TradePersistenceMode::Durable {
                    assert_eq!(
                        a.f.state
                            .persistence
                            .get(&a.f.request_id)
                            .await
                            .unwrap()
                            .unwrap()
                            .status,
                        expected
                    );
                }
                assert_eq!(a.f.state.wallets.wallet_count(), 0);
            }
        }
    }
}

#[tokio::test]
async fn ordinary_sol_buy_lookup_failure_precedes_amount_and_slippage_errors() {
    let mut a = aggregate_fixture(ExecutionMethod::Sdk, false, TradePersistenceMode::Durable).await;
    a.trace.available.store(false, Ordering::SeqCst);
    a.f.command.amount = TradeAmount::SellPercent { percent: 100 };
    a.f.command.execution.slippage_bps = u64::MAX;
    a.f.execute().await;
    assert_eq!(a.events(), ["resolve"]);
    assert_eq!(a.f.status(), TradeStatus::Failed);
    assert_eq!(
        a.f.state
            .trades
            .get(&a.f.request_id)
            .unwrap()
            .error
            .as_deref(),
        Some("wallet_id fixture-wallet not found")
    );
    assert_eq!(a.f.resolves(), 0);
    assert_eq!(a.f.calls(), 0);
}

#[tokio::test]
async fn ordinary_sol_buy_disabled_broadcast_retains_live_rejection_and_simulation_allowance() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for simulate in [false, true] {
            let mut a = aggregate_fixture(method, simulate, TradePersistenceMode::Memory).await;
            Arc::get_mut(&mut a.f.state.config)
                .unwrap()
                .security
                .broadcast = BroadcastPolicy::Disabled;
            a.f.execute().await;
            assert_eq!(
                a.f.status(),
                if simulate {
                    TradeStatus::Simulated
                } else {
                    TradeStatus::Rejected
                }
            );
            assert_eq!(a.events().is_empty(), !simulate);
            assert_eq!(a.f.resolves(), 0);
        }
    }
}

#[tokio::test]
async fn ordinary_sol_buy_building_write_failure_or_absent_record_stops_before_lookup() {
    for missing in [false, true] {
        let a = aggregate_fixture(ExecutionMethod::Sdk, false, TradePersistenceMode::Durable).await;
        if missing {
            a.f.state.trades.remove(&a.f.request_id);
        } else {
            sqlx::raw_sql("CREATE TRIGGER aggregation_failure BEFORE UPDATE OF status ON trades WHEN NEW.status='building_tx' BEGIN SELECT RAISE(FAIL,'synthetic persistence failure'); END")
                .execute(&a.f.state.persistence.pool_for_tests()).await.unwrap();
        }
        a.f.execute().await;
        assert!(a.events().is_empty());
        assert_eq!(a.f.resolves(), 0);
        assert_eq!(a.f.calls(), 0);
        assert_eq!(
            a.f.state
                .persistence
                .get(&a.f.request_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TradeStatus::Accepted
        );
    }
}

#[tokio::test]
async fn ordinary_sol_buy_stage_failure_finishes_once_and_preserves_legacy_cache_update() {
    for (failure, expected) in [
        ("validate", vec!["resolve", "buy", "map", "validate"]),
        ("quote", vec!["resolve", "buy", "map", "validate", "quote"]),
        (
            "bind",
            vec!["resolve", "buy", "map", "validate", "quote", "bind"],
        ),
        (
            "submit",
            vec![
                "resolve", "buy", "map", "validate", "quote", "bind", "submit",
            ],
        ),
    ] {
        let a = aggregate_fixture(ExecutionMethod::Sdk, false, TradePersistenceMode::Durable).await;
        *a.trace.failure.lock().unwrap() = Some(failure);
        a.f.execute().await;
        assert_eq!(a.events(), expected);
        assert_eq!(a.f.status(), TradeStatus::Failed);
        assert_eq!(
            a.f.state
                .trades
                .get(&a.f.request_id)
                .unwrap()
                .error
                .as_deref(),
            Some(format!("synthetic {failure} failure").as_str())
        );
        assert!(a
            .f
            .state
            .params
            .get(&cache_key(
                a.f.command.route.pool,
                &a.f.command.route.mint.to_string(),
                None
            ))
            .is_some());
        assert_eq!(a.f.resolves(), 0);
    }
}

#[tokio::test]
async fn ordinary_sol_buy_retained_handle_and_final_db_failure_do_not_repeat_execution() {
    for close in [false, true] {
        let a = aggregate_fixture(ExecutionMethod::Sdk, false, TradePersistenceMode::Durable).await;
        a.trace.remove_after_lookup.store(true, Ordering::SeqCst);
        a.trace.close_after_submit.store(close, Ordering::SeqCst);
        a.f.execute().await;
        assert_eq!(a.f.status(), TradeStatus::Submitted);
        assert_eq!(
            a.events(),
            ["resolve", "buy", "map", "validate", "quote", "bind", "submit"]
        );
        assert!(!a.trace.available.load(Ordering::SeqCst));
        assert_eq!(
            a.f.state
                .trades
                .get(&a.f.request_id)
                .unwrap()
                .signature
                .as_deref(),
            Some("synthetic-aggregate-signature")
        );
        assert_eq!(a.f.state.persistence.healthy().await, !close);
    }
}

#[tokio::test]
async fn ordinary_sol_buy_unknown_without_signature_is_not_retried_or_downgraded_to_failed() {
    let a = aggregate_fixture(ExecutionMethod::Sdk, false, TradePersistenceMode::Durable).await;
    *a.trace.outcome.lock().unwrap() = QuoteSubmissionOutcome::Unknown;
    *a.trace.signature.lock().unwrap() = None;
    a.f.execute().await;
    assert_eq!(a.f.status(), TradeStatus::Unknown);
    assert!(a
        .f
        .state
        .trades
        .get(&a.f.request_id)
        .unwrap()
        .signature
        .is_none());
    assert_eq!(
        a.events(),
        ["resolve", "buy", "map", "validate", "quote", "bind", "submit"]
    );
    assert_eq!(a.f.resolves(), 0);
    let saved =
        a.f.state
            .persistence
            .get(&a.f.request_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.status, TradeStatus::Unknown);
    assert!(saved.signature.is_none());
    assert_eq!(
        saved.error.as_deref(),
        Some("SOL buy submission outcome uncertain; reconcile original signature")
    );
}

#[tokio::test]
async fn ordinary_sol_buy_uncertainty_keeps_memory_signature_when_final_storage_fails() {
    let a = aggregate_fixture(ExecutionMethod::Sdk, false, TradePersistenceMode::Durable).await;
    *a.trace.outcome.lock().unwrap() = QuoteSubmissionOutcome::Unknown;
    a.trace.close_after_submit.store(true, Ordering::SeqCst);
    a.f.execute().await;
    let record = a.f.state.trades.get(&a.f.request_id).unwrap();
    assert_eq!(record.status, TradeStatus::Unknown);
    assert_eq!(
        record.signature.as_deref(),
        Some("synthetic-aggregate-signature")
    );
    assert_eq!(
        a.events(),
        ["resolve", "buy", "map", "validate", "quote", "bind", "submit"]
    );
    assert_eq!(a.f.resolves(), 0);
    assert!(!a.f.state.persistence.healthy().await);
}
