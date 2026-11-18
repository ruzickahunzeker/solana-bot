//! In-process HTTP contracts: no .env, wallet keys, RPC clients or socket listeners.
//! Readiness uses an injectable source; the empty registry cannot sign or execute trades.

mod pool_query_tests;
mod quote_preflight_tests;
mod quote_query_tests;
mod quote_reconcile_tests;
mod quote_route_group_tests;
mod quote_submit_tests;
mod readiness_tests;
mod request_settings_tests;
mod sol_transfer_execution_tests;
mod sol_transfer_tests;
mod state_resource_tests;
mod trade_execution_tests;
mod trade_intake_tests;
mod trade_query_tests;
mod vanity_application_tests;
mod wallet_balance_query_tests;
mod wallet_boundary_tests;
mod wallet_management_tests;
mod wsol_conversion_tests;
mod wsol_quote_unsigned_tests;
mod wsol_status_boundary_tests;
mod wsol_storage_tests;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

use crate::config::*;
use crate::{
    http, AppState, BlockhashCache, TradePersistence, WalletOps, WalletRegistry, WalletRepository,
};

const TOKEN: &str = "Bearer http-contract-test-only";
const LOCAL: &str = "127.0.0.1:12345";
const ALLOWED: &str = "192.0.2.10:12345";
const DENIED: &str = "192.0.2.11:12345";

struct ReadinessMustBeInjected;
impl crate::application::readiness::ReadinessQuery for ReadinessMustBeInjected {
    fn inspect(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = crate::domain::readiness::ReadinessReport> + Send + '_,
        >,
    > {
        panic!("authorized readiness tests must inject a read-only fixture source")
    }
}

struct PoolQueriesMustBeInjected;
impl crate::application::pool_queries::PoolQueries for PoolQueriesMustBeInjected {
    fn resolve(
        &self,
        _: crate::domain::trade::Address,
    ) -> crate::application::pool_queries::PoolQueryFuture<'_, crate::domain::route::PoolRouteHint>
    {
        panic!("authorized pool tests must inject a read-only source")
    }
    fn warmup(
        &self,
        _: crate::domain::route::PoolRouteHint,
    ) -> crate::application::pool_queries::PoolQueryFuture<'_, String> {
        panic!("authorized warmup tests must inject a read-only source")
    }
    fn sol_hop(
        &self,
        _: crate::domain::trade::Address,
    ) -> crate::application::pool_queries::PoolQueryFuture<'_, crate::domain::trade::Address> {
        panic!("authorized hop tests must inject a read-only source")
    }
}

struct OrdinaryExecutionMustBeInjected;
impl crate::application::ports::ordinary_execution::OrdinaryExecutionBackend
    for OrdinaryExecutionMustBeInjected
{
    fn resolve(
        &self,
        _: &str,
    ) -> Option<Box<dyn crate::application::ports::ordinary_execution::OrdinaryExecutionWallet>>
    {
        panic!("execution tests must inject a wallet-free backend")
    }
}

pub(crate) async fn state() -> AppState {
    state_with_blockhash().await.0
}

pub(crate) async fn state_with_blockhash() -> (AppState, Arc<BlockhashCache>) {
    let provider = ProviderConfig {
        enabled: false,
        auth_token: None,
        region: "test".into(),
        custom_url: None,
    };
    let config = Arc::new(Config {
        shutdown: Default::default(),
        ordinary_observation: Default::default(),
        rpc_urls: vec![],
        wallets: vec![],
        default_wallet_id: "fixture-wallet".into(),
        wallet_storage: WalletStorageConfig {
            mode: WalletStorageMode::File,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
            passphrase: None,
        },
        allow_internal_system_wallet_transfer: false,
        local_trade_api_token: "http-contract-test-only".into(),
        security: Default::default(),
        trade_http_addr: "127.0.0.1:0".parse().unwrap(),
        trade_allowed_client_ips: vec!["192.0.2.10".parse().unwrap()],
        default_buy_sol: 0.001,
        default_slippage_percent: 15.0,
        default_tip_sol: 0.0,
        enable_simulate: true,
        wait_tx_confirmed: false,
        enable_sdk_log: false,
        trade_response_wait_ms: 0,
        mev_submit_verify_ms: 0,
        blockhash_refresh_ms: 400,
        blockhash_max_age_ms: 2_000,
        mev: MevConfig {
            enable_mev_protection: false,
            require_mev_submit: false,
            use_backup_rpcs: false,
            jito: provider.clone(),
            temporal: provider.clone(),
            flashblock: provider.clone(),
            blockrazor: BlockRazorConfig {
                provider: provider.clone(),
                transport: "http".into(),
            },
            astralane: AstralaneConfig {
                provider: provider.clone(),
                transport: "http".into(),
            },
            speedlanding: provider,
        },
        pumpapi: PumpApiConfig {
            enabled: false,
            url: "https://example.invalid".into(),
            timeout_ms: 100,
            default_priority_fee_sol: 0.0,
            lightning_enabled: false,
        },
        stonkfun_via_sol: StonkFunViaSolConfig {
            enabled: false,
            max_buy_sol: 0.1,
        },
        trade_persistence: TradePersistenceConfig {
            mode: TradePersistenceMode::Memory,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        },
        conversion_database_url: "sqlite::memory:".into(),
        conversion_quotes_enabled: false,
        conversion_live_enabled: false,
    });
    let wallets = Arc::new(WalletRegistry::empty_for_http_tests(
        config.default_wallet_id.clone(),
        config.mev.clone(),
    ));
    let wallet_repository = Arc::new(
        WalletRepository::new(&config.wallet_storage, &[], &config.default_wallet_id)
            .await
            .unwrap(),
    );
    let wallet_ops = Arc::new(
        WalletOps::new(wallet_repository.clone(), wallets.clone())
            .await
            .unwrap(),
    );
    let persistence = Arc::new(
        TradePersistence::new(&config.trade_persistence)
            .await
            .unwrap(),
    );
    let wallet_identity = Arc::new(
        crate::adapters::sdk::wallet_identity::SdkWalletIdentityDirectory::new(wallets.clone()),
    );
    let aggregator_quotes = Arc::new(crate::aggregator_quote::QuoteStore::default());
    let read_rpc =
        Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
    let ordinary_sol_buy = Arc::new(
        crate::adapters::sdk::ordinary_sol_buy::SdkOrdinarySolBuyBackend::new(
            wallets.clone(),
            aggregator_quotes.clone(),
            persistence.clone(),
            read_rpc.clone(),
        ),
    );
    let quote_queries = Arc::new(crate::application::sol_buy_quotes::SolBuyQuoteService::new(
        crate::adapters::aggregator::quotes::AggregatorQuoteSource,
        wallet_identity.clone(),
        aggregator_quotes.clone(),
    ));
    let quote_preflights = Arc::new(
        crate::application::quote_preflight::QuotePreflightService::new(
            Arc::new(
                crate::adapters::sdk::quote_preflight::SdkQuotePreflightBackend::new(
                    wallets.clone(),
                    read_rpc.clone(),
                ),
            ),
            aggregator_quotes.clone(),
        ),
    );
    let quote_submissions = Arc::new(
        crate::application::sol_buy_submit::SolBuySubmissionService::new(
            crate::adapters::sdk::quote_submit::SdkQuoteSubmissionRuntime::new(
                wallets.clone(),
                persistence.clone(),
                read_rpc.clone(),
            ),
            aggregator_quotes.clone(),
        ),
    );
    let quote_reconciliations = Arc::new(
        crate::application::quote_reconciliation::QuoteReconciliationService::new(
            crate::adapters::sdk::quote_reconciliation::SdkQuoteReconciliationRuntime::new(
                wallets.clone(),
                persistence.clone(),
                read_rpc.clone(),
            ),
        ),
    );
    let sol_transfers = Arc::new(crate::application::sol_transfers::SolTransferService::new(
        crate::adapters::sdk::sol_transfer::SdkSolTransferRuntime::new(
            wallets.clone(),
            wallet_repository.clone(),
            read_rpc.clone(),
        ),
        crate::adapters::sdk::address_parser::SdkAddressParser,
    ));
    let wallet_management = crate::adapters::sdk::wallet_management::SdkWalletManagement::new(
        wallet_repository.clone(),
        wallets.clone(),
    );
    let params = Arc::new(crate::param_cache::ParamCache::default());
    let blockhash = Arc::new(BlockhashCache::default());
    let state = AppState {
        trade_tasks: Default::default(),
        vanity_tasks: Arc::new(crate::application::vanity::VanityService::new(wallet_ops)),
        wallet_management: Arc::new(
            crate::application::wallet_management::WalletManagementService::new(
                wallet_management.clone(),
                wallet_management,
            ),
        ),
        wallet_balance_queries: Arc::new(
            crate::application::wallet_balances::WalletBalanceService::new(
                crate::adapters::persistence::wallet_metadata::RepositoryWalletMetadata::new(
                    wallet_repository.clone(),
                ),
                crate::adapters::sdk::wallet_balances::SdkWalletBalanceRuntime::new(
                    wallets.clone(),
                    read_rpc.clone(),
                ),
                crate::adapters::sdk::address_parser::SdkAddressParser,
                crate::state::now_ms,
            ),
        ),
        wsol_queries: Arc::new(crate::application::wsol_queries::WsolQueryService::new(
            crate::adapters::sdk::wsol_queries::SdkWsolQuerySource::new(
                wallets.clone(),
                wallet_repository.clone(),
                read_rpc.clone(),
            ),
            None::<crate::adapters::persistence::wsol_conversion::SqliteWsolJournal>,
            crate::adapters::sdk::address_parser::SdkAddressParser,
        )),
        wsol_submissions: Arc::new(
            crate::application::wsol_submission::WsolSubmissionService::new(
                crate::adapters::sdk::wsol_conversion::SdkWsolRuntime::new(
                    wallets.clone(),
                    wallet_repository.clone(),
                    read_rpc,
                ),
                None::<crate::adapters::persistence::wsol_conversion::SqliteWsolJournal>,
            ),
        ),
        request_settings: Arc::new(
            crate::config::request_settings::RequestSettings::from_config(&config),
        ),
        config,
        wallets,
        wallet_repository,
        persistence,
        cached_blockhash: blockhash.clone(),
        params: params.clone(),
        param_cache_writer: params,
        trades: Arc::default(),
        idempotency: Arc::default(),
        aggregator_quotes,
        conversions: None,
        readiness: Arc::new(ReadinessMustBeInjected),
        pool_queries: Arc::new(PoolQueriesMustBeInjected),
        ordinary_execution: Arc::new(OrdinaryExecutionMustBeInjected),
        ordinary_sol_buy,
        quote_queries,
        quote_preflights,
        quote_submissions,
        quote_reconciliations,
        sol_transfers,
        wallet_identity,
    };
    (state, blockhash)
}

fn trade() -> Value {
    serde_json::from_str::<Value>(include_str!("../tests/fixtures/contracts/trade_v1.json"))
        .unwrap()["minimal_buy"]
        .clone()
}

fn request(
    method: &str,
    path: &str,
    peer: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .extension(ConnectInfo(peer.parse::<std::net::SocketAddr>().unwrap()));
    if let Some(token) = token {
        builder = builder.header("authorization", token);
    }
    let body = if let Some(value) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&value).unwrap())
    } else {
        Body::empty()
    };
    builder.body(body).unwrap()
}

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    peer: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Response {
    app.clone()
        .oneshot(request(method, path, peer, token, body))
        .await
        .unwrap()
}

async fn json_body(response: Response, status: StatusCode) -> Value {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["content-type"], "application/json");
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

async fn error_body(response: Response, status: StatusCode, message: &str) {
    assert_eq!(json_body(response, status).await, json!({"error": message}));
}

#[tokio::test]
async fn health_is_public_but_unknown_routes_and_methods_are_not_successful() {
    let app = http::router(state().await);
    assert_eq!(
        json_body(
            send(&app, "GET", "/health", DENIED, None, None).await,
            StatusCode::OK
        )
        .await,
        json!({"ok": true})
    );
    assert_eq!(
        send(&app, "POST", "/health", LOCAL, None, None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        send(&app, "GET", "/v1/not-a-route", LOCAL, None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, "DELETE", "/v1/trades", LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn every_public_protected_operation_requires_a_token() {
    let app = http::router(state().await);
    let cases = vec![
        ("GET", "/ready", None),
        ("GET", "/v1/capabilities", None),
        ("GET", "/v1/wallets", None),
        ("GET", "/v1/wallets/manage", None),
        (
            "POST",
            "/v1/wallets",
            Some(json!({"owner_type":"test","owner_id":"test","label":"test"})),
        ),
        (
            "PATCH",
            "/v1/wallets/missing",
            Some(json!({"label":"test"})),
        ),
        ("DELETE", "/v1/wallets/missing", None),
        ("POST", "/v1/wallets/missing/export", None),
        (
            "POST",
            "/v1/cache/warmup",
            Some(json!({"pool":"pump","mint":"test"})),
        ),
        ("POST", "/v1/pools/resolve", Some(json!({"mint":"test"}))),
        (
            "POST",
            "/v1/pools/sol-hop",
            Some(json!({"quote_mint":"test"})),
        ),
        (
            "POST",
            "/v1/quotes/sol-buy",
            Some(json!({"wallet_id":"test","mint":"test","amount_lamports":1,"slippage_bps":100})),
        ),
        ("GET", "/v1/quotes/sol-buy/missing?wallet_id=test", None),
        (
            "GET",
            "/v1/quotes/sol-buy/missing/submission?wallet_id=test&idempotency_key=test",
            None,
        ),
        (
            "POST",
            "/v1/quotes/sol-buy/missing/preflight?wallet_id=test",
            None,
        ),
        (
            "POST",
            "/v1/quotes/sol-buy/missing/submit",
            Some(
                json!({"wallet_id":"test","confirmed":false,"simulate":true,"idempotency_key":"test"}),
            ),
        ),
        ("POST", "/v1/trades", Some(trade())),
        ("GET", "/v1/trades", None),
        ("GET", "/v1/trades/missing", None),
        (
            "POST",
            "/v1/batch/trades",
            Some(json!({"wallet_ids":["missing"],"trade":trade()})),
        ),
    ];
    for (method, path, body) in cases {
        let response = send(&app, method, path, LOCAL, None, body).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
        error_body(response, StatusCode::UNAUTHORIZED, "unauthorized").await;
    }
}

#[tokio::test]
async fn internal_operations_require_loopback_even_with_allowed_ip_and_token() {
    let app = http::router(state().await);
    let owner = "owner_type=test&owner_id=test";
    let submission = json!({"quote_id":"missing","wallet_id":"test","owner_type":"test","owner_id":"test","simulate":true,"idempotency_key":"test"});
    let cases = vec![
        ("GET", format!("/internal/wallet-balances?{owner}"), None),
        (
            "GET",
            format!("/internal/wallets/test/quote-balance/test?{owner}"),
            None,
        ),
        (
            "POST",
            "/internal/wallets/vanity-jobs".into(),
            Some(
                json!({"owner_type":"test","owner_id":"test","label":"test","match_side":"prefix","pattern":"1"}),
            ),
        ),
        (
            "GET",
            format!("/internal/wallets/vanity-jobs/missing?{owner}"),
            None,
        ),
        (
            "DELETE",
            format!("/internal/wallets/vanity-jobs/missing?{owner}"),
            None,
        ),
        (
            "POST",
            "/internal/wallets/test/transfer-sol".into(),
            Some(
                json!({"owner_type":"test","owner_id":"test","amount_mode":"fixed","amount_lamports":1,"idempotency_key":"test"}),
            ),
        ),
        (
            "POST",
            "/internal/conversions/wsol/quotes".into(),
            Some(
                json!({"wallet_id":"test","owner_type":"test","owner_id":"test","direction":"wrap","amount_lamports":1}),
            ),
        ),
        (
            "POST",
            "/internal/conversions/wsol/simulate".into(),
            Some(submission.clone()),
        ),
        (
            "POST",
            "/internal/conversions/wsol/submit".into(),
            Some(submission),
        ),
        (
            "GET",
            format!("/internal/conversions/wsol/submissions/missing?{owner}"),
            None,
        ),
    ];
    for (method, path, body) in cases {
        let response = send(&app, method, &path, ALLOWED, Some(TOKEN), body.clone()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
        error_body(
            response,
            StatusCode::FORBIDDEN,
            "internal wallet API is loopback-only",
        )
        .await;
        error_body(
            send(&app, method, &path, LOCAL, None, body).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
}

#[tokio::test]
async fn ip_allowlist_precedes_bearer_authentication_and_ignores_forwarded_headers() {
    let app = http::router(state().await);
    for token in [None, Some(TOKEN)] {
        let mut req = request("GET", "/v1/wallets", DENIED, token, None);
        req.headers_mut()
            .insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
        error_body(
            app.clone().oneshot(req).await.unwrap(),
            StatusCode::FORBIDDEN,
            "client ip not allowed",
        )
        .await;
    }
    error_body(
        send(&app, "GET", "/v1/wallets", ALLOWED, None, None).await,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    for peer in [LOCAL, ALLOWED, "[::1]:12345"] {
        assert_eq!(
            json_body(
                send(&app, "GET", "/v1/wallets", peer, Some(TOKEN), None).await,
                StatusCode::OK
            )
            .await,
            json!({"default_wallet_id":"fixture-wallet","wallets":[]})
        );
    }
}

#[tokio::test]
async fn bearer_format_is_enforced_by_the_router() {
    let app = http::router(state().await);
    for token in [
        "Bearer wrong",
        "Basic http-contract-test-only",
        "Bearer http-contract-test-only extra",
    ] {
        error_body(
            send(&app, "GET", "/v1/wallets", LOCAL, Some(token), None).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/wallets",
            LOCAL,
            Some("bEaReR http-contract-test-only"),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn trade_json_extraction_errors_precede_handler_authentication() {
    let app = http::router(state().await);
    for token in [None, Some(TOKEN)] {
        assert_eq!(
            send(&app, "POST", "/v1/trades", LOCAL, token, None)
                .await
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            send(&app, "POST", "/v1/trades", LOCAL, token, Some(json!({})))
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let mut req = request("POST", "/v1/trades", LOCAL, token, Some(trade()));
        *req.body_mut() = Body::from("{");
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn missing_wallet_rejection_does_not_create_an_order() {
    let state = state().await;
    let app = http::router(state.clone());
    for wallet in [None, Some("missing")] {
        let mut body = trade();
        body["wallet_id"] = json!(wallet);
        error_body(
            send(&app, "POST", "/v1/trades", LOCAL, Some(TOKEN), Some(body)).await,
            StatusCode::BAD_REQUEST,
            &format!("wallet_id {} not found", wallet.unwrap_or("fixture-wallet")),
        )
        .await;
    }
    assert!(state.trades.recent(100).is_empty());
}

#[tokio::test]
async fn missing_trade_and_empty_list_have_distinct_contracts() {
    let app = http::router(state().await);
    error_body(
        send(&app, "GET", "/v1/trades/missing", LOCAL, Some(TOKEN), None).await,
        StatusCode::NOT_FOUND,
        "trade not found",
    )
    .await;
    assert_eq!(
        json_body(
            send(&app, "GET", "/v1/trades?limit=0", LOCAL, Some(TOKEN), None).await,
            StatusCode::OK
        )
        .await,
        json!([])
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/trades?limit=invalid",
            LOCAL,
            Some(TOKEN),
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn capability_response_keeps_unimplemented_protocols_closed() {
    let app = http::router(state().await);
    let body = json_body(
        send(&app, "GET", "/v1/capabilities", LOCAL, Some(TOKEN), None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(body["schema_version"], 1);
    assert!(body["generated_at_ms"].as_u64().unwrap() > 0);
    let protocols = body["protocols"].as_array().unwrap();
    assert!(!protocols.is_empty());
    let clmm = protocols
        .iter()
        .find(|p| p["protocol_id"] == "raydium-clmm")
        .unwrap();
    assert_eq!(clmm["status"], "observe");
    assert_eq!(clmm["supports_buy"], false);
    assert_eq!(clmm["supports_sell"], false);
}

#[tokio::test]
async fn disabled_conversion_submit_is_rejected_before_wallet_or_rpc_access() {
    let app = http::router(state().await);
    let body = json!({"quote_id":"missing","wallet_id":"test","owner_type":"test","owner_id":"test","simulate":false,"confirmed":true,"idempotency_key":"test"});
    error_body(
        send(
            &app,
            Method::POST.as_str(),
            "/internal/conversions/wsol/submit",
            LOCAL,
            Some(TOKEN),
            Some(body),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "live WSOL conversion is disabled or request is simulation",
    )
    .await;
}

#[tokio::test]
async fn submission_status_route_is_protected_and_requires_explicit_identity() {
    let app = http::router(state().await);
    let path = "/v1/quotes/sol-buy/test/submission?wallet_id=fixture-wallet&idempotency_key=test";
    error_body(
        send(&app, "GET", path, DENIED, Some(TOKEN), None).await,
        StatusCode::FORBIDDEN,
        "client ip not allowed",
    )
    .await;
    error_body(
        send(&app, "GET", path, LOCAL, Some(TOKEN), None).await,
        StatusCode::BAD_REQUEST,
        "wallet_id not found",
    )
    .await;
    assert_eq!(
        send(&app, "POST", path, LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/quotes/sol-buy/test/submission?wallet_id=fixture-wallet",
            LOCAL,
            Some(TOKEN),
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
}

async fn separate_admin_state() -> AppState {
    let mut state = state().await;
    state.edit_fixture_config(|config| {
        config.security = crate::config::SecurityConfig::parse(
            None,
            Some("separate"),
            Some("admin-contract-test-only"),
            "http-contract-test-only",
        )
        .unwrap();
    });
    state
}

#[tokio::test]
async fn separate_admin_protects_every_wallet_management_operation() {
    let app = http::router(separate_admin_state().await);
    let cases = [
        ("GET", "/v1/wallets/manage", None),
        (
            "POST",
            "/v1/wallets",
            Some(json!({"owner_type":"test","owner_id":"test","label":"test"})),
        ),
        (
            "PATCH",
            "/v1/wallets/missing",
            Some(json!({"label":"test"})),
        ),
        ("DELETE", "/v1/wallets/missing", None),
        ("POST", "/v1/wallets/missing/export", None),
        (
            "POST",
            "/internal/wallets/vanity-jobs",
            Some(
                json!({"owner_type":"test","owner_id":"test","label":"test","match_side":"prefix","pattern":"1"}),
            ),
        ),
        (
            "GET",
            "/internal/wallets/vanity-jobs/missing?owner_type=test&owner_id=test",
            None,
        ),
        (
            "DELETE",
            "/internal/wallets/vanity-jobs/missing?owner_type=test&owner_id=test",
            None,
        ),
    ];
    for (method, path, body) in cases {
        error_body(
            send(&app, method, path, LOCAL, Some(TOKEN), body.clone()).await,
            StatusCode::UNAUTHORIZED,
            "wallet admin authorization required",
        )
        .await;
        error_body(
            send(
                &app,
                method,
                path,
                ALLOWED,
                Some("Bearer admin-contract-test-only"),
                body,
            )
            .await,
            StatusCode::FORBIDDEN,
            "wallet admin API is loopback-only",
        )
        .await;
    }
}

#[tokio::test]
async fn admin_credential_is_not_a_trade_credential_and_can_authenticate_only_on_loopback() {
    let state = separate_admin_state().await;
    let app = http::router(state.clone());
    // File-mode empty repository rejects after successful auth, without mutating any wallet.
    assert_eq!(
        send(
            &app,
            "PATCH",
            "/v1/wallets/missing",
            LOCAL,
            Some("Bearer admin-contract-test-only"),
            Some(json!({"label":"test"}))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    for path in ["/v1/wallets", "/v1/trades", "/v1/capabilities"] {
        error_body(
            send(
                &app,
                "GET",
                path,
                LOCAL,
                Some("Bearer admin-contract-test-only"),
                None,
            )
            .await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    for peer in [LOCAL, "[::1]:12345"] {
        let req = request(
            "POST",
            "/v1/wallets/missing/export",
            peer,
            Some("Bearer admin-contract-test-only"),
            None,
        );
        assert!(crate::api::middleware::auth::authorize_wallet_admin(
            state.config.api_access_policy(),
            req.headers(),
            peer.parse().unwrap(),
            false
        )
        .is_ok());
    }
    let mut req = request(
        "POST",
        "/v1/wallets/missing/export",
        ALLOWED,
        Some("Bearer admin-contract-test-only"),
        None,
    );
    req.headers_mut()
        .insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
    error_body(
        app.clone().oneshot(req).await.unwrap(),
        StatusCode::FORBIDDEN,
        "wallet admin API is loopback-only",
    )
    .await;
    assert_eq!(
        send(&app, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn legacy_admin_mode_keeps_original_allowlist_and_token_behavior() {
    let state = state().await;
    let req = request(
        "POST",
        "/v1/wallets/missing/export",
        ALLOWED,
        Some(TOKEN),
        None,
    );
    assert!(crate::api::middleware::auth::authorize_wallet_admin(
        state.config.api_access_policy(),
        req.headers(),
        ALLOWED.parse().unwrap(),
        false
    )
    .is_ok());
    assert!(crate::api::middleware::auth::authorize_wallet_admin(
        state.config.api_access_policy(),
        req.headers(),
        ALLOWED.parse().unwrap(),
        true
    )
    .is_err());
}

#[tokio::test]
async fn api_access_policy_config_projection_borrows_only_original_access_values() {
    let state = state().await;
    let mut config = (*state.config).clone();
    config.local_trade_api_token = "projection-trade-fixture".into();
    config.security.wallet_admin = crate::config::security::WalletAdminAuth::Separate {
        token: "projection-admin-fixture".into(),
    };
    config.rpc_urls.clear();
    config.wallets.clear();
    let policy = config.api_access_policy();
    assert_eq!(
        policy.trade_token.as_ptr(),
        config.local_trade_api_token.as_ptr()
    );
    assert_eq!(policy.trade_token.len(), config.local_trade_api_token.len());
    assert_eq!(
        policy.allowed_client_ips.as_ptr(),
        config.trade_allowed_client_ips.as_ptr()
    );
    assert_eq!(
        policy.allowed_client_ips.len(),
        config.trade_allowed_client_ips.len()
    );
    assert!(std::ptr::eq(
        policy.wallet_admin,
        &config.security.wallet_admin
    ));
    assert!(crate::api::middleware::auth::authorize_wallet_admin(
        policy,
        request(
            "GET",
            "/unused",
            LOCAL,
            Some("Bearer projection-admin-fixture"),
            None
        )
        .headers(),
        LOCAL.parse().unwrap(),
        false,
    )
    .is_ok());
}

#[tokio::test]
async fn hard_broadcast_policy_blocks_transfer_and_wsol_before_storage_or_wallet_access() {
    let mut state = state().await;
    state.edit_fixture_config(|config| {
        config.security.broadcast = crate::domain::execution_policy::BroadcastPolicy::Disabled;
        config.conversion_live_enabled = true;
        config.conversion_quotes_enabled = true;
        config.enable_simulate = false;
    });
    let app = http::router(state.clone());
    for (path, body) in [
        (
            "/internal/wallets/missing/transfer-sol",
            json!({"owner_type":"test","owner_id":"test","amount_mode":"fixed","amount_lamports":1,"idempotency_key":"test"}),
        ),
        (
            "/internal/conversions/wsol/submit",
            json!({"quote_id":"missing","wallet_id":"test","owner_type":"test","owner_id":"test","simulate":false,"confirmed":true,"idempotency_key":"test"}),
        ),
    ] {
        error_body(
            send(&app, "POST", path, LOCAL, Some(TOKEN), Some(body)).await,
            StatusCode::FORBIDDEN,
            crate::domain::execution_policy::BROADCAST_DISABLED_ERROR,
        )
        .await;
    }
    assert_eq!(
        send(&app, "GET", "/health", LOCAL, None, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, "GET", "/v1/capabilities", LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::OK
    );
    let mut submission: crate::conversion::WsolSubmission = serde_json::from_value(json!({"quote_id":"missing","wallet_id":"test","owner_type":"test","owner_id":"test","simulate":false,"confirmed":true,"idempotency_key":"test"})).unwrap();
    assert_eq!(
        state
            .wsol_submissions
            .submit(
                submission.input(),
                crate::application::wsol_submission::WsolSubmitPolicy {
                    broadcast: state.config.security.broadcast,
                    live_enabled: state.config.conversion_live_enabled,
                }
            )
            .await
            .err()
            .unwrap()
            .to_string(),
        crate::domain::execution_policy::BROADCAST_DISABLED_ERROR
    );
    submission.simulate = true;
    assert_eq!(
        state
            .wsol_submissions
            .simulate(submission.input())
            .await
            .err()
            .unwrap()
            .to_string(),
        "conversion quotes are disabled"
    );
}

#[tokio::test]
async fn broadcast_policy_resolves_missing_and_explicit_simulation_without_mev_bypass() {
    use crate::{
        domain::execution_policy::BroadcastPolicy,
        models::{ExecutionMethod, MevProvider, TradeRequest},
        validation::{validate_execution_policy, validate_trade},
    };
    let mut config = (*state().await.config).clone();
    let request: TradeRequest = serde_json::from_value(trade()).unwrap();
    let mut normalized = validate_trade(&request, &config).unwrap();
    for disabled in [false, true] {
        config.security.broadcast = if disabled {
            BroadcastPolicy::Disabled
        } else {
            BroadcastPolicy::Enabled
        };
        for default_simulate in [false, true] {
            config.enable_simulate = default_simulate;
            for simulate in [None, Some(false), Some(true)] {
                normalized.simulate = simulate;
                normalized.use_mev = Some(true);
                normalized.mev_provider = Some(MevProvider::Temporal);
                for (method, aggregator) in [
                    (ExecutionMethod::Sdk, false),
                    (ExecutionMethod::Sdk, true),
                    (ExecutionMethod::Pumpapi, false),
                ] {
                    normalized.execution_method = method;
                    normalized.aggregator_sol = aggregator;
                    let expected = !disabled
                        || ((method != ExecutionMethod::Sdk || aggregator)
                            && simulate.unwrap_or(default_simulate));
                    assert_eq!(
                        validate_execution_policy(&normalized, &config).is_ok(),
                        expected
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn executor_checks_policy_even_when_http_is_bypassed() {
    use crate::{
        domain::execution_policy::BroadcastPolicy,
        models::{TradeRequest, TradeStatus},
        validation::validate_trade,
    };
    let mut state = state().await;
    state.edit_fixture_config(|config| config.security.broadcast = BroadcastPolicy::Disabled);
    let request: TradeRequest = serde_json::from_value(trade()).unwrap();
    let normalized = validate_trade(&request, &state.config).unwrap();
    let record = state
        .trades
        .create(&request, "fixture-wallet".into(), "fixture-pubkey".into());
    let command = request
        .to_command(
            "fixture-wallet".into(),
            &normalized,
            state.config.trade_command_defaults(),
        )
        .unwrap();
    crate::executor::execute_trade(state.clone(), record.request_id.clone(), command).await;
    let blocked = state.trades.get(&record.request_id).unwrap();
    assert_eq!(blocked.status, TradeStatus::Rejected);
    assert!(blocked.signature.is_none());
    assert_eq!(
        blocked.error.as_deref(),
        Some(crate::domain::execution_policy::BROADCAST_DISABLED_ERROR)
    );
}
