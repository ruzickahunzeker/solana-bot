use super::*;
use crate::{
    application::{
        ports::{
            ordinary_execution::{
                OrdinaryExecutionBackend, OrdinaryExecutionWallet, SdkExecutionContext,
                TradeExecutionFuture,
            },
            wallet_identity::{LoadedWalletDirectory, LoadedWalletIdentity},
        },
        trade_command::TradeCommand,
        trade_execution::TradeExecutionResult,
    },
    domain::{blockhash::Blockhash, trade::Address},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
struct Trace {
    dropped: AtomicUsize,
    entered: Notify,
    release: Notify,
    calls: AtomicUsize,
}
struct Identity(Arc<Trace>);
impl Drop for Identity {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl LoadedWalletIdentity for Identity {
    fn pubkey(&self) -> Address {
        Address::from_bytes([9; 32])
    }
}
struct Directory(Arc<Trace>);
impl LoadedWalletDirectory for Directory {
    fn resolve_id(&self, requested: Option<&str>) -> String {
        requested.unwrap_or("loaded").into()
    }
    fn lookup(&self, id: &str) -> Option<Box<dyn LoadedWalletIdentity>> {
        assert_eq!(id, "loaded");
        Some(Box::new(Identity(self.0.clone())))
    }
}
struct Backend(Arc<Trace>);
impl OrdinaryExecutionBackend for Backend {
    fn resolve(&self, id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        assert_eq!(id, "loaded");
        Some(Box::new(Backend(self.0.clone())))
    }
}
impl OrdinaryExecutionWallet for Backend {
    fn sdk<'a>(
        &'a self,
        command: &'a TradeCommand,
        _: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(async move {
            assert_eq!(command.wallet_id, "loaded");
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            self.0.entered.notify_one();
            self.0.release.notified().await;
            Ok(TradeExecutionResult {
                success: true,
                signature: Some("synthetic-intake-result".into()),
                error: None,
            })
        })
    }
    fn pumpapi<'a>(&'a self, _: &'a TradeCommand, _: bool) -> TradeExecutionFuture<'a> {
        Box::pin(async { panic!("no PumpAPI execution in this compatibility fixture") })
    }
}

#[tokio::test]
async fn ordinary_intake_http_retains_wallet_identity_lease_through_dispatch_and_response_wait() {
    let mut state = state().await;
    state.edit_fixture_config(|config| {
        config.trade_response_wait_ms = 5000;
        config.blockhash_max_age_ms = u64::MAX;
    });
    state.blockhash_cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([9; 32]),
        crate::state::now_ms(),
    ));
    let trace = Arc::new(Trace::default());
    state.wallet_identity = Arc::new(Directory(trace.clone()));
    state.ordinary_execution = Arc::new(Backend(trace.clone()));
    let app = http::router(state);
    let task = tokio::spawn(async move {
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(json!({
                "side":"buy","pool":"pump","mint":"11111111111111111111111111111111",
                "amount_lamports":17,"simulate":false,"idempotency_key":"intake-lease"
            })),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
        .await
        .unwrap();
    assert!(!task.is_finished());
    assert_eq!(trace.dropped.load(Ordering::SeqCst), 0);
    trace.release.notify_one();
    let response = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    let body = json_body(response, StatusCode::OK).await;
    assert_eq!(body["status"], "submitted");
    assert_eq!(body["signature"], "synthetic-intake-result");
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    assert_eq!(trace.dropped.load(Ordering::SeqCst), 1);
}

fn task_request(key: &str) -> Value {
    json!({
        "side":"buy", "pool":"pump", "mint":"11111111111111111111111111111111",
        "amount_lamports":17, "simulate":false, "idempotency_key":key
    })
}

async fn task_fixture(mode: TradePersistenceMode, wait: u64) -> (AppState, Arc<Trace>) {
    let mut state = state().await;
    state.edit_fixture_config(|config| {
        config.trade_response_wait_ms = wait;
        config.blockhash_max_age_ms = u64::MAX;
    });
    state.blockhash_cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([9; 32]),
        crate::state::now_ms(),
    ));
    if mode != TradePersistenceMode::Memory {
        state.persistence = Arc::new(
            crate::persistence::TradePersistence::new(&crate::config::TradePersistenceConfig {
                mode,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 100,
            })
            .await
            .unwrap(),
        );
    }
    let trace = Arc::new(Trace::default());
    state.wallet_identity = Arc::new(Directory(trace.clone()));
    state.ordinary_execution = Arc::new(Backend(trace.clone()));
    (state, trace)
}

#[tokio::test]
async fn trade_tasks_http_closed_intake_keeps_auth_batch_and_read_only_query_contracts() {
    let state = state().await;
    let tasks = state.trade_tasks.clone();
    tasks.close();
    let app = http::router(state.clone());
    let body = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(task_request("closed")),
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE,
    )
    .await;
    assert_eq!(body, json!({"error":"trade service is shutting down"}));
    let response = send(
        &app,
        "POST",
        "/v1/trades",
        LOCAL,
        None,
        Some(task_request("unauthorized")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = json_body(
        send(
            &app,
            "POST",
            "/v1/batch/trades",
            LOCAL,
            Some(TOKEN),
            Some(json!({
                "wallet_ids":["loaded","missing"],"trade":task_request("batch-closed")
            })),
        )
        .await,
        StatusCode::ACCEPTED,
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
    for item in body["items"].as_array().unwrap() {
        assert!(item["request_id"].is_null());
        assert!(item["error"]
            .as_str()
            .unwrap()
            .contains("503 Service Unavailable: trade service is shutting down"));
    }
    assert!(state
        .trade_queries()
        .recent(Some(10))
        .await
        .unwrap()
        .is_empty());
    let queried = json_body(
        send(&app, "GET", "/v1/trades", LOCAL, Some(TOKEN), None).await,
        StatusCode::OK,
    )
    .await;
    assert!(queried.as_array().unwrap().is_empty());
    assert!(tasks.drain(std::time::Duration::from_secs(1)).await.clean());
}

#[tokio::test]
async fn trade_tasks_http_accepted_background_work_outlives_response_in_all_original_modes() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let (state, trace) = task_fixture(mode, 0).await;
        let tasks = state.trade_tasks.clone();
        let app = http::router(state);
        let body = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(task_request("tracked-original")),
            )
            .await,
            StatusCode::ACCEPTED,
        )
        .await;
        let id = body["request_id"].as_str().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
            .await
            .unwrap();
        let report = tasks.drain(std::time::Duration::from_millis(1)).await;
        assert_eq!(
            (report.active, report.admitting, report.abandoned),
            (1, 0, 0)
        );
        assert_eq!(report.request_ids, [id]);
        trace.release.notify_one();
        assert!(tasks.drain(std::time::Duration::from_secs(2)).await.clean());
        let queried = json_body(
            send(
                &app,
                "GET",
                &format!("/v1/trades/{id}"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(queried["status"], "submitted");
        assert_eq!(queried["signature"], "synthetic-intake-result");
        assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn trade_tasks_http_request_cancellation_does_not_cancel_the_accepted_executor() {
    let (state, trace) = task_fixture(TradePersistenceMode::Durable, 5000).await;
    let tasks = state.trade_tasks.clone();
    let app = http::router(state.clone());
    let request_app = app.clone();
    let request = tokio::spawn(async move {
        send(
            &request_app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(task_request("request-cancelled")),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
        .await
        .unwrap();
    request.abort();
    let Err(cancelled) = request.await else {
        panic!("HTTP request must be cancelled")
    };
    assert!(cancelled.is_cancelled());
    let report = tasks.drain(std::time::Duration::from_millis(1)).await;
    assert_eq!((report.active, report.abandoned), (1, 0));
    let id = report.request_ids[0].clone();
    trace.release.notify_one();
    assert!(tasks.drain(std::time::Duration::from_secs(2)).await.clean());
    let queried = json_body(
        send(
            &app,
            "GET",
            &format!("/v1/trades/{id}"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(queried["status"], "submitted");
    assert_eq!(queried["signature"], "synthetic-intake-result");
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn trade_tasks_http_duplicate_and_rejected_requests_do_not_register_extra_executors() {
    let (state, trace) = task_fixture(TradePersistenceMode::Durable, 0).await;
    let tasks = state.trade_tasks.clone();
    let app = http::router(state);
    let first = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(task_request("same-original")),
        )
        .await,
        StatusCode::ACCEPTED,
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
        .await
        .unwrap();
    let again = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(task_request("same-original")),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(first["request_id"], again["request_id"]);
    assert_eq!(again["duplicate"], true);
    let mut invalid = task_request("invalid-original");
    invalid["amount_lamports"] = json!(0);
    let rejected = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(invalid),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(rejected["status"], "rejected");
    let report = tasks.drain(std::time::Duration::from_millis(1)).await;
    assert_eq!(
        (report.active, report.admitting, report.abandoned),
        (1, 0, 0)
    );
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    trace.release.notify_one();
    assert!(tasks.drain(std::time::Duration::from_secs(2)).await.clean());
}
