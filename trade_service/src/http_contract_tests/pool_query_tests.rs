use super::*;
use crate::{
    application::{
        pool_queries::PoolQueryService,
        ports::pool_queries::{ParamCacheWriter, PoolReadBackend},
    },
    domain::{
        route::PoolRouteHint,
        trade::{Address, Pool, WSOL_MINT},
    },
    param_cache::ParamCache,
};
use std::sync::Mutex;

const MINT: &str = "11111111111111111111111111111111";

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/contracts/pool_queries_v1.json"
    ))
    .unwrap()
}

#[test]
fn pool_query_response_v1_baseline() {
    use crate::models::{
        Pool, ResolvePoolResponse, ResolveSolHopResponse, WarmupResponse, WSOL_MINT,
    };
    let expected = fixture();
    for (name, pool, pool_id) in [
        ("pump", Pool::Pump, None),
        ("pump_amm", Pool::PumpAmm, Some(MINT.to_string())),
        ("dbc", Pool::MeteoraDbc, Some(MINT.to_string())),
    ] {
        assert_eq!(
            serde_json::to_value(ResolvePoolResponse {
                mint: MINT.into(),
                pool,
                pool_id,
                quote_mint: WSOL_MINT.into()
            })
            .unwrap(),
            expected[name]
        );
    }
    assert_eq!(
        serde_json::to_value(WarmupResponse {
            ok: true,
            cache_key: format!("pump:{MINT}")
        })
        .unwrap(),
        expected["warmup"]
    );
    assert_eq!(
        serde_json::to_value(ResolveSolHopResponse {
            available: true,
            pool: Some(Pool::RaydiumCpmm),
            pool_id: Some(MINT.into()),
            reason: None
        })
        .unwrap(),
        expected["hop_available"]
    );
    assert_eq!(
        serde_json::to_value(ResolveSolHopResponse {
            available: false,
            pool: None,
            pool_id: None,
            reason: Some("synthetic discovery timeout".into())
        })
        .unwrap(),
        expected["hop_unavailable"]
    );
}

#[tokio::test]
async fn invalid_query_parameters_fail_before_sdk_access() {
    let mut state = state().await;
    Arc::get_mut(&mut state.config)
        .unwrap()
        .stonkfun_via_sol
        .enabled = true;
    let app = http::router(state.clone());
    for (path, body, message) in [
        (
            "/v1/pools/resolve",
            json!({"mint":"invalid"}),
            "mint must be a valid pubkey",
        ),
        (
            "/v1/pools/sol-hop",
            json!({"quote_mint":"invalid"}),
            "quote_mint must be a valid pubkey",
        ),
        (
            "/v1/cache/warmup",
            json!({"pool":"pump","mint":"invalid"}),
            "mint must be a valid pubkey",
        ),
        (
            "/v1/cache/warmup",
            json!({"pool":"raydium-cpmm","mint":MINT}),
            "pool_id is required for RaydiumCpmm",
        ),
        (
            "/v1/cache/warmup",
            json!({"pool":"pump","mint":MINT,"pool_id":MINT}),
            "pool_id is not used for this pool",
        ),
    ] {
        error_body(
            send(&app, "POST", path, LOCAL, Some(TOKEN), Some(body)).await,
            StatusCode::BAD_REQUEST,
            message,
        )
        .await;
    }
    assert!(state.params.get(&format!("pump:{MINT}")).is_none());
}

#[tokio::test]
async fn disabled_sol_hop_precedes_pubkey_validation_and_does_not_access_wallet() {
    let state = state().await;
    let app = http::router(state);
    for mint in [MINT, "invalid"] {
        assert_eq!(
            json_body(
                send(
                    &app,
                    "POST",
                    "/v1/pools/sol-hop",
                    LOCAL,
                    Some(TOKEN),
                    Some(json!({"quote_mint":mint}))
                )
                .await,
                StatusCode::OK
            )
            .await,
            fixture()["hop_disabled"]
        );
    }
}

#[tokio::test]
async fn query_json_errors_and_pool_aliases_keep_existing_extraction_rules() {
    let app = http::router(state().await);
    for (path, body) in [
        ("/v1/pools/resolve", json!({})),
        ("/v1/pools/sol-hop", json!({"quote_mint":123})),
        ("/v1/cache/warmup", json!({"pool":"pumpswap","mint":MINT})),
        (
            "/v1/cache/warmup",
            json!({"pool":"raydium-clmm","mint":MINT}),
        ),
    ] {
        assert_eq!(
            send(&app, "POST", path, LOCAL, None, Some(body))
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Resolve(Address),
    Warmup(PoolRouteHint),
    Hop(Address),
    Cache(PoolRouteHint),
}

struct Backend {
    hint: PoolRouteHint,
    failure: Option<&'static str>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl PoolReadBackend for Backend {
    async fn resolve(&self, mint: Address) -> anyhow::Result<PoolRouteHint> {
        self.calls.lock().unwrap().push(Call::Resolve(mint));
        if self.failure == Some("resolve") {
            anyhow::bail!("synthetic resolver failure");
        }
        Ok(self.hint.clone())
    }
    async fn warmup(&self, hint: &PoolRouteHint) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(Call::Warmup(hint.clone()));
        if self.failure == Some("warmup") {
            anyhow::bail!("synthetic warmup failure");
        }
        Ok(())
    }
    async fn sol_hop(&self, quote_mint: Address) -> anyhow::Result<Address> {
        self.calls.lock().unwrap().push(Call::Hop(quote_mint));
        match self.failure {
            Some("hop") => anyhow::bail!("synthetic discovery timeout"),
            Some("hop-url") => {
                anyhow::bail!("RPC https://rpc.example.invalid/secret-token timed out")
            }
            _ => Ok(Address::from_bytes([0; 32])),
        }
    }
}

struct Cache {
    inner: Arc<ParamCache>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl ParamCacheWriter for Cache {
    fn record(&self, hint: &PoolRouteHint) -> String {
        self.calls.lock().unwrap().push(Call::Cache(hint.clone()));
        self.inner.record(hint)
    }
}

fn address(value: &str) -> Address {
    Address::from_bytes(bs58::decode(value).into_vec().unwrap().try_into().unwrap())
}

fn hint(pool: Pool) -> PoolRouteHint {
    PoolRouteHint {
        pool,
        mint: address(MINT),
        quote_mint: address(WSOL_MINT),
        pool_id: if pool == Pool::Pump {
            None
        } else {
            Some(address(MINT))
        },
    }
}

async fn with_backend(
    hint: PoolRouteHint,
    failure: Option<&'static str>,
) -> (AppState, Arc<Mutex<Vec<Call>>>) {
    let mut state = state().await;
    let calls = Arc::new(Mutex::new(vec![]));
    state.pool_queries = Arc::new(PoolQueryService::new(
        Backend {
            hint,
            failure,
            calls: calls.clone(),
        },
        Cache {
            inner: state.params.clone(),
            calls: calls.clone(),
        },
    ));
    (state, calls)
}

#[tokio::test]
async fn real_resolve_route_preserves_old_payloads_and_records_cache_after_query() {
    for (name, pool, key) in [
        ("pump", Pool::Pump, format!("pump:{MINT}")),
        ("pump_amm", Pool::PumpAmm, format!("pump-amm:{MINT}")),
        ("dbc", Pool::MeteoraDbc, format!("meteora-dbc:{MINT}")),
    ] {
        let expected_hint = hint(pool);
        let (state, calls) = with_backend(expected_hint.clone(), None).await;
        assert_eq!(state.wallets.wallet_count(), 0);
        let response = send(
            &http::router(state.clone()),
            "POST",
            "/v1/pools/resolve",
            LOCAL,
            Some(TOKEN),
            Some(json!({"mint":MINT})),
        )
        .await;
        assert_eq!(json_body(response, StatusCode::OK).await, fixture()[name]);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                Call::Resolve(address(MINT)),
                Call::Cache(expected_hint.clone())
            ]
        );
        let cached = state.params.get(&key).unwrap();
        assert_eq!(cached.pool, pool);
        assert_eq!(cached.mint, MINT);
        assert_eq!(
            cached.pool_id,
            expected_hint.pool_id.map(|id| id.to_string())
        );
    }
}

#[tokio::test]
async fn resolve_keeps_mint_pool_and_quote_addresses_distinct() {
    let hint = PoolRouteHint {
        pool: Pool::PumpAmm,
        mint: Address::from_bytes([1; 32]),
        pool_id: Some(Address::from_bytes([2; 32])),
        quote_mint: Address::from_bytes([3; 32]),
    };
    let (state, calls) = with_backend(hint.clone(), None).await;
    let response = send(
        &http::router(state),
        "POST",
        "/v1/pools/resolve",
        LOCAL,
        Some(TOKEN),
        Some(json!({"mint":hint.mint.to_string()})),
    )
    .await;
    assert_eq!(
        json_body(response, StatusCode::OK).await,
        json!({"mint":hint.mint.to_string(),"pool":"pump-amm","pool_id":hint.pool_id.unwrap().to_string(),"quote_mint":hint.quote_mint.to_string()})
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![Call::Resolve(hint.mint), Call::Cache(hint)]
    );
}

#[tokio::test]
async fn real_warmup_route_returns_original_key_after_successful_backend() {
    let hint = hint(Pool::Pump);
    let (state, calls) = with_backend(hint.clone(), None).await;
    let response = send(
        &http::router(state.clone()),
        "POST",
        "/v1/cache/warmup",
        LOCAL,
        Some(TOKEN),
        Some(json!({"pool":"pump","mint":MINT})),
    )
    .await;
    assert_eq!(
        json_body(response, StatusCode::OK).await,
        fixture()["warmup"]
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![Call::Warmup(hint.clone()), Call::Cache(hint)]
    );
    assert_eq!(
        state.params.get(&format!("pump:{MINT}")).unwrap().mint,
        MINT
    );
}

#[tokio::test]
async fn warmup_aliases_and_missing_pool_ids_keep_old_normalization_and_keys() {
    for (pool, pool_id, quote_mint, key, expected_pool, expected_quote) in [
        (
            "pump-amm",
            None,
            None,
            format!("pump-amm-mint:{MINT}"),
            Pool::PumpAmm,
            WSOL_MINT,
        ),
        (
            "pump-amm",
            Some(MINT),
            Some(crate::models::SOL_MINT),
            format!("pump-amm:{MINT}"),
            Pool::PumpAmm,
            WSOL_MINT,
        ),
        (
            "stonk-fun",
            Some(MINT),
            Some(crate::models::USDC_MINT),
            format!("stonkfun:{MINT}"),
            Pool::StonkFun,
            crate::models::USDC_MINT,
        ),
    ] {
        let expected_hint = PoolRouteHint {
            pool: expected_pool,
            mint: address(MINT),
            quote_mint: address(expected_quote),
            pool_id: pool_id.map(address),
        };
        let (state, calls) = with_backend(expected_hint.clone(), None).await;
        let response = send(
            &http::router(state.clone()),
            "POST",
            "/v1/cache/warmup",
            LOCAL,
            Some(TOKEN),
            Some(json!({"pool":pool,"mint":MINT,"pool_id":pool_id,"quote_mint":quote_mint})),
        )
        .await;
        assert_eq!(
            json_body(response, StatusCode::OK).await,
            json!({"ok":true,"cache_key":key})
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                Call::Warmup(expected_hint.clone()),
                Call::Cache(expected_hint)
            ]
        );
        assert!(state.params.get(&key).is_some());
    }
}

#[tokio::test]
async fn resolve_and_warmup_failure_preserve_old_cache_and_502_contract() {
    for (operation, path, body, message) in [
        (
            "resolve",
            "/v1/pools/resolve",
            json!({"mint":MINT}),
            "synthetic resolver failure",
        ),
        (
            "warmup",
            "/v1/cache/warmup",
            json!({"pool":"pump","mint":MINT}),
            "synthetic warmup failure",
        ),
    ] {
        let (state, calls) = with_backend(hint(Pool::Pump), Some(operation)).await;
        let key = format!("pump:{MINT}");
        state
            .params
            .insert(key.clone(), Pool::Pump, "synthetic-old-marker".into(), None);
        let original = state.params.get(&key).unwrap();
        error_body(
            send(
                &http::router(state.clone()),
                "POST",
                path,
                LOCAL,
                Some(TOKEN),
                Some(body),
            )
            .await,
            StatusCode::BAD_GATEWAY,
            message,
        )
        .await;
        let after = state.params.get(&key).unwrap();
        assert_eq!(after.mint, original.mint);
        assert_eq!(after.updated_at_ms, original.updated_at_ms);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(!calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Call::Cache(_))));
    }
}

#[tokio::test]
async fn enabled_sol_hop_returns_original_shape_without_metadata_cache_write() {
    let (mut state, calls) = with_backend(hint(Pool::Pump), None).await;
    Arc::get_mut(&mut state.config)
        .unwrap()
        .stonkfun_via_sol
        .enabled = true;
    let response = send(
        &http::router(state.clone()),
        "POST",
        "/v1/pools/sol-hop",
        LOCAL,
        Some(TOKEN),
        Some(json!({"quote_mint":MINT})),
    )
    .await;
    assert_eq!(
        json_body(response, StatusCode::OK).await,
        fixture()["hop_available"]
    );
    assert_eq!(*calls.lock().unwrap(), vec![Call::Hop(address(MINT))]);
    assert!(state.params.get(&format!("pump:{MINT}")).is_none());
}

#[tokio::test]
async fn sol_hop_failure_keeps_200_unavailable_and_url_redaction() {
    for (failure, reason) in [
        ("hop", "synthetic discovery timeout"),
        ("hop-url", "RPC <redacted-url> timed out"),
    ] {
        let (mut state, calls) = with_backend(hint(Pool::Pump), Some(failure)).await;
        Arc::get_mut(&mut state.config)
            .unwrap()
            .stonkfun_via_sol
            .enabled = true;
        let response = send(
            &http::router(state),
            "POST",
            "/v1/pools/sol-hop",
            LOCAL,
            Some(TOKEN),
            Some(json!({"quote_mint":MINT})),
        )
        .await;
        let body = json_body(response, StatusCode::OK).await;
        assert_eq!(
            body,
            json!({"available":false,"pool":null,"pool_id":null,"reason":reason})
        );
        assert!(!body.to_string().contains("secret-token"));
        assert_eq!(*calls.lock().unwrap(), vec![Call::Hop(address(MINT))]);
    }
}

#[tokio::test]
async fn unauthorized_or_disabled_queries_never_invoke_backend_or_cache() {
    let (state, calls) = with_backend(hint(Pool::Pump), None).await;
    let app = http::router(state);
    for (path, body) in [
        ("/v1/pools/resolve", json!({"mint":MINT})),
        ("/v1/cache/warmup", json!({"pool":"pump","mint":MINT})),
        ("/v1/pools/sol-hop", json!({"quote_mint":MINT})),
    ] {
        error_body(
            send(&app, "POST", path, LOCAL, None, Some(body)).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    let response = send(
        &app,
        "POST",
        "/v1/pools/sol-hop",
        LOCAL,
        Some(TOKEN),
        Some(json!({"quote_mint":"invalid"})),
    )
    .await;
    assert_eq!(
        json_body(response, StatusCode::OK).await,
        fixture()["hop_disabled"]
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sol_hop_keeps_literal_quote_address_instead_of_applying_warmup_aliases() {
    let (mut state, calls) = with_backend(hint(Pool::Pump), None).await;
    Arc::get_mut(&mut state.config)
        .unwrap()
        .stonkfun_via_sol
        .enabled = true;
    let response = send(
        &http::router(state),
        "POST",
        "/v1/pools/sol-hop",
        LOCAL,
        Some(TOKEN),
        Some(json!({"quote_mint":crate::models::SOL_MINT})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![Call::Hop(address(crate::models::SOL_MINT))]
    );
}

#[tokio::test]
async fn read_input_boundary_retains_required_errors_and_resolve_vs_warmup_alias_policy() {
    let (mut state, calls) = with_backend(hint(Pool::Pump), None).await;
    Arc::get_mut(&mut state.config)
        .unwrap()
        .stonkfun_via_sol
        .enabled = true;
    let app = http::router(state);
    for (path, body, message) in [
        (
            "/v1/pools/resolve",
            json!({"mint":" "}),
            "mint must be a valid pubkey",
        ),
        (
            "/v1/pools/sol-hop",
            json!({"quote_mint":" "}),
            "quote_mint must be a valid pubkey",
        ),
        (
            "/v1/cache/warmup",
            json!({"pool":"pump","mint":" "}),
            "mint is required",
        ),
        (
            "/v1/cache/warmup",
            json!({"pool":"pump","mint":MINT,"quote_mint":""}),
            "quote_mint must be a valid pubkey",
        ),
    ] {
        error_body(
            send(&app, "POST", path, LOCAL, Some(TOKEN), Some(body)).await,
            StatusCode::BAD_REQUEST,
            message,
        )
        .await;
    }
    assert!(calls.lock().unwrap().is_empty());
    let response = send(
        &app,
        "POST",
        "/v1/pools/resolve",
        LOCAL,
        Some(TOKEN),
        Some(json!({"mint":crate::models::SOL_MINT})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        calls.lock().unwrap()[0],
        Call::Resolve(address(crate::models::SOL_MINT))
    );
    calls.lock().unwrap().clear();
    let response = send(
        &app,
        "POST",
        "/v1/cache/warmup",
        LOCAL,
        Some(TOKEN),
        Some(json!({"pool":"pump","mint":MINT,"quote_mint":crate::models::SOL_MINT})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &*calls.lock().unwrap(),
        &[
            Call::Warmup(hint(Pool::Pump)),
            Call::Cache(hint(Pool::Pump))
        ]
    );
}

#[tokio::test]
async fn pool_read_routes_preserve_methods_and_json_extraction_before_authentication() {
    let (state, calls) = with_backend(hint(Pool::Pump), None).await;
    for app in [
        http::router(state.clone()),
        crate::api::routes::pool_routes().with_state(state.clone()),
    ] {
        for (path, body) in [
            ("/v1/pools/resolve", json!({"mint": MINT})),
            ("/v1/cache/warmup", json!({"pool": "pump", "mint": MINT})),
            ("/v1/pools/sol-hop", json!({"quote_mint": MINT})),
        ] {
            for method in ["GET", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS"] {
                let response = send(&app, method, path, LOCAL, None, None).await;
                assert_eq!(
                    response.status(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {path}"
                );
                assert_eq!(response.headers()["allow"], "POST");
            }
            for (content_type, raw, status) in [
                (None, "{}", StatusCode::UNSUPPORTED_MEDIA_TYPE),
                (Some("text/plain"), "{}", StatusCode::UNSUPPORTED_MEDIA_TYPE),
                (Some("application/json"), "{", StatusCode::BAD_REQUEST),
                (
                    Some("application/json"),
                    "{}",
                    StatusCode::UNPROCESSABLE_ENTITY,
                ),
            ] {
                let mut request = request("POST", path, LOCAL, None, None);
                if let Some(value) = content_type {
                    request
                        .headers_mut()
                        .insert("content-type", value.parse().unwrap());
                }
                *request.body_mut() = Body::from(raw);
                let response = app.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), status, "{path} {content_type:?} {raw}");
            }
            error_body(
                send(&app, "POST", path, LOCAL, None, Some(body.clone())).await,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            )
            .await;
            error_body(
                send(&app, "POST", path, DENIED, Some(TOKEN), Some(body)).await,
                StatusCode::FORBIDDEN,
                "client ip not allowed",
            )
            .await;
        }
    }
    assert!(calls.lock().unwrap().is_empty());
    assert!(state.params.get(&format!("pump:{MINT}")).is_none());
}

#[tokio::test]
async fn pool_read_routes_use_trade_credentials_independently_of_admin_and_broadcast_policy() {
    for disabled in ["false", "true"] {
        let (mut state, calls) = with_backend(hint(Pool::Pump), None).await;
        let config = Arc::get_mut(&mut state.config).unwrap();
        config.stonkfun_via_sol.enabled = true;
        config.security = SecurityConfig::parse(
            Some(disabled),
            Some("separate"),
            Some("admin-contract-test-only"),
            "http-contract-test-only",
        )
        .unwrap();
        for app in [
            http::router(state.clone()),
            crate::api::routes::pool_routes().with_state(state.clone()),
        ] {
            for (path, body, expected) in [
                (
                    "/v1/pools/resolve",
                    json!({"mint": MINT}),
                    fixture()["pump"].clone(),
                ),
                (
                    "/v1/cache/warmup",
                    json!({"pool": "pump", "mint": MINT}),
                    fixture()["warmup"].clone(),
                ),
                (
                    "/v1/pools/sol-hop",
                    json!({"quote_mint": MINT}),
                    fixture()["hop_available"].clone(),
                ),
            ] {
                error_body(
                    send(
                        &app,
                        "POST",
                        path,
                        LOCAL,
                        Some("Bearer admin-contract-test-only"),
                        Some(body.clone()),
                    )
                    .await,
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                )
                .await;
                assert!(calls.lock().unwrap().is_empty());
                assert_eq!(
                    json_body(
                        send(&app, "POST", path, ALLOWED, Some(TOKEN), Some(body)).await,
                        StatusCode::OK
                    )
                    .await,
                    expected,
                );
                let expected_calls = match path {
                    "/v1/pools/resolve" => {
                        vec![Call::Resolve(address(MINT)), Call::Cache(hint(Pool::Pump))]
                    }
                    "/v1/cache/warmup" => vec![
                        Call::Warmup(hint(Pool::Pump)),
                        Call::Cache(hint(Pool::Pump)),
                    ],
                    _ => vec![Call::Hop(address(MINT))],
                };
                assert_eq!(*calls.lock().unwrap(), expected_calls);
                calls.lock().unwrap().clear();
            }
        }
        assert!(state.trades.recent(10).is_empty());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}

#[tokio::test]
async fn standalone_pool_route_group_preserves_failure_responses_without_exposing_other_operations()
{
    for (failure, path, body, status, expected) in [
        (
            "resolve",
            "/v1/pools/resolve",
            json!({"mint": MINT}),
            StatusCode::BAD_GATEWAY,
            json!({"error": "synthetic resolver failure"}),
        ),
        (
            "warmup",
            "/v1/cache/warmup",
            json!({"pool": "pump", "mint": MINT}),
            StatusCode::BAD_GATEWAY,
            json!({"error": "synthetic warmup failure"}),
        ),
        (
            "hop-url",
            "/v1/pools/sol-hop",
            json!({"quote_mint": MINT}),
            StatusCode::OK,
            json!({"available": false, "pool": null, "pool_id": null, "reason": "RPC <redacted-url> timed out"}),
        ),
    ] {
        let (mut state, calls) = with_backend(hint(Pool::Pump), Some(failure)).await;
        Arc::get_mut(&mut state.config)
            .unwrap()
            .stonkfun_via_sol
            .enabled = true;
        let app = crate::api::routes::pool_routes().with_state(state.clone());
        assert_eq!(
            json_body(
                send(&app, "POST", path, LOCAL, Some(TOKEN), Some(body)).await,
                status
            )
            .await,
            expected,
        );
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(state.params.get(&format!("pump:{MINT}")).is_none());
        for path in [
            "/health",
            "/ready",
            "/v1/capabilities",
            "/v1/trades",
            "/v1/wallets",
            "/v1/pools/missing",
        ] {
            assert_eq!(
                send(&app, "GET", path, LOCAL, Some(TOKEN), None)
                    .await
                    .status(),
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(state.trades.recent(10).is_empty());
    }
}
