use super::*;
use crate::{
    application::{ports::readiness::ReadinessSource, readiness::ReadinessService},
    domain::{
        readiness::{ReadinessSnapshot, ReadinessWalletMetadata},
        trade::Address,
        wallet::WalletDescriptor,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Source {
    snapshot: ReadinessSnapshot,
    calls: Arc<AtomicUsize>,
}

impl ReadinessSource for Source {
    async fn read(&self) -> ReadinessSnapshot {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.snapshot.clone()
    }
}

fn expected() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/contracts/readiness_v1.json"
    ))
    .unwrap()
}

fn snapshot() -> ReadinessSnapshot {
    let main = Address::from_bytes([0; 32]);
    let wsol = Address::from_bytes(
        bs58::decode(crate::models::WSOL_MINT)
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap(),
    );
    ReadinessSnapshot {
        default_wallet_id: "fixture-main".into(),
        wallet_count: 2,
        wallets: vec![
            WalletDescriptor {
                wallet_id: "fixture-main".into(),
                pubkey: main,
            },
            WalletDescriptor {
                wallet_id: "fixture-other".into(),
                pubkey: wsol,
            },
        ],
        wallet_metadata: ReadinessWalletMetadata {
            wallet: main,
            rpc_lanes: 3,
            swqos_lanes: vec!["Jito".into(), "Temporal".into(), "Default".into()],
            active_swqos_lanes: vec!["Jito".into(), "Temporal".into()],
            mev_protection: true,
            require_mev_submit: true,
        },
        has_blockhash: true,
        blockhash_age_ms: Some(15),
        rpc_ok: true,
        paused: false,
        pumpapi_executor: false,
        trade_persistence_mode: "durable".into(),
        trade_database_healthy: true,
    }
}

async fn with_source(snapshot: ReadinessSnapshot) -> (AppState, Arc<AtomicUsize>) {
    let mut state = state().await;
    let calls = Arc::new(AtomicUsize::new(0));
    state.readiness = Arc::new(ReadinessService::new(Source {
        snapshot,
        calls: calls.clone(),
    }));
    (state, calls)
}

#[tokio::test]
async fn real_ready_route_preserves_full_old_success_response_without_an_sdk_wallet() {
    let (state, calls) = with_source(snapshot()).await;
    assert_eq!(state.wallets.wallet_count(), 0);
    let app = http::router(state);
    let response = send(&app, "GET", "/ready", LOCAL, Some(TOKEN), None).await;
    assert_eq!(json_body(response, StatusCode::OK).await, expected());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn readiness_truth_table_keeps_pause_database_and_mode_independent() {
    for mode in ["memory", "shadow", "durable"] {
        for has_blockhash in [false, true] {
            for rpc_ok in [false, true] {
                for paused in [false, true] {
                    for database_healthy in [false, true] {
                        let mut snapshot = snapshot();
                        snapshot.has_blockhash = has_blockhash;
                        snapshot.rpc_ok = rpc_ok;
                        snapshot.paused = paused;
                        snapshot.trade_database_healthy = database_healthy;
                        snapshot.trade_persistence_mode = mode.into();
                        let (state, calls) = with_source(snapshot).await;
                        let response = send(
                            &http::router(state),
                            "GET",
                            "/ready",
                            LOCAL,
                            Some(TOKEN),
                            None,
                        )
                        .await;
                        let body = json_body(response, StatusCode::OK).await;
                        assert_eq!(body["ready"], has_blockhash && rpc_ok);
                        assert_eq!(body["paused"], paused);
                        assert_eq!(body["trade_database_healthy"], database_healthy);
                        assert_eq!(body["trade_persistence_mode"], mode);
                        assert_eq!(calls.load(Ordering::Relaxed), 1);
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn every_boolean_maps_to_its_original_wire_field() {
    for field in [
        "has_blockhash",
        "rpc_ok",
        "paused",
        "mev_protection",
        "require_mev_submit",
        "pumpapi_executor",
        "trade_database_healthy",
    ] {
        let mut snapshot = snapshot();
        let mut expected = expected();
        let value = !expected[field].as_bool().unwrap();
        match field {
            "has_blockhash" => snapshot.has_blockhash = value,
            "rpc_ok" => snapshot.rpc_ok = value,
            "paused" => snapshot.paused = value,
            "mev_protection" => snapshot.wallet_metadata.mev_protection = value,
            "require_mev_submit" => snapshot.wallet_metadata.require_mev_submit = value,
            "pumpapi_executor" => snapshot.pumpapi_executor = value,
            "trade_database_healthy" => snapshot.trade_database_healthy = value,
            _ => unreachable!(),
        }
        expected[field] = json!(value);
        expected["ready"] = json!(snapshot.has_blockhash && snapshot.rpc_ok);
        let (state, _) = with_source(snapshot).await;
        let response = send(
            &http::router(state),
            "GET",
            "/ready",
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await;
        assert_eq!(
            json_body(response, StatusCode::OK).await,
            expected,
            "{field}"
        );
    }
}

#[tokio::test]
async fn age_null_and_large_integer_remain_exact_and_are_not_renamed() {
    for age in [None, Some(u128::from(u64::MAX))] {
        let mut snapshot = snapshot();
        snapshot.blockhash_age_ms = age;
        let mut expected = expected();
        expected["blockhash_age_ms"] = serde_json::to_value(age).unwrap();
        let (state, _) = with_source(snapshot).await;
        let response = send(
            &http::router(state),
            "GET",
            "/ready",
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await;
        assert_eq!(json_body(response, StatusCode::OK).await, expected);
    }
}

#[tokio::test]
async fn failed_auth_never_invokes_the_readonly_source() {
    let (state, calls) = with_source(snapshot()).await;
    let app = http::router(state);
    for (peer, token, status, message) in [
        (LOCAL, None, StatusCode::UNAUTHORIZED, "unauthorized"),
        (
            LOCAL,
            Some("Bearer wrong"),
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        (
            DENIED,
            Some(TOKEN),
            StatusCode::FORBIDDEN,
            "client ip not allowed",
        ),
    ] {
        error_body(
            send(&app, "GET", "/ready", peer, token, None).await,
            status,
            message,
        )
        .await;
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }
    assert_eq!(
        send(&app, "GET", "/ready", ALLOWED, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn separate_admin_token_still_cannot_query_trade_readiness() {
    let (mut state, calls) = with_source(snapshot()).await;
    state.edit_fixture_config(|config| {
        config.security = crate::config::SecurityConfig::parse(
            None,
            Some("separate"),
            Some("admin-contract-test-only"),
            "http-contract-test-only",
        )
        .unwrap();
    });
    let app = http::router(state);
    error_body(
        send(
            &app,
            "GET",
            "/ready",
            LOCAL,
            Some("Bearer admin-contract-test-only"),
            None,
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        send(&app, "GET", "/ready", LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn wallet_and_lane_metadata_keep_their_distinct_values_and_order() {
    let mut snapshot = snapshot();
    snapshot.wallet_count = 7;
    snapshot.default_wallet_id = "different-default".into();
    snapshot.wallet_metadata.rpc_lanes = 11;
    snapshot.wallet_metadata.swqos_lanes = vec!["Temporal".into(), "Default".into()];
    snapshot.wallet_metadata.active_swqos_lanes = vec!["Temporal".into()];
    snapshot.wallets.reverse();
    let mut expected = expected();
    expected["wallet_count"] = json!(7);
    expected["default_wallet_id"] = json!("different-default");
    expected["rpc_lanes"] = json!(11);
    expected["swqos_lanes"] = json!(["Temporal", "Default"]);
    expected["active_swqos_lanes"] = json!(["Temporal"]);
    expected["wallets"].as_array_mut().unwrap().reverse();
    let (state, _) = with_source(snapshot).await;
    let response = send(
        &http::router(state),
        "GET",
        "/ready",
        LOCAL,
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!(json_body(response, StatusCode::OK).await, expected);
}

#[tokio::test]
async fn system_routes_preserve_public_health_protected_heads_and_method_rejections() {
    let (mut state, calls) = with_source(snapshot()).await;
    state.edit_fixture_config(|config| {
        config.security = SecurityConfig::parse(
            Some("true"),
            Some("separate"),
            Some("admin-contract-test-only"),
            "http-contract-test-only",
        )
        .unwrap();
    });
    for app in [
        http::router(state.clone()),
        crate::api::routes::system_routes().with_state(state.clone()),
    ] {
        calls.store(0, Ordering::Relaxed);
        assert_eq!(
            json_body(
                send(&app, "GET", "/health", DENIED, None, None).await,
                StatusCode::OK
            )
            .await,
            json!({"ok": true}),
        );
        for path in ["/health", "/ready", "/v1/capabilities"] {
            for method in ["POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
                let response = send(&app, method, path, LOCAL, None, None).await;
                assert_eq!(
                    response.status(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {path}"
                );
                let mut allow = response.headers()["allow"]
                    .to_str()
                    .unwrap()
                    .split(',')
                    .map(str::trim)
                    .collect::<Vec<_>>();
                allow.sort_unstable();
                assert_eq!(allow, vec!["GET", "HEAD"]);
            }
            if path != "/health" {
                for token in [None, Some("Bearer admin-contract-test-only")] {
                    let response = send(&app, "HEAD", path, LOCAL, token, None).await;
                    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                    assert!(to_bytes(response.into_body(), 128 * 1024)
                        .await
                        .unwrap()
                        .is_empty());
                }
                let response = send(&app, "HEAD", path, DENIED, Some(TOKEN), None).await;
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
                assert!(to_bytes(response.into_body(), 128 * 1024)
                    .await
                    .unwrap()
                    .is_empty());
            }
            assert_eq!(calls.load(Ordering::Relaxed), 0);
        }
        for (path, peer, token, expected_calls) in [
            ("/health", DENIED, None, 0),
            ("/v1/capabilities", ALLOWED, Some(TOKEN), 0),
            ("/ready", ALLOWED, Some(TOKEN), 1),
        ] {
            let response = send(&app, "HEAD", path, peer, token, None).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "application/json");
            assert!(to_bytes(response.into_body(), 128 * 1024)
                .await
                .unwrap()
                .is_empty());
            assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
        }
    }
}

#[tokio::test]
async fn standalone_system_route_group_keeps_payloads_without_exposing_other_operations() {
    for enabled in [false, true] {
        let (mut state, calls) = with_source(snapshot()).await;
        state.edit_fixture_config(|config| config.stonkfun_via_sol.enabled = enabled);
        let app = crate::api::routes::system_routes().with_state(state.clone());
        for (path, mut expected) in [
            ("/health", json!({"ok": true})),
            ("/ready", expected()),
            (
                "/v1/capabilities",
                serde_json::to_value(crate::protocol_capabilities::current(enabled)).unwrap(),
            ),
        ] {
            let before = crate::state::now_ms();
            let actual = json_body(
                send(&app, "GET", path, LOCAL, Some(TOKEN), None).await,
                StatusCode::OK,
            )
            .await;
            let after = crate::state::now_ms();
            if path == "/v1/capabilities" {
                let generated = u128::from(actual["generated_at_ms"].as_u64().unwrap());
                assert!((before..=after).contains(&generated));
                expected["generated_at_ms"] = actual["generated_at_ms"].clone();
            }
            assert_eq!(actual, expected);
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        for (method, path) in [
            ("POST", "/v1/cache/warmup"),
            ("POST", "/v1/pools/resolve"),
            ("POST", "/v1/pools/sol-hop"),
            ("GET", "/v1/wallets"),
            ("GET", "/v1/trades"),
            ("GET", "/v1/system/missing"),
        ] {
            assert_eq!(
                send(&app, method, path, LOCAL, Some(TOKEN), None)
                    .await
                    .status(),
                StatusCode::NOT_FOUND,
                "{method} {path}"
            );
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(state.wallets.wallet_count(), 0);
        assert!(state.trades.recent(10).is_empty());
    }
}
