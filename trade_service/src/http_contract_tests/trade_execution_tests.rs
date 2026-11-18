//! Actual Router -> acceptance -> executor -> temporary persistence, without keys or RPC.
mod aggregate_uncertainty;

use super::*;
use crate::{
    application::{
        ports::{
            ordinary_execution::{
                OrdinaryExecutionBackend, OrdinaryExecutionWallet, SdkExecutionContext,
                TradeExecutionFuture,
            },
            ordinary_sol_buy::{OrdinarySolBuyBackend, OrdinarySolBuyWallet},
            wallet_identity::{LoadedWalletDirectory, LoadedWalletIdentity},
        },
        trade_command::TradeCommand,
        trade_execution::{TradeExecutionResult, UncertainSdkSubmission},
    },
    domain::{
        blockhash::Blockhash,
        execution_policy::BroadcastPolicy,
        trade::{Address, ExecutionMethod},
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};
use tokio::sync::Notify;

#[derive(Default)]
struct Trace {
    ids: Mutex<Vec<Option<String>>>,
    identities: Mutex<Vec<String>>,
    resolves: Mutex<Vec<String>>,
    commands: Mutex<Vec<(ExecutionMethod, TradeCommand)>>,
    aggregate_resolves: Mutex<Vec<String>>,
    aggregate_commands: Mutex<Vec<TradeCommand>>,
    leases: AtomicUsize,
    leases_at_execution: Mutex<Vec<usize>>,
    execution_missing: AtomicBool,
    preview_outcome: Mutex<Option<bool>>,
    sdk_uncertainty: Mutex<Option<Vec<String>>>,
    gate: Mutex<Option<Arc<Notify>>>,
    started: Notify,
}

struct Directory {
    trace: Arc<Trace>,
    wallets: BTreeMap<String, Address>,
}
struct Identity {
    trace: Arc<Trace>,
    address: Address,
}

impl LoadedWalletIdentity for Identity {
    fn pubkey(&self) -> Address {
        self.address
    }
}
impl Drop for Identity {
    fn drop(&mut self) {
        self.trace.leases.fetch_sub(1, Ordering::SeqCst);
    }
}
impl LoadedWalletDirectory for Directory {
    fn resolve_id(&self, requested: Option<&str>) -> String {
        self.trace
            .ids
            .lock()
            .unwrap()
            .push(requested.map(str::to_owned));
        requested.unwrap_or("alpha").into()
    }
    fn lookup(&self, wallet_id: &str) -> Option<Box<dyn LoadedWalletIdentity>> {
        self.trace.identities.lock().unwrap().push(wallet_id.into());
        self.wallets.get(wallet_id).map(|address| {
            self.trace.leases.fetch_add(1, Ordering::SeqCst);
            Box::new(Identity {
                trace: self.trace.clone(),
                address: *address,
            }) as Box<dyn LoadedWalletIdentity>
        })
    }
}

struct Backend(Arc<Trace>);
struct Wallet(Arc<Trace>);
struct AggregateBackend(Arc<Trace>);
struct AggregateWallet(Arc<Trace>);

impl OrdinarySolBuyBackend for AggregateBackend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        self.0
            .aggregate_resolves
            .lock()
            .unwrap()
            .push(wallet_id.into());
        if self.0.execution_missing.load(Ordering::SeqCst) {
            None
        } else {
            Some(Box::new(AggregateWallet(self.0.clone())))
        }
    }
}
impl OrdinarySolBuyWallet for AggregateWallet {
    fn buy<'a>(
        &'a self,
        execution: &'a crate::application::trade_admission::AdmittedTrade,
        _owner: Option<&'a crate::application::ports::ordinary_claims::ExecutionOwner>,
    ) -> crate::application::ports::ordinary_sol_buy::SolBuyExecutionFuture<'a> {
        let command = execution.command();
        Box::pin(async move {
            self.0
                .aggregate_commands
                .lock()
                .unwrap()
                .push(command.clone());
            Ok(
                crate::application::ports::ordinary_sol_buy::SolBuyExecutionOutcome::Legacy(
                    TradeExecutionResult {
                        success: true,
                        signature: Some(format!("synthetic-aggregate-{}", command.wallet_id)),
                        error: None,
                    },
                ),
            )
        })
    }
}
impl OrdinaryExecutionBackend for Backend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        self.0.resolves.lock().unwrap().push(wallet_id.into());
        if self.0.execution_missing.load(Ordering::SeqCst) {
            None
        } else {
            Some(Box::new(Wallet(self.0.clone())))
        }
    }
}
impl Wallet {
    fn run<'a>(
        &'a self,
        method: ExecutionMethod,
        command: &TradeCommand,
    ) -> TradeExecutionFuture<'a> {
        let command = command.clone();
        Box::pin(async move {
            self.0
                .commands
                .lock()
                .unwrap()
                .push((method, command.clone()));
            self.0
                .leases_at_execution
                .lock()
                .unwrap()
                .push(self.0.leases.load(Ordering::SeqCst));
            self.0.started.notify_one();
            let gate = self.0.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            if method == ExecutionMethod::Sdk {
                let uncertainty = self.0.sdk_uncertainty.lock().unwrap().clone();
                if let Some(signatures) = uncertainty {
                    return Err(UncertainSdkSubmission::new(
                        signatures,
                        "synthetic SDK invocation outcome uncertain".into(),
                    )
                    .into());
                }
            }
            let preview = *self.0.preview_outcome.lock().unwrap();
            Ok(match preview {
                Some(success) => TradeExecutionResult {
                    success,
                    signature: None,
                    error: (!success).then(|| "synthetic preview program failure".into()),
                },
                None => TradeExecutionResult {
                    success: true,
                    signature: Some(format!("synthetic-signature-{}", command.wallet_id)),
                    error: None,
                },
            })
        })
    }
}
impl OrdinaryExecutionWallet for Wallet {
    fn sdk<'a>(
        &'a self,
        command: &'a TradeCommand,
        context: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a> {
        assert_eq!(context.blockhash.to_bytes(), [42; 32]);
        self.run(ExecutionMethod::Sdk, command)
    }
    fn pumpapi<'a>(&'a self, command: &'a TradeCommand, _: bool) -> TradeExecutionFuture<'a> {
        self.run(ExecutionMethod::Pumpapi, command)
    }
}

async fn with_backend(mode: TradePersistenceMode) -> (AppState, Arc<Trace>) {
    let mut state = state().await;
    state.edit_fixture_config(|config| {
        config.trade_response_wait_ms = 5_000;
        config.blockhash_max_age_ms = u64::MAX;
        config.pumpapi.enabled = true;
    });
    if mode != TradePersistenceMode::Memory {
        state.persistence = Arc::new(
            TradePersistence::new(&TradePersistenceConfig {
                mode,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 100,
            })
            .await
            .unwrap(),
        );
    }
    let trace = Arc::new(Trace::default());
    state.wallet_identity = Arc::new(Directory {
        trace: trace.clone(),
        wallets: BTreeMap::from([
            ("alpha".into(), Address::from_bytes([1; 32])),
            ("beta".into(), Address::from_bytes([2; 32])),
        ]),
    });
    state.ordinary_execution = Arc::new(Backend(trace.clone()));
    state.ordinary_sol_buy = Arc::new(AggregateBackend(trace.clone()));
    state.blockhash_cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([42; 32]),
        crate::state::now_ms(),
    ));
    (state, trace)
}

fn buy(key: &str) -> Value {
    json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111",
        "amount_sol":0.001,"simulate":false,"idempotency_key":key})
}

#[tokio::test]
async fn damm_unsigned_simulation_http_status_null_signature_and_original_key_are_compatible() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        for success in [false, true] {
            let (state, trace) = with_backend(mode).await;
            *trace.preview_outcome.lock().unwrap() = Some(success);
            let app = http::router(state.clone());
            let body = json!({
                "wallet_id":"alpha", "execution_method":"sdk", "side":"sell",
                "pool":crate::domain::trade::Pool::MeteoraDammV2,
                "mint":Address::from_bytes([9;32]).to_string(),
                "pool_id":Address::from_bytes([3;32]).to_string(),
                "sell_percent":50,"simulate":true,"idempotency_key":"damm-unsigned-simulation"
            });
            let first = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(body.clone()),
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(
                first["status"],
                if success { "simulated" } else { "failed" }
            );
            assert!(first["signature"].is_null());
            assert_eq!(first["execution_method"], "sdk");
            let duplicate = json_body(
                send(&app, "POST", "/v1/trades", LOCAL, Some(TOKEN), Some(body)).await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(duplicate["request_id"], first["request_id"]);
            assert_eq!(duplicate["duplicate"], true);
            assert!(duplicate["signature"].is_null());
            let record = terminal(&app, first["request_id"].as_str().unwrap()).await;
            assert_eq!(record["status"], first["status"]);
            assert!(record["signature"].is_null());
            assert_eq!(trace.commands.lock().unwrap().len(), 1);
            assert!(trace.aggregate_commands.lock().unwrap().is_empty());
            assert_eq!(state.wallets.wallet_count(), 0);
        }
    }
}

#[tokio::test]
async fn ordinary_trade_route_group_preserves_auth_methods_and_extractor_errors() {
    let state = state().await;
    for app in [
        http::router(state.clone()),
        crate::api::routes::trade_routes().with_state(state.clone()),
    ] {
        let cases = [
            ("POST", "/v1/trades", Some(buy("route-contract"))),
            ("GET", "/v1/trades", None),
            (
                "POST",
                "/v1/batch/trades",
                Some(json!({"wallet_ids":[], "trade":buy("route-batch")})),
            ),
            ("GET", "/v1/trades/missing", None),
        ];
        for (method, path, body) in cases {
            assert_eq!(
                send(&app, method, path, LOCAL, None, body.clone())
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED,
            );
            assert_eq!(
                send(&app, method, path, DENIED, Some(TOKEN), body)
                    .await
                    .status(),
                StatusCode::FORBIDDEN,
            );
            assert_eq!(
                send(&app, "PATCH", path, LOCAL, Some(TOKEN), None)
                    .await
                    .status(),
                StatusCode::METHOD_NOT_ALLOWED,
            );
        }
        error_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(buy("missing")),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "wallet_id fixture-wallet not found",
        )
        .await;
        error_body(
            send(
                &app,
                "POST",
                "/v1/batch/trades",
                LOCAL,
                Some(TOKEN),
                Some(json!({"wallet_ids":[], "trade":buy("empty")})),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "wallet_ids is required",
        )
        .await;
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
            json!([]),
        );
        // Query/JSON extraction still precedes handler authentication.
        for token in [None, Some(TOKEN)] {
            assert_eq!(
                send(&app, "GET", "/v1/trades?limit=invalid", LOCAL, token, None)
                    .await
                    .status(),
                StatusCode::BAD_REQUEST,
            );
            assert_eq!(
                send(&app, "POST", "/v1/trades", LOCAL, token, Some(json!({})))
                    .await
                    .status(),
                StatusCode::UNPROCESSABLE_ENTITY,
            );
        }
    }
    assert!(state.trades.recent(10).is_empty());
    assert_eq!(state.wallets.wallet_count(), 0);
}

#[tokio::test]
async fn ordinary_trade_route_group_preserves_sdk_execution_replay_batch_and_reads() {
    for standalone in [false, true] {
        for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
            let (state, trace) = with_backend(mode).await;
            let app = if standalone {
                crate::api::routes::trade_routes().with_state(state.clone())
            } else {
                http::router(state.clone())
            };
            let body = buy("route-sdk");
            let first = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(body.clone()),
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(first["status"], "submitted");
            assert_eq!(first["execution_method"], "sdk");
            assert_eq!(first["wallet_id"], "alpha");
            assert_eq!(first["signature"], "synthetic-signature-alpha");
            let duplicate = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(body.clone()),
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(duplicate["request_id"], first["request_id"]);
            assert_eq!(duplicate["duplicate"], true);
            let mut conflict = body.clone();
            conflict["amount_sol"] = json!(0.002);
            error_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(conflict),
                )
                .await,
                StatusCode::CONFLICT,
                "idempotency key was already used for a different trade",
            )
            .await;
            let batch = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/batch/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(json!({"wallet_ids":["alpha", "beta", "missing"], "trade":body})),
                )
                .await,
                StatusCode::ACCEPTED,
            )
            .await;
            assert_eq!(batch["items"][0]["request_id"], first["request_id"]);
            assert_eq!(batch["items"][0]["duplicate"], true);
            assert_eq!(batch["items"][1]["status"], "submitted");
            assert_eq!(
                batch["items"][2]["error"],
                "400 Bad Request: wallet_id missing not found"
            );
            for item in [&first, &batch["items"][1]] {
                let record = terminal(&app, item["request_id"].as_str().unwrap()).await;
                assert_eq!(record["status"], "submitted");
                assert_eq!(record["wallet_id"], item["wallet_id"]);
            }
            let list = json_body(
                send(&app, "GET", "/v1/trades", LOCAL, Some(TOKEN), None).await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(list.as_array().unwrap().len(), 2);
            assert_eq!(trace.commands.lock().unwrap().len(), 2);
            assert!(trace
                .commands
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _)| *method == ExecutionMethod::Sdk));
            assert!(trace.aggregate_commands.lock().unwrap().is_empty());
            assert_eq!(state.wallets.wallet_count(), 0);
        }
    }
}

#[tokio::test]
async fn pumpapi_h2a_unsigned_preview_http_status_null_signature_and_replay_are_compatible() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
        for success in [false, true] {
            for side in ["buy", "sell"] {
                let (mut state, trace) = with_backend(mode).await;
                state.edit_fixture_config(|config| {
                    config.security.broadcast = BroadcastPolicy::Disabled
                });
                *trace.preview_outcome.lock().unwrap() = Some(success);
                let app = http::router(state.clone());
                let mut body = buy("unsigned-preview");
                body["execution_method"] = json!("pumpapi");
                body["simulate"] = json!(true);
                if side == "sell" {
                    body["side"] = json!("sell");
                    body.as_object_mut().unwrap().remove("amount_sol");
                    body["sell_percent"] = json!(100);
                }
                let first = json_body(
                    send(
                        &app,
                        "POST",
                        "/v1/trades",
                        LOCAL,
                        Some(TOKEN),
                        Some(body.clone()),
                    )
                    .await,
                    StatusCode::OK,
                )
                .await;
                let expected = if success { "simulated" } else { "failed" };
                assert_eq!(first.as_object().unwrap().len(), 8);
                assert_eq!(first["status"], expected);
                assert_eq!(first["signature"], Value::Null);
                assert_eq!(
                    first["error"],
                    if success {
                        Value::Null
                    } else {
                        json!("synthetic preview program failure")
                    }
                );
                let id = first["request_id"].as_str().unwrap();
                let terminal = terminal(&app, id).await;
                assert_eq!(terminal["status"], expected);
                assert_eq!(terminal["signature"], Value::Null);
                assert_eq!(terminal["side"], side);
                let replay = json_body(
                    send(&app, "POST", "/v1/trades", LOCAL, Some(TOKEN), Some(body)).await,
                    StatusCode::OK,
                )
                .await;
                assert_eq!(replay["request_id"], first["request_id"]);
                assert_eq!(replay["status"], expected);
                assert_eq!(replay["signature"], Value::Null);
                assert_eq!(replay["duplicate"], true);
                assert_eq!(trace.commands.lock().unwrap().len(), 1);
                assert!(trace.commands.lock().unwrap()[0]
                    .1
                    .execution
                    .mode
                    .simulate());
                assert_eq!(state.wallets.wallet_count(), 0);
                if mode == TradePersistenceMode::Durable {
                    let record = state.persistence.get(id).await.unwrap().unwrap();
                    assert_eq!(serde_json::to_value(record.status).unwrap(), expected);
                    assert!(record.signature.is_none());
                }
            }
        }
    }
}

#[tokio::test]
async fn sdk_unknown_http_preserves_wire_signature_diagnostics_and_idempotency_in_all_modes() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        for side in ["buy", "sell"] {
            for signatures in [
                vec![],
                vec!["original-a".to_string()],
                vec!["original-a".to_string(), "original-b".to_string()],
            ] {
                let (state, trace) = with_backend(mode).await;
                *trace.sdk_uncertainty.lock().unwrap() = Some(signatures.clone());
                let app = http::router(state.clone());
                let mut request = buy("sdk-uncertain");
                if side == "sell" {
                    request["side"] = json!(side);
                    request.as_object_mut().unwrap().remove("amount_sol");
                    request["sell_percent"] = json!(100);
                }
                let first = json_body(
                    send(
                        &app,
                        "POST",
                        "/v1/trades",
                        LOCAL,
                        Some(TOKEN),
                        Some(request.clone()),
                    )
                    .await,
                    StatusCode::OK,
                )
                .await;
                assert_eq!(first.as_object().unwrap().len(), 8);
                assert_eq!(first["status"], "unknown");
                assert_eq!(first["signature"], json!(signatures.first()));
                assert_eq!(first["duplicate"], false);
                assert_eq!(first["execution_method"], "sdk");
                let expected_error = if signatures.len() > 1 {
                    "synthetic SDK invocation outcome uncertain; known SDK signatures=[original-a,original-b]"
                } else {
                    "synthetic SDK invocation outcome uncertain"
                };
                assert_eq!(first["error"], expected_error);
                let id = first["request_id"].as_str().unwrap();
                let record = terminal(&app, id).await;
                assert_eq!(record["status"], "unknown");
                assert_eq!(record["signature"], first["signature"]);
                assert_eq!(record["error"], first["error"]);
                for _ in 0..2 {
                    let replay = json_body(
                        send(
                            &app,
                            "POST",
                            "/v1/trades",
                            LOCAL,
                            Some(TOKEN),
                            Some(request.clone()),
                        )
                        .await,
                        StatusCode::OK,
                    )
                    .await;
                    assert_eq!(replay["request_id"], first["request_id"]);
                    assert_eq!(replay["status"], "unknown");
                    assert_eq!(replay["signature"], first["signature"]);
                    assert_eq!(replay["error"], first["error"]);
                    assert_eq!(replay["duplicate"], true);
                }
                assert_eq!(trace.commands.lock().unwrap().len(), 1);
                assert!(trace.aggregate_commands.lock().unwrap().is_empty());
                if mode != TradePersistenceMode::Memory {
                    tokio::time::timeout(std::time::Duration::from_secs(2), async {
                        loop {
                            if let Some(saved) = state.persistence.get(id).await.unwrap() {
                                if saved.status == crate::models::TradeStatus::Unknown {
                                    assert_eq!(saved.signature, signatures.first().cloned());
                                    assert_eq!(saved.error.as_deref(), Some(expected_error));
                                    break;
                                }
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .expect("temporary persistence must retain SDK uncertainty");
                }
            }
        }
    }
}

#[tokio::test]
async fn sdk_unknown_http_file_reopen_keeps_original_receipt_without_a_backend_call() {
    let dir = std::env::temp_dir().join(format!("ordinary-sdk-unknown-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 100,
    };
    let (mut state, trace) = with_backend(TradePersistenceMode::Memory).await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let pool = state.persistence.pool_for_tests();
    *trace.sdk_uncertainty.lock().unwrap() = Some(vec!["original-a".into(), "original-b".into()]);
    let app = http::router(state);
    let request = buy("sdk-unknown-reopen");
    let first = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(first["status"], "unknown");
    assert_eq!(trace.commands.lock().unwrap().len(), 1);
    drop(app);
    pool.close().await;

    let (mut restored, trace) = with_backend(TradePersistenceMode::Memory).await;
    restored.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let pool = restored.persistence.pool_for_tests();
    let app = http::router(restored);
    let saved = terminal(&app, first["request_id"].as_str().unwrap()).await;
    assert_eq!(saved["status"], "unknown");
    assert_eq!(saved["signature"], first["signature"]);
    assert_eq!(saved["error"], first["error"]);
    let replay = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(replay["request_id"], first["request_id"]);
    assert_eq!(replay["status"], "unknown");
    assert_eq!(replay["signature"], first["signature"]);
    assert_eq!(replay["error"], first["error"]);
    assert_eq!(replay["duplicate"], true);
    assert!(trace.commands.lock().unwrap().is_empty());
    assert!(trace.resolves.lock().unwrap().is_empty());
    drop(app);
    pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

async fn terminal(app: &Router, request_id: &str) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let body = json_body(
                send(
                    app,
                    "GET",
                    &format!("/v1/trades/{request_id}"),
                    LOCAL,
                    Some(TOKEN),
                    None,
                )
                .await,
                StatusCode::OK,
            )
            .await;
            if !["accepted", "warming_params", "building_tx", "submitting"]
                .contains(&body["status"].as_str().unwrap())
            {
                break body;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("synthetic execution must finish")
}

#[tokio::test]
async fn real_trade_router_runs_both_methods_and_directions_without_sdk_wallets() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
        for method in ["sdk", "pumpapi"] {
            for side in ["buy", "sell"] {
                let (state, trace) = with_backend(mode).await;
                let app = http::router(state.clone());
                let mut body = buy("http-success");
                body["execution_method"] = json!(method);
                if side == "sell" {
                    body["side"] = json!(side);
                    body.as_object_mut().unwrap().remove("amount_sol");
                    body["sell_percent"] = json!(100);
                }
                let response = json_body(
                    send(&app, "POST", "/v1/trades", LOCAL, Some(TOKEN), Some(body)).await,
                    StatusCode::OK,
                )
                .await;
                assert_eq!(response.as_object().unwrap().len(), 8);
                assert_eq!(
                    response,
                    json!({"request_id":response["request_id"],"wallet_id":"alpha",
                    "wallet_pubkey":Address::from_bytes([1;32]).to_string(),"execution_method":method,
                    "status":"submitted","signature":"synthetic-signature-alpha","error":null,"duplicate":false})
                );
                let record = terminal(&app, response["request_id"].as_str().unwrap()).await;
                assert_eq!(record["side"], side);
                assert_eq!(record["status"], "submitted");
                assert_eq!(trace.resolves.lock().unwrap().as_slice(), &["alpha"]);
                let calls = trace.commands.lock().unwrap();
                assert_eq!(calls.len(), 1);
                assert_eq!(
                    serde_json::to_value(calls[0].1.amount.side()).unwrap(),
                    side
                );
                assert_eq!(trace.leases_at_execution.lock().unwrap().as_slice(), &[1]);
                assert_eq!(trace.leases.load(Ordering::SeqCst), 0);
                assert_eq!(state.wallets.wallet_count(), 0);
            }
        }
    }
}

#[tokio::test]
async fn explicit_and_default_wallet_identity_map_to_matching_execution_and_public_key() {
    let (state, trace) = with_backend(TradePersistenceMode::Memory).await;
    let app = http::router(state);
    for (wallet, expected, seed) in [(None, "alpha", 1), (Some("beta"), "beta", 2)] {
        let mut body = buy(expected);
        body["wallet_id"] = json!(wallet);
        let result = json_body(
            send(&app, "POST", "/v1/trades", LOCAL, Some(TOKEN), Some(body)).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(result["wallet_id"], expected);
        assert_eq!(
            result["wallet_pubkey"],
            Address::from_bytes([seed; 32]).to_string()
        );
        assert_eq!(
            result["signature"],
            format!("synthetic-signature-{expected}")
        );
    }
    assert_eq!(
        trace.ids.lock().unwrap().as_slice(),
        &[None, Some("beta".into())]
    );
    assert_eq!(
        trace.resolves.lock().unwrap().as_slice(),
        &["alpha", "beta"]
    );
}

#[tokio::test]
async fn identical_replay_and_conflicting_amount_keep_original_order_and_single_execution() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
        let (state, trace) = with_backend(mode).await;
        let app = http::router(state.clone());
        let first = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(buy("same-key")),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        let replay = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(buy("same-key")),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(replay["request_id"], first["request_id"]);
        assert_eq!(replay["duplicate"], true);
        assert_eq!(replay["signature"], first["signature"]);
        let mut conflict = buy("same-key");
        conflict["amount_sol"] = json!(0.002);
        error_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(conflict),
            )
            .await,
            StatusCode::CONFLICT,
            "idempotency key was already used for a different trade",
        )
        .await;
        assert_eq!(trace.commands.lock().unwrap().len(), 1);
        assert_eq!(trace.resolves.lock().unwrap().len(), 1);
        assert_eq!(state.trades.recent(10).len(), 1);
    }
}

#[tokio::test]
async fn batch_forces_each_wallet_and_deduplicates_repeated_wallet_ids() {
    let (state, trace) = with_backend(TradePersistenceMode::Durable).await;
    let app = http::router(state);
    let mut trade = buy("batch-key");
    trade["wallet_id"] = json!("not-used");
    let response = json_body(
        send(
            &app,
            "POST",
            "/v1/batch/trades",
            LOCAL,
            Some(TOKEN),
            Some(json!({"wallet_ids":["alpha","beta","alpha","missing"],"trade":trade})),
        )
        .await,
        StatusCode::ACCEPTED,
    )
    .await;
    let items = response["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["wallet_id"], "alpha");
    assert_eq!(items[1]["wallet_id"], "beta");
    assert_eq!(items[0]["status"], "submitted");
    assert_eq!(items[1]["status"], "submitted");
    assert_eq!(items[2]["duplicate"], true);
    assert_eq!(items[2]["request_id"], items[0]["request_id"]);
    assert_eq!(items[3]["request_id"], Value::Null);
    assert_eq!(
        items[3]["error"],
        "400 Bad Request: wallet_id missing not found"
    );
    assert!(trace.ids.lock().unwrap().is_empty());
    assert_eq!(
        trace.identities.lock().unwrap().as_slice(),
        &["alpha", "beta", "alpha", "missing"]
    );
    assert_eq!(
        trace.resolves.lock().unwrap().as_slice(),
        &["alpha", "beta"]
    );
    assert_eq!(trace.commands.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn missing_identity_precedes_validation_and_rejected_parameters_do_not_execute() {
    let (state, trace) = with_backend(TradePersistenceMode::Memory).await;
    let app = http::router(state.clone());
    let mut invalid = buy("invalid");
    invalid["mint"] = json!("invalid");
    invalid["wallet_id"] = json!("missing");
    error_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(invalid.clone()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "wallet_id missing not found",
    )
    .await;
    assert!(state.trades.recent(10).is_empty());
    invalid["wallet_id"] = Value::Null;
    let response = json_body(
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
    assert_eq!(response["status"], "rejected");
    assert_eq!(response["wallet_id"], "alpha");
    assert_eq!(response["error"], "mint must be a valid pubkey");
    assert_eq!(state.trades.recent(10).len(), 1);
    assert!(trace.commands.lock().unwrap().is_empty());
    assert!(trace.resolves.lock().unwrap().is_empty());
}

#[tokio::test]
async fn authorization_and_json_extraction_never_touch_identity_or_execution_sources() {
    let (state, trace) = with_backend(TradePersistenceMode::Memory).await;
    let app = http::router(state);
    error_body(
        send(&app, "POST", "/v1/trades", LOCAL, None, Some(buy("auth"))).await,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/trades",
            DENIED,
            Some(TOKEN),
            Some(buy("ip"))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, "POST", "/v1/trades", LOCAL, None, Some(json!({})))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(trace.ids.lock().unwrap().is_empty());
    assert!(trace.identities.lock().unwrap().is_empty());
    assert!(trace.resolves.lock().unwrap().is_empty());
    assert!(trace.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rejection_baseline_preserves_rows_and_never_reserves_the_valid_trade_key() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let (state, trace) = with_backend(mode).await;
        let app = http::router(state.clone());
        let mut invalid = buy("rejection-baseline");
        invalid["mint"] = json!("invalid");
        invalid["source"] = json!("literal source");
        invalid["source_signature"] = json!("metadata not parsed");
        let mut ids = vec![];
        for _ in 0..2 {
            let body = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(invalid.clone()),
                )
                .await,
                StatusCode::BAD_REQUEST,
            )
            .await;
            assert_eq!(
                body,
                json!({"request_id":body["request_id"],"wallet_id":"alpha",
                "wallet_pubkey":Address::from_bytes([1;32]).to_string(),"execution_method":"sdk",
                "status":"rejected","signature":null,"error":"mint must be a valid pubkey","duplicate":false})
            );
            let id = body["request_id"].as_str().unwrap().to_owned();
            let record = state.trades.get(&id).unwrap();
            assert_eq!(record.status, crate::domain::trade::TradeStatus::Rejected);
            assert!(record.updated_at_ms >= record.created_at_ms);
            assert_eq!(record.mint, "invalid");
            assert!(record.signature.is_none());
            assert!(!record.duplicate);
            ids.push(id.clone());
            if mode != TradePersistenceMode::Memory {
                let pool = state.persistence.pool_for_tests();
                let row = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let row: Option<(String, String, String)> = sqlx::query_as("SELECT idempotency_key, request_hash, request_json FROM trades WHERE request_id=?")
                            .bind(&id).fetch_optional(&pool).await.unwrap();
                        if let Some(row) = row { break row; }
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
                let mut resolved = invalid.clone();
                resolved["wallet_id"] = json!("alpha");
                let request: crate::models::TradeRequest =
                    serde_json::from_value(resolved).unwrap();
                let (key, snapshot) =
                    crate::adapters::persistence::trade_snapshot::rejected("alpha", &id, &request)
                        .unwrap()
                        .into_storage();
                assert_eq!(row.0, key);
                assert_eq!(row.0, format!("__rejected__:{id}"));
                assert_eq!(row.1, snapshot.hash());
                assert_eq!(row.2, snapshot.json());
                assert_eq!(row.2, serde_json::to_string(&request).unwrap());
            }
        }
        assert_ne!(ids[0], ids[1]);
        assert_eq!(state.trades.recent(10).len(), 2);
        assert!(trace.resolves.lock().unwrap().is_empty());
        assert!(trace.commands.lock().unwrap().is_empty());
        let result = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(buy("rejection-baseline")),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(result["status"], "submitted");
        assert_eq!(result["duplicate"], false);
        let duplicate = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(buy("rejection-baseline")),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(duplicate["request_id"], result["request_id"]);
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(trace.commands.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn rejected_trade_persistence_failure_preserves_400_and_in_memory_evidence() {
    for durable in [false, true] {
        let (mut state, trace) = with_backend(if durable {
            TradePersistenceMode::Durable
        } else {
            TradePersistenceMode::Memory
        })
        .await;
        if durable {
            state.persistence.pool_for_tests().close().await;
        } else {
            state.persistence = Arc::new(TradePersistence::unavailable_shadow_for_tests());
        }
        let app = http::router(state.clone());
        let mut invalid = buy("rejection-storage-failure");
        invalid["mint"] = json!("invalid");
        let result = json_body(
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
        assert_eq!(result["status"], "rejected");
        assert_eq!(result["error"], "mint must be a valid pubkey");
        assert_eq!(result["signature"], Value::Null);
        let record = state
            .trades
            .get(result["request_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(record.error.as_deref(), Some("mint must be a valid pubkey"));
        assert!(trace.commands.lock().unwrap().is_empty());
        assert!(trace.resolves.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn real_ordinary_sol_buy_router_preserves_sol_units_and_rejects_cross_quote_key_reuse() {
    use crate::domain::trade::{TradeAmount, USDC_MINT};
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
        for method in ["sdk", "pumpapi"] {
            for simulate in [false, true] {
                let (state, trace) = with_backend(mode).await;
                let app = http::router(state.clone());
                let mut body = buy("ordinary-sol-buy");
                body["pool"] = json!("pump-amm");
                body["quote_mint"] = json!(USDC_MINT);
                body["execution_method"] = json!(method);
                body["simulate"] = json!(simulate);
                let result = json_body(
                    send(
                        &app,
                        "POST",
                        "/v1/trades",
                        LOCAL,
                        Some(TOKEN),
                        Some(body.clone()),
                    )
                    .await,
                    StatusCode::OK,
                )
                .await;
                assert_eq!(
                    result["status"],
                    if simulate { "simulated" } else { "submitted" }
                );
                assert_eq!(result["wallet_id"], "alpha");
                assert_eq!(result["signature"], "synthetic-aggregate-alpha");
                assert_eq!(result["duplicate"], false);
                let duplicate = json_body(
                    send(
                        &app,
                        "POST",
                        "/v1/trades",
                        LOCAL,
                        Some(TOKEN),
                        Some(body.clone()),
                    )
                    .await,
                    StatusCode::OK,
                )
                .await;
                assert_eq!(duplicate["request_id"], result["request_id"]);
                assert_eq!(duplicate["duplicate"], true);
                body.as_object_mut().unwrap().remove("amount_sol");
                body["amount_usdc"] = json!(1.0);
                error_body(
                    send(&app, "POST", "/v1/trades", LOCAL, Some(TOKEN), Some(body)).await,
                    StatusCode::CONFLICT,
                    "idempotency key was already used for a different trade",
                )
                .await;
                assert!(trace.resolves.lock().unwrap().is_empty());
                assert!(trace.commands.lock().unwrap().is_empty());
                assert_eq!(&*trace.aggregate_resolves.lock().unwrap(), &["alpha"]);
                let commands = trace.aggregate_commands.lock().unwrap();
                assert_eq!(commands.len(), 1);
                assert!(commands[0].aggregator_sol);
                assert_eq!(
                    commands[0].amount,
                    TradeAmount::BuySol {
                        lamports: 1_000_000
                    }
                );
                assert_eq!(commands[0].route.quote_mint.to_string(), USDC_MINT);
                assert_eq!(commands[0].execution.mode.simulate(), simulate);
                assert_eq!(state.wallets.wallet_count(), 0);
            }
        }
    }
}

#[tokio::test]
async fn disabled_policy_rejects_valid_identity_before_order_reservation_or_backend() {
    let (mut state, trace) = with_backend(TradePersistenceMode::Memory).await;
    state.edit_fixture_config(|config| config.security.broadcast = BroadcastPolicy::Disabled);
    let app = http::router(state.clone());
    error_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(buy("disabled")),
        )
        .await,
        StatusCode::FORBIDDEN,
        crate::domain::execution_policy::BROADCAST_DISABLED_ERROR,
    )
    .await;
    assert_eq!(trace.identities.lock().unwrap().len(), 1);
    assert_eq!(trace.leases.load(Ordering::SeqCst), 0);
    assert!(state.trades.recent(10).is_empty());
    assert!(trace.resolves.lock().unwrap().is_empty());
}

#[tokio::test]
async fn durable_reservation_failure_returns_503_and_never_starts_executor() {
    let (state, trace) = with_backend(TradePersistenceMode::Durable).await;
    state.persistence.pool_for_tests().close().await;
    let app = http::router(state.clone());
    error_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(buy("closed")),
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE,
        "trade persistence unavailable; trade was not started",
    )
    .await;
    assert!(state.trades.recent(10).is_empty());
    assert!(trace.resolves.lock().unwrap().is_empty());
    assert!(trace.commands.lock().unwrap().is_empty());
    assert_eq!(trace.leases.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn admitted_identity_does_not_replace_execution_time_wallet_resolution() {
    let (state, trace) = with_backend(TradePersistenceMode::Memory).await;
    trace.execution_missing.store(true, Ordering::SeqCst);
    let app = http::router(state);
    let body = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(buy("removed")),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["error"], "wallet_id alpha not found");
    assert_eq!(trace.identities.lock().unwrap().as_slice(), &["alpha"]);
    assert_eq!(trace.resolves.lock().unwrap().as_slice(), &["alpha"]);
    assert!(trace.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pending_submission_returns_202_and_original_order_can_be_queried_after_completion() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        for wait_ms in [0, 1] {
            let (mut state, trace) = with_backend(mode).await;
            state.edit_fixture_config(|config| config.trade_response_wait_ms = wait_ms);
            let gate = Arc::new(Notify::new());
            *trace.gate.lock().unwrap() = Some(gate.clone());
            let app = http::router(state);
            let response = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(buy("pending")),
                )
                .await,
                StatusCode::ACCEPTED,
            )
            .await;
            assert_eq!(response["signature"], Value::Null);
            tokio::time::timeout(std::time::Duration::from_secs(5), trace.started.notified())
                .await
                .unwrap();
            let replay = json_body(
                send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(buy("pending")),
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(replay["request_id"], response["request_id"]);
            assert_eq!(replay["duplicate"], true);
            assert_eq!(replay["signature"], Value::Null);
            assert_eq!(trace.resolves.lock().unwrap().len(), 1);
            gate.notify_one();
            let record = terminal(&app, response["request_id"].as_str().unwrap()).await;
            assert_eq!(record["status"], "submitted");
            assert_eq!(record["signature"], "synthetic-signature-alpha");
            assert_eq!(trace.commands.lock().unwrap().len(), 1);
            assert_eq!(trace.leases.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn concurrent_http_replays_share_one_reservation_and_one_backend_call() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
        let (state, trace) = with_backend(mode).await;
        let app = http::router(state);
        let mut handles = Vec::new();
        for _ in 0..2 {
            let app = app.clone();
            handles.push(tokio::spawn(async move {
                let response = send(
                    &app,
                    "POST",
                    "/v1/trades",
                    LOCAL,
                    Some(TOKEN),
                    Some(buy("concurrent")),
                )
                .await;
                let status = response.status();
                assert!(matches!(status, StatusCode::OK | StatusCode::ACCEPTED));
                json_body(response, status).await
            }));
        }
        let first = handles.remove(0).await.unwrap();
        let second = handles.remove(0).await.unwrap();
        assert_eq!(first["request_id"], second["request_id"]);
        assert_eq!(
            usize::from(first["duplicate"].as_bool().unwrap())
                + usize::from(second["duplicate"].as_bool().unwrap()),
            1
        );
        let record = terminal(&app, first["request_id"].as_str().unwrap()).await;
        assert_eq!(record["signature"], "synthetic-signature-alpha");
        assert_eq!(trace.commands.lock().unwrap().len(), 1);
        assert_eq!(trace.resolves.lock().unwrap().len(), 1);
        assert_eq!(trace.leases.load(Ordering::SeqCst), 0);
    }
}
