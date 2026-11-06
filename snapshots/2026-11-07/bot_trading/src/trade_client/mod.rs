#![allow(dead_code)]
pub mod conversions;

use anyhow::{Context, Result};
use reqwest::{Client, RequestBuilder, Url};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

pub fn validate_pool_quote(pool: &str, quote: Option<&str>) -> Result<()> {
    if pool.trim().eq_ignore_ascii_case("pump")
        && quote.is_some()
        && !matches!(quote, Some(WSOL_MINT | USDC_MINT))
    {
        anyhow::bail!("Pump 池的报价币不受支持，仅支持 WSOL 或 USDC");
    }
    if pool.trim().eq_ignore_ascii_case("raydium-cpmm")
        && !matches!(quote, Some(WSOL_MINT | USDC_MINT | USDT_MINT))
    {
        anyhow::bail!("Raydium CPMM 池的报价币不受支持，仅支持 WSOL、USDC 或 USDT");
    }
    if pool.trim().eq_ignore_ascii_case("meteora-damm-v2")
        && !matches!(quote, Some(WSOL_MINT | USDC_MINT))
    {
        anyhow::bail!(
            "Meteora DAMM V2 池的报价币不受支持，仅支持 WSOL 或 USDC；请刷新交易面板并重新确认池"
        );
    }
    Ok(())
}

#[derive(Clone)]
pub struct TradeServiceClient {
    base_url: String,
    token: Option<String>,
    route_decision_base_url: Option<String>,
    route_decision_token: Option<String>,
    client: Client,
    protocol_capability_cache: Arc<Mutex<Option<(Instant, ProtocolCapabilitiesResponse)>>>,
    submission_enabled: bool,
}

#[cfg(test)]
mod submission_policy_tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn request(simulate: bool) -> TradeRequest {
        serde_json::from_value(serde_json::json!({
            "execution_method": "sdk", "side": "buy", "pool": "pump",
            "mint": "test-mint", "amount_sol": 0.001, "slippage_percent": 1.0,
            "tip_sol": 0.0, "use_mev": false, "simulate": simulate,
            "idempotency_key": "local-policy-test"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn unsupported_cpmm_quote_is_blocked_before_network() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TradeServiceClient::new(
            format!("http://{}", listener.local_addr().unwrap()),
            None,
            1,
        )
        .unwrap()
        .with_submission_enabled(true);
        let mut trade = request(false);
        trade.pool = "raydium-cpmm".into();
        for quote in [None, Some("DJTu7vi8norVzdVAffgvb39VP7wjKeTsgaMBJrzfxvoF")] {
            trade.quote_mint = quote.map(str::to_string);
            let error = client.submit_trade(&trade).await.unwrap_err();
            assert!(error.to_string().contains("Raydium CPMM"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
        assert!(validate_pool_quote("raydium-cpmm", Some(WSOL_MINT)).is_ok());
        assert!(validate_pool_quote("raydium-cpmm", Some(USDC_MINT)).is_ok());
        assert!(validate_pool_quote("raydium-cpmm", Some(USDT_MINT)).is_ok());
    }

    #[tokio::test]
    async fn unsupported_pump_quote_is_blocked_before_network() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TradeServiceClient::new(
            format!("http://{}", listener.local_addr().unwrap()),
            None,
            1,
        )
        .unwrap()
        .with_submission_enabled(true);
        for simulate in [false, true] {
            let mut trade = request(simulate);
            trade.quote_mint = Some("WXMRyRZhsa19ety5erZhHg4N3xj3EVN92u94422teJp".into());
            assert!(client
                .submit_trade(&trade)
                .await
                .unwrap_err()
                .to_string()
                .contains("Pump 池"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
        for quote in [None, Some(WSOL_MINT), Some(USDC_MINT)] {
            assert!(validate_pool_quote("pump", quote).is_ok());
        }
    }

    #[tokio::test]
    async fn unsupported_meteora_quote_and_sol_usdc_are_blocked_before_network() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TradeServiceClient::new(
            format!("http://{}", listener.local_addr().unwrap()),
            None,
            1,
        )
        .unwrap()
        .with_submission_enabled(true);
        let mut trade = request(false);
        trade.pool = "meteora-damm-v2".into();
        for quote in [
            None,
            Some("2JnYVAeY4gYjRSmGRPQi9EudJm88AT6LgVKeKvxeb9Qp"),
            Some(USDC_MINT),
        ] {
            trade.quote_mint = quote.map(str::to_string);
            assert!(client.submit_trade(&trade).await.is_err());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
        assert!(validate_pool_quote("meteora-damm-v2", Some(WSOL_MINT)).is_ok());
        assert!(validate_pool_quote("meteora-damm-v2", Some(USDC_MINT)).is_ok());
        assert!(validate_pool_quote("stonkfun", Some("custom-quote")).is_ok());
    }

    #[tokio::test]
    async fn disabled_policy_blocks_live_and_simulated_requests_before_network() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TradeServiceClient::new(
            format!("http://{}", listener.local_addr().unwrap()),
            None,
            1,
        )
        .unwrap();
        for simulate in [false, true] {
            let error = client
                .clone()
                .submit_trade(&request(simulate))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("交易提交已禁用"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn disabled_policy_keeps_reads_and_explicit_enable_allows_mock_submission() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for expected in ["GET /health ", "POST /v1/trades "] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(String::from_utf8_lossy(&buffer[..count]).starts_with(expected));
                let body = "{\"status\":\"accepted\",\"request_id\":\"mock-only\"}";
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let client = TradeServiceClient::new(format!("http://{address}"), None, 1).unwrap();
        assert!(client.health().await.is_ok());
        let result = client
            .with_submission_enabled(true)
            .submit_trade(&request(true))
            .await
            .unwrap();
        assert_eq!(result.request_id.as_deref(), Some("mock-only"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn isolated_sdk_simulation_uses_only_decision_origin() {
        let trade_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_url = format!("http://{}", isolated_listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let responses = [
                (
                    "GET /ready ",
                    r#"{"ready":true,"default_wallet_id":"w1","wallets":[{"wallet_id":"w1","pubkey":"test-pubkey"}]}"#,
                ),
                (
                    "POST /v1/routes/decide ",
                    r#"{"schema_version":1,"status":"sdk_verified","reason":null,"wallet_id":"w1","wallet_pubkey":"test-pubkey","decision_id":"decision-1","route_id":"route-1","expires_at_ms":9999999999999,"simulate_only":true}"#,
                ),
                (
                    "POST /v1/trades ",
                    r#"{"request_id":"sim-1","wallet_id":"w1","wallet_pubkey":"test-pubkey","status":"simulated","signature":null}"#,
                ),
            ];
            for (expected, body) in responses {
                let (mut socket, _) = isolated_listener.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                assert!(wire.starts_with(expected), "{wire}");
                assert!(wire
                    .to_ascii_lowercase()
                    .contains("authorization: bearer isolated-token"));
                if expected == "POST /v1/trades " {
                    assert!(wire.contains("\"sdk_decision_id\":\"decision-1\""));
                    assert!(wire.contains("\"simulate\":true"));
                }
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let client = TradeServiceClient::new(
            format!("http://{}", trade_listener.local_addr().unwrap()),
            Some("trade-token".into()),
            1,
        )
        .unwrap()
        .with_route_decision_endpoint(Some(isolated_url), Some("isolated-token".into()))
        .unwrap();
        let mut request = request(true);
        request.wallet_id = Some("w1".into());
        let (result, bound_request) = client
            .submit_isolated_sdk_simulation(&request, "test-pubkey")
            .await
            .unwrap();
        assert_eq!(result.status.as_deref(), Some("simulated"));
        assert_eq!(bound_request.sdk_decision_id.as_deref(), Some("decision-1"));
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), trade_listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn isolated_cpmm_usdt_sell_requires_verified_decision_and_uses_only_candidate() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", isolated.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let replies = [
                (
                    "GET /ready ",
                    r#"{"ready":true,"default_wallet_id":"w1","wallets":[{"wallet_id":"w1","pubkey":"test-pubkey"}]}"#,
                ),
                (
                    "POST /v1/routes/decide ",
                    r#"{"schema_version":1,"status":"sdk_verified","reason":null,"wallet_id":"w1","wallet_pubkey":"test-pubkey","decision_id":"sdk-decision:sell:one","route_id":"route-sell","expires_at_ms":9999999999999,"simulate_only":true,"sdk_capability":{"package_builder_family":"StonkFunQuoteRoute","service_adapter":"cpmm_two_hop_sell_simulate_only","route_evidence":"unsigned_simulation_verified","sdk_unsupported_proven":false,"jupiter_candidate_allowed":false}}"#,
                ),
                (
                    "POST /v1/trades ",
                    r#"{"request_id":"sell-1","wallet_id":"w1","wallet_pubkey":"test-pubkey","status":"simulated","signature":null}"#,
                ),
            ];
            for (path, body) in replies {
                let (mut socket, _) = isolated.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let size = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..size]);
                assert!(wire.starts_with(path), "{wire}");
                if path.starts_with("POST") {
                    assert!(wire.contains("\"side\":\"sell\""));
                    assert!(wire.contains("\"settlement_asset\":\"sol\""));
                    assert!(wire.contains("\"sol_hop_pool_id\""));
                }
                if path == "POST /v1/trades " {
                    assert!(wire.contains("\"sdk_decision_id\":\"sdk-decision:sell:one\""));
                }
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let client =
            TradeServiceClient::new(format!("http://{}", primary.local_addr().unwrap()), None, 5)
                .unwrap()
                .with_route_decision_endpoint(Some(base_url), Some("token".into()))
                .unwrap();
        let mut trade = request(true);
        trade.wallet_id = Some("w1".into());
        trade.side = "sell".into();
        trade.pool = "raydium-cpmm".into();
        trade.mint = "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh".into();
        trade.pool_id = Some("bsitkqbNk5dPgyrVpHRjiAPNQk66xCfNs8wE981Cd8e".into());
        trade.quote_mint = Some(USDT_MINT.into());
        trade.settlement_asset = Some("sol".into());
        trade.sol_hop_pool = Some("raydium-cpmm".into());
        trade.sol_hop_pool_id = Some("AddZXoGEs6pR6bgp51vjNasKN8jFnizsu6j5P3PSphqk".into());
        trade.amount_sol = None;
        trade.sell_percent = Some(100);
        trade.tip_sol = None;
        trade.use_mev = None;
        let (result, bound) = client
            .submit_isolated_sdk_simulation(&trade, "test-pubkey")
            .await
            .unwrap();
        assert_eq!(result.status.as_deref(), Some("simulated"));
        assert_eq!(
            bound.sdk_decision_id.as_deref(),
            Some("sdk-decision:sell:one")
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn isolated_sdk_bridge_requires_unsigned_preview_before_trade() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", isolated.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let responses = [
                (
                    "GET /ready ",
                    r#"{"ready":true,"default_wallet_id":"w1","wallets":[{"wallet_id":"w1","pubkey":"test-pubkey"}]}"#,
                ),
                (
                    "POST /v1/routes/decide ",
                    r#"{"schema_version":1,"status":"unresolved_retryable","reason":"bridge_verified_sdk_capability_unresolved","wallet_id":"w1","wallet_pubkey":"test-pubkey","decision_id":null,"route_id":null,"expires_at_ms":null,"simulate_only":true,"observed_bridge":{"observation_id":"observe:sol-bridge:one","wallet_id":"w1","wallet_pubkey":"test-pubkey"}}"#,
                ),
                (
                    "POST /v1/routes/observations/sdk-preview ",
                    r#"{"observation_id":"observe:sol-bridge:refreshed","preview_id":"observe:sdk-preview:one","executable":false,"simulate_only":true,"preview":{"simulation":{"ok":true,"minimum_output_verified":true}}}"#,
                ),
                (
                    "POST /v1/routes/decide ",
                    r#"{"schema_version":1,"status":"sdk_verified","reason":null,"wallet_id":"w1","wallet_pubkey":"test-pubkey","decision_id":"sdk-decision:bridge:one","route_id":"observe:sol-bridge:refreshed","expires_at_ms":9999999999999,"simulate_only":true,"sdk_capability":{"package_builder_family":"PumpSwapQuoteRoute","service_adapter":"simulate_only_bridge_decision","route_evidence":"unsigned_simulation_verified","sdk_unsupported_proven":false,"jupiter_candidate_allowed":false}}"#,
                ),
                (
                    "POST /v1/trades ",
                    r#"{"request_id":"sim-bridge-1","wallet_id":"w1","wallet_pubkey":"test-pubkey","status":"simulated","signature":null}"#,
                ),
            ];
            for (index, (expected, body)) in responses.into_iter().enumerate() {
                let (mut socket, _) = isolated.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                assert!(wire.starts_with(expected), "{wire}");
                if index == 2 {
                    assert!(wire.contains("\"simulate_transaction\":true"));
                    assert!(wire.contains("\"observation_id\":\"observe:sol-bridge:one\""));
                }
                if index == 3 {
                    assert!(wire.contains("\"sdk_preview_id\":\"observe:sdk-preview:one\""));
                }
                if index == 4 {
                    assert!(wire.contains("\"sdk_decision_id\":\"sdk-decision:bridge:one\""));
                    assert!(wire.contains("\"simulate\":true"));
                    assert!(!wire.contains("\"sdk_preview_id\""));
                }
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let client =
            TradeServiceClient::new(format!("http://{}", primary.local_addr().unwrap()), None, 2)
                .unwrap()
                .with_route_decision_endpoint(Some(url), Some("isolated-token".into()))
                .unwrap();
        let mut trade = request(true);
        trade.wallet_id = Some("w1".into());
        trade.pool = "pump-amm".into();
        trade.quote_mint = Some("TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo".into());
        let (response, bound) = client
            .submit_isolated_sdk_simulation(&trade, "test-pubkey")
            .await
            .unwrap();
        assert_eq!(response.status.as_deref(), Some("simulated"));
        assert_eq!(
            bound.sdk_decision_id.as_deref(),
            Some("sdk-decision:bridge:one")
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn isolated_sdk_bridge_rejects_failed_preview_before_trade() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", isolated.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let responses = [
                (
                    "GET /ready ",
                    r#"{"ready":true,"default_wallet_id":"w1","wallets":[{"wallet_id":"w1","pubkey":"test-pubkey"}]}"#,
                ),
                (
                    "POST /v1/routes/decide ",
                    r#"{"schema_version":1,"status":"unresolved_retryable","reason":"bridge_verified_sdk_capability_unresolved","wallet_id":"w1","wallet_pubkey":"test-pubkey","simulate_only":true,"observed_bridge":{"observation_id":"observe:sol-bridge:one","wallet_id":"w1","wallet_pubkey":"test-pubkey"}}"#,
                ),
                (
                    "POST /v1/routes/observations/sdk-preview ",
                    r#"{"observation_id":"observe:sol-bridge:one","preview_id":"observe:sdk-preview:one","executable":false,"simulate_only":true,"preview":{"simulation":{"ok":false,"minimum_output_verified":false}}}"#,
                ),
            ];
            for (expected, body) in responses {
                let (mut socket, _) = isolated.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(String::from_utf8_lossy(&buffer[..count]).starts_with(expected));
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(30), isolated.accept())
                    .await
                    .is_err()
            );
        });
        let client =
            TradeServiceClient::new(format!("http://{}", primary.local_addr().unwrap()), None, 2)
                .unwrap()
                .with_route_decision_endpoint(Some(url), Some("isolated-token".into()))
                .unwrap();
        let mut trade = request(true);
        trade.wallet_id = Some("w1".into());
        trade.pool = "pump-amm".into();
        trade.quote_mint = Some("TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo".into());
        assert!(client
            .submit_isolated_sdk_simulation(&trade, "test-pubkey")
            .await
            .is_err());
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn explicit_pool_resolves_quote_only_from_isolated_chain_verification() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", isolated.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = isolated.accept().await.unwrap();
            let mut buffer = [0; 8192];
            let count = socket.read(&mut buffer).await.unwrap();
            let wire = String::from_utf8_lossy(&buffer[..count]);
            assert!(wire.starts_with("POST /v1/pools/resolve-candidates "));
            assert!(wire.contains("\"quote_mint\":null"));
            let body = r#"{"chain_verified":true,"protocol_id":"pump-amm","pool_id":"G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b","mint":"nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump","quote_mint":"TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo"}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let client =
            TradeServiceClient::new(format!("http://{}", primary.local_addr().unwrap()), None, 2)
                .unwrap()
                .with_route_decision_endpoint(Some(url), Some("isolated-token".into()))
                .unwrap();
        let route = client
            .isolated_sdk_resolve_explicit_pool(
                "nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump",
                "pump-amm",
                "G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b",
            )
            .await
            .unwrap();
        assert_eq!(
            route.quote_mint,
            "TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo"
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn isolated_sdk_simulation_rejects_live_and_non_sdk_before_network() {
        let trade_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TradeServiceClient::new(
            format!("http://{}", trade_listener.local_addr().unwrap()),
            None,
            1,
        )
        .unwrap()
        .with_route_decision_endpoint(
            Some(format!(
                "http://{}",
                isolated_listener.local_addr().unwrap()
            )),
            Some("token".into()),
        )
        .unwrap();
        let mut request = request(false);
        request.wallet_id = Some("w1".into());
        assert!(client
            .submit_isolated_sdk_simulation(&request, "test-pubkey")
            .await
            .is_err());
        request.simulate = true;
        request.execution_method = "pumpapi".into();
        assert!(client
            .submit_isolated_sdk_simulation(&request, "test-pubkey")
            .await
            .is_err());
        request.execution_method = "sdk".into();
        assert!(client
            .submit_isolated_sdk_simulation(&request, "")
            .await
            .is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(30), isolated_listener.accept())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), trade_listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn isolated_wsol_sell_pool_comes_only_from_candidate_origin() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_url = format!("http://{}", isolated.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = isolated.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let count = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]);
            assert!(request.starts_with("POST /v1/pools/sol-hop "));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer isolated-token"));
            assert!(request.contains(USDC_MINT));
            let body = r#"{"available":true,"pool":"raydium-cpmm","pool_id":"fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2","reason":null}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let client =
            TradeServiceClient::new(format!("http://{}", primary.local_addr().unwrap()), None, 1)
                .unwrap()
                .with_route_decision_endpoint(Some(isolated_url), Some("isolated-token".into()))
                .unwrap();
        let route = client.isolated_wsol_usdc_sell_pool().await.unwrap();
        assert_eq!(route.mint, WSOL_MINT);
        assert_eq!(route.quote_mint, USDC_MINT);
        assert_eq!(
            route.pool_id.as_deref(),
            Some("fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2")
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn disabled_submission_still_reads_server_decision_without_trade_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for body in [
                r#"{"schema_version":1,"status":"unresolved_retryable","reason":"bridge_verified_sdk_capability_unresolved","wallet_id":"w1","decision_id":null,"route_id":null,"expires_at_ms":null,"sdk_capability":{"package_builder_family":"StonkFunQuoteRoute","service_adapter":"cpmm_cpmm_simulate_only","route_evidence":"chain_verified_bridge_pool_identity","sdk_unsupported_proven":false,"jupiter_candidate_allowed":false},"simulate_only":true}"#,
                r#"{"schema_version":1,"status":"sdk_unsupported","reason":null,"wallet_id":"w1","decision_id":null,"route_id":null,"expires_at_ms":null,"simulate_only":true}"#,
                r#"{"schema_version":1,"status":"unresolved_retryable","reason":null,"wallet_id":"w1","decision_id":null,"route_id":null,"expires_at_ms":null,"sdk_capability":{"package_builder_family":"PumpSwapQuoteRoute","service_adapter":"simulate_only_preview","route_evidence":"chain_verified_bridge_pool_identity","sdk_unsupported_proven":false,"jupiter_candidate_allowed":true},"simulate_only":true}"#,
                r#"{"schema_version":1,"status":"sdk_verified","reason":null,"wallet_id":"w1","decision_id":null,"route_id":null,"expires_at_ms":null,"simulate_only":true}"#,
                r#"{"schema_version":1,"status":"sdk_verified","reason":null,"wallet_id":"w1","decision_id":"decision-1","route_id":"route-1","expires_at_ms":9999999999999,"sdk_capability":{"package_builder_family":"PumpSwapQuoteRoute","service_adapter":"direct_sdk","route_evidence":"chain_verified","sdk_unsupported_proven":true,"jupiter_candidate_allowed":true},"simulate_only":true}"#,
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(String::from_utf8_lossy(&buffer[..count])
                    .starts_with("POST /v1/routes/decide "));
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(30), listener.accept())
                    .await
                    .is_err()
            );
        });
        let client = TradeServiceClient::new(format!("http://{address}"), None, 1).unwrap();
        let mut trade = request(true);
        trade.wallet_id = Some("w1".into());
        let decision = client.route_decision(&trade, None).await.unwrap();
        assert_eq!(decision.status, "unresolved_retryable");
        assert_eq!(
            decision
                .sdk_capability
                .as_ref()
                .unwrap()
                .package_builder_family,
            "StonkFunQuoteRoute"
        );
        assert_eq!(
            decision.read_only_route_hint(),
            Some("服务端已找到 SDK 两跳候选；完成本次预检与模拟前不会执行，Jupiter 尚未获得授权。")
        );
        assert!(client.route_decision(&trade, None).await.is_err());
        assert!(client.route_decision(&trade, None).await.is_err());
        assert!(client.route_decision(&trade, None).await.is_err());
        assert!(client.route_decision(&trade, None).await.is_err());
        assert!(client.submit_trade(&trade).await.is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn separate_decision_origin_never_receives_trade_submission() {
        let trade_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let decision_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let decision_url = format!("http://{}", decision_listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (wallet_pubkey, simulate_only) in [
                (Some("test-pubkey"), true),
                (Some("wrong-pubkey"), true),
                (None, true),
                (Some("test-pubkey"), false),
            ] {
                let (mut socket, _) = decision_listener.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..count]);
                assert!(request.starts_with("POST /v1/routes/decide "));
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer decision-token"));
                assert!(!request.contains("trade-token"));
                let body = serde_json::json!({
                    "schema_version": 1,
                    "status": "unresolved_retryable",
                    "reason": "route_verification_unavailable",
                    "wallet_id": "w1",
                    "wallet_pubkey": wallet_pubkey,
                    "decision_id": null,
                    "route_id": null,
                    "expires_at_ms": null,
                    "simulate_only": simulate_only,
                })
                .to_string();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let client = TradeServiceClient::new(
            format!("http://{}", trade_listener.local_addr().unwrap()),
            Some("trade-token".into()),
            1,
        )
        .unwrap()
        .with_route_decision_endpoint(Some(decision_url), Some("decision-token".into()))
        .unwrap();
        let mut trade = request(true);
        trade.wallet_id = Some("w1".into());
        assert!(client.route_decision(&trade, None).await.is_err());
        assert_eq!(
            client
                .route_decision(&trade, Some("test-pubkey"))
                .await
                .unwrap()
                .status,
            "unresolved_retryable"
        );
        assert!(client
            .route_decision(&trade, Some("test-pubkey"))
            .await
            .is_err());
        assert!(client
            .route_decision(&trade, Some("test-pubkey"))
            .await
            .is_err());
        assert!(client
            .route_decision(&trade, Some("test-pubkey"))
            .await
            .is_err());
        assert!(client.submit_trade(&trade).await.is_err());
        assert!(client
            .clone()
            .with_submission_enabled(true)
            .submit_trade(&trade)
            .await
            .is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(30), trade_listener.accept())
                .await
                .is_err()
        );
        server.await.unwrap();
    }

    #[test]
    fn separate_decision_origin_requires_local_endpoint_and_closed_submission() {
        let client = TradeServiceClient::new("http://127.0.0.1:18788".into(), None, 1).unwrap();
        for url in [
            "https://127.0.0.1:18887",
            "http://example.test:18887",
            "http://127.0.0.1:18788",
            "http://127.0.0.1:18887/path",
            "http://user:secret@127.0.0.1:18887",
        ] {
            assert!(client
                .clone()
                .with_route_decision_endpoint(Some(url.into()), Some("token".into()))
                .is_err());
        }
        assert!(client
            .clone()
            .with_route_decision_endpoint(Some("http://127.0.0.1:18887".into()), None)
            .is_err());
        assert!(
            TradeServiceClient::new("http://127.0.0.1:18887".into(), None, 1)
                .unwrap()
                .with_route_decision_endpoint(
                    Some("http://127.0.0.1:18887".into()),
                    Some("token".into())
                )
                .is_err()
        );
        assert!(client
            .with_submission_enabled(true)
            .with_route_decision_endpoint(
                Some("http://127.0.0.1:18887".into()),
                Some("token".into())
            )
            .is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRequest {
    pub wallet_id: Option<String>,
    pub execution_method: String,
    pub side: String,
    pub pool: String,
    pub mint: String,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
    pub amount_quote_units: Option<u64>,
    pub settlement_asset: Option<String>,
    pub sol_hop_pool: Option<String>,
    pub sol_hop_pool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_decision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_preview_id: Option<String>,
    pub amount_sol: Option<f64>,
    pub sell_percent: Option<u64>,
    pub slippage_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip_sol: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_mev: Option<bool>,
    pub mev_provider: Option<String>,
    pub simulate: bool,
    pub source: Option<String>,
    pub source_signature: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResponse {
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub wallet_id: Option<String>,
    #[serde(default)]
    pub wallet_pubkey: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub duplicate: bool,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteDecisionResponse {
    pub schema_version: u32,
    pub status: String,
    pub reason: Option<String>,
    pub wallet_id: String,
    #[serde(default)]
    pub wallet_pubkey: Option<String>,
    pub decision_id: Option<String>,
    pub route_id: Option<String>,
    pub expires_at_ms: Option<u64>,
    #[serde(default)]
    pub observed_bridge: Option<ObservedSdkBridge>,
    #[serde(default)]
    pub sdk_capability: Option<SdkCapabilityEvidence>,
    #[serde(default)]
    pub simulate_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservedSdkBridge {
    pub observation_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
}

#[derive(Debug, Deserialize)]
struct SdkBridgePreviewResponse {
    preview_id: Option<String>,
    observation_id: String,
    executable: bool,
    simulate_only: bool,
    preview: SdkBridgePreview,
}

#[derive(Debug, Deserialize)]
struct SdkBridgePreview {
    simulation: Option<SdkBridgeSimulation>,
}

#[derive(Debug, Deserialize)]
struct SdkBridgeSimulation {
    ok: bool,
    minimum_output_verified: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SdkCapabilityEvidence {
    pub package_builder_family: String,
    pub service_adapter: String,
    pub route_evidence: String,
    pub sdk_unsupported_proven: bool,
    pub jupiter_candidate_allowed: bool,
}

impl RouteDecisionResponse {
    /// This text is advisory only. It never selects an execution backend.
    pub fn read_only_route_hint(&self) -> Option<&'static str> {
        let evidence = self.sdk_capability.as_ref()?;
        match (self.status.as_str(), evidence.service_adapter.as_str()) {
            (
                "unresolved_retryable",
                "simulate_only_preview"
                | "simulate_only_bridge_decision"
                | "cpmm_pumpswap_simulate_only"
                | "cpmm_amm_v4_simulate_only"
                | "cpmm_cpmm_simulate_only",
            ) if !evidence.sdk_unsupported_proven && !evidence.jupiter_candidate_allowed => Some(
                "服务端已找到 SDK 两跳候选；完成本次预检与模拟前不会执行，Jupiter 尚未获得授权。",
            ),
            ("unresolved_retryable", "explicit_via_sol_request")
                if !evidence.sdk_unsupported_proven && !evidence.jupiter_candidate_allowed =>
            {
                Some("服务端需要明确的 SDK 两跳请求；Jupiter 尚未获得这笔交易的授权。")
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedPoolRoute {
    pub mint: String,
    pub pool: String,
    pub pool_id: Option<String>,
    pub quote_mint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SolHopResponse {
    pub available: bool,
    pub pool: Option<String>,
    pub pool_id: Option<String>,
    pub reason: Option<String>,
}

impl SolHopResponse {
    pub fn verified_pool_id(&self) -> Result<&str> {
        anyhow::ensure!(self.available, "SOL 两跳不可用，请刷新路由");
        anyhow::ensure!(
            self.pool.as_deref() == Some("raydium-cpmm"),
            "SOL 两跳池类型不匹配"
        );
        let id = self.pool_id.as_deref().context("SOL 两跳缺少池地址")?;
        anyhow::ensure!(
            bs58::decode(id)
                .into_vec()
                .is_ok_and(|bytes| bytes.len() == 32),
            "SOL 两跳池地址无效"
        );
        Ok(id)
    }
}

#[derive(Debug, Serialize)]
struct ResolvePoolRequest<'a> {
    mint: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeServiceWallet {
    pub wallet_id: String,
    pub pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletsResponse {
    pub default_wallet_id: String,
    pub wallets: Vec<TradeServiceWallet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub default_wallet_id: String,
    pub wallets: Vec<TradeServiceWallet>,
    #[serde(default)]
    pub rpc_ok: bool,
    #[serde(default)]
    pub has_blockhash: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AggregatorProviderCapability {
    pub provider: String,
    #[serde(default)]
    pub supports_quote: bool,
    #[serde(default)]
    pub supports_build: bool,
    #[serde(default)]
    pub supports_preflight: bool,
    #[serde(default)]
    pub supports_submit: bool,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AggregatorCapabilitiesResponse {
    pub schema_version: u32,
    #[serde(default)]
    pub simulate_only: bool,
    pub providers: Vec<AggregatorProviderCapability>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtocolCapability {
    pub protocol_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub supports_buy: bool,
    #[serde(default)]
    pub supports_sell: bool,
    #[serde(default)]
    pub supports_direct_quote: bool,
    #[serde(default)]
    pub supports_sol_route: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtocolCapabilitiesResponse {
    pub schema_version: u32,
    #[serde(default)]
    pub generated_at_ms: u128,
    #[serde(default)]
    pub protocols: Vec<ProtocolCapability>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AggregatorShadowMetric {
    pub provider: String,
    pub wallet_id: String,
    pub input_mint: String,
    pub output_mint: String,
    pub amount_lamports: u64,
    pub slippage_bps: u16,
    pub status: String,
    pub latency_ms: u64,
    pub expected_output_units: Option<u64>,
    pub minimum_output_units: Option<u64>,
    pub route_fingerprint: Option<String>,
    pub price_impact_percent: Option<String>,
    pub error_class: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AggregatorShadowMetricsResponse {
    pub schema_version: u32,
    pub read_only: bool,
    pub metrics: Vec<AggregatorShadowMetric>,
    #[serde(default)]
    pub summary: Option<AggregatorShadowSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AggregatorShadowSummary {
    pub sample_count: u64,
    pub quoted_count: u64,
    pub failed_count: u64,
    pub unavailable_count: u64,
    pub p50_latency_ms: Option<u64>,
    pub p95_latency_ms: Option<u64>,
    pub p99_latency_ms: Option<u64>,
    pub success_rate: Option<f64>,
    pub expiry_rate: Option<f64>,
    #[serde(default)]
    pub min_output_checked_count: Option<u64>,
    #[serde(default)]
    pub min_output_missing_count: Option<u64>,
    #[serde(default)]
    pub min_output_pass_count: Option<u64>,
    #[serde(default)]
    pub min_output_coverage_rate: Option<f64>,
    #[serde(default)]
    pub min_output_pass_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedWalletResponse {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub pubkey: String,
    pub is_default: bool,
    pub enabled: bool,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedWalletsResponse {
    pub wallets: Vec<ManagedWalletResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletBalanceItem {
    pub wallet_id: String,
    pub pubkey: String,
    pub balance_lamports: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletBalancesResponse {
    pub fetched_at_ms: u128,
    pub wallets: Vec<WalletBalanceItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuoteBalanceResponse {
    pub quote_mint: String,
    pub decimals: u8,
    pub balance_units: u64,
    pub balance_lamports: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VanityJobResponse {
    pub job_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub match_side: String,
    pub pattern: String,
    pub status: String,
    pub attempts: u64,
    pub max_attempts: u64,
    pub wallet_id: Option<String>,
    pub pubkey: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateVanityJobRequest<'a> {
    owner_type: &'a str,
    owner_id: String,
    label: &'a str,
    match_side: &'a str,
    pattern: &'a str,
}

#[derive(Debug, Serialize)]
pub struct SolTransferRequest<'a> {
    pub owner_type: &'a str,
    pub owner_id: String,
    pub to_wallet_id: Option<&'a str>,
    pub to_address: Option<&'a str>,
    pub amount_mode: &'a str,
    pub amount_lamports: Option<u64>,
    pub percent: Option<u64>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolTransferResponse {
    pub transfer_id: String,
    pub wallet_id: String,
    pub from_pubkey: String,
    pub to_pubkey: String,
    pub status: String,
    pub amount_lamports: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub balance_before_lamports: Option<u64>,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
}

#[derive(Debug, Serialize)]
struct CreateWalletRequest<'a> {
    owner_type: &'a str,
    owner_id: String,
    label: &'a str,
    private_key_base58: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct UpdateWalletRequest<'a> {
    label: &'a str,
}

#[derive(Debug, Deserialize)]
struct ExportWalletResponse {
    private_key_base58: String,
}

impl TradeServiceClient {
    pub fn new(base_url: String, token: Option<String>, timeout_secs: u64) -> Result<Self> {
        let parsed =
            Url::parse(base_url.trim_end_matches('/')).context("invalid trade_service URL")?;
        anyhow::ensure!(
            matches!(parsed.scheme(), "http" | "https")
                && parsed.host_str().is_some()
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none(),
            "trade_service URL must not contain credentials, query, or fragment"
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .build()
            .context("failed to build trade_service HTTP client")?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            route_decision_base_url: None,
            route_decision_token: None,
            client,
            protocol_capability_cache: Arc::new(Mutex::new(None)),
            submission_enabled: false,
        })
    }

    pub fn with_submission_enabled(mut self, enabled: bool) -> Self {
        self.submission_enabled = enabled;
        self
    }

    /// Configure a separate local route decision service. Ordinary transaction
    /// requests continue to use `base_url` behind the submit guard. The
    /// explicit isolated SDK simulation method uses this same local origin.
    pub fn with_route_decision_endpoint(
        mut self,
        url: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        match (url, token) {
            (None, None) => Ok(self),
            (Some(url), Some(token)) => {
                anyhow::ensure!(
                    !self.submission_enabled,
                    "separate route decision endpoint requires disabled trade submission"
                );
                let parsed =
                    Url::parse(url.trim_end_matches('/')).context("invalid route decision URL")?;
                anyhow::ensure!(
                    parsed.scheme() == "http"
                        && parsed.host_str() == Some("127.0.0.1")
                        && parsed.port().is_some()
                        && parsed.port() != Some(18788)
                        && url.trim_end_matches('/') != self.base_url
                        && parsed.path() == "/"
                        && parsed.username().is_empty()
                        && parsed.password().is_none()
                        && parsed.query().is_none()
                        && parsed.fragment().is_none(),
                    "route decision URL must be a separate loopback HTTP origin"
                );
                anyhow::ensure!(!token.trim().is_empty(), "route decision token is empty");
                self.route_decision_base_url = Some(url.trim_end_matches('/').to_string());
                self.route_decision_token = Some(token);
                Ok(self)
            }
            _ => anyhow::bail!("route decision URL and token must be configured together"),
        }
    }

    pub fn submission_enabled(&self) -> bool {
        self.submission_enabled && self.route_decision_base_url.is_none()
    }

    pub fn isolated_sdk_simulation_available(&self) -> bool {
        !self.submission_enabled && self.route_decision_base_url.is_some()
    }

    pub fn ensure_submission_enabled(&self) -> Result<()> {
        if !self.submission_enabled() {
            anyhow::bail!("交易提交已禁用：当前为钱包验收模式");
        }
        Ok(())
    }

    pub async fn health(&self) -> Result<String> {
        let response = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .context("failed to call trade_service health")?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("trade_service health returned {status}");
        }
        Ok(body)
    }

    /// Read-only protocol capability snapshot. It is diagnostic only and does
    /// not authorize route selection or trade submission.
    pub async fn protocol_capabilities(&self) -> Result<ProtocolCapabilitiesResponse> {
        if let Some((cached_at, capabilities)) = self
            .protocol_capability_cache
            .lock()
            .expect("protocol capability cache poisoned")
            .as_ref()
        {
            if cached_at.elapsed() < Duration::from_secs(30) {
                return Ok(capabilities.clone());
            }
        }
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/v1/capabilities", self.base_url)),
            )
            .send()
            .await
            .context("failed to query protocol capabilities")?;
        let capabilities: ProtocolCapabilitiesResponse = self
            .decode_response(response, "query protocol capabilities")
            .await?;
        *self
            .protocol_capability_cache
            .lock()
            .expect("protocol capability cache poisoned") =
            Some((Instant::now(), capabilities.clone()));
        Ok(capabilities)
    }

    /// Read-only provider diagnostics. The bot must not use this endpoint to
    /// select an execution route or submit an aggregator transaction.
    pub async fn aggregator_capabilities(&self) -> Result<AggregatorCapabilitiesResponse> {
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/v1/aggregators/capabilities", self.base_url)),
            )
            .send()
            .await
            .context("failed to query aggregator capabilities")?;
        self.decode_response(response, "query aggregator capabilities")
            .await
    }

    /// Read-only Shadow observations for diagnostics and reporting.
    pub async fn aggregator_shadow_metrics(
        &self,
        wallet_id: Option<&str>,
        provider: Option<&str>,
        limit: u32,
    ) -> Result<AggregatorShadowMetricsResponse> {
        let mut query = Vec::new();
        if let Some(wallet_id) = wallet_id {
            query.push(("wallet_id", wallet_id.to_string()));
        }
        if let Some(provider) = provider {
            query.push(("provider", provider.to_string()));
        }
        query.push(("limit", limit.clamp(1, 100).to_string()));
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/v1/aggregators/shadow/metrics", self.base_url))
                    .query(&query),
            )
            .send()
            .await
            .context("failed to query aggregator shadow metrics")?;
        self.decode_response(response, "query aggregator shadow metrics")
            .await
    }

    pub async fn submit_trade(&self, request: &TradeRequest) -> Result<TradeResponse> {
        self.ensure_submission_enabled()?;
        validate_pool_quote(&request.pool, request.quote_mint.as_deref())?;
        anyhow::ensure!(
            !(request.pool == "raydium-cpmm" && request.quote_mint.as_deref() == Some(USDT_MINT)),
            "CPMM/USDT is only available through the isolated SDK simulation"
        );
        if request.pool.eq_ignore_ascii_case("meteora-damm-v2")
            && request.quote_mint.as_deref() == Some(USDC_MINT)
            && request.amount_sol.is_some()
        {
            anyhow::bail!("该 Meteora 池使用 USDC 报价，不能直接提交 SOL 买入金额；请选择 WSOL 池或使用明确的 USDC 金额");
        }
        let response = self
            .authorized(self.client.post(format!("{}/v1/trades", self.base_url)))
            .json(request)
            .send()
            .await
            .context("failed to submit trade")?;
        self.decode_response(response, "submit trade").await
    }

    /// Manually simulate an SDK trade on the isolated decision service. This
    /// path never uses the normal submission origin or enables live trading.
    pub async fn submit_isolated_sdk_simulation(
        &self,
        request: &TradeRequest,
        expected_wallet_pubkey: &str,
    ) -> Result<(TradeResponse, TradeRequest)> {
        let base_url = self
            .route_decision_base_url
            .as_deref()
            .context("isolated SDK simulation endpoint is not configured")?;
        anyhow::ensure!(
            !self.submission_enabled,
            "live submission must remain disabled"
        );
        anyhow::ensure!(request.simulate, "isolated SDK trade must be simulated");
        anyhow::ensure!(
            request.execution_method == "sdk",
            "isolated trade must use SDK"
        );
        anyhow::ensure!(
            request.sdk_decision_id.is_none() && request.sdk_preview_id.is_none(),
            "SDK credentials must be issued by the isolated service"
        );
        anyhow::ensure!(
            request
                .wallet_id
                .as_deref()
                .is_some_and(|id| !id.is_empty()),
            "isolated trade requires a wallet ID"
        );
        anyhow::ensure!(
            !expected_wallet_pubkey.is_empty(),
            "isolated trade requires a local wallet public key"
        );
        validate_pool_quote(&request.pool, request.quote_mint.as_deref())?;

        let token = self
            .route_decision_token
            .as_ref()
            .context("isolated SDK token is missing")?;
        let ready_response = self
            .client
            .get(format!("{base_url}/ready"))
            .bearer_auth(token)
            .send()
            .await
            .context("failed to query isolated SDK readiness")?;
        let ready: ReadyResponse = self
            .decode_response(ready_response, "query isolated SDK readiness")
            .await?;
        anyhow::ensure!(ready.ready, "isolated SDK service is not ready");
        anyhow::ensure!(
            ready.wallets.iter().any(|wallet| wallet.wallet_id
                == request.wallet_id.as_deref().unwrap_or_default()
                && wallet.pubkey == expected_wallet_pubkey),
            "isolated SDK wallet public key does not match"
        );

        let mut decision = self
            .route_decision(request, Some(expected_wallet_pubkey))
            .await?;
        if decision.status == "unresolved_retryable"
            && decision.reason.as_deref() == Some("bridge_verified_sdk_capability_unresolved")
        {
            anyhow::ensure!(
                request.side == "buy"
                    && request
                        .amount_sol
                        .is_some_and(|value| value.is_finite() && value > 0.0)
                    && request.amount_quote_units.is_none(),
                "isolated SDK bridge preview requires a SOL buy"
            );
            let observation = decision
                .observed_bridge
                .as_ref()
                .context("SDK bridge observation is missing")?;
            anyhow::ensure!(
                observation
                    .observation_id
                    .starts_with("observe:sol-bridge:")
                    && observation.wallet_id == request.wallet_id.as_deref().unwrap_or_default()
                    && observation.wallet_pubkey == expected_wallet_pubkey,
                "SDK bridge observation wallet does not match"
            );
            let response = self
                .client
                .post(format!("{base_url}/v1/routes/observations/sdk-preview"))
                .timeout(Duration::from_secs(30))
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "observation_id": observation.observation_id,
                    "trade": request,
                    "simulate_transaction": true,
                }))
                .send()
                .await
                .context("failed to request isolated SDK bridge preview")?;
            let preview: SdkBridgePreviewResponse = self
                .decode_response(response, "request isolated SDK bridge preview")
                .await?;
            anyhow::ensure!(
                preview.observation_id.starts_with("observe:sol-bridge:")
                    && preview.observation_id.len() > "observe:sol-bridge:".len()
                    && preview.observation_id.len() <= 128,
                "SDK bridge preview observation ID is invalid"
            );
            anyhow::ensure!(
                preview.simulate_only
                    && !preview.executable
                    && preview
                        .preview
                        .simulation
                        .as_ref()
                        .is_some_and(|simulation| {
                            simulation.ok && simulation.minimum_output_verified
                        }),
                "SDK bridge unsigned simulation is not verified"
            );
            let preview_id = preview
                .preview_id
                .context("SDK bridge preview ID is missing")?;
            anyhow::ensure!(
                preview_id.starts_with("observe:sdk-preview:"),
                "SDK bridge preview ID is invalid"
            );
            let mut preview_request = request.clone();
            preview_request.sdk_preview_id = Some(preview_id);
            decision = self
                .route_decision(&preview_request, Some(expected_wallet_pubkey))
                .await?;
            anyhow::ensure!(
                decision
                    .decision_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("sdk-decision:bridge:"))
                    && decision.route_id.as_deref() == Some(preview.observation_id.as_str())
                    && decision.sdk_capability.as_ref().is_some_and(|capability| {
                        capability.route_evidence == "unsigned_simulation_verified"
                            && matches!(
                                (request.pool.as_str(), capability.service_adapter.as_str()),
                                ("pump-amm", "simulate_only_bridge_decision")
                                    | ("raydium-cpmm", "cpmm_cpmm_simulate_only")
                                    | ("raydium-amm-v4", "cpmm_amm_v4_simulate_only")
                            )
                            && !capability.sdk_unsupported_proven
                            && !capability.jupiter_candidate_allowed
                    }),
                "SDK bridge decision lacks unsigned simulation evidence"
            );
        }
        anyhow::ensure!(
            decision.status == "sdk_verified",
            "SDK route is not verified for this trade: {}",
            decision.reason.as_deref().unwrap_or("unspecified")
        );
        if request.side == "buy"
            && request.amount_sol.is_some()
            && request
                .quote_mint
                .as_deref()
                .is_some_and(|quote| quote != WSOL_MINT)
            && request.sol_hop_pool_id.is_none()
        {
            anyhow::ensure!(
                decision
                    .sdk_capability
                    .as_ref()
                    .is_some_and(
                        |capability| capability.route_evidence == "unsigned_simulation_verified"
                    ),
                "SDK bridge buy has no unsigned simulation evidence"
            );
        }
        if request.side == "sell" && request.sol_hop_pool_id.is_some() {
            anyhow::ensure!(
                request.pool == "raydium-cpmm"
                    && request.sol_hop_pool.as_deref() == Some("raydium-cpmm")
                    && request.quote_mint.as_deref() == Some(USDT_MINT)
                    && decision.sdk_capability.as_ref().is_some_and(|capability| {
                        capability.service_adapter == "cpmm_two_hop_sell_simulate_only"
                            && capability.route_evidence == "unsigned_simulation_verified"
                            && !capability.sdk_unsupported_proven
                            && !capability.jupiter_candidate_allowed
                    }),
                "SDK two-hop sell has no verified unsigned simulation evidence"
            );
        }
        let decision_id = decision.decision_id.context("SDK decision ID is missing")?;
        let mut bound_request = request.clone();
        bound_request.sdk_decision_id = Some(decision_id);
        let response = self
            .client
            .post(format!("{base_url}/v1/trades"))
            .bearer_auth(token)
            .json(&bound_request)
            .send()
            .await
            .context("failed to submit isolated SDK simulation")?;
        let trade: TradeResponse = self
            .decode_response(response, "submit isolated SDK simulation")
            .await?;
        anyhow::ensure!(
            trade.wallet_id.as_deref() == request.wallet_id.as_deref()
                && trade.wallet_pubkey.as_deref() == Some(expected_wallet_pubkey),
            "isolated SDK response wallet does not match"
        );
        anyhow::ensure!(
            trade.signature.is_none(),
            "isolated SDK simulation returned a submitted signature"
        );
        anyhow::ensure!(
            trade.request_id.as_deref().is_some_and(|id| !id.is_empty()),
            "isolated SDK response has no request ID"
        );
        anyhow::ensure!(
            !matches!(trade.status.as_deref(), Some("submitted" | "confirmed")),
            "isolated SDK service returned a live trade status"
        );
        Ok((trade, bound_request))
    }

    pub async fn isolated_sdk_ready(&self) -> Result<ReadyResponse> {
        anyhow::ensure!(
            self.isolated_sdk_simulation_available(),
            "isolated SDK simulation is unavailable"
        );
        let base_url = self
            .route_decision_base_url
            .as_deref()
            .context("isolated SDK URL is missing")?;
        let token = self
            .route_decision_token
            .as_ref()
            .context("isolated SDK token is missing")?;
        let response = self
            .client
            .get(format!("{base_url}/ready"))
            .bearer_auth(token)
            .send()
            .await
            .context("failed to query isolated SDK readiness")?;
        self.decode_response(response, "query isolated SDK readiness")
            .await
    }

    /// Resolve the candidate's verified WSOL/USDC CPMM pool for a manual
    /// simulate-only sell. The returned route is still checked by the SDK
    /// decision endpoint before any simulation request is accepted.
    pub async fn isolated_wsol_usdc_sell_pool(&self) -> Result<ResolvedPoolRoute> {
        anyhow::ensure!(
            self.isolated_sdk_simulation_available(),
            "isolated SDK simulation is unavailable"
        );
        let base_url = self
            .route_decision_base_url
            .as_deref()
            .context("isolated SDK URL is missing")?;
        let token = self
            .route_decision_token
            .as_ref()
            .context("isolated SDK token is missing")?;
        let response = self
            .client
            .post(format!("{base_url}/v1/pools/sol-hop"))
            .bearer_auth(token)
            .json(&serde_json::json!({"quote_mint": USDC_MINT}))
            .send()
            .await
            .context("failed to resolve isolated WSOL/USDC pool")?;
        let hop: SolHopResponse = self
            .decode_response(response, "resolve isolated WSOL/USDC pool")
            .await?;
        let pool_id = hop.verified_pool_id()?.to_string();
        Ok(ResolvedPoolRoute {
            mint: WSOL_MINT.to_string(),
            pool: "raydium-cpmm".to_string(),
            pool_id: Some(pool_id),
            quote_mint: USDC_MINT.to_string(),
        })
    }

    pub async fn get_isolated_sdk_simulation_trade(
        &self,
        request_id: &str,
    ) -> Result<TradeResponse> {
        anyhow::ensure!(
            self.isolated_sdk_simulation_available(),
            "isolated SDK simulation is unavailable"
        );
        anyhow::ensure!(
            !request_id.is_empty()
                && request_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
            "invalid isolated SDK request ID"
        );
        let base_url = self
            .route_decision_base_url
            .as_deref()
            .context("isolated SDK URL is missing")?;
        let token = self
            .route_decision_token
            .as_ref()
            .context("isolated SDK token is missing")?;
        let response = self
            .client
            .get(format!("{base_url}/v1/trades/{request_id}"))
            .bearer_auth(token)
            .send()
            .await
            .context("failed to query isolated SDK simulation")?;
        let trade: TradeResponse = self
            .decode_response(response, "query isolated SDK simulation")
            .await?;
        anyhow::ensure!(
            trade.request_id.as_deref() == Some(request_id),
            "isolated SDK request ID does not match"
        );
        anyhow::ensure!(
            trade.signature.is_none(),
            "isolated SDK simulation returned a submitted signature"
        );
        anyhow::ensure!(
            !matches!(trade.status.as_deref(), Some("submitted" | "confirmed")),
            "isolated SDK service returned a live trade status"
        );
        Ok(trade)
    }

    /// Server-side SDK route decision. This method does not depend on the
    /// submission switch and never calls /v1/trades.
    pub async fn route_decision(
        &self,
        request: &TradeRequest,
        expected_wallet_pubkey: Option<&str>,
    ) -> Result<RouteDecisionResponse> {
        if self.route_decision_base_url.is_some() {
            anyhow::ensure!(
                expected_wallet_pubkey.is_some_and(|value| !value.trim().is_empty()),
                "isolated route decision requires the local wallet public key"
            );
        }
        let base_url = self
            .route_decision_base_url
            .as_deref()
            .unwrap_or(&self.base_url);
        let token = self.route_decision_token.as_ref().or(self.token.as_ref());
        let builder = self.client.post(format!("{base_url}/v1/routes/decide"));
        let builder = if self.route_decision_base_url.is_some()
            && request.side == "buy"
            && request.amount_sol.is_some()
            && request
                .quote_mint
                .as_deref()
                .is_some_and(|quote| quote != WSOL_MINT)
        {
            builder.timeout(Duration::from_secs(30))
        } else {
            builder
        };
        let builder = match token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        };
        let response = builder
            .json(request)
            .send()
            .await
            .context("failed to query SDK route decision")?;
        let decision: RouteDecisionResponse = self
            .decode_response(response, "query SDK route decision")
            .await?;
        anyhow::ensure!(
            self.route_decision_base_url.is_none() || decision.simulate_only,
            "isolated route decision service is not simulate-only"
        );
        anyhow::ensure!(
            self.route_decision_base_url.is_none()
                || decision.wallet_pubkey.as_deref() == expected_wallet_pubkey,
            "isolated route decision wallet public key does not match"
        );
        anyhow::ensure!(
            decision.schema_version == 1,
            "unsupported SDK route decision schema"
        );
        anyhow::ensure!(
            matches!(
                decision.status.as_str(),
                "sdk_verified" | "sdk_unsupported" | "unresolved_retryable" | "policy_blocked"
            ),
            "unknown SDK route decision status"
        );
        anyhow::ensure!(
            decision.wallet_id == request.wallet_id.as_deref().unwrap_or_default(),
            "SDK route decision wallet does not match request"
        );
        anyhow::ensure!(
            decision.status != "sdk_verified"
                || (decision.decision_id.is_some()
                    && decision.route_id.is_some()
                    && decision.expires_at_ms.is_some()),
            "verified SDK route decision has no server binding"
        );
        anyhow::ensure!(
            !decision.sdk_capability.as_ref().is_some_and(
                |value| value.jupiter_candidate_allowed && !value.sdk_unsupported_proven
            ),
            "Jupiter candidate has no SDK unsupported proof"
        );
        anyhow::ensure!(
            decision.status != "sdk_unsupported"
                || (decision.decision_id.is_some()
                    && decision.route_id.is_some()
                    && decision.expires_at_ms.is_some()
                    && decision.sdk_capability.as_ref().is_some_and(|value| {
                        value.sdk_unsupported_proven && value.jupiter_candidate_allowed
                    })),
            "SDK unsupported candidate has no server binding or capability proof"
        );
        anyhow::ensure!(
            decision.status == "sdk_unsupported"
                || !decision.sdk_capability.as_ref().is_some_and(|value| {
                    value.sdk_unsupported_proven || value.jupiter_candidate_allowed
                }),
            "only an SDK unsupported decision can authorize Jupiter candidate"
        );
        Ok(decision)
    }

    pub async fn resolve_pool(&self, mint: &str) -> Result<ResolvedPoolRoute> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/pools/resolve", self.base_url)),
            )
            .json(&ResolvePoolRequest { mint })
            .send()
            .await
            .context("failed to resolve mint pool")?;
        self.decode_response(response, "resolve mint pool").await
    }

    pub async fn isolated_sdk_resolve_pool(&self, mint: &str) -> Result<ResolvedPoolRoute> {
        anyhow::ensure!(
            self.isolated_sdk_simulation_available(),
            "isolated SDK pool resolution is unavailable"
        );
        let base_url = self.route_decision_base_url.as_deref().unwrap();
        let token = self.route_decision_token.as_ref().unwrap();
        let response = self
            .client
            .post(format!("{base_url}/v1/pools/resolve"))
            .bearer_auth(token)
            .json(&ResolvePoolRequest { mint })
            .send()
            .await
            .context("failed to resolve isolated SDK pool")?;
        let route: ResolvedPoolRoute = self
            .decode_response(response, "resolve isolated SDK pool")
            .await?;
        anyhow::ensure!(route.mint == mint, "isolated SDK pool mint does not match");
        Ok(route)
    }

    pub async fn isolated_sdk_verify_pool_hint(
        &self,
        hint: &ResolvedPoolRoute,
    ) -> Result<ResolvedPoolRoute> {
        anyhow::ensure!(
            self.isolated_sdk_simulation_available(),
            "isolated SDK pool verification is unavailable"
        );
        let base_url = self.route_decision_base_url.as_deref().unwrap();
        let token = self.route_decision_token.as_ref().unwrap();
        let response = self
            .client
            .post(format!("{base_url}/v1/pools/resolve-candidates"))
            .bearer_auth(token)
            .json(&serde_json::json!({
                "protocol_id": hint.pool,
                "program_id": null,
                "pool_id": hint.pool_id,
                "mint": hint.mint,
                "quote_mint": hint.quote_mint,
            }))
            .send()
            .await
            .context("failed to verify isolated SDK pool hint")?;
        let verified: serde_json::Value = self
            .decode_response(response, "verify isolated SDK pool hint")
            .await?;
        anyhow::ensure!(
            verified["chain_verified"] == true
                && verified["protocol_id"].as_str() == Some(hint.pool.as_str())
                && verified["pool_id"].as_str() == hint.pool_id.as_deref()
                && verified["mint"].as_str() == Some(hint.mint.as_str())
                && verified["quote_mint"].as_str() == Some(hint.quote_mint.as_str()),
            "isolated SDK pool hint did not match chain-verified route"
        );
        Ok(hint.clone())
    }

    pub async fn isolated_sdk_resolve_explicit_pool(
        &self,
        mint: &str,
        pool: &str,
        pool_id: &str,
    ) -> Result<ResolvedPoolRoute> {
        anyhow::ensure!(
            self.isolated_sdk_simulation_available(),
            "isolated SDK pool resolution is unavailable"
        );
        anyhow::ensure!(
            matches!(pool, "pump-amm" | "raydium-cpmm" | "raydium-amm-v4"),
            "unsupported explicit pool protocol"
        );
        for address in [mint, pool_id] {
            anyhow::ensure!(
                bs58::decode(address)
                    .into_vec()
                    .is_ok_and(|bytes| bytes.len() == 32),
                "explicit pool Mint or pool ID is invalid"
            );
        }
        let base_url = self.route_decision_base_url.as_deref().unwrap();
        let token = self.route_decision_token.as_ref().unwrap();
        let response = self
            .client
            .post(format!("{base_url}/v1/pools/resolve-candidates"))
            .bearer_auth(token)
            .json(&serde_json::json!({
                "protocol_id": pool,
                "program_id": null,
                "pool_id": pool_id,
                "mint": mint,
                "quote_mint": null,
            }))
            .send()
            .await
            .context("failed to resolve explicit SDK pool")?;
        let verified: serde_json::Value = self
            .decode_response(response, "resolve explicit SDK pool")
            .await?;
        let quote_mint = verified["quote_mint"]
            .as_str()
            .context("chain-verified SDK pool has no Quote Mint")?;
        anyhow::ensure!(
            verified["chain_verified"] == true
                && verified["protocol_id"].as_str() == Some(pool)
                && verified["pool_id"].as_str() == Some(pool_id)
                && verified["mint"].as_str() == Some(mint)
                && bs58::decode(quote_mint)
                    .into_vec()
                    .is_ok_and(|bytes| bytes.len() == 32),
            "explicit SDK pool did not match chain-verified route"
        );
        validate_pool_quote(pool, Some(quote_mint))?;
        Ok(ResolvedPoolRoute {
            mint: mint.to_string(),
            pool: pool.to_string(),
            pool_id: Some(pool_id.to_string()),
            quote_mint: quote_mint.to_string(),
        })
    }

    pub async fn resolve_sol_hop(&self, quote_mint: &str) -> Result<SolHopResponse> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/pools/sol-hop", self.base_url)),
            )
            .json(&serde_json::json!({"quote_mint": quote_mint}))
            .send()
            .await
            .context("failed to resolve SOL hop")?;
        let hop: SolHopResponse = self.decode_response(response, "resolve SOL hop").await?;
        if hop.available {
            hop.verified_pool_id()?;
        }
        Ok(hop)
    }

    pub async fn isolated_sdk_resolve_sol_hop(&self, quote_mint: &str) -> Result<SolHopResponse> {
        let base_url = self
            .route_decision_base_url
            .as_deref()
            .context("isolated SDK endpoint is not configured")?;
        let token = self
            .route_decision_token
            .as_deref()
            .context("isolated SDK token is missing")?;
        let response = self
            .client
            .post(format!("{base_url}/v1/pools/sol-hop"))
            .bearer_auth(token)
            .json(&serde_json::json!({"quote_mint": quote_mint}))
            .send()
            .await
            .context("failed to resolve isolated SDK SOL hop")?;
        let hop: SolHopResponse = self
            .decode_response(response, "resolve isolated SDK SOL hop")
            .await?;
        if hop.available {
            hop.verified_pool_id()?;
        }
        Ok(hop)
    }

    pub async fn get_trade(&self, request_id: &str) -> Result<TradeResponse> {
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/v1/trades/{request_id}", self.base_url)),
            )
            .send()
            .await
            .context("failed to query trade")?;
        self.decode_response(response, "query trade").await
    }

    pub async fn wallets(&self) -> Result<WalletsResponse> {
        let response = self
            .authorized(self.client.get(format!("{}/v1/wallets", self.base_url)))
            .send()
            .await
            .context("failed to query trade_service wallets")?;
        self.decode_response(response, "query wallets").await
    }

    pub async fn ready(&self) -> Result<ReadyResponse> {
        let response = self
            .authorized(self.client.get(format!("{}/ready", self.base_url)))
            .send()
            .await
            .context("failed to query trade_service readiness")?;
        self.decode_response(response, "query readiness").await
    }

    pub async fn wallet_id_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self
            .wallets()
            .await?
            .wallets
            .into_iter()
            .find(|wallet| wallet.pubkey == pubkey)
            .map(|wallet| wallet.wallet_id))
    }

    pub async fn create_wallet(&self, user_id: i64, label: &str) -> Result<ManagedWalletResponse> {
        self.create_or_import_wallet(user_id, label, None).await
    }

    pub async fn managed_wallets(&self) -> Result<Vec<ManagedWalletResponse>> {
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/v1/wallets/manage", self.base_url)),
            )
            .send()
            .await
            .context("failed to query managed trade_service wallets")?;
        let response: ManagedWalletsResponse = self
            .decode_response(response, "query managed wallets")
            .await?;
        Ok(response.wallets)
    }

    pub async fn wallet_balances(&self, user_id: i64) -> Result<WalletBalancesResponse> {
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/internal/wallet-balances", self.base_url))
                    .query(&[
                        ("owner_type", "copy_trading"),
                        ("owner_id", &user_id.to_string()),
                    ]),
            )
            .send()
            .await
            .context("failed to query wallet balances")?;
        self.decode_response(response, "query wallet balances")
            .await
    }

    pub async fn quote_balance(
        &self,
        user_id: i64,
        wallet_id: &str,
        mint: &str,
    ) -> Result<QuoteBalanceResponse> {
        let response = self
            .authorized(self.client.get(format!(
                "{}/internal/wallets/{}/quote-balance/{}",
                self.base_url, wallet_id, mint
            )))
            .query(&[
                ("owner_type", "copy_trading"),
                ("owner_id", &user_id.to_string()),
            ])
            .send()
            .await
            .context("failed to query quote balance")?;
        self.decode_response(response, "query quote balance").await
    }

    pub async fn create_vanity_job(
        &self,
        user_id: i64,
        side: &str,
        pattern: &str,
    ) -> Result<VanityJobResponse> {
        let request = CreateVanityJobRequest {
            owner_type: "copy_trading",
            owner_id: user_id.to_string(),
            label: "Vanity",
            match_side: side,
            pattern,
        };
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/internal/wallets/vanity-jobs", self.base_url)),
            )
            .json(&request)
            .send()
            .await
            .context("failed to create vanity wallet job")?;
        self.decode_response(response, "create vanity wallet job")
            .await
    }

    pub async fn vanity_job(&self, user_id: i64, job_id: &str) -> Result<VanityJobResponse> {
        let owner_id = user_id.to_string();
        let response = self
            .authorized(
                self.client
                    .get(format!(
                        "{}/internal/wallets/vanity-jobs/{job_id}",
                        self.base_url
                    ))
                    .query(&[
                        ("owner_type", "copy_trading"),
                        ("owner_id", owner_id.as_str()),
                    ]),
            )
            .send()
            .await
            .context("failed to query vanity wallet job")?;
        self.decode_response(response, "query vanity wallet job")
            .await
    }

    pub async fn cancel_vanity_job(&self, user_id: i64, job_id: &str) -> Result<VanityJobResponse> {
        let owner_id = user_id.to_string();
        let response = self
            .authorized(
                self.client
                    .delete(format!(
                        "{}/internal/wallets/vanity-jobs/{job_id}",
                        self.base_url
                    ))
                    .query(&[
                        ("owner_type", "copy_trading"),
                        ("owner_id", owner_id.as_str()),
                    ]),
            )
            .send()
            .await
            .context("failed to cancel vanity wallet job")?;
        self.decode_response(response, "cancel vanity wallet job")
            .await
    }

    pub async fn transfer_sol(
        &self,
        user_id: i64,
        wallet_id: &str,
        to_wallet_id: Option<&str>,
        to_address: Option<&str>,
        amount_lamports: Option<u64>,
        percent: Option<u64>,
        idempotency_key: String,
    ) -> Result<SolTransferResponse> {
        let request = SolTransferRequest {
            owner_type: "copy_trading",
            owner_id: user_id.to_string(),
            to_wallet_id,
            to_address,
            amount_mode: if amount_lamports.is_some() {
                "fixed"
            } else {
                "percent"
            },
            amount_lamports,
            percent,
            idempotency_key,
        };
        let response = self
            .authorized(self.client.post(format!(
                "{}/internal/wallets/{wallet_id}/transfer-sol",
                self.base_url
            )))
            .json(&request)
            .send()
            .await
            .context("failed to transfer SOL")?;
        self.decode_response(response, "transfer SOL").await
    }

    pub async fn import_wallet(
        &self,
        user_id: i64,
        label: &str,
        private_key_base58: &str,
    ) -> Result<ManagedWalletResponse> {
        self.create_or_import_wallet(user_id, label, Some(private_key_base58))
            .await
    }

    pub async fn rename_wallet(&self, wallet_id: &str, label: &str) -> Result<()> {
        let response = self
            .authorized(
                self.client
                    .patch(format!("{}/v1/wallets/{wallet_id}", self.base_url)),
            )
            .json(&UpdateWalletRequest { label })
            .send()
            .await
            .context("failed to rename trade_service wallet")?;
        let _: ManagedWalletResponse = self.decode_response(response, "rename wallet").await?;
        Ok(())
    }

    pub async fn delete_wallet(&self, wallet_id: &str) -> Result<()> {
        let response = self
            .authorized(
                self.client
                    .delete(format!("{}/v1/wallets/{wallet_id}", self.base_url)),
            )
            .send()
            .await
            .context("failed to delete trade_service wallet")?;
        let _: ManagedWalletResponse = self.decode_response(response, "delete wallet").await?;
        Ok(())
    }

    pub async fn export_wallet(&self, wallet_id: &str) -> Result<String> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/wallets/{wallet_id}/export", self.base_url)),
            )
            .send()
            .await
            .context("failed to export trade_service wallet")?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("trade_service export wallet returned {status}");
        }
        let response: ExportWalletResponse =
            serde_json::from_str(&text).context("invalid trade_service export wallet response")?;
        Ok(response.private_key_base58)
    }

    async fn create_or_import_wallet(
        &self,
        user_id: i64,
        label: &str,
        private_key_base58: Option<&str>,
    ) -> Result<ManagedWalletResponse> {
        let request = CreateWalletRequest {
            owner_type: "copy_trading",
            owner_id: user_id.to_string(),
            label,
            private_key_base58,
        };
        let response = self
            .authorized(self.client.post(format!("{}/v1/wallets", self.base_url)))
            .json(&request)
            .send()
            .await
            .context("failed to create trade_service wallet")?;
        self.decode_response(response, "create wallet").await
    }

    fn authorized(&self, builder: RequestBuilder) -> RequestBuilder {
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    async fn decode_response<T: for<'de> Deserialize<'de>>(
        &self,
        response: reqwest::Response,
        action: &str,
    ) -> Result<T> {
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("trade_service {action} returned {status}");
        }
        serde_json::from_str(&text)
            .with_context(|| format!("invalid trade_service {action} response"))
    }
}

#[cfg(test)]
mod aggregator_shadow_contract_tests {
    use super::*;

    #[test]
    fn protocol_capabilities_parse_simulate_only_snapshot() {
        let response: ProtocolCapabilitiesResponse = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "generated_at_ms": 123,
            "protocols": [{
                "protocol_id": "pump",
                "status": "simulate",
                "supports_buy": true,
                "supports_sell": true,
                "supports_direct_quote": true,
                "supports_sol_route": true
            }]
        }))
        .unwrap();
        assert_eq!(response.protocols.len(), 1);
        assert_eq!(response.protocols[0].status, "simulate");
    }

    #[test]
    fn shadow_summary_is_additive_and_optional_for_old_services() {
        let old: AggregatorShadowMetricsResponse = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "read_only": true,
            "metrics": []
        }))
        .unwrap();
        assert!(old.summary.is_none());

        let legacy_summary: AggregatorShadowMetricsResponse =
            serde_json::from_value(serde_json::json!({
                "schema_version": 1,
                "read_only": true,
                "metrics": [],
                "summary": {
                    "sample_count": 1,
                    "quoted_count": 1,
                    "failed_count": 0,
                    "unavailable_count": 0,
                    "p50_latency_ms": 12,
                    "p95_latency_ms": 12,
                    "p99_latency_ms": 12,
                    "success_rate": 1.0,
                    "expiry_rate": 0.0
                }
            }))
            .unwrap();
        assert_eq!(
            legacy_summary.summary.unwrap().min_output_checked_count,
            None
        );

        let current: AggregatorShadowMetricsResponse = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "read_only": true,
            "metrics": [],
            "summary": {
                "sample_count": 1,
                "quoted_count": 1,
                "failed_count": 0,
                "unavailable_count": 0,
                "p50_latency_ms": 12,
                "p95_latency_ms": 12,
                "p99_latency_ms": 12,
                "success_rate": 1.0,
                "expiry_rate": 0.0,
                "min_output_checked_count": 1,
                "min_output_missing_count": 0,
                "min_output_pass_count": 1,
                "min_output_coverage_rate": 1.0,
                "min_output_pass_rate": 1.0
            }
        }))
        .unwrap();
        let summary = current.summary.unwrap();
        assert_eq!(summary.p95_latency_ms, Some(12));
        assert_eq!(summary.min_output_checked_count, Some(1));
        assert_eq!(summary.min_output_pass_count, Some(1));
    }

    #[test]
    fn trade_service_url_rejects_embedded_credentials_and_query() {
        assert!(
            TradeServiceClient::new("http://user:secret@example.test".into(), None, 1,).is_err()
        );
        assert!(
            TradeServiceClient::new("http://example.test/?token=secret".into(), None, 1,).is_err()
        );
        assert!(TradeServiceClient::new("http://127.0.0.1:18788".into(), None, 1).is_ok());
    }
}
