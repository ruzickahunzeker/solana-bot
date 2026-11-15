use super::*;
use crate::{
    adapters::{
        persistence::wallet_metadata::RepositoryWalletMetadata,
        sdk::address_parser::SdkAddressParser,
    },
    application::wallet_balances::{test_support::Runtime, WalletBalanceService},
    domain::trade::Address,
};
use sqlx::SqlitePool;

const OWNER: &str = "owner_type=telegram&owner_id=42";
const MINT: &str = "11111111111111111111111111111111";

async fn setup(runtime: Runtime) -> (AppState, SqlitePool) {
    let (mut state, pool) = super::wallet_boundary_tests::metadata_state().await;
    let trace = runtime.trace.clone();
    state.wallet_balance_queries = Arc::new(WalletBalanceService::new(
        RepositoryWalletMetadata::new(state.wallet_repository.clone()),
        runtime,
        SdkAddressParser,
        move || {
            trace.lock().unwrap().push("clock".into());
            123
        },
    ));
    (state, pool)
}

#[tokio::test]
async fn balance_query_http_uses_real_metadata_and_fake_read_only_runtime_for_exact_v1_success() {
    let runtime = Runtime::new(Arc::default());
    let (state, pool) = setup(runtime.clone()).await;
    let app = http::router(state.clone());
    let quote = format!("/internal/wallets/owned/quote-balance/{MINT}?{OWNER}");
    assert_eq!(
        json_body(
            send(&app, "GET", &quote, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK
        )
        .await,
        json!({"wallet_id":"owned","quote_mint":MINT,"decimals":6,"balance_units":u64::MAX,"balance_lamports":42})
    );
    assert_eq!(
        json_body(
            send(
                &app,
                "GET",
                &format!("/internal/wallet-balances?{OWNER}"),
                LOCAL,
                Some(TOKEN),
                None
            )
            .await,
            StatusCode::OK
        )
        .await,
        json!({"fetched_at_ms":123,"wallets":[
            {"wallet_id":"fixture-wallet","pubkey":Address::from_bytes([1;32]).to_string(),"balance_lamports":11,"error":null},
            {"wallet_id":"owned","pubkey":Address::from_bytes([2;32]).to_string(),"balance_lamports":22,"error":null},
            {"wallet_id":"global","pubkey":Address::from_bytes([4;32]).to_string(),"balance_lamports":33,"error":null}
        ]})
    );
    assert_eq!(
        *runtime.inputs.lock().unwrap(),
        [vec![
            Address::from_bytes([1; 32]),
            Address::from_bytes([2; 32]),
            Address::from_bytes([4; 32])
        ]]
    );
    assert_eq!(
        *runtime.trace.lock().unwrap(),
        [
            "reader:owned".to_string(),
            format!("quote:{MINT}"),
            "default".to_string(),
            "batch:3".to_string(),
            "clock".to_string()
        ]
    );
    assert_eq!(state.wallets.wallet_count(), 0);
    assert!(state.trades.recent(10).is_empty());
    let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallet_audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audit, 0);
    pool.close().await;
}

#[tokio::test]
async fn native_balance_cardinality_http_preserves_every_visible_wallet_and_per_row_200() {
    for balances in [vec![], vec![7], vec![7, 8], vec![7, 8, 9, 10]] {
        let mut runtime = Runtime::new(Arc::default());
        runtime.balances = balances;
        let (state, pool) = setup(runtime).await;
        let body = json_body(
            send(
                &http::router(state.clone()),
                "GET",
                &format!("/internal/wallet-balances?{OWNER}"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(body["fetched_at_ms"], 123);
        let rows = body["wallets"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .map(|row| row["wallet_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["fixture-wallet", "owned", "global"]
        );
        assert!(rows.iter().all(|row| row["balance_lamports"].is_null()
            && row["error"] == "wallet balances unavailable: RPC account count mismatch"));
        assert!(state.trades.recent(10).is_empty());
        let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallet_audit_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(audit, 0);
        pool.close().await;
    }
}

#[tokio::test]
async fn balance_query_http_quote_error_is_502_redacted_while_native_error_remains_per_row_200() {
    let mut runtime = Runtime::new(Arc::default());
    let message = "synthetic RPC failure https://rpc.example.invalid/non-secret-marker";
    runtime.quote_error = Some(message);
    runtime.native_error = Some(message);
    let (state, pool) = setup(runtime.clone()).await;
    let app = http::router(state);
    let quote = json_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallets/owned/quote-balance/{MINT}?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::BAD_GATEWAY,
    )
    .await;
    assert!(!quote.to_string().contains("non-secret-marker"));
    let native = json_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallet-balances?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(native["fetched_at_ms"], 123);
    let rows = native["wallets"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows
        .iter()
        .all(|row| row["balance_lamports"].is_null() && row["error"] == message));
    assert_eq!(
        runtime
            .trace
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "clock")
            .count(),
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn balance_query_http_owner_parse_and_storage_failures_do_not_obtain_a_runtime_reader() {
    let runtime = Runtime::new(Arc::default());
    let (state, pool) = setup(runtime.clone()).await;
    let app = http::router(state.clone());
    for id in ["disabled", "other"] {
        error_body(
            send(
                &app,
                "GET",
                &format!("/internal/wallets/{id}/quote-balance/invalid?{OWNER}"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::FORBIDDEN,
            "wallet owner mismatch",
        )
        .await;
    }
    error_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallets/owned/quote-balance/invalid?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "invalid quote mint",
    )
    .await;
    error_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallets/missing/quote-balance/{MINT}?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "wallet not found",
    )
    .await;
    sqlx::query("UPDATE wallets SET pubkey='invalid' WHERE wallet_id='owned'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/internal/wallet-balances?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None
        )
        .await
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(runtime.trace.lock().unwrap().is_empty());
    pool.close().await;
    let message = state
        .wallet_repository
        .get("owned")
        .await
        .unwrap_err()
        .to_string();
    let list_message = state
        .wallet_repository
        .list()
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(message, "wallet not found");
    assert_ne!(list_message, message);
    error_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallets/owned/quote-balance/{MINT}?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        &message,
    )
    .await;
    error_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallet-balances?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE,
        &list_message,
    )
    .await;
    assert!(runtime.trace.lock().unwrap().is_empty());
}

#[tokio::test]
async fn balance_query_http_auth_extractors_missing_runtime_and_head_keep_existing_contracts() {
    let mut runtime = Runtime::new(Arc::default());
    runtime.loaded = false;
    let (state, pool) = setup(runtime.clone()).await;
    let app = http::router(state);
    let quote = format!("/internal/wallets/owned/quote-balance/{MINT}?{OWNER}");
    for path in [&quote, &format!("/internal/wallet-balances?{OWNER}")] {
        error_body(
            send(&app, "GET", path, ALLOWED, Some(TOKEN), None).await,
            StatusCode::FORBIDDEN,
            "internal wallet API is loopback-only",
        )
        .await;
        error_body(
            send(&app, "GET", path, LOCAL, None, None).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    assert_eq!(
        send(&app, "GET", "/internal/wallet-balances", DENIED, None, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(runtime.trace.lock().unwrap().is_empty());
    error_body(
        send(&app, "GET", &quote, LOCAL, Some(TOKEN), None).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "wallet is not loaded",
    )
    .await;
    assert_eq!(*runtime.trace.lock().unwrap(), ["reader:owned"]);
    let head = send(
        &app,
        "HEAD",
        &format!("/internal/wallet-balances?{OWNER}"),
        LOCAL,
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()["content-type"], "application/json");
    assert!(to_bytes(head.into_body(), 1024).await.unwrap().is_empty());
    assert_eq!(runtime.inputs.lock().unwrap().len(), 1);
    pool.close().await;
}
