use super::*;
use crate::domain::trade::WSOL_MINT;
use crate::{
    adapters::aggregator::quotes::AggregatorQuoteSource,
    api::dto::sol_buy::SolBuyQuoteRequest,
    application::{
        ports::{
            quote_cache::{QuoteBinding, QuoteCache},
            sol_buy_quotes::{SolBuyQuoteInput, SolBuyQuoteSource},
            wallet_identity::{LoadedWalletDirectory, LoadedWalletIdentity},
        },
        sol_buy_quotes::SolBuyQuoteService,
    },
    domain::{
        execution_policy::BroadcastPolicy,
        quote::{BoundQuote, QuoteEstimate, QuoteRouteLeg},
        trade::Address,
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

#[derive(Default)]
struct ReadTrace {
    events: Mutex<Vec<&'static str>>,
    requests: Mutex<Vec<SolBuyQuoteRequest>>,
    lookups: Mutex<Vec<String>>,
    wallets: Mutex<BTreeMap<String, Address>>,
    fetch_error: AtomicBool,
    bind_error: AtomicBool,
    read_error: Mutex<Option<&'static str>>,
}
impl ReadTrace {
    fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
    fn clear(&self) {
        self.events.lock().unwrap().clear();
        self.requests.lock().unwrap().clear();
        self.lookups.lock().unwrap().clear();
    }
}
struct Source(Arc<ReadTrace>);
struct Directory(Arc<ReadTrace>);
struct Identity {
    trace: Arc<ReadTrace>,
    address: Address,
}
struct Cache {
    trace: Arc<ReadTrace>,
    inner: Arc<crate::aggregator_quote::QuoteStore>,
}

fn estimate(request: &SolBuyQuoteRequest) -> QuoteEstimate {
    QuoteEstimate {
        input_mint: WSOL_MINT.into(),
        output_mint: request.mint.clone(),
        amount_lamports: request.amount_lamports,
        expected_output_units: u64::MAX,
        minimum_output_units: 15_679_732_462_653_118_872,
        price_impact_percent: "0.01234567890123456789".into(),
        route: vec![QuoteRouteLeg {
            label: "synthetic-read-only".into(),
            input_mint: WSOL_MINT.into(),
            output_mint: request.mint.clone(),
        }],
        executable: false,
    }
}
impl SolBuyQuoteSource for Source {
    type Request = SolBuyQuoteRequest;
    fn map_request(&self, input: SolBuyQuoteInput<'_>) -> Self::Request {
        self.0.events.lock().unwrap().push("map");
        let request = AggregatorQuoteSource.map_request(input);
        self.0.requests.lock().unwrap().push(request.clone());
        request
    }
    fn validate(&self, request: &Self::Request) -> anyhow::Result<()> {
        self.0.events.lock().unwrap().push("validate");
        AggregatorQuoteSource.validate(request)
    }
    async fn fetch(&self, request: &Self::Request) -> anyhow::Result<QuoteEstimate> {
        self.0.events.lock().unwrap().push("fetch");
        if self.0.fetch_error.load(Ordering::SeqCst) {
            anyhow::bail!("quote source https://example.invalid/secret-key timed out");
        }
        Ok(estimate(request))
    }
}
impl LoadedWalletDirectory for Directory {
    fn resolve_id(&self, _: Option<&str>) -> String {
        panic!("quote queries require explicit literal wallet IDs")
    }
    fn lookup(&self, id: &str) -> Option<Box<dyn LoadedWalletIdentity>> {
        self.0.events.lock().unwrap().push("lookup");
        self.0.lookups.lock().unwrap().push(id.into());
        self.0.wallets.lock().unwrap().get(id).map(|address| {
            Box::new(Identity {
                trace: self.0.clone(),
                address: *address,
            }) as Box<dyn LoadedWalletIdentity>
        })
    }
}
impl LoadedWalletIdentity for Identity {
    fn pubkey(&self) -> Address {
        self.trace.events.lock().unwrap().push("pubkey");
        self.address
    }
}
impl Drop for Identity {
    fn drop(&mut self) {
        self.trace.events.lock().unwrap().push("drop");
    }
}
impl QuoteCache for Cache {
    fn bind(&self, binding: QuoteBinding<'_>, quote: QuoteEstimate) -> anyhow::Result<BoundQuote> {
        self.trace.events.lock().unwrap().push("bind");
        if self.trace.bind_error.load(Ordering::SeqCst) {
            anyhow::bail!("quote capacity reached; retry after active quotes expire");
        }
        self.inner.bind(binding, quote)
    }
    fn read(&self, id: &str, wallet: &str, payer: &str) -> anyhow::Result<BoundQuote> {
        self.trace.events.lock().unwrap().push("read");
        if let Some(error) = *self.trace.read_error.lock().unwrap() {
            anyhow::bail!("{error}");
        }
        self.inner.read(id, wallet, payer)
    }
}
async fn with_quote_sources() -> (AppState, Arc<ReadTrace>) {
    let mut state = state().await;
    let trace = Arc::new(ReadTrace::default());
    trace
        .wallets
        .lock()
        .unwrap()
        .insert("fixture-wallet".into(), Address::from_bytes([3; 32]));
    state.quote_queries = Arc::new(SolBuyQuoteService::new(
        Source(trace.clone()),
        Arc::new(Directory(trace.clone())),
        Arc::new(Cache {
            trace: trace.clone(),
            inner: state.aggregator_quotes.clone(),
        }),
    ));
    (state, trace)
}

fn quote_request() -> Value {
    json!({"wallet_id":"fixture-wallet","mint":"11111111111111111111111111111111","amount_lamports":1_000_000,"slippage_bps":1500})
}

#[tokio::test]
async fn sol_buy_http_parameter_baseline_precedes_missing_wallet_and_keeps_get_lookup_first() {
    let app = http::router(state().await);
    for (patch, message) in [
        (
            json!({"wallet_id":" ","mint":"invalid","amount_lamports":0,"slippage_bps":0}),
            "wallet_id is required and must be at most 128 characters",
        ),
        (
            json!({"mint":"invalid","amount_lamports":0,"slippage_bps":0}),
            "mint must be a valid pubkey",
        ),
        (
            json!({"mint":WSOL_MINT,"amount_lamports":0,"slippage_bps":0}),
            "output mint must differ from WSOL",
        ),
        (
            json!({"amount_lamports":99_999,"slippage_bps":0}),
            "amount_lamports must be between 0.0001 and 0.1 SOL",
        ),
        (
            json!({"amount_lamports":100_000_001}),
            "amount_lamports must be between 0.0001 and 0.1 SOL",
        ),
        (
            json!({"amount_lamports":u64::MAX}),
            "amount_lamports must be between 0.0001 and 0.1 SOL",
        ),
        (
            json!({"slippage_bps":0}),
            "slippage_bps must be between 1 and 3000",
        ),
        (
            json!({"slippage_bps":3001}),
            "slippage_bps must be between 1 and 3000",
        ),
        (json!({}), "wallet_id not found"),
    ] {
        let mut body = quote_request();
        body.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        error_body(
            send(
                &app,
                "POST",
                "/v1/quotes/sol-buy",
                LOCAL,
                Some(TOKEN),
                Some(body),
            )
            .await,
            StatusCode::BAD_REQUEST,
            message,
        )
        .await;
    }
    for wallet in ["fixture-wallet", ""] {
        error_body(
            send(
                &app,
                "GET",
                &format!("/v1/quotes/sol-buy/missing?wallet_id={wallet}"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::BAD_REQUEST,
            "wallet_id not found",
        )
        .await;
    }
}

#[tokio::test]
async fn quote_http_create_and_get_keep_wire_bytes_and_share_legacy_cache_without_sdk_wallets() {
    let (state, trace) = with_quote_sources().await;
    let app = http::router(state.clone());
    let request: SolBuyQuoteRequest = serde_json::from_value(quote_request()).unwrap();
    let created = json_body(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            LOCAL,
            Some(TOKEN),
            Some(quote_request()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let expected = BoundQuote {
        quote_id: created["quote_id"].as_str().unwrap().into(),
        wallet_id: "fixture-wallet".into(),
        wallet_pubkey: Address::from_bytes([3; 32]).to_string(),
        quote_slippage_bps: 1500,
        expires_at_ms: created["expires_at_ms"].as_u64().unwrap(),
        quote: estimate(&request),
    };
    assert_eq!(created, serde_json::to_value(&expected).unwrap());
    assert_eq!(created.as_object().unwrap().len(), 13);
    assert_eq!(created["expected_output_units"], json!(u64::MAX));
    assert_eq!(created["executable"], false);
    assert_eq!(
        trace.events(),
        ["map", "validate", "lookup", "fetch", "pubkey", "bind", "drop"]
    );
    let legacy = state
        .aggregator_quotes
        .get(
            &expected.quote_id,
            &expected.wallet_id,
            &expected.wallet_pubkey,
        )
        .unwrap();
    assert_eq!(serde_json::to_value(legacy).unwrap(), created);
    trace.clear();
    let retrieved = json_body(
        send(
            &app,
            "GET",
            &format!(
                "/v1/quotes/sol-buy/{}?wallet_id=fixture-wallet",
                expected.quote_id
            ),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(retrieved, created);
    assert_eq!(trace.events(), ["lookup", "pubkey", "read", "drop"]);
    assert!(trace.requests.lock().unwrap().is_empty());
    assert_eq!(state.wallets.wallet_count(), 0);
}

#[tokio::test]
async fn quote_http_validation_and_missing_identity_stop_before_fetch_and_cache() {
    let (state, trace) = with_quote_sources().await;
    let app = http::router(state);
    let mut invalid = quote_request();
    invalid["mint"] = json!("invalid");
    error_body(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            LOCAL,
            Some(TOKEN),
            Some(invalid),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "mint must be a valid pubkey",
    )
    .await;
    assert_eq!(trace.events(), ["map", "validate"]);
    trace.clear();
    trace.wallets.lock().unwrap().clear();
    error_body(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            LOCAL,
            Some(TOKEN),
            Some(quote_request()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "wallet_id not found",
    )
    .await;
    assert_eq!(trace.events(), ["map", "validate", "lookup"]);
}

#[tokio::test]
async fn quote_http_backend_and_cache_write_failure_keep_old_status_and_url_redaction() {
    for source_error in [false, true] {
        let (state, trace) = with_quote_sources().await;
        trace.fetch_error.store(source_error, Ordering::SeqCst);
        trace.bind_error.store(!source_error, Ordering::SeqCst);
        error_body(
            send(
                &http::router(state),
                "POST",
                "/v1/quotes/sol-buy",
                LOCAL,
                Some(TOKEN),
                Some(quote_request()),
            )
            .await,
            if source_error {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            if source_error {
                "quote source <redacted-url> timed out"
            } else {
                "quote capacity reached; retry after active quotes expire"
            },
        )
        .await;
        assert_eq!(
            trace.events(),
            if source_error {
                vec!["map", "validate", "lookup", "fetch", "drop"]
            } else {
                vec![
                    "map", "validate", "lookup", "fetch", "pubkey", "bind", "drop",
                ]
            }
        );
    }
}

#[tokio::test]
async fn quote_http_get_checks_current_loaded_identity_before_cache_and_never_refetches() {
    let (state, trace) = with_quote_sources().await;
    let app = http::router(state);
    let created = json_body(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            LOCAL,
            Some(TOKEN),
            Some(quote_request()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let path = format!(
        "/v1/quotes/sol-buy/{}?wallet_id=fixture-wallet",
        created["quote_id"].as_str().unwrap()
    );
    trace.clear();
    trace
        .wallets
        .lock()
        .unwrap()
        .insert("fixture-wallet".into(), Address::from_bytes([4; 32]));
    error_body(
        send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await,
        StatusCode::GONE,
        "quote wallet mismatch",
    )
    .await;
    assert_eq!(trace.events(), ["lookup", "pubkey", "read", "drop"]);
    trace.clear();
    trace.wallets.lock().unwrap().clear();
    error_body(
        send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await,
        StatusCode::BAD_REQUEST,
        "wallet_id not found",
    )
    .await;
    assert_eq!(trace.events(), ["lookup"]);
    trace.clear();
    trace
        .wallets
        .lock()
        .unwrap()
        .insert("fixture-wallet".into(), Address::from_bytes([3; 32]));
    *trace.read_error.lock().unwrap() = Some("quote expired");
    error_body(
        send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await,
        StatusCode::GONE,
        "quote expired",
    )
    .await;
    assert_eq!(trace.events(), ["lookup", "pubkey", "read", "drop"]);
    trace.clear();
    *trace.read_error.lock().unwrap() = None;
    error_body(
        send(
            &app,
            "GET",
            "/v1/quotes/sol-buy/missing?wallet_id=fixture-wallet",
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::GONE,
        "quote not found or service restarted",
    )
    .await;
    assert!(trace.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn quote_http_auth_and_extractors_never_touch_source_identity_or_cache() {
    let (state, trace) = with_quote_sources().await;
    let app = http::router(state);
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            LOCAL,
            None,
            Some(quote_request())
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            DENIED,
            Some(TOKEN),
            Some(quote_request())
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    for patch in [json!({"mint":123}), json!({"slippage_bps":65_536})] {
        let mut body = quote_request();
        body.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert_eq!(
            send(&app, "POST", "/v1/quotes/sol-buy", LOCAL, None, Some(body))
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    // Query extraction rejects missing fields with 400, unlike JSON's 422 above.
    assert_eq!(
        send(&app, "GET", "/v1/quotes/sol-buy/missing", LOCAL, None, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/quotes/sol-buy/missing?wallet_id=fixture-wallet",
            LOCAL,
            None,
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(trace.events().is_empty());
    assert!(trace.requests.lock().unwrap().is_empty());
    assert!(trace.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn quote_http_read_only_queries_preserve_explicit_wallet_ids_under_broadcast_disable() {
    let (mut state, trace) = with_quote_sources().await;
    Arc::get_mut(&mut state.config).unwrap().security.broadcast = BroadcastPolicy::Disabled;
    let app = http::router(state.clone());
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            LOCAL,
            Some(TOKEN),
            Some(quote_request())
        )
        .await
        .status(),
        StatusCode::OK
    );
    trace.clear();
    let mut literal = quote_request();
    literal["wallet_id"] = json!(" fixture-wallet ");
    error_body(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy",
            LOCAL,
            Some(TOKEN),
            Some(literal),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "wallet_id not found",
    )
    .await;
    error_body(
        send(
            &app,
            "GET",
            "/v1/quotes/sol-buy/missing?wallet_id=%20",
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "wallet_id not found",
    )
    .await;
    assert_eq!(&*trace.lookups.lock().unwrap(), &[" fixture-wallet ", " "]);
    assert!(state.trades.recent(10).is_empty());
    assert_eq!(state.wallets.wallet_count(), 0);
}
