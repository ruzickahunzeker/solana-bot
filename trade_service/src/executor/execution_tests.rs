//! Executes the real ordinary orchestration with no keys, SDK runtime or network.
use super::*;
use crate::{
    application::ports::ordinary_execution::{
        OrdinaryExecutionBackend, OrdinaryExecutionWallet, TradeExecutionFuture,
    },
    config::{TradePersistenceConfig, TradePersistenceMode},
    domain::{blockhash::Blockhash, execution_policy::BroadcastPolicy, trade::Side},
    models::TradeRequest,
    persistence::TradePersistence,
    state::{now_ms, TradeStore},
    validation::validate_trade,
    BlockhashCache,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

mod param_cache_tests;
mod sol_buy_tests;

#[derive(Clone)]
enum Reply {
    Report {
        success: bool,
        signature: Option<&'static str>,
        error: Option<&'static str>,
    },
    Error,
}

struct Call {
    method: ExecutionMethod,
    command: TradeCommand,
    context: Option<SdkExecutionContext>,
    wait_tx_confirmed: bool,
}

struct Trace {
    resolves: Mutex<Vec<String>>,
    calls: Mutex<Vec<Call>>,
    available: AtomicBool,
    delete_after_resolve: AtomicBool,
    fail_final_persist: AtomicBool,
    reply: Mutex<Reply>,
}

struct Backend {
    trace: Arc<Trace>,
    trades: Arc<TradeStore>,
    persistence: Arc<TradePersistence>,
    request_id: String,
}

struct Wallet {
    trace: Arc<Trace>,
    trades: Arc<TradeStore>,
    persistence: Arc<TradePersistence>,
    request_id: String,
}

impl OrdinaryExecutionBackend for Backend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        assert_eq!(
            self.trades.get(&self.request_id).unwrap().status,
            TradeStatus::BuildingTx
        );
        self.trace.resolves.lock().unwrap().push(wallet_id.into());
        if !self.trace.available.load(Ordering::SeqCst) {
            return None;
        }
        if self.trace.delete_after_resolve.load(Ordering::SeqCst) {
            self.trace.available.store(false, Ordering::SeqCst);
        }
        Some(Box::new(Wallet {
            trace: self.trace.clone(),
            trades: self.trades.clone(),
            persistence: self.persistence.clone(),
            request_id: self.request_id.clone(),
        }))
    }
}

impl Wallet {
    async fn run(&self, call: Call) -> anyhow::Result<ExecuteResult> {
        assert_eq!(
            self.trades.get(&self.request_id).unwrap().status,
            TradeStatus::Submitting
        );
        if self.persistence.is_durable() {
            assert_eq!(
                self.persistence
                    .get(&self.request_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                TradeStatus::Submitting
            );
        }
        self.trace.calls.lock().unwrap().push(call);
        if self.trace.fail_final_persist.load(Ordering::SeqCst) {
            self.persistence.pool_for_tests().close().await;
        }
        match self.trace.reply.lock().unwrap().clone() {
            Reply::Report {
                success,
                signature,
                error,
            } => Ok(ExecuteResult {
                success,
                signature: signature.map(str::to_owned),
                error: error.map(str::to_owned),
            }),
            Reply::Error => anyhow::bail!("synthetic backend failure"),
        }
    }
}

impl OrdinaryExecutionWallet for Wallet {
    fn sdk<'a>(
        &'a self,
        command: &'a TradeCommand,
        context: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(self.run(Call {
            method: ExecutionMethod::Sdk,
            command: command.clone(),
            context: Some(context),
            wait_tx_confirmed: context.wait_tx_confirmed,
        }))
    }
    fn pumpapi<'a>(
        &'a self,
        command: &'a TradeCommand,
        wait_tx_confirmed: bool,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(self.run(Call {
            method: ExecutionMethod::Pumpapi,
            command: command.clone(),
            context: None,
            wait_tx_confirmed,
        }))
    }
}

struct Fixture {
    state: AppState,
    command: TradeCommand,
    request_id: String,
    trace: Arc<Trace>,
}

async fn fixture(
    method: ExecutionMethod,
    side: Side,
    simulate: bool,
    mode: TradePersistenceMode,
) -> Fixture {
    let mut state = crate::http_contract_tests::state().await;
    state.edit_fixture_config(|config| {
        config.pumpapi.enabled = true;
        config.wait_tx_confirmed = true;
        config.mev_submit_verify_ms = 171;
        config.blockhash_max_age_ms = u64::MAX;
    });
    let request: TradeRequest = serde_json::from_value(serde_json::json!({
        "side":side,"pool":"pump","mint":"11111111111111111111111111111111",
        "wallet_id":"fixture-wallet","execution_method":method,"simulate":simulate,
        "amount_sol":if side == Side::Buy {Some(0.001)} else {None},
        "sell_percent":if side == Side::Sell {Some(100)} else {None},
        "slippage_bps":1234,"tip_sol":0.00037,"idempotency_key":"execution-port-fixture"
    }))
    .unwrap();
    let normalized = validate_trade(&request, &state.config).unwrap();
    let command = request
        .to_command(
            "fixture-wallet".into(),
            &normalized,
            state.config.trade_command_defaults(),
        )
        .unwrap();
    if mode != TradePersistenceMode::Memory {
        state.persistence = Arc::new(
            TradePersistence::new(&TradePersistenceConfig {
                mode,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 100,
            })
            .await
            .unwrap(),
        );
    }
    state.blockhash_cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([42; 32]),
        now_ms(),
    ));
    let record = state.trades.create(
        &request,
        "fixture-wallet".into(),
        "11111111111111111111111111111111".into(),
    );
    state
        .persistence
        .reserve(&record, &request, &normalized)
        .await
        .unwrap();
    let trace = Arc::new(Trace {
        resolves: Mutex::default(),
        calls: Mutex::default(),
        available: AtomicBool::new(true),
        delete_after_resolve: AtomicBool::new(false),
        fail_final_persist: AtomicBool::new(false),
        reply: Mutex::new(Reply::Report {
            success: true,
            signature: Some("synthetic-original-signature"),
            error: None,
        }),
    });
    state.ordinary_execution = Arc::new(Backend {
        trace: trace.clone(),
        trades: state.trades.clone(),
        persistence: state.persistence.clone(),
        request_id: record.request_id.clone(),
    });
    Fixture {
        state,
        command,
        request_id: record.request_id,
        trace,
    }
}

impl Fixture {
    async fn execute(&self) {
        execute_trade(
            self.state.clone(),
            self.request_id.clone(),
            self.command.clone(),
        )
        .await;
    }
    fn status(&self) -> TradeStatus {
        self.state.trades.get(&self.request_id).unwrap().status
    }
    fn calls(&self) -> usize {
        self.trace.calls.lock().unwrap().len()
    }
    fn resolves(&self) -> usize {
        self.trace.resolves.lock().unwrap().len()
    }
}

#[tokio::test]
async fn sdk_and_pumpapi_real_orchestration_preserves_command_context_and_results() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for side in [Side::Buy, Side::Sell] {
            for simulate in [false, true] {
                let f = fixture(method, side, simulate, TradePersistenceMode::Durable).await;
                f.execute().await;
                assert_eq!(
                    f.status(),
                    if simulate {
                        TradeStatus::Simulated
                    } else {
                        TradeStatus::Submitted
                    }
                );
                assert_eq!(f.resolves(), 1);
                assert_eq!(f.calls(), 1);
                let stored = f
                    .state
                    .persistence
                    .get(&f.request_id)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(stored.status, f.status());
                assert_eq!(
                    stored.signature.as_deref(),
                    Some("synthetic-original-signature")
                );
                let calls = f.trace.calls.lock().unwrap();
                let call = &calls[0];
                assert_eq!(call.method, method);
                assert_eq!(call.command.amount.side(), side);
                assert_eq!(call.command.wallet_id, f.command.wallet_id);
                assert_eq!(call.command.idempotency_key, f.command.idempotency_key);
                assert_eq!(call.command.route.mint, f.command.route.mint);
                assert_eq!(call.command.quote_mint(), f.command.quote_mint());
                assert_eq!(call.command.execution.mode.simulate(), simulate);
                assert_eq!(
                    call.command.amount.buy_units(),
                    f.command.amount.buy_units()
                );
                assert_eq!(
                    call.command.amount.sell_percent(),
                    f.command.amount.sell_percent()
                );
                assert_eq!(call.command.execution.slippage_bps, 1234);
                assert_eq!(
                    call.command.execution.tip_sol.to_bits(),
                    0.00037f64.to_bits()
                );
                assert_eq!(
                    call.command.pumpapi_priority_fee_sol.to_bits(),
                    0.00037f64.to_bits()
                );
                assert!(call.wait_tx_confirmed);
                if let Some(context) = call.context {
                    assert_eq!(method, ExecutionMethod::Sdk);
                    assert_eq!(context.blockhash.to_bytes(), [42; 32]);
                    assert_eq!(context.mev_submit_verify_ms, 171);
                } else {
                    assert_eq!(method, ExecutionMethod::Pumpapi);
                }
                assert!(f
                    .state
                    .params
                    .get(&cache_key(
                        f.command.route.pool,
                        &f.command.route.mint.to_string(),
                        None
                    ))
                    .is_some());
                assert_eq!(f.state.wallets.wallet_count(), 0);
            }
        }
    }
}

#[tokio::test]
async fn missing_wallet_stops_before_blockhash_or_backend_calls() {
    let mut f = fixture(
        ExecutionMethod::Sdk,
        Side::Buy,
        false,
        TradePersistenceMode::Durable,
    )
    .await;
    f.trace.available.store(false, Ordering::SeqCst);
    f.state.blockhash_cache = Arc::default();
    f.execute().await;
    assert_eq!(f.status(), TradeStatus::Failed);
    assert_eq!(f.resolves(), 1);
    assert_eq!(f.calls(), 0);
    assert_eq!(
        f.state
            .persistence
            .get(&f.request_id)
            .await
            .unwrap()
            .unwrap()
            .error
            .as_deref(),
        Some("wallet_id fixture-wallet not found")
    );
}

#[tokio::test]
async fn blockhash_is_required_only_for_sdk_and_after_wallet_resolution() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for expired in [false, true] {
            let mut f = fixture(method, Side::Buy, false, TradePersistenceMode::Durable).await;
            f.state.blockhash_cache = if expired {
                f.state
                    .edit_fixture_config(|config| config.blockhash_max_age_ms = 0);
                Arc::new(BlockhashCache::with_snapshot_for_tests(
                    Blockhash::from_bytes([41; 32]),
                    now_ms().saturating_sub(10_000),
                ))
            } else {
                Arc::default()
            };
            f.execute().await;
            assert_eq!(f.resolves(), 1);
            assert_eq!(
                f.status(),
                if method == ExecutionMethod::Sdk {
                    TradeStatus::Rejected
                } else {
                    TradeStatus::Submitted
                }
            );
            assert_eq!(f.calls(), usize::from(method == ExecutionMethod::Pumpapi));
            if method == ExecutionMethod::Sdk {
                assert_eq!(
                    f.state.trades.get(&f.request_id).unwrap().error.as_deref(),
                    Some("blockhash cache is empty or stale")
                );
            }
        }
    }
}

#[tokio::test]
async fn resolved_wallet_survives_registry_removal_without_a_second_lookup() {
    let f = fixture(
        ExecutionMethod::Sdk,
        Side::Buy,
        false,
        TradePersistenceMode::Durable,
    )
    .await;
    f.trace.delete_after_resolve.store(true, Ordering::SeqCst);
    f.execute().await;
    assert!(!f.trace.available.load(Ordering::SeqCst));
    assert_eq!(f.status(), TradeStatus::Submitted);
    assert_eq!(f.resolves(), 1);
    assert_eq!(f.calls(), 1);
}

#[tokio::test]
async fn disabled_broadcast_rejects_before_wallet_resolution_with_original_simulate_policy() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for simulate in [false, true] {
            let mut f = fixture(method, Side::Buy, simulate, TradePersistenceMode::Memory).await;
            f.state.edit_fixture_config(|config| {
                config.security.broadcast = BroadcastPolicy::Disabled
            });
            f.execute().await;
            let allowed = method == ExecutionMethod::Pumpapi && simulate;
            assert_eq!(
                f.status(),
                if allowed {
                    TradeStatus::Simulated
                } else {
                    TradeStatus::Rejected
                }
            );
            assert_eq!(f.calls(), usize::from(allowed));
            assert_eq!(f.resolves(), usize::from(allowed));
        }
    }
}

#[tokio::test]
async fn durable_write_failures_stop_before_resolution_or_execution_at_each_stage() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for stage in ["building_tx", "submitting"] {
            let f = fixture(method, Side::Buy, false, TradePersistenceMode::Durable).await;
            sqlx::raw_sql(&format!("CREATE TRIGGER execution_fixture_failure BEFORE UPDATE OF status ON trades WHEN NEW.status='{stage}' BEGIN SELECT RAISE(FAIL, 'synthetic persistence failure'); END"))
            .execute(&f.state.persistence.pool_for_tests()).await.unwrap();
            f.execute().await;
            assert_eq!(f.calls(), 0);
            assert_eq!(f.resolves(), usize::from(stage == "submitting"));
            assert_eq!(
                f.status(),
                if stage == "building_tx" {
                    TradeStatus::BuildingTx
                } else {
                    TradeStatus::Submitting
                }
            );
            assert_eq!(
                f.state
                    .persistence
                    .get(&f.request_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                if stage == "building_tx" {
                    TradeStatus::Accepted
                } else {
                    TradeStatus::BuildingTx
                }
            );
        }
    }
}

#[tokio::test]
async fn missing_in_memory_record_does_not_resolve_or_execute_a_wallet() {
    let f = fixture(
        ExecutionMethod::Sdk,
        Side::Buy,
        false,
        TradePersistenceMode::Memory,
    )
    .await;
    f.state.trades.remove(&f.request_id);
    f.execute().await;
    assert_eq!(f.resolves(), 0);
    assert_eq!(f.calls(), 0);
}

#[tokio::test]
async fn backend_uncertainty_and_errors_keep_original_signature_and_never_retry() {
    for (reply, expected, signature) in [
        (
            Reply::Report {
                success: false,
                signature: Some("original"),
                error: Some("timeout"),
            },
            TradeStatus::Unknown,
            Some("original"),
        ),
        (
            Reply::Report {
                success: true,
                signature: None,
                error: None,
            },
            TradeStatus::Unknown,
            None,
        ),
        (
            Reply::Report {
                success: false,
                signature: Some("original"),
                error: Some("sendTransaction failed"),
            },
            TradeStatus::Failed,
            Some("original"),
        ),
        (Reply::Error, TradeStatus::Failed, None),
    ] {
        let f = fixture(
            ExecutionMethod::Sdk,
            Side::Sell,
            false,
            TradePersistenceMode::Durable,
        )
        .await;
        *f.trace.reply.lock().unwrap() = reply;
        f.execute().await;
        assert_eq!(f.status(), expected);
        assert_eq!(f.resolves(), 1);
        assert_eq!(f.calls(), 1);
        let stored = f
            .state
            .persistence
            .get(&f.request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, expected);
        assert_eq!(stored.signature.as_deref(), signature);
    }
}

#[tokio::test]
async fn final_persistence_failure_does_not_repeat_backend_or_lose_in_memory_signature() {
    let f = fixture(
        ExecutionMethod::Sdk,
        Side::Buy,
        false,
        TradePersistenceMode::Durable,
    )
    .await;
    f.trace.fail_final_persist.store(true, Ordering::SeqCst);
    f.execute().await;
    assert_eq!(f.status(), TradeStatus::Submitted);
    assert_eq!(f.calls(), 1);
    assert_eq!(
        f.state
            .trades
            .get(&f.request_id)
            .unwrap()
            .signature
            .as_deref(),
        Some("synthetic-original-signature")
    );
    assert!(!f.state.persistence.healthy().await);
}

#[tokio::test]
async fn unavailable_shadow_writer_keeps_legacy_in_memory_execution() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        let mut f = fixture(method, Side::Buy, false, TradePersistenceMode::Memory).await;
        f.state.persistence = Arc::new(TradePersistence::unavailable_shadow_for_tests());
        f.state.ordinary_execution = Arc::new(Backend {
            trace: f.trace.clone(),
            trades: f.state.trades.clone(),
            persistence: f.state.persistence.clone(),
            request_id: f.request_id.clone(),
        });
        f.execute().await;
        assert_eq!(f.status(), TradeStatus::Submitted);
        assert_eq!(f.calls(), 1);
        assert_eq!(f.resolves(), 1);
        assert_eq!(f.state.persistence.mode(), TradePersistenceMode::Shadow);
        assert!(!f.state.persistence.healthy().await);
    }
}

#[tokio::test]
async fn legacy_aggregator_branch_does_not_resolve_ordinary_wallet_backend() {
    let mut f = fixture(
        ExecutionMethod::Pumpapi,
        Side::Buy,
        false,
        TradePersistenceMode::Memory,
    )
    .await;
    // Deliberately synthetic internal flag combination to freeze branch precedence.
    f.command.aggregator_sol = true;
    f.execute().await;
    assert_eq!(f.status(), TradeStatus::Failed);
    assert_eq!(
        f.state.trades.get(&f.request_id).unwrap().error.as_deref(),
        Some("wallet_id fixture-wallet not found")
    );
    assert_eq!(f.resolves(), 0);
    assert_eq!(f.calls(), 0);
}

#[tokio::test]
async fn legacy_aggregator_request_and_validation_baseline_before_extraction() {
    use crate::domain::trade::{Address, TradeAmount, WSOL_MINT};
    use sha2::{Digest, Sha256};
    let f = fixture(
        ExecutionMethod::Sdk,
        Side::Buy,
        false,
        TradePersistenceMode::Memory,
    )
    .await;
    let mut rows = vec![];
    for wallet in [
        "fixture-wallet".to_owned(),
        " literal wallet ".into(),
        " ".into(),
        "w".repeat(129),
    ] {
        for mint in [
            Address::from_bytes([0; 32]),
            Address::from_bytes(
                bs58::decode(WSOL_MINT)
                    .into_vec()
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
        ] {
            for units in [
                None,
                Some(0),
                Some(99_999),
                Some(100_000),
                Some(1_000_000),
                Some(100_000_000),
                Some(100_000_001),
                Some(u64::MAX),
            ] {
                for slippage in [
                    0,
                    1,
                    3_000,
                    3_001,
                    u64::from(u16::MAX),
                    u64::from(u16::MAX) + 1,
                    u64::MAX,
                ] {
                    let mut command = f.command.clone();
                    command.wallet_id = wallet.clone();
                    command.route.mint = mint;
                    command.amount = units
                        .map(|lamports| TradeAmount::BuySol { lamports })
                        .unwrap_or(TradeAmount::SellPercent { percent: 100 });
                    command.execution.slippage_bps = slippage;
                    let output = match crate::application::ordinary_sol_buy::intent(&command)
                        .map(crate::adapters::sdk::ordinary_sol_buy::quote_request)
                    {
                        Ok(request) => {
                            serde_json::json!({"request":{"wallet_id":request.wallet_id,"mint":request.mint,
                            "amount_lamports":request.amount_lamports,"slippage_bps":request.slippage_bps},
                            "validation_error":crate::aggregator_quote::validate_request(&request).err().map(|error|error.to_string())})
                        }
                        Err(error) => serde_json::json!({"error":error.to_string()}),
                    };
                    rows.push(serde_json::json!({"wallet":wallet,"mint":mint.to_string(),"units":units,"slippage":slippage,"output":output}));
                }
            }
        }
    }
    assert_eq!(rows.len(), 448);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&rows).unwrap())),
        "26eb7fea8587588a6bf1bf99c7ee1bd41c2102b6b219d6c55432d39266f448b1"
    );
}
