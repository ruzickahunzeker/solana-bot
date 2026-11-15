use super::*;
use crate::{
    config::{TradePersistenceConfig, TradePersistenceMode},
    domain::trade::{Address, TradeAmount},
    idempotency::IdempotencyStore,
    models::TradeRequest,
    persistence::TradePersistence,
    state::TradeStore,
    AppState,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[derive(Default)]
struct Trace {
    resolve: AtomicUsize,
    lookup: AtomicUsize,
    parse: AtomicUsize,
    dropped: AtomicUsize,
}
struct Identity(Arc<Trace>);
impl Drop for Identity {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl LoadedWalletIdentity for Identity {
    fn pubkey(&self) -> Address {
        Address::from_bytes([9; 32])
    }
}
struct Directory {
    available: AtomicBool,
    trace: Arc<Trace>,
}
impl LoadedWalletDirectory for Directory {
    fn resolve_id(&self, requested: Option<&str>) -> String {
        self.trace.resolve.fetch_add(1, Ordering::SeqCst);
        requested.unwrap_or("default-wallet").into()
    }
    fn lookup(&self, _: &str) -> Option<Box<dyn LoadedWalletIdentity>> {
        self.trace.lookup.fetch_add(1, Ordering::SeqCst);
        self.available
            .load(Ordering::SeqCst)
            .then(|| Box::new(Identity(self.trace.clone())) as Box<dyn LoadedWalletIdentity>)
    }
}
struct Parser(Arc<Trace>);
impl TradeAddressParser for Parser {
    fn parse(&self, value: &str) -> anyhow::Result<Address> {
        self.0.parse.fetch_add(1, Ordering::SeqCst);
        Ok(Address::from_bytes(
            bs58::decode(value)
                .into_vec()?
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected 32 bytes"))?,
        ))
    }
}
struct Fixture {
    state: AppState,
    directory: Directory,
    parser: Parser,
    trace: Arc<Trace>,
}
impl Fixture {
    async fn new(mode: TradePersistenceMode) -> Self {
        let mut state = crate::http_contract_tests::state().await;
        state.persistence = Arc::new(
            TradePersistence::new(&TradePersistenceConfig {
                mode,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 100,
            })
            .await
            .unwrap(),
        );
        let trace = Arc::new(Trace::default());
        Self {
            state,
            directory: Directory {
                available: AtomicBool::new(true),
                trace: trace.clone(),
            },
            parser: Parser(trace.clone()),
            trace,
        }
    }
    fn service(
        &self,
        broadcast: BroadcastPolicy,
    ) -> TradeIntakeService<'_, TradePersistence, TradeStore, IdempotencyStore> {
        let mut settings = self.state.config.trade_intake_settings();
        settings.broadcast = broadcast;
        TradeIntakeService::new(
            self.state.persistence.as_ref(),
            self.state.trades.as_ref(),
            self.state.idempotency.as_ref(),
            &self.directory,
            &self.parser,
            settings,
            crate::state::new_request_id,
            crate::state::now_ms,
        )
    }
}
fn request() -> TradeRequest {
    serde_json::from_value(json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111",
        "amount_lamports":17,"simulate":false,"source":"metadata only","source_signature":"not a parsed address",
        "idempotency_key":" literal-key "})).unwrap()
}
fn modes() -> [TradePersistenceMode; 3] {
    [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ]
}

async fn stored_snapshot(f: &Fixture, request_id: &str) -> (String, String) {
    let pool = f.state.persistence.pool_for_tests();
    // Shadow accepts into its existing queue; observe the write instead of assuming it is done.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let row: Option<(String, String)> =
                sqlx::query_as("SELECT request_hash,request_json FROM trades WHERE request_id=?")
                    .bind(request_id)
                    .fetch_optional(&pool)
                    .await
                    .unwrap();
            if let Some(row) = row {
                break row;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the original accepted snapshot must be persisted")
}

#[tokio::test]
async fn ordinary_intake_resolves_forced_wallet_preserving_original_v1_bytes_and_lease_across_modes(
) {
    for mode in modes() {
        let f = Fixture::new(mode).await;
        let mut request = request();
        let outcome = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut request, Some(" forced-wallet ".into()))
            .await
            .unwrap();
        assert_eq!(f.trace.resolve.load(Ordering::SeqCst), 0);
        assert_eq!(request.wallet_id.as_deref(), Some(" forced-wallet "));
        assert_eq!(outcome.wallet_id, " forced-wallet ");
        assert_eq!(
            outcome.record.wallet_pubkey,
            Address::from_bytes([9; 32]).to_string()
        );
        assert_eq!(outcome.record.status, TradeStatus::Accepted);
        let IntakeDecision::Created(execution) = &outcome.decision else {
            panic!("one original admitted input required")
        };
        assert_eq!(
            execution.command().amount,
            TradeAmount::BuySol { lamports: 17 }
        );
        assert_eq!(execution.command().idempotency_key, " literal-key ");
        let normalized = TradeValidator::new(&f.parser)
            .validate(
                &request.validation_input(),
                &f.state.config.trade_validation_settings(),
            )
            .unwrap();
        let hash = crate::adapters::persistence::trade_snapshot::request_hash(
            " forced-wallet ",
            &request,
            &normalized,
        )
        .unwrap();
        assert_eq!(execution.reservation().request_hash(), hash);
        if mode != TradePersistenceMode::Memory {
            let row = stored_snapshot(&f, &outcome.record.request_id).await;
            assert_eq!(row, (hash, serde_json::to_string(&request).unwrap()));
        }
        assert_eq!(f.trace.dropped.load(Ordering::SeqCst), 0);
        drop(outcome);
        assert_eq!(f.trace.dropped.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn ordinary_intake_missing_wallet_precedes_parsing_and_does_not_create_an_order() {
    for mode in modes() {
        let f = Fixture::new(mode).await;
        f.directory.available.store(false, Ordering::SeqCst);
        let mut request = request();
        request.mint = "not-a-pubkey".into();
        let error = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut request, None)
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind, IntakeFailureKind::UnknownWallet);
        assert_eq!(error.message, "wallet_id default-wallet not found");
        assert_eq!(request.wallet_id.as_deref(), Some("default-wallet"));
        assert_eq!(f.trace.resolve.load(Ordering::SeqCst), 1);
        assert_eq!(f.trace.lookup.load(Ordering::SeqCst), 1);
        assert_eq!(f.trace.parse.load(Ordering::SeqCst), 0);
        assert!(f.state.trades.recent(100).is_empty());
    }
}

#[tokio::test]
async fn ordinary_intake_validation_precedes_policy_and_neither_rejection_reserves_normal_key() {
    for mode in modes() {
        let f = Fixture::new(mode).await;
        let mut invalid = request();
        invalid.mint = "invalid".into();
        let rejected = f
            .service(BroadcastPolicy::Disabled)
            .accept(&mut invalid, None)
            .await
            .unwrap();
        assert!(matches!(rejected.decision, IntakeDecision::Rejected));
        assert_eq!(
            rejected.record.error.as_deref(),
            Some("mint must be a valid pubkey")
        );
        let mut valid = request();
        let forbidden = f
            .service(BroadcastPolicy::Disabled)
            .accept(&mut valid, None)
            .await
            .err()
            .unwrap();
        assert_eq!(forbidden.kind, IntakeFailureKind::Forbidden);
        assert_eq!(
            forbidden.message,
            crate::domain::execution_policy::BROADCAST_DISABLED_ERROR
        );
        assert_eq!(f.state.trades.recent(100).len(), 1);
        let accepted = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut valid, None)
            .await
            .unwrap();
        assert!(matches!(accepted.decision, IntakeDecision::Created(_)));
        assert_eq!(f.state.trades.recent(100).len(), 2);
    }
}

#[tokio::test]
async fn ordinary_intake_duplicate_and_conflict_keep_original_record_without_another_execution_input(
) {
    for mode in modes() {
        let f = Fixture::new(mode).await;
        let mut request = request();
        let original = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut request, None)
            .await
            .unwrap();
        request.source = Some("changed metadata".into());
        let duplicate = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut request, None)
            .await
            .unwrap();
        assert!(matches!(duplicate.decision, IntakeDecision::Duplicate));
        assert_eq!(duplicate.record.request_id, original.record.request_id);
        assert!(duplicate.record.duplicate);
        request.amount_lamports = Some(18);
        let error = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut request, None)
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind, IntakeFailureKind::Conflict);
        assert_eq!(
            error.message,
            "idempotency key was already used for a different trade"
        );
        let rows = f.state.trades.recent(100);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, original.record.request_id);
        assert_eq!(rows[0].status, TradeStatus::Accepted);
        if mode != TradePersistenceMode::Memory {
            let (_, json) = stored_snapshot(&f, &original.record.request_id).await;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&json).unwrap()["source"],
                "metadata only"
            );
        }
    }
}

#[tokio::test]
async fn ordinary_intake_storage_failures_preserve_durable_shadow_and_best_effort_rejection_rules()
{
    for mode in [TradePersistenceMode::Shadow, TradePersistenceMode::Durable] {
        let f = Fixture::new(mode).await;
        f.state.persistence.pool_for_tests().close().await;
        let mut valid = request();
        let result = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut valid, None)
            .await;
        if mode == TradePersistenceMode::Durable {
            let error = result.err().unwrap();
            assert_eq!(error.kind, IntakeFailureKind::Unavailable);
            assert_eq!(
                error.message,
                "trade persistence unavailable; trade was not started"
            );
            assert!(f.state.trades.recent(100).is_empty());
        } else {
            assert!(matches!(
                result.unwrap().decision,
                IntakeDecision::Created(_)
            ));
            assert!(matches!(
                f.service(BroadcastPolicy::Enabled)
                    .accept(&mut valid, None)
                    .await
                    .unwrap()
                    .decision,
                IntakeDecision::Duplicate
            ));
        }
        let mut invalid = request();
        invalid.mint = "invalid".into();
        let rejected = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut invalid, None)
            .await
            .unwrap();
        assert!(matches!(rejected.decision, IntakeDecision::Rejected));
        assert_eq!(rejected.record.status, TradeStatus::Rejected);
    }
}

#[tokio::test]
async fn ordinary_intake_all_existing_normalized_samples_match_original_dto_command_mapping() {
    let mut f = Fixture::new(TradePersistenceMode::Memory).await;
    let config = Arc::get_mut(&mut f.state.config).unwrap();
    config.stonkfun_via_sol.enabled = true;
    config.default_tip_sol = 0.0001;
    config.pumpapi.default_priority_fee_sol = 0.0002;
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/trade_normalization_v1.json"
    ))
    .unwrap();
    for (index, case) in fixtures["accepted"].as_array().unwrap().iter().enumerate() {
        let mut value = json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111","idempotency_key":format!(" intake-sample-{index} ")});
        value
            .as_object_mut()
            .unwrap()
            .extend(case["patch"].as_object().unwrap().clone());
        let mut request: TradeRequest = serde_json::from_value(value).unwrap();
        let normalized = TradeValidator::new(&f.parser)
            .validate(
                &request.validation_input(),
                &f.state.config.trade_validation_settings(),
            )
            .unwrap();
        let expected = request
            .to_command(
                request
                    .wallet_id
                    .clone()
                    .unwrap_or_else(|| "default-wallet".into()),
                &normalized,
                f.state.config.trade_command_defaults(),
            )
            .unwrap();
        let outcome = f
            .service(BroadcastPolicy::Enabled)
            .accept(&mut request, None)
            .await
            .unwrap();
        let IntakeDecision::Created(actual) = outcome.decision else {
            panic!("sample {} must remain admitted", case["name"])
        };
        let actual = actual.command();
        assert_eq!(actual.wallet_id, expected.wallet_id);
        assert_eq!(actual.idempotency_key, expected.idempotency_key);
        assert_eq!(actual.route.pool, expected.route.pool);
        assert_eq!(actual.route.mint, expected.route.mint);
        assert_eq!(actual.route.quote_mint, expected.route.quote_mint);
        assert_eq!(actual.route.pool_id, expected.route.pool_id);
        assert_eq!(actual.route.sol_hop_pool, expected.route.sol_hop_pool);
        assert_eq!(actual.route.sol_hop_pool_id, expected.route.sol_hop_pool_id);
        assert_eq!(actual.amount, expected.amount);
        assert_eq!(actual.execution.method, expected.execution.method);
        assert_eq!(
            actual.execution.slippage_bps,
            expected.execution.slippage_bps
        );
        assert_eq!(
            actual.execution.tip_sol.to_bits(),
            expected.execution.tip_sol.to_bits()
        );
        assert_eq!(actual.execution.mode, expected.execution.mode);
        assert_eq!(actual.execution.use_mev, expected.execution.use_mev);
        assert_eq!(
            actual.execution.mev_provider,
            expected.execution.mev_provider
        );
        assert_eq!(actual.aggregator_sol, expected.aggregator_sol);
        assert_eq!(
            actual.pumpapi_priority_fee_sol.to_bits(),
            expected.pumpapi_priority_fee_sol.to_bits()
        );
    }
}

#[tokio::test]
async fn ordinary_intake_preserves_original_acceptance_and_rejection_log_targets() {
    use std::sync::Mutex;
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::{layer::Context, prelude::*, Layer};

    struct Targets(Arc<Mutex<Vec<(tracing::Level, &'static str)>>>);
    impl<S: tracing::Subscriber> Layer<S> for Targets {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            let metadata = event.metadata();
            if metadata.fields().field("idempotency_key").is_some() {
                self.0
                    .lock()
                    .unwrap()
                    .push((*metadata.level(), metadata.target()));
            }
        }
    }

    let f = Fixture::new(TradePersistenceMode::Memory).await;
    let targets = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Targets(targets.clone()));
    async {
        let service = f.service(BroadcastPolicy::Enabled);
        let mut valid = request();
        assert!(matches!(
            service.accept(&mut valid, None).await.unwrap().decision,
            IntakeDecision::Created(_)
        ));
        let mut invalid = request();
        invalid.mint = "invalid".into();
        assert!(matches!(
            service.accept(&mut invalid, None).await.unwrap().decision,
            IntakeDecision::Rejected
        ));
    }
    .with_subscriber(subscriber)
    .await;
    let original_target = concat!(env!("CARGO_CRATE_NAME"), "::api::handlers::trades");
    assert_eq!(
        targets.lock().unwrap().as_slice(),
        &[
            (tracing::Level::INFO, original_target),
            (tracing::Level::WARN, original_target)
        ]
    );
}
