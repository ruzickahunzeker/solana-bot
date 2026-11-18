//! Quote and unsigned simulation HTTP contracts without wallet secrets or network.
use super::*;
use crate::{
    application::{
        wsol_queries::{quote_boundary_tests::estimate, tests as queries},
        wsol_submission::{tests as submissions, unsigned_boundary_tests::simulation_input},
    },
    conversion::{ConversionStore, WsolSubmission},
    domain::{execution_policy::BroadcastPolicy, trade::Address},
};

const BASE: &str = "/internal/conversions/wsol";

fn quote_request() -> Value {
    json!({"wallet_id":"fixture-wallet","owner_type":"telegram","owner_id":"42","direction":"wrap","amount_lamports":1000000})
}
async fn post(app: &Router, endpoint: &str, request: Value) -> Response {
    send(
        app,
        "POST",
        &format!("{BASE}/{endpoint}"),
        LOCAL,
        Some(TOKEN),
        Some(request),
    )
    .await
}
fn unsigned_request() -> Value {
    serde_json::to_value(WsolSubmission::from(&simulation_input())).unwrap()
}

async fn quote_fixture(mode: usize) -> (Router, Arc<queries::Shared>) {
    let (service, shared) = queries::harness().await;
    shared.mode(mode);
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.security.broadcast = BroadcastPolicy::Disabled;
    state.replace_fixture_config(Arc::new(config));
    state.wsol_queries = service;
    (http::router(state), shared)
}
async fn simulation_fixture(mode: usize) -> (Router, Arc<submissions::Shared>) {
    let (service, shared) = submissions::harness().await;
    shared.set_runtime_mode(mode);
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.security.broadcast = BroadcastPolicy::Disabled;
    config.conversion_live_enabled = false;
    state.replace_fixture_config(Arc::new(config));
    state.wsol_submissions = service;
    (http::router(state), shared)
}
async fn failure(response: Response, status: StatusCode, code: &str, stage: &str) {
    assert_eq!(response.headers()["x-wsol-error-code"], code);
    assert_eq!(response.headers()["x-wsol-error-stage"], stage);
    assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
    assert_eq!(response.headers()["cache-control"], "no-store");
    for name in ["x-wsol-submission-id", "x-wsol-signature", "retry-after"] {
        assert!(response.headers().get(name).is_none());
    }
    let body = json_body(response, status).await;
    assert_eq!(body.as_object().unwrap().len(), 1);
    let message = body["error"].as_str().unwrap();
    assert!(!message.contains("private-api-key") && !message.contains("private-key"));
    if status == StatusCode::SERVICE_UNAVAILABLE {
        assert!(message.ends_with("WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key"));
    }
}

#[tokio::test]
async fn wsol_quote_unsigned_router_quote_identity_and_estimate_faults_never_save_or_publish_quotes(
) {
    let (app, shared) = quote_fixture(20).await;
    failure(
        post(&app, "quotes", quote_request()).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "wsol_integrity_error",
        "quote_metadata",
    )
    .await;
    assert_eq!(shared.events(), ["metadata"]);
    for field in 0..6 {
        let (app, shared) = quote_fixture(0).await;
        let mut bad = estimate();
        match field {
            0 => bad.amount_lamports = 0,
            1 => bad.amount_lamports += 1,
            2 => bad.source_balance_lamports = 1,
            3 => bad.temporary_rent_lamports = 1,
            4 => bad.rent_refund_lamports = 1,
            _ => bad.estimated_network_fee_lamports = u64::MAX,
        }
        shared.replace_estimate(bad);
        failure(
            post(&app, "quotes", quote_request()).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_integrity_error",
            "quote_estimate",
        )
        .await;
        assert!(shared.saved_quote().is_none());
        assert!(shared.events().iter().all(|event| event != "save"));
    }
}

#[tokio::test]
async fn wsol_quote_unsigned_router_quote_phase_errors_preserve_typed_rejections_and_save_uncertainty(
) {
    for (mode, status, code, stage) in [
        (
            1,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "quote_metadata",
        ),
        (
            21,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "quote_metadata",
        ),
        (
            22,
            StatusCode::BAD_REQUEST,
            "wsol_rejected",
            "quote_metadata",
        ),
        (
            4,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_integrity_error",
            "quote_metadata",
        ),
        (
            5,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "quote_estimate",
        ),
        (
            7,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "quote_estimate",
        ),
        (
            23,
            StatusCode::BAD_REQUEST,
            "wsol_rejected",
            "quote_estimate",
        ),
        (
            12,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_outcome_unknown",
            "quote_save",
        ),
        (
            24,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_outcome_unknown",
            "quote_save",
        ),
        (
            25,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_outcome_unknown",
            "quote_save",
        ),
    ] {
        let (app, shared) = quote_fixture(mode).await;
        failure(
            post(&app, "quotes", quote_request()).await,
            status,
            code,
            stage,
        )
        .await;
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
        if matches!(mode, 24 | 25) {
            let quote = shared.saved_quote().unwrap();
            assert_eq!(
                shared.store.get_quote(&quote.quote_id).await.unwrap(),
                Some((&quote).into())
            );
        }
    }
}

#[tokio::test]
async fn wsol_quote_unsigned_router_real_metadata_absence_decode_and_unloaded_wallet_are_distinct()
{
    for (exists, corrupt) in [
        (false, None),
        (true, None),
        (true, Some("owner_id")),
        (true, Some("pubkey")),
        (true, Some("enabled")),
    ] {
        let mut state = state().await;
        let (repository, pool) =
            crate::wallet_repository::WalletRepository::empty_for_transfer_tests()
                .await
                .unwrap();
        if exists {
            sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES('fixture-wallet','telegram','42','fixture',?,'not-a-wallet-secret',1)")
                .bind(Address::from_bytes([1;32]).to_string()).execute(&pool).await.unwrap();
        }
        if let Some(column) = corrupt {
            sqlx::query(&format!("UPDATE wallets SET {column}=X'FF'"))
                .execute(&pool)
                .await
                .unwrap();
        }
        let store = Arc::new(ConversionStore::new("sqlite::memory:").await.unwrap());
        state.wsol_queries = Arc::new(crate::application::wsol_queries::WsolQueryService::new(
            crate::adapters::sdk::wsol_queries::SdkWsolQuerySource::new(
                state.wallets.clone(),
                Arc::new(repository),
                Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap()),
            ),
            Some(
                crate::adapters::persistence::wsol_conversion::SqliteWsolJournal::new(
                    store.clone(),
                ),
            ),
            crate::adapters::sdk::address_parser::SdkAddressParser,
        ));
        assert_eq!(state.wallets.wallet_count(), 0);
        let app = http::router(state);
        failure(
            post(&app, "quotes", quote_request()).await,
            if exists {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::BAD_REQUEST
            },
            if corrupt.is_some() {
                "wsol_integrity_error"
            } else if exists {
                "wsol_unavailable"
            } else {
                "wsol_rejected"
            },
            if exists && corrupt.is_none() {
                "quote_estimate"
            } else {
                "quote_metadata"
            },
        )
        .await;
        assert!(store.get_by_key("key").await.unwrap().is_none());
    }
}

#[tokio::test]
async fn wsol_quote_unsigned_router_unsigned_faults_never_reserve_sign_or_return_success() {
    for (mode, status, code, stage) in [
        (
            1,
            StatusCode::BAD_REQUEST,
            "wsol_rejected",
            "unsigned_metadata",
        ),
        (
            4,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_integrity_error",
            "unsigned_wallet",
        ),
        (
            6,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "unsigned_prepare",
        ),
        (
            13,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "unsigned_prepare",
        ),
        (
            14,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "unsigned_rpc",
        ),
        (15, StatusCode::BAD_REQUEST, "wsol_rejected", "unsigned_rpc"),
        (
            16,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_integrity_error",
            "unsigned_rpc",
        ),
        (
            21,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_unavailable",
            "unsigned_metadata",
        ),
        (
            22,
            StatusCode::SERVICE_UNAVAILABLE,
            "wsol_integrity_error",
            "unsigned_wallet",
        ),
    ] {
        let (app, shared) = simulation_fixture(mode).await;
        failure(
            post(&app, "simulate", unsigned_request()).await,
            status,
            code,
            stage,
        )
        .await;
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
        assert!(shared
            .events()
            .iter()
            .all(|event| !event.starts_with("sign:")
                && !event.starts_with("broadcast:")
                && event != "reserve"));
    }
}

#[tokio::test]
async fn wsol_quote_unsigned_router_validation_auth_and_extractors_precede_unsigned_side_effects() {
    let (app, shared) = simulation_fixture(0).await;
    let endpoint = format!("{BASE}/simulate");
    assert_eq!(
        send(
            &app,
            "POST",
            &endpoint,
            LOCAL,
            None,
            Some(unsigned_request())
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "POST",
            &endpoint,
            ALLOWED,
            Some(TOKEN),
            Some(unsigned_request())
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, "POST", &endpoint, LOCAL, Some(TOKEN), Some(json!({})))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(shared.events().is_empty());
    for (field, stage) in [
        ("simulate", "unsigned_admission"),
        ("idempotency_key", "unsigned_input"),
        ("owner_id", "unsigned_input"),
    ] {
        let mut request = unsigned_request();
        request[field] = if field == "simulate" {
            json!(false)
        } else {
            json!("")
        };
        failure(
            post(&app, "simulate", request).await,
            StatusCode::BAD_REQUEST,
            "wsol_rejected",
            stage,
        )
        .await;
    }
    assert!(shared.store.get_by_key("key").await.unwrap().is_none());
    assert!(shared.events().iter().all(|event| event == "quote"));
}

#[tokio::test]
async fn wsol_quote_unsigned_router_preserves_success_wire_shapes_and_existing_live_record() {
    let (app, shared) = quote_fixture(0).await;
    let response = post(&app, "quotes", quote_request()).await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = json_body(response, StatusCode::OK).await;
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert_eq!(body["quote"].as_object().unwrap().len(), 13);
    assert_eq!(body["executable"], false);
    assert_eq!(body["quote"]["amount_lamports"], 1_000_000);
    assert!(shared.store.get_by_key("key").await.unwrap().is_none());
    let (app, shared) = simulation_fixture(0).await;
    let quote = shared.store.get_quote("q1").await.unwrap().unwrap();
    let (original, _) = shared
        .store
        .reserve_live(&WsolSubmission::from(&submissions::input()), &quote, 1)
        .await
        .unwrap();
    let response = post(&app, "simulate", unsigned_request()).await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = json_body(response, StatusCode::OK).await;
    assert_eq!(
        body,
        json!({"status":"simulated","quote_id":"q1","network_fee_lamports":10000,"units_consumed":999})
    );
    assert_eq!(
        shared.store.get_by_key("key").await.unwrap(),
        Some(original)
    );
    assert_eq!(
        shared.events(),
        [
            "quote",
            "metadata",
            "resolve:Some(1)",
            "prepare:1",
            "unsigned:1"
        ]
    );
}
