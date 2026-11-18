use super::*;
use crate::domain::trade::Address;
use sqlx::SqlitePool;

const OWNER: &str = "owner_type=telegram&owner_id=42";
const PUBLIC_MINT: &str = "11111111111111111111111111111111";
const NON_SECRET_MARKER: &str = "not-an-encrypted-key-test-only";

pub(super) async fn metadata_state() -> (AppState, SqlitePool) {
    let mut state = state().await;
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    state.wallet_repository = Arc::new(repository);
    let management = crate::adapters::sdk::wallet_management::SdkWalletManagement::new(
        state.wallet_repository.clone(),
        state.wallets.clone(),
    );
    state.wallet_management = Arc::new(
        crate::application::wallet_management::WalletManagementService::new(
            management.clone(),
            management,
        ),
    );
    state.wallet_balance_queries = Arc::new(
        crate::application::wallet_balances::WalletBalanceService::new(
            crate::adapters::persistence::wallet_metadata::RepositoryWalletMetadata::new(
                state.wallet_repository.clone(),
            ),
            crate::adapters::sdk::wallet_balances::SdkWalletBalanceRuntime::new(
                state.wallets.clone(),
                Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap()),
            ),
            crate::adapters::sdk::address_parser::SdkAddressParser,
            crate::state::now_ms,
        ),
    );
    let wallet_ops = Arc::new(
        WalletOps::new(state.wallet_repository.clone(), state.wallets.clone())
            .await
            .unwrap(),
    );
    state.vanity_tasks = Arc::new(crate::application::vanity::VanityService::new(wallet_ops));
    for (index, id, owner_type, owner_id, default, enabled) in [
        (1, "fixture-wallet", "telegram", "42", true, true),
        (2, "owned", "telegram", "42", false, true),
        (3, "disabled", "telegram", "42", false, false),
        (4, "global", "system", "global", false, true),
        (5, "other", "telegram", "99", false, true),
    ] {
        sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,is_default,enabled,created_at) VALUES(?,?,?,?,?,?,?,?,?)")
            .bind(id).bind(owner_type).bind(owner_id).bind(format!("label-{id}"))
            .bind(Address::from_bytes([index; 32]).to_string()).bind(NON_SECRET_MARKER)
            .bind(default).bind(enabled).bind(format!("2026-01-0{index} 00:00:00"))
            .execute(&pool).await.unwrap();
    }
    (state, pool)
}

#[test]
fn wallet_wire_types_keep_v1_fields_optional_values_and_vanity_case_rules() {
    use crate::models::*;
    let request: CreateWalletRequest = serde_json::from_value(json!({
        "owner_type":"telegram", "owner_id":"42", "label":" example ", "unknown":true
    }))
    .unwrap();
    assert_eq!(request.label, " example ");
    assert!(request.private_key_base58.is_none());
    let request: CreateWalletRequest = serde_json::from_value(json!({
        "owner_type":"telegram", "owner_id":"42", "label":"", "private_key_base58":null
    }))
    .unwrap();
    assert!(request.private_key_base58.is_none());
    let update: UpdateWalletRequest = serde_json::from_value(json!({"label":" renamed "})).unwrap();
    assert_eq!(update.label, " renamed ");
    for side in ["prefix", "suffix"] {
        let request: CreateVanityJobRequest = serde_json::from_value(json!({
            "owner_type":"telegram", "owner_id":"42", "label":"vanity",
            "match_side":side, "pattern":"9999"
        }))
        .unwrap();
        assert_eq!(request.match_side.as_str(), side);
        assert!(request.max_attempts.is_none());
    }
    assert!(serde_json::from_value::<CreateVanityJobRequest>(json!({
        "owner_type":"telegram", "owner_id":"42", "label":"vanity",
        "match_side":"Prefix", "pattern":"9999"
    }))
    .is_err());
    assert_eq!(
        serde_json::to_value(QuoteBalanceResponse {
            wallet_id: "owned".into(),
            quote_mint: PUBLIC_MINT.into(),
            decimals: 6,
            balance_units: u64::MAX,
            balance_lamports: 1,
        })
        .unwrap(),
        json!({"wallet_id":"owned","quote_mint":PUBLIC_MINT,"decimals":6,"balance_units":u64::MAX,"balance_lamports":1})
    );
    assert_eq!(
        serde_json::to_value(WalletBalancesResponse {
            fetched_at_ms: 123,
            wallets: vec![WalletBalanceItem {
                wallet_id: "owned".into(),
                pubkey: PUBLIC_MINT.into(),
                balance_lamports: None,
                error: Some("synthetic unavailable".into()),
            }],
        })
        .unwrap(),
        json!({"fetched_at_ms":123,"wallets":[{"wallet_id":"owned","pubkey":PUBLIC_MINT,"balance_lamports":null,"error":"synthetic unavailable"}]})
    );
    // The value is deliberately not a key; this only freezes the existing wire field name.
    assert_eq!(
        serde_json::to_value(ExportWalletResponse {
            wallet_id: "owned".into(),
            private_key_base58: NON_SECRET_MARKER.into(),
        })
        .unwrap(),
        json!({"wallet_id":"owned","private_key_base58":NON_SECRET_MARKER})
    );
}

#[tokio::test]
async fn wallet_metadata_list_rename_and_existing_partial_delete_semantics_are_preserved() {
    let (state, pool) = metadata_state().await;
    let app = http::router(state.clone());
    let list = json_body(
        send(&app, "GET", "/v1/wallets/manage", LOCAL, Some(TOKEN), None).await,
        StatusCode::OK,
    )
    .await;
    let rows = list["wallets"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(
        rows[0],
        json!({
            "wallet_id":"fixture-wallet","owner_type":"telegram","owner_id":"42",
            "label":"label-fixture-wallet","pubkey":Address::from_bytes([1;32]).to_string(),
            "is_default":true,"enabled":true,"created":false,
        })
    );
    assert!(!list.to_string().contains(NON_SECRET_MARKER));
    assert_eq!(
        json_body(
            send(&app, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None).await,
            StatusCode::OK
        )
        .await,
        json!({"default_wallet_id":"fixture-wallet","wallets":[]})
    );
    let renamed = json_body(
        send(
            &app,
            "PATCH",
            "/v1/wallets/owned",
            LOCAL,
            Some(TOKEN),
            Some(json!({"label":"  renamed  "})),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(renamed["label"], "renamed");
    assert_eq!(renamed["wallet_id"], "owned");
    assert_eq!(renamed["created"], false);
    error_body(
        send(
            &app,
            "DELETE",
            "/v1/wallets/fixture-wallet",
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "default wallet cannot be deleted",
    )
    .await;
    // Existing behavior: the DB disable precedes the missing runtime-wallet error.
    error_body(
        send(
            &app,
            "DELETE",
            "/v1/wallets/owned",
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "wallet owned is not loaded",
    )
    .await;
    assert!(!state.wallet_repository.get("owned").await.unwrap().enabled);
    error_body(
        send(
            &app,
            "PATCH",
            "/v1/wallets/disabled",
            LOCAL,
            Some(TOKEN),
            Some(json!({"label":"x"})),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "wallet is unavailable",
    )
    .await;
    assert_eq!(state.wallets.wallet_count(), 0);
    pool.close().await;
}

#[tokio::test]
async fn wallet_quote_balance_checks_owner_and_enabled_before_mint_and_loaded_runtime() {
    let (state, pool) = metadata_state().await;
    let app = http::router(state);
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
    for id in ["owned", "global"] {
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
            StatusCode::BAD_REQUEST,
            "invalid quote mint",
        )
        .await;
        error_body(
            send(
                &app,
                "GET",
                &format!("/internal/wallets/{id}/quote-balance/{PUBLIC_MINT}?{OWNER}"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet is not loaded",
        )
        .await;
    }
    error_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallets/missing/quote-balance/invalid?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "wallet not found",
    )
    .await;
    pool.close().await;
}

#[tokio::test]
async fn wallet_balances_preserve_repository_and_invalid_public_metadata_errors_without_rpc() {
    let file = http::router(state().await);
    error_body(
        send(
            &file,
            "GET",
            &format!("/internal/wallet-balances?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE,
        "wallet database mode is not enabled",
    )
    .await;
    let (state, pool) = metadata_state().await;
    sqlx::query("UPDATE wallets SET pubkey='invalid' WHERE wallet_id='owned'")
        .execute(&pool)
        .await
        .unwrap();
    let app = http::router(state);
    let response = json_body(
        send(
            &app,
            "GET",
            &format!("/internal/wallet-balances?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert!(response["error"].is_string());
    assert!(!response.to_string().contains(NON_SECRET_MARKER));
    pool.close().await;
}

#[tokio::test]
async fn vanity_query_cancel_and_invalid_create_do_not_launch_generation() {
    let (state, pool) = metadata_state().await;
    sqlx::query("INSERT INTO wallet_generation_jobs(job_id,owner_type,owner_id,label,match_side,pattern,status,attempts,max_attempts,error) VALUES('public-job','telegram','42','fixture','prefix','9999','failed',42,100,'synthetic failure')")
        .execute(&pool).await.unwrap();
    let app = http::router(state);
    let path = format!("/internal/wallets/vanity-jobs/public-job?{OWNER}");
    let expected = json!({"job_id":"public-job","owner_type":"telegram","owner_id":"42","label":"fixture",
        "match_side":"prefix","pattern":"9999","status":"failed","attempts":42,"max_attempts":100,
        "wallet_id":null,"pubkey":null,"error":"synthetic failure"});
    for method in ["GET", "DELETE"] {
        assert_eq!(
            json_body(
                send(&app, method, &path, LOCAL, Some(TOKEN), None).await,
                StatusCode::OK
            )
            .await,
            expected
        );
    }
    let other = "/internal/wallets/vanity-jobs/public-job?owner_type=telegram&owner_id=99";
    error_body(
        send(&app, "GET", other, LOCAL, Some(TOKEN), None).await,
        StatusCode::FORBIDDEN,
        "wallet generation job owner mismatch",
    )
    .await;
    error_body(
        send(&app, "DELETE", other, LOCAL, Some(TOKEN), None).await,
        StatusCode::BAD_REQUEST,
        "wallet generation job does not belong to this owner",
    )
    .await;
    for (pattern, message) in [
        ("abcde", "pattern must contain 1..4 Base58 characters"),
        (
            "0",
            "pattern contains invalid Base58 characters (0, O, I and l are not allowed)",
        ),
    ] {
        error_body(send(&app,"POST","/internal/wallets/vanity-jobs",LOCAL,Some(TOKEN),Some(json!({
            "owner_type":"telegram","owner_id":"42","label":"fixture","match_side":"prefix","pattern":pattern
        }))).await,StatusCode::BAD_REQUEST,message).await;
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallet_generation_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    pool.close().await;
}

#[tokio::test]
async fn wallet_extractors_keep_rejections_before_handler_auth_and_methods_keep_head() {
    let app = http::router(state().await);
    for (method, path, body, status) in [
        (
            "POST",
            "/v1/wallets",
            Some(json!({})),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "PATCH",
            "/v1/wallets/missing",
            Some(json!({"label":123})),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "GET",
            "/internal/wallet-balances",
            None,
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/internal/wallets/missing/quote-balance/invalid",
            None,
            StatusCode::BAD_REQUEST,
        ),
        (
            "DELETE",
            "/internal/wallets/vanity-jobs/missing",
            None,
            StatusCode::BAD_REQUEST,
        ),
        (
            "POST",
            "/internal/wallets/vanity-jobs",
            Some(json!({
                "owner_type":"telegram","owner_id":"42","label":"fixture","match_side":"Prefix","pattern":"9999"
            })),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        assert_eq!(
            send(&app, method, path, DENIED, None, body).await.status(),
            status,
            "{method} {path}"
        );
    }
    let head = send(&app, "HEAD", "/v1/wallets", LOCAL, Some(TOKEN), None).await;
    assert_eq!(head.status(), StatusCode::OK);
    assert!(to_bytes(head.into_body(), 1024).await.unwrap().is_empty());
    for (method, path) in [
        ("DELETE", "/v1/wallets/manage"),
        ("GET", "/v1/wallets/missing/export"),
        ("PATCH", "/internal/wallets/vanity-jobs/missing"),
    ] {
        assert_eq!(
            send(&app, method, path, LOCAL, Some(TOKEN), None)
                .await
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}

#[tokio::test]
async fn wallet_missing_passphrase_and_invalid_import_stay_errors_without_key_or_runtime_access() {
    let (state, pool) = metadata_state().await;
    let app = http::router(state.clone());
    let response = send(
        &app,
        "POST",
        "/v1/wallets/owned/export",
        LOCAL,
        Some(TOKEN),
        None,
    )
    .await;
    assert!(response.headers().get("cache-control").is_none());
    error_body(
        response,
        StatusCode::BAD_REQUEST,
        "wallet database passphrase is unavailable",
    )
    .await;
    error_body(send(&app,"POST","/v1/wallets",LOCAL,Some(TOKEN),Some(json!({
        "owner_type":"telegram","owner_id":"42","label":"invalid","private_key_base58":"invalid-key-not-secret"
    }))).await,StatusCode::BAD_REQUEST,"private key must be a valid base58 Solana keypair").await;
    assert_eq!(state.wallet_repository.list().await.unwrap().len(), 5);
    assert_eq!(state.wallets.wallet_count(), 0);
    pool.close().await;
}

#[tokio::test]
async fn standalone_wallet_routes_preserve_methods_auth_and_repository_failures() {
    let state = state().await;
    let group = crate::api::routes::wallet_routes().with_state(state.clone());
    let integrated = http::router(state.clone());
    let cases = [
        (
            "GET",
            "/v1/wallets/manage",
            None,
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet database mode is not enabled",
        ),
        (
            "POST",
            "/v1/wallets",
            Some(
                json!({"owner_type":"telegram","owner_id":"42","label":"x","private_key_base58":"invalid-key-not-secret"}),
            ),
            StatusCode::BAD_REQUEST,
            "private key must be a valid base58 Solana keypair",
        ),
        (
            "PATCH",
            "/v1/wallets/missing",
            Some(json!({"label":"x"})),
            StatusCode::BAD_REQUEST,
            "wallet database mode is not enabled",
        ),
        (
            "DELETE",
            "/v1/wallets/missing",
            None,
            StatusCode::BAD_REQUEST,
            "wallet database mode is not enabled",
        ),
        (
            "POST",
            "/v1/wallets/missing/export",
            None,
            StatusCode::BAD_REQUEST,
            "wallet database mode is not enabled",
        ),
    ];
    for app in [&group, &integrated] {
        assert_eq!(
            json_body(
                send(app, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None).await,
                StatusCode::OK
            )
            .await,
            json!({"default_wallet_id":"fixture-wallet","wallets":[]})
        );
        for (method, path, body, status, message) in &cases {
            error_body(
                send(app, method, path, LOCAL, None, body.clone()).await,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            )
            .await;
            error_body(
                send(app, method, path, DENIED, Some(TOKEN), body.clone()).await,
                StatusCode::FORBIDDEN,
                "client ip not allowed",
            )
            .await;
            error_body(
                send(app, method, path, LOCAL, Some(TOKEN), body.clone()).await,
                *status,
                message,
            )
            .await;
        }
        for (method, path) in [
            ("DELETE", "/v1/wallets"),
            ("PUT", "/v1/wallets/missing"),
            ("DELETE", "/v1/wallets/manage"),
            ("GET", "/v1/wallets/missing/export"),
        ] {
            assert_eq!(
                send(app, method, path, LOCAL, Some(TOKEN), None)
                    .await
                    .status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
        }
    }
    for path in [
        "/health",
        "/v1/trades",
        "/internal/wallet-balances",
        "/internal/wallets/vanity-jobs",
    ] {
        assert_eq!(
            send(&group, "GET", path, LOCAL, Some(TOKEN), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(state.wallets.wallet_count(), 0);
    assert!(state.trades.recent(10).is_empty());
}

#[tokio::test]
async fn standalone_balance_and_vanity_groups_keep_loopback_owner_extraction_and_isolation() {
    let state = state().await;
    let balances = crate::api::routes::wallet_balance_routes().with_state(state.clone());
    let vanity = crate::api::routes::wallet_vanity_routes().with_state(state.clone());
    let integrated = http::router(state.clone());
    let balance_cases = [
        (
            "GET",
            format!("/internal/wallet-balances?{OWNER}"),
            None,
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet database mode is not enabled",
        ),
        (
            "GET",
            format!("/internal/wallets/missing/quote-balance/{PUBLIC_MINT}?{OWNER}"),
            None,
            StatusCode::NOT_FOUND,
            "wallet database mode is not enabled",
        ),
    ];
    let vanity_cases = [
        (
            "POST",
            "/internal/wallets/vanity-jobs".into(),
            Some(
                json!({"owner_type":"telegram","owner_id":"42","label":"x","match_side":"prefix","pattern":"abcde"}),
            ),
            StatusCode::BAD_REQUEST,
            "pattern must contain 1..4 Base58 characters",
        ),
        (
            "GET",
            format!("/internal/wallets/vanity-jobs/missing?{OWNER}"),
            None,
            StatusCode::NOT_FOUND,
            "wallet database mode is not enabled",
        ),
        (
            "DELETE",
            format!("/internal/wallets/vanity-jobs/missing?{OWNER}"),
            None,
            StatusCode::BAD_REQUEST,
            "wallet database mode is not enabled",
        ),
    ];
    for (group, cases) in [
        (&balances, balance_cases.as_slice()),
        (&vanity, vanity_cases.as_slice()),
    ] {
        for app in [group, &integrated] {
            for (method, path, body, status, message) in cases {
                error_body(
                    send(app, method, path, ALLOWED, Some(TOKEN), body.clone()).await,
                    StatusCode::FORBIDDEN,
                    "internal wallet API is loopback-only",
                )
                .await;
                error_body(
                    send(app, method, path, LOCAL, None, body.clone()).await,
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                )
                .await;
                error_body(
                    send(app, method, path, LOCAL, Some(TOKEN), body.clone()).await,
                    *status,
                    message,
                )
                .await;
            }
        }
        assert_eq!(
            send(group, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        send(
            &balances,
            "POST",
            "/internal/wallets/vanity-jobs",
            LOCAL,
            Some(TOKEN),
            Some(json!({}))
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &vanity,
            "GET",
            &format!("/internal/wallet-balances?{OWNER}"),
            LOCAL,
            Some(TOKEN),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    for (app, path) in [
        (&balances, "/internal/wallet-balances"),
        (&vanity, "/internal/wallets/vanity-jobs/missing"),
        (&integrated, "/internal/wallet-balances"),
        (&integrated, "/internal/wallets/vanity-jobs/missing"),
    ] {
        assert_eq!(
            send(app, "GET", path, DENIED, None, None).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(state.wallets.wallet_count(), 0);
}

#[tokio::test]
async fn standalone_wallet_groups_keep_separate_admin_credentials_distinct_from_trade_and_balance()
{
    use crate::config::security::WalletAdminAuth;
    let mut state = state().await;
    state.edit_fixture_config(|config| {
        config.security.wallet_admin = WalletAdminAuth::Separate {
            token: "wallet-test-admin-only".into(),
        };
        config.security.broadcast = crate::domain::execution_policy::BroadcastPolicy::Disabled;
    });
    let admin = "Bearer wallet-test-admin-only";
    let groups = crate::api::routes::wallet_routes()
        .merge(crate::api::routes::wallet_balance_routes())
        .merge(crate::api::routes::wallet_vanity_routes())
        .with_state(state.clone());
    let integrated = http::router(state);
    for app in [&groups, &integrated] {
        error_body(
            send(app, "GET", "/v1/wallets/manage", LOCAL, Some(TOKEN), None).await,
            StatusCode::UNAUTHORIZED,
            "wallet admin authorization required",
        )
        .await;
        error_body(
            send(app, "GET", "/v1/wallets/manage", ALLOWED, Some(admin), None).await,
            StatusCode::FORBIDDEN,
            "wallet admin API is loopback-only",
        )
        .await;
        error_body(
            send(app, "GET", "/v1/wallets/manage", LOCAL, Some(admin), None).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet database mode is not enabled",
        )
        .await;
        error_body(
            send(app, "GET", "/v1/wallets", LOCAL, Some(admin), None).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
        assert_eq!(
            send(app, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None)
                .await
                .status(),
            StatusCode::OK
        );
        let balance = format!("/internal/wallet-balances?{OWNER}");
        error_body(
            send(app, "GET", &balance, LOCAL, Some(admin), None).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
        error_body(
            send(app, "GET", &balance, LOCAL, Some(TOKEN), None).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet database mode is not enabled",
        )
        .await;
        let vanity = format!("/internal/wallets/vanity-jobs/missing?{OWNER}");
        error_body(
            send(app, "GET", &vanity, LOCAL, Some(TOKEN), None).await,
            StatusCode::UNAUTHORIZED,
            "wallet admin authorization required",
        )
        .await;
        error_body(
            send(app, "GET", &vanity, LOCAL, Some(admin), None).await,
            StatusCode::NOT_FOUND,
            "wallet database mode is not enabled",
        )
        .await;
    }
}
