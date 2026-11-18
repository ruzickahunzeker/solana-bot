use super::*;
mod journaled;
use crate::{
    application::{
        legacy_quote_submission, ordinary_sol_buy,
        ports::{
            ordinary_sol_buy::{OrdinarySolBuyPipeline, SolBuyIntent},
            quote_execution::QuoteSubmissionBackend,
        },
        quote_submission_types::{QuoteSubmissionCommand, QuoteSubmissionResult},
    },
    domain::{
        quote::{BoundQuote, QuoteEstimate, QuotePreflight},
        submission::SignedSubmission,
    },
};

const ORIGINAL: &str = "synthetic-original-aggregate-signature";
const ORIGINAL_BYTES: [u8; 3] = [17, 23, 42];

impl crate::application::ports::reconciliation::SignatureStatusReader for LegacyWallet {
    async fn lookup_signature(
        &self,
        signature: &str,
    ) -> anyhow::Result<Option<crate::domain::reconciliation::ChainObservation>> {
        assert_eq!(signature, ORIGINAL);
        self.evidence
            .events
            .lock()
            .unwrap()
            .push("lookup_signature");
        Ok(self.evidence.observation.lock().unwrap().clone())
    }
}

#[derive(Clone, Copy)]
enum Receipt {
    Match,
    Different,
    Error(&'static str),
    WalletChanged,
}

struct Evidence {
    events: Mutex<Vec<&'static str>>,
    receipt: Receipt,
    observation: Mutex<Option<crate::domain::reconciliation::ChainObservation>>,
    anchors: Mutex<
        Vec<(
            String,
            String,
            String,
            String,
            crate::application::ports::ordinary_journal::TradeJournalMode,
        )>,
    >,
}

struct LegacyBackend {
    trace: Arc<Trace>,
    evidence: Arc<Evidence>,
}
struct LegacyWallet {
    trace: Arc<Trace>,
    evidence: Arc<Evidence>,
}

impl OrdinarySolBuyBackend for LegacyBackend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        self.trace
            .aggregate_resolves
            .lock()
            .unwrap()
            .push(wallet_id.into());
        Some(Box::new(LegacyWallet {
            trace: self.trace.clone(),
            evidence: self.evidence.clone(),
        }))
    }
}

impl OrdinarySolBuyWallet for LegacyWallet {
    fn buy<'a>(
        &'a self,
        execution: &'a crate::application::trade_admission::AdmittedTrade,
        owner: Option<&'a crate::application::ports::ordinary_claims::ExecutionOwner>,
    ) -> crate::application::ports::ordinary_sol_buy::SolBuyExecutionFuture<'a> {
        let command = execution.command();
        Box::pin(async move {
            if !command.execution.mode.simulate() {
                owner
                    .expect("live original owner")
                    .ensure_binding(execution)?;
            }
            let anchor = execution.reservation();
            self.evidence.anchors.lock().unwrap().push((
                anchor.request_id().into(),
                anchor.wallet_id().into(),
                anchor.idempotency_key().into(),
                anchor.request_hash().into(),
                anchor.mode(),
            ));
            let current = if matches!(self.evidence.receipt, Receipt::WalletChanged) {
                Address::from_bytes([2; 32])
            } else {
                Address::from_bytes([1; 32])
            };
            execution.reservation().ensure_wallet("alpha", current)?;
            self.trace
                .aggregate_commands
                .lock()
                .unwrap()
                .push(command.clone());
            ordinary_sol_buy::execute(self, command)
                .await
                .map(crate::application::ports::ordinary_sol_buy::SolBuyExecutionOutcome::Legacy)
        })
    }
}

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap()
}

impl LegacyWallet {
    fn event(&self, event: &'static str) {
        self.evidence.events.lock().unwrap().push(event);
    }
}

impl OrdinarySolBuyPipeline for LegacyWallet {
    type Request = SolBuyIntent;

    fn map_request(&self, input: SolBuyIntent) -> Self::Request {
        self.event("map");
        input
    }
    fn validate_request(&self, request: &Self::Request) -> anyhow::Result<()> {
        self.event("validate");
        assert_eq!(request.wallet_id, "alpha");
        assert_eq!(request.amount_lamports, 1_000_000);
        Ok(())
    }
    async fn quote(&self, request: &Self::Request) -> anyhow::Result<QuoteEstimate> {
        self.event("quote");
        let mut quote = serde_json::from_value::<BoundQuote>(fixture()["bound"].clone())?.quote;
        quote.input_mint = crate::domain::trade::WSOL_MINT.into();
        quote.output_mint = request.mint.to_string();
        quote.amount_lamports = request.amount_lamports;
        quote.route[0].input_mint = quote.input_mint.clone();
        quote.route[0].output_mint = crate::domain::trade::USDC_MINT.into();
        quote.route[1].input_mint = crate::domain::trade::USDC_MINT.into();
        quote.route[1].output_mint = quote.output_mint.clone();
        Ok(quote)
    }
    fn bind(&self, request: &Self::Request, quote: QuoteEstimate) -> anyhow::Result<BoundQuote> {
        self.event("bind");
        assert_eq!(quote.output_mint, request.mint.to_string());
        assert_eq!(quote.amount_lamports, request.amount_lamports);
        let mut bound: BoundQuote = serde_json::from_value(fixture()["bound"].clone())?;
        bound.wallet_id = request.wallet_id.clone();
        bound.wallet_pubkey = Address::from_bytes([1; 32]).to_string();
        bound.quote_slippage_bps = request.slippage_bps;
        bound.quote = quote;
        Ok(bound)
    }
    async fn submit(
        &self,
        bound: &BoundQuote,
        request: &QuoteSubmissionCommand,
    ) -> anyhow::Result<QuoteSubmissionResult> {
        self.event("submit");
        legacy_quote_submission::execute(self, bound, request).await
    }
}

impl QuoteSubmissionBackend for LegacyWallet {
    type Prepared = [u8; 3];

    fn ensure_live_allowed(&self) -> anyhow::Result<()> {
        self.event("policy");
        Ok(())
    }
    async fn prepare(
        &self,
        bound: &BoundQuote,
    ) -> anyhow::Result<(Self::Prepared, QuotePreflight)> {
        self.event("prepare");
        let mut preflight: QuotePreflight = serde_json::from_value(fixture()["preflight"].clone())?;
        preflight.wallet_id = bound.wallet_id.clone();
        preflight.quote_id = bound.quote_id.clone();
        Ok((ORIGINAL_BYTES, preflight))
    }
    fn sign(&self, prepared: Self::Prepared) -> anyhow::Result<SignedSubmission> {
        self.event("sign");
        assert_eq!(prepared, ORIGINAL_BYTES);
        // This records a fake signer call; no cryptographic signing or key material exists.
        Ok(SignedSubmission {
            signature: ORIGINAL.into(),
            bytes: prepared.to_vec(),
        })
    }
    async fn broadcast(&self, signed: &SignedSubmission) -> anyhow::Result<String> {
        self.event("broadcast");
        assert_eq!(signed.signature, ORIGINAL);
        assert_eq!(signed.bytes, ORIGINAL_BYTES);
        match self.evidence.receipt {
            Receipt::Match => Ok(ORIGINAL.into()),
            Receipt::Different => Ok("different-not-authorized-receipt".into()),
            Receipt::Error(message) => anyhow::bail!(message),
            Receipt::WalletChanged => {
                panic!("changed wallet must stop before quote/sign/broadcast")
            }
        }
    }
}

impl crate::application::ports::quote_execution::OrdinaryQuoteBackend for LegacyWallet {
    fn prepared_message(&self, prepared: &Self::Prepared) -> anyhow::Result<Vec<u8>> {
        Ok(prepared.to_vec())
    }
    fn signed_message(&self, signed: &SignedSubmission) -> anyhow::Result<Vec<u8>> {
        Ok(signed.bytes.clone())
    }
}

async fn with_legacy_backend(
    mode: TradePersistenceMode,
    receipt: Receipt,
) -> (AppState, Arc<Trace>, Arc<Evidence>) {
    let (mut state, trace) = with_backend(mode).await;
    let evidence = Arc::new(Evidence {
        events: Mutex::default(),
        receipt,
        observation: Mutex::new(None),
        anchors: Mutex::default(),
    });
    state.ordinary_sol_buy = Arc::new(LegacyBackend {
        trace: trace.clone(),
        evidence: evidence.clone(),
    });
    (state, trace, evidence)
}

#[tokio::test]
async fn ordinary_aggregate_anchor_matches_original_v1_hash_and_durable_parent_without_a_second_key(
) {
    use crate::application::ports::ordinary_journal::TradeJournalMode;
    for (mode, expected_mode) in [
        (TradePersistenceMode::Memory, TradeJournalMode::Memory),
        (TradePersistenceMode::Durable, TradeJournalMode::Durable),
    ] {
        let (state, trace, evidence) = with_legacy_backend(mode, Receipt::Match).await;
        let request = aggregate_request();
        let mut dto: crate::models::TradeRequest = serde_json::from_value(request.clone()).unwrap();
        dto.wallet_id = Some("alpha".into());
        let normalized = crate::validation::validate_trade(&dto, &state.config).unwrap();
        let expected_hash =
            crate::adapters::persistence::trade_snapshot::request_hash("alpha", &dto, &normalized)
                .unwrap();
        let app = http::router(state.clone());
        let first = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(request.clone()),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(first["status"], "submitted");
        let id = first["request_id"].as_str().unwrap();
        assert_eq!(
            *evidence.anchors.lock().unwrap(),
            [(
                id.into(),
                "alpha".into(),
                " literal-aggregate-unknown ".into(),
                expected_hash.clone(),
                expected_mode
            )]
        );
        if mode == TradePersistenceMode::Durable {
            let row: (String, String, String) = sqlx::query_as(
                "SELECT request_hash, wallet_id, idempotency_key FROM trades WHERE request_id=?",
            )
            .bind(id)
            .fetch_one(&state.persistence.pool_for_tests())
            .await
            .unwrap();
            assert_eq!(
                row,
                (
                    expected_hash,
                    "alpha".into(),
                    " literal-aggregate-unknown ".into()
                )
            );
        }
        let replay = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(request),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(replay["request_id"], first["request_id"]);
        assert_eq!(replay["duplicate"], true);
        assert_eq!(evidence.anchors.lock().unwrap().len(), 1);
        assert_one_submission(&trace, &evidence);
    }
}

#[tokio::test]
async fn ordinary_aggregate_wallet_remap_stops_before_quote_and_does_not_release_original_key() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let (state, trace, evidence) = with_legacy_backend(mode, Receipt::WalletChanged).await;
        let app = http::router(state);
        let request = aggregate_request();
        let first = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(request.clone()),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(first["status"], "failed");
        assert_eq!(first["signature"], Value::Null);
        assert_eq!(
            first["error"],
            "ordinary execution wallet identity changed after admission"
        );
        let replay = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(request),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(replay["request_id"], first["request_id"]);
        assert_eq!(replay["duplicate"], true);
        assert!(evidence.events.lock().unwrap().is_empty());
        assert_eq!(evidence.anchors.lock().unwrap().len(), 1);
        assert_eq!(*trace.aggregate_resolves.lock().unwrap(), ["alpha"]);
        assert!(trace.aggregate_commands.lock().unwrap().is_empty());
        assert!(trace.commands.lock().unwrap().is_empty());
    }
}

fn aggregate_request() -> Value {
    let mut request = buy(" literal-aggregate-unknown ");
    request["pool"] = json!("pump-amm");
    request["quote_mint"] = json!(crate::domain::trade::USDC_MINT);
    request
}

fn assert_one_submission(trace: &Trace, evidence: &Evidence) {
    assert_eq!(
        *evidence.events.lock().unwrap(),
        [
            "map",
            "validate",
            "quote",
            "bind",
            "submit",
            "policy",
            "prepare",
            "policy",
            "sign",
            "broadcast",
        ]
    );
    assert_eq!(trace.aggregate_commands.lock().unwrap().len(), 1);
    assert_eq!(*trace.aggregate_resolves.lock().unwrap(), ["alpha"]);
    assert!(trace.commands.lock().unwrap().is_empty());
    assert!(trace.resolves.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ordinary_aggregate_claim_storage_failure_preserves_pending_http_order_and_key() {
    let (mut state, trace, evidence) =
        with_legacy_backend(TradePersistenceMode::Durable, Receipt::Match).await;
    state.edit_fixture_config(|config| config.trade_response_wait_ms = 30);
    let pool = state.persistence.pool_for_tests();
    sqlx::raw_sql("CREATE TRIGGER deny_ordinary_claim BEFORE INSERT ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'synthetic claim storage failure'); END;")
        .execute(&pool).await.unwrap();
    let app = http::router(state.clone());
    let request = aggregate_request();
    let first = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
        StatusCode::ACCEPTED,
    )
    .await;
    assert_eq!(first["status"], "accepted");
    assert_eq!(first["signature"], Value::Null);
    assert_eq!(first["error"], Value::Null);
    let replay = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(replay["request_id"], first["request_id"]);
    assert_eq!(replay["duplicate"], true);
    assert_eq!(replay["status"], "accepted");
    let mut other = request;
    other["amount_sol"] = json!(0.002);
    error_body(
        send(&app, "POST", "/v1/trades", LOCAL, Some(TOKEN), Some(other)).await,
        StatusCode::CONFLICT,
        "idempotency key was already used for a different trade",
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM trades")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(trace.aggregate_resolves.lock().unwrap().is_empty());
    assert!(evidence.events.lock().unwrap().is_empty());
    assert!(evidence.anchors.lock().unwrap().is_empty());
    pool.close().await;
}

#[tokio::test]
async fn ordinary_aggregate_unknown_retains_original_receipt_across_http_queries_duplicates_and_modes(
) {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        for (receipt, message) in [
            (
                Receipt::Error("sendTransaction failed: https://rpc.example.invalid/private-key"),
                "sendTransaction failed: <redacted-url>",
            ),
            (
                Receipt::Error("submit signature not visible"),
                "submit signature not visible",
            ),
            (
                Receipt::Error("landed with transaction error"),
                "landed with transaction error",
            ),
            (
                Receipt::Different,
                "RPC returned a different signature; reconcile original signature",
            ),
        ] {
            let (state, trace, evidence) = with_legacy_backend(mode, receipt).await;
            let app = http::router(state.clone());
            let request = aggregate_request();
            let first = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(request.clone()),
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(first.as_object().unwrap().len(), 8);
            assert_eq!(first["status"], "unknown");
            assert_eq!(first["signature"], ORIGINAL);
            assert_eq!(first["error"], message);
            assert_eq!(first["duplicate"], false);
            let id = first["request_id"].as_str().unwrap();
            let record = terminal(&app, id).await;
            assert_eq!(record["status"], "unknown");
            assert_eq!(record["signature"], ORIGINAL);
            assert_eq!(record["error"], message);
            for _ in 0..2 {
                let replay = json_body(
                    send(
                        &app,
                        "POST",
                        "/v1/trades",
                        LOCAL,
                        Some(TOKEN),
                        Some(request.clone()),
                    )
                    .await,
                    StatusCode::OK,
                )
                .await;
                assert_eq!(replay["request_id"], first["request_id"]);
                assert_eq!(replay["duplicate"], true);
                assert_eq!(replay["status"], "unknown");
                assert_eq!(replay["signature"], ORIGINAL);
                assert_eq!(replay["error"], message);
            }
            let mut different = request;
            different["amount_sol"] = json!(0.002);
            error_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(different),
                )
                .await,
                StatusCode::CONFLICT,
                "idempotency key was already used for a different trade",
            )
            .await;
            if mode != TradePersistenceMode::Memory {
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        if let Some(saved) = state.persistence.get(id).await.unwrap() {
                            if saved.status == crate::domain::trade::TradeStatus::Unknown {
                                assert_eq!(saved.signature.as_deref(), Some(ORIGINAL));
                                assert_eq!(saved.error.as_deref(), Some(message));
                                break;
                            }
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
            assert_one_submission(&trace, &evidence);
            assert_eq!(state.wallets.wallet_count(), 0);
        }
    }
}

#[tokio::test]
async fn ordinary_aggregate_unknown_file_reopen_keeps_original_order_without_requoting_or_resigning(
) {
    let dir = std::env::temp_dir().join(format!(
        "ordinary-aggregate-unknown-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 100,
    };
    let (mut state, trace, evidence) =
        with_legacy_backend(TradePersistenceMode::Memory, Receipt::Different).await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let pool = state.persistence.pool_for_tests();
    let app = http::router(state);
    let request = aggregate_request();
    let first = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(first["status"], "unknown");
    assert_eq!(first["signature"], ORIGINAL);
    assert_one_submission(&trace, &evidence);
    drop(app);
    pool.close().await;

    let (mut state, trace, evidence) =
        with_legacy_backend(TradePersistenceMode::Memory, Receipt::Match).await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let pool = state.persistence.pool_for_tests();
    let app = http::router(state);
    let saved = terminal(&app, first["request_id"].as_str().unwrap()).await;
    assert_eq!(saved["status"], "unknown");
    assert_eq!(saved["signature"], ORIGINAL);
    assert_eq!(saved["error"], first["error"]);
    let replay = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(replay["request_id"], first["request_id"]);
    assert_eq!(replay["status"], "unknown");
    assert_eq!(replay["signature"], ORIGINAL);
    assert_eq!(replay["duplicate"], true);
    assert!(trace.aggregate_commands.lock().unwrap().is_empty());
    assert!(trace.aggregate_resolves.lock().unwrap().is_empty());
    assert!(trace.commands.lock().unwrap().is_empty());
    assert!(evidence.events.lock().unwrap().is_empty());
    drop(app);
    pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
