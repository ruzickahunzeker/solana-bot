use super::*;
use crate::{
    adapters::{
        persistence::wsol_conversion::SqliteWsolJournal,
        sdk::{
            address_parser::SdkAddressParser, wsol_conversion::SdkWsolRuntime,
            wsol_queries::SdkWsolQuerySource,
        },
    },
    application::{wsol_queries::WsolQueryService, wsol_submission::WsolSubmissionService},
    conversion::ConversionStore,
};

#[tokio::test]
async fn state_resource_services_retain_original_arcs_without_fixture_handles_and_release_them_at_shutdown(
) {
    let mut state = state().await;
    let store = Arc::new(ConversionStore::new("sqlite::memory:").await.unwrap());
    let read_rpc =
        Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
    state.wsol_queries = Arc::new(WsolQueryService::new(
        SdkWsolQuerySource::new(
            state.wallets.clone(),
            state.wallet_repository.clone(),
            read_rpc.clone(),
        ),
        Some(SqliteWsolJournal::new(store.clone())),
        SdkAddressParser,
    ));
    state.wsol_submissions = Arc::new(WsolSubmissionService::new(
        SdkWsolRuntime::new(
            state.wallets.clone(),
            state.wallet_repository.clone(),
            read_rpc.clone(),
        ),
        Some(SqliteWsolJournal::new(store.clone())),
    ));
    state.conversions = Some(store.clone());
    let wallets = Arc::downgrade(&state.wallets);
    let repository = Arc::downgrade(&state.wallet_repository);
    let quotes = Arc::downgrade(&state.aggregator_quotes);
    let conversions = Arc::downgrade(&store);
    let reads = Arc::downgrade(&read_rpc);
    drop(read_rpc);
    drop(store);

    let (replacement_repository, pool) =
        WalletRepository::empty_for_transfer_tests().await.unwrap();
    state.wallets = Arc::new(WalletRegistry::empty_for_http_tests(
        "unused-fixture-default".into(),
        state.config.mev.clone(),
    ));
    state.wallet_repository = Arc::new(replacement_repository);
    state.aggregator_quotes = Arc::new(crate::aggregator_quote::QuoteStore::default());
    state.conversions = None;

    assert!(wallets.upgrade().is_some());
    assert!(repository.upgrade().is_some());
    assert!(quotes.upgrade().is_some());
    assert!(conversions.upgrade().is_some());
    assert!(reads.upgrade().is_some());
    let app = http::router(state);
    assert_eq!(
        json_body(
            send(&app, "GET", "/v1/wallets", LOCAL, Some(TOKEN), None).await,
            StatusCode::OK
        )
        .await,
        json!({"default_wallet_id":"fixture-wallet","wallets":[]})
    );
    error_body(
        send(&app, "GET", "/v1/wallets/manage", LOCAL, Some(TOKEN), None).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "wallet database mode is not enabled",
    )
    .await;
    drop(app);

    assert!(wallets.upgrade().is_none());
    assert!(repository.upgrade().is_none());
    assert!(quotes.upgrade().is_none());
    assert!(conversions.upgrade().is_none());
    assert!(reads.upgrade().is_none());
    pool.close().await;
}

#[tokio::test]
async fn state_resource_fixture_store_is_not_an_authority_that_can_enable_disabled_conversion_services(
) {
    let mut state = state().await;
    let store = Arc::new(ConversionStore::new("sqlite::memory:").await.unwrap());
    let weak = Arc::downgrade(&store);
    state.conversions = Some(store);
    let app = http::router(state);
    let response=send(&app,"POST","/internal/conversions/wsol/quotes",LOCAL,Some(TOKEN),Some(json!({
        "wallet_id":"fixture-wallet","owner_type":"telegram","owner_id":"42","direction":"wrap","amount_lamports":1
    }))).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-wsol-error-code"], "wsol_unavailable");
    assert_eq!(response.headers()["x-wsol-error-stage"], "quote_store");
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = json_body(response, StatusCode::SERVICE_UNAVAILABLE).await;
    assert!(body["error"]
        .as_str()
        .unwrap()
        .starts_with("conversion quotes are disabled"));
    assert!(weak.upgrade().is_some());
    drop(app);
    assert!(weak.upgrade().is_none());
}
