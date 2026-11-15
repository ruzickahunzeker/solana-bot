use super::*;
use crate::{
    application::wallet_management::{
        test_support::{Backend, EXPORT_MARKER},
        WalletManagementService,
    },
    domain::trade::Address,
};
use std::sync::atomic::Ordering;

fn input(secret: Option<&str>) -> Value {
    json!({"owner_type":"telegram","owner_id":"42","label":"new wallet","private_key_base58":secret})
}

async fn setup(backend: &Backend) -> AppState {
    let mut state = state().await;
    state.wallet_management = Arc::new(WalletManagementService::new(
        backend.clone(),
        backend.clone(),
    ));
    state
}

#[tokio::test]
async fn wallet_management_http_loaded_managed_created_existing_and_restored_keep_exact_public_wire(
) {
    let backend = Backend::new();
    let state = setup(&backend).await;
    let app = http::router(state.clone());
    assert_eq!(
        json_body(
            send(&app, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None).await,
            StatusCode::OK
        )
        .await,
        json!({"default_wallet_id":" runtime-default ","wallets":[{"wallet_id":"runtime-wallet","pubkey":Address::from_bytes([2;32]).to_string()}]})
    );
    let managed = json_body(
        send(&app, "GET", "/v1/wallets/manage", LOCAL, Some(TOKEN), None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        managed,
        json!({"wallets":[{"wallet_id":"managed-wallet","owner_type":"telegram","owner_id":"42","label":"public label","pubkey":Address::from_bytes([1;32]).to_string(),"is_default":false,"enabled":true,"created":false}]})
    );
    let created = json_body(
        send(
            &app,
            "POST",
            "/v1/wallets",
            LOCAL,
            Some(TOKEN),
            Some(input(None)),
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(
        created,
        json!({"wallet_id":"managed-wallet","owner_type":"telegram","owner_id":"42","label":"new wallet","pubkey":Address::from_bytes([1;32]).to_string(),"is_default":false,"enabled":true,"created":true})
    );
    for (restored, status) in [(false, StatusCode::OK), (true, StatusCode::CREATED)] {
        backend.state.lock().unwrap().import_created = restored;
        let result = json_body(
            send(
                &app,
                "POST",
                "/v1/wallets",
                LOCAL,
                Some(TOKEN),
                Some(input(Some("fake-import-not-a-key"))),
            )
            .await,
            status,
        )
        .await;
        let mut expected = created.clone();
        expected["created"] = json!(restored);
        assert_eq!(result, expected);
        assert!(!result.to_string().contains("fake-import"));
        assert!(!result.to_string().contains(EXPORT_MARKER));
    }
    assert_eq!(backend.drops.load(Ordering::SeqCst), 3);
    assert_eq!(
        backend.events(),
        [
            "loaded",
            "list",
            "create",
            "install:managed-wallet",
            "import",
            "install:managed-wallet",
            "import",
            "install:managed-wallet"
        ]
    );
    assert_eq!(state.wallets.wallet_count(), 0);
    assert!(state.trades.recent(10).is_empty());
}

#[tokio::test]
async fn wallet_management_http_failures_map_original_statuses_and_do_not_hide_load_compensation() {
    for (op, method, path, body, status) in [
        (
            "list",
            "GET",
            "/v1/wallets/manage",
            None,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            "create",
            "POST",
            "/v1/wallets",
            Some(input(None)),
            StatusCode::BAD_REQUEST,
        ),
        (
            "import",
            "POST",
            "/v1/wallets",
            Some(input(Some("fake-import-not-a-key"))),
            StatusCode::BAD_REQUEST,
        ),
        (
            "install",
            "POST",
            "/v1/wallets",
            Some(input(None)),
            StatusCode::BAD_GATEWAY,
        ),
        (
            "rename",
            "PATCH",
            "/v1/wallets/managed-wallet",
            Some(json!({"label":" new "})),
            StatusCode::BAD_REQUEST,
        ),
        (
            "disable",
            "DELETE",
            "/v1/wallets/managed-wallet",
            None,
            StatusCode::BAD_REQUEST,
        ),
        (
            "remove",
            "DELETE",
            "/v1/wallets/managed-wallet",
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            "export",
            "POST",
            "/v1/wallets/managed-wallet/export",
            None,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let backend = Backend::new();
        backend.fail(op);
        let app = http::router(setup(&backend).await);
        let response = send(&app, method, path, LOCAL, Some(TOKEN), body).await;
        assert!(response.headers().get("cache-control").is_none());
        error_body(response, status, &format!("synthetic {op} failure")).await;
        if op == "install" {
            assert_eq!(
                backend.events(),
                ["create", "install:managed-wallet", "disable:managed-wallet"]
            );
            assert!(!backend.state.lock().unwrap().wallet.enabled);
        }
        if op == "remove" {
            assert_eq!(
                backend.events(),
                ["disable:managed-wallet", "remove:managed-wallet"]
            );
            assert!(!backend.state.lock().unwrap().wallet.enabled);
        }
    }
}

#[tokio::test]
async fn wallet_management_http_rename_delete_and_explicit_export_keep_secret_and_cache_boundaries()
{
    let backend = Backend::new();
    let app = http::router(setup(&backend).await);
    let renamed = json_body(
        send(
            &app,
            "PATCH",
            "/v1/wallets/managed-wallet",
            LOCAL,
            Some(TOKEN),
            Some(json!({"label":" renamed "})),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(renamed["label"], "renamed");
    assert_eq!(renamed["created"], false);
    assert!(!renamed.to_string().contains(EXPORT_MARKER));
    let exported = send(
        &app,
        "POST",
        "/v1/wallets/managed-wallet/export",
        LOCAL,
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!(exported.headers()["cache-control"], "no-store");
    assert_eq!(
        json_body(exported, StatusCode::OK).await,
        json!({"wallet_id":"managed-wallet","private_key_base58":EXPORT_MARKER})
    );
    let deleted = json_body(
        send(
            &app,
            "DELETE",
            "/v1/wallets/managed-wallet",
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(deleted["enabled"], false);
    assert_eq!(deleted["created"], false);
    assert!(!deleted.to_string().contains(EXPORT_MARKER));
    assert_eq!(
        backend.events(),
        [
            "rename:managed-wallet:renamed",
            "export:managed-wallet",
            "disable:managed-wallet",
            "remove:managed-wallet"
        ]
    );
}

#[tokio::test]
async fn wallet_management_http_auth_extractors_and_broadcast_policy_stay_outside_management_core()
{
    use crate::{config::security::WalletAdminAuth, domain::execution_policy::BroadcastPolicy};
    let backend = Backend::new();
    let mut state = setup(&backend).await;
    let config = Arc::get_mut(&mut state.config).unwrap();
    config.security.wallet_admin = WalletAdminAuth::Separate {
        token: "test-wallet-admin-only".into(),
    };
    config.security.broadcast = BroadcastPolicy::Disabled;
    let app = http::router(state);
    let admin = "Bearer test-wallet-admin-only";
    for (method, path, body) in [
        ("GET", "/v1/wallets/manage", None),
        ("POST", "/v1/wallets", Some(input(None))),
        (
            "PATCH",
            "/v1/wallets/managed-wallet",
            Some(json!({"label":"x"})),
        ),
        ("DELETE", "/v1/wallets/managed-wallet", None),
        ("POST", "/v1/wallets/managed-wallet/export", None),
    ] {
        error_body(
            send(&app, method, path, LOCAL, Some(TOKEN), body.clone()).await,
            StatusCode::UNAUTHORIZED,
            "wallet admin authorization required",
        )
        .await;
        error_body(
            send(&app, method, path, ALLOWED, Some(admin), body).await,
            StatusCode::FORBIDDEN,
            "wallet admin API is loopback-only",
        )
        .await;
    }
    assert_eq!(
        send(&app, "POST", "/v1/wallets", DENIED, None, Some(json!({})))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(backend.events().is_empty());
    assert_eq!(
        send(&app, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::OK
    );
    error_body(
        send(&app, "GET", "/v1/wallets", LOCAL, Some(admin), None).await,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/wallets",
            LOCAL,
            Some(admin),
            Some(input(None))
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        backend.events(),
        ["loaded", "create", "install:managed-wallet"]
    );
}
