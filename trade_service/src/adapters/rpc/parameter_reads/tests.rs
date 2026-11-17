use super::*;
use crate::adapters::rpc::read_http::{HttpReply, ReadRequest};
use base64::Engine;
use reqwest::{StatusCode, Url};
use serde_json::json;
use solana_sdk::pubkey::Pubkey;
use std::collections::VecDeque;
use tokio::sync::Notify;

mod discovery;

#[derive(Clone, Copy)]
enum Mode {
    Good,
    Rate,
    Hold,
    WrongId,
    BadAccount,
    RpcError,
    Oversize,
    Missing,
    Slow,
}
struct Fake {
    modes: Mutex<VecDeque<Mode>>,
    calls: Mutex<Vec<(String, Value, Instant)>>,
    entered: Notify,
    dropped: AtomicUsize,
}
struct Dropped<'a>(&'a AtomicUsize);
impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn account() -> Value {
    json!({"lamports":1000,"owner":Pubkey::new_from_array([8;32]).to_string(),"data":[base64::engine::general_purpose::STANDARD.encode([1,2,3]),"base64"],"space":3,"executable":false,"rentEpoch":0})
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let body = request.body();
        let id = body["id"].as_u64().unwrap();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone(), Instant::now()));
        if matches!(request, ReadRequest::LatestBlockhash { .. }) {
            return Ok(HttpReply { status: StatusCode::OK, retry_after: None, body: serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":42},"value":{"blockhash":bs58::encode([7u8;32]).into_string(),"lastValidBlockHeight":50}}})).unwrap() });
        }
        let mode = self
            .modes
            .lock()
            .unwrap()
            .pop_front()
            .expect("no unbudgeted parameter request");
        let _dropped = Dropped(&self.dropped);
        self.entered.notify_one();
        let mut result = match body["method"].as_str().unwrap() {
            "getAccountInfo" => json!({"context":{"slot":42},"value":account()}),
            "getMultipleAccounts" => {
                json!({"context":{"slot":42},"value":body["params"][0].as_array().unwrap().iter().map(|_|account()).collect::<Vec<_>>()})
            }
            "getEpochInfo" => {
                json!({"epoch":17,"slotIndex":1,"slotsInEpoch":432000,"absoluteSlot":7344001,"blockHeight":42,"transactionCount":100})
            }
            "getProgramAccounts" => json!([]),
            _ => panic!("non-parameter capability reached transport"),
        };
        if matches!(mode, Mode::Hold) {
            std::future::pending::<()>().await;
        }
        if matches!(mode, Mode::Slow) {
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        if matches!(mode, Mode::Rate) {
            return Ok(HttpReply {
                status: StatusCode::TOO_MANY_REQUESTS,
                retry_after: Some(60),
                body: vec![],
            });
        }
        if matches!(mode, Mode::Oversize) {
            return Ok(HttpReply {
                status: StatusCode::OK,
                retry_after: None,
                body: vec![b' '; request.max_body() + 1],
            });
        }
        if matches!(mode, Mode::BadAccount) {
            result["value"]["data"] = json!(["bad", "base64+zstd"]);
        }
        if matches!(mode, Mode::Missing) {
            result["value"] = Value::Null;
        }
        let envelope = if matches!(mode, Mode::RpcError) {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32005,"message":"https://fixture.invalid/?key=fixture-secret"}})
        } else {
            json!({"jsonrpc":"2.0","id":id+u64::from(matches!(mode,Mode::WrongId)),"result":result})
        };
        Ok(HttpReply {
            status: StatusCode::OK,
            retry_after: None,
            body: serde_json::to_vec(&envelope).unwrap(),
        })
    }
}
fn urls() -> Vec<String> {
    vec![
        "https://first.invalid/?key=fixture-secret".into(),
        "https://second.invalid".into(),
    ]
}
fn runtime(modes: Vec<Mode>) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &urls(),
        Fake {
            modes: Mutex::new(modes.into()),
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            dropped: AtomicUsize::new(0),
        },
    ))
}
fn session(r: &Arc<SharedReadRuntime<Fake>>) -> ParameterReadSession<Fake> {
    ParameterReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        ParameterReadScope::new(
            Pool::MeteoraDammV2,
            Address::from_bytes([1; 32]),
            Some(Address::from_bytes([2; 32])),
        ),
    )
}
fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

#[tokio::test]
async fn parameter_scope_real_sdk_methods_preserve_bytes_commitment_order_and_pacing() {
    let r = runtime(vec![Mode::Good; 3]);
    let session = session(&r);
    assert_eq!(
        session.rpc().get_account(&key(1)).await.unwrap().data,
        [1, 2, 3]
    );
    assert_eq!(
        session
            .rpc()
            .get_multiple_accounts(&[key(2), key(3)])
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(session.rpc().get_epoch_info().await.unwrap().epoch, 17);
    session.finish(Ok(())).unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[0].1["params"][1],
        json!({"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":null})
    );
    assert_eq!(
        calls[1].1["params"][0],
        json!([key(2).to_string(), key(3).to_string()])
    );
    assert_eq!(calls[2].1["params"], json!([{"commitment":"processed"}]));
    for pair in calls.windows(2) {
        assert!(pair[1].2.duration_since(pair[0].2) >= Duration::from_millis(240));
    }
    drop(calls);
    assert!(r.parameter_slot().is_ok());
}

#[tokio::test]
async fn parameter_scope_parallel_sdk_reads_are_serialized_and_other_sessions_are_busy() {
    let r = runtime(vec![Mode::Good; 4]);
    let a = session(&r);
    let b = session(&r);
    let first = key(1);
    let second = key(2);
    let (one, two) = tokio::join!(a.rpc().get_account(&first), a.rpc().get_account(&second));
    assert!(one.is_ok() && two.is_ok());
    assert!(b.rpc().get_account(&key(3)).await.is_err());
    assert!(b.finish(Ok(())).is_err());
    a.finish(Ok(())).unwrap();
    session(&r).rpc().get_account(&key(4)).await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(240));
}

#[tokio::test]
async fn parameter_scope_denies_money_simulation_and_other_queries_before_http() {
    for method in [
        RpcRequest::SendTransaction,
        RpcRequest::SimulateTransaction,
        RpcRequest::GetBalance,
        RpcRequest::GetLatestBlockhash,
        RpcRequest::GetTokenAccountsByOwner,
    ] {
        let r = runtime(vec![]);
        let s = session(&r);
        assert!(s.rpc().send::<Value>(method, json!([])).await.is_err());
        let error = s.finish(Ok(())).err().unwrap();
        assert!(!error
            .downcast_ref::<ParameterReadFailure>()
            .unwrap()
            .allows_lane_fallback());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn parameter_scope_swallowed_errors_remain_sticky_and_cannot_open_another_request() {
    for (mode, unavailable) in [
        (Mode::Rate, true),
        (Mode::RpcError, true),
        (Mode::WrongId, false),
        (Mode::BadAccount, false),
        (Mode::Oversize, false),
    ] {
        let r = runtime(vec![mode]);
        let s = session(&r);
        let _ignored = s.rpc().get_account(&key(1)).await;
        assert!(s.rpc().get_account(&key(2)).await.is_err());
        let error = s.finish(Ok("SDK optional default")).err().unwrap();
        assert_eq!(
            error
                .downcast_ref::<ParameterReadFailure>()
                .unwrap()
                .allows_lane_fallback(),
            unavailable
        );
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn parameter_scope_cancelled_sdk_read_is_not_an_optional_missing_configuration() {
    let r = runtime(vec![Mode::Hold]);
    let s = session(&r);
    let sender = Arc::new(Sender {
        runtime: r.clone(),
        url: urls()[0].clone(),
        state: s.state.clone(),
    });
    let child = sender.clone();
    let task = tokio::spawn(async move {
        child
            .send(
                RpcRequest::GetAccountInfo,
                json!([key(1).to_string(),{"encoding":"base64","commitment":"processed"}]),
            )
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        r.test_transport().entered.notified(),
    )
    .await
    .unwrap();
    r.latest_blockhash().await.unwrap();
    assert!(r.parameter_slot().is_err());
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(s.finish(Ok("ignored cancellation")).is_err());
    assert_eq!(r.test_transport().dropped.load(Ordering::SeqCst), 1);
    assert!(r.parameter_slot().is_ok());
}

#[tokio::test]
async fn parameter_scope_drop_closes_retained_readers_and_releases_after_request_cancellation() {
    let r = runtime(vec![Mode::Hold]);
    let s = session(&r);
    let sender = Arc::new(Sender {
        runtime: r.clone(),
        url: urls()[0].clone(),
        state: s.state.clone(),
    });
    let child = sender.clone();
    let task = tokio::spawn(async move {
        child
            .send(
                RpcRequest::GetAccountInfo,
                json!([key(1).to_string(),{"encoding":"base64","commitment":"processed"}]),
            )
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        r.test_transport().entered.notified(),
    )
    .await
    .unwrap();
    drop(s);
    assert!(tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(sender
        .send(
            RpcRequest::GetAccountInfo,
            json!([key(2).to_string(),{"encoding":"base64"}])
        )
        .await
        .is_err());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert!(r.parameter_slot().is_ok());
}

#[tokio::test]
async fn parameter_scope_429_cools_other_profiles_without_switching_the_current_read() {
    let r = runtime(vec![Mode::Rate]);
    let s = session(&r);
    assert!(s.rpc().get_account(&key(1)).await.is_err());
    assert!(s.finish(Ok(())).is_err());
    assert!(r
        .preparation_account_exists(
            &urls()[0],
            Address::from_bytes([1; 32]),
            Address::from_bytes([2; 32])
        )
        .await
        .is_err());
    r.latest_blockhash().await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, Url::parse(&urls()[1]).unwrap().to_string());
}

#[tokio::test]
async fn parameter_scope_explicit_missing_account_is_distinct_from_an_unavailable_rpc() {
    let r = runtime(vec![Mode::Missing]);
    let s = session(&r);
    assert!(s.rpc().get_account(&key(1)).await.is_err());
    assert_eq!(
        s.finish(Ok("optional account absent")).unwrap(),
        "optional account absent"
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn parameter_scope_request_count_is_bounded_before_queued_sdk_reads_can_fan_out() {
    let r = runtime(vec![Mode::Hold]);
    let s = session(&r);
    let sender = Arc::new(Sender {
        runtime: r.clone(),
        url: urls()[0].clone(),
        state: s.state.clone(),
    });
    let mut tasks = tokio::task::JoinSet::new();
    for n in 0..32 {
        let sender = sender.clone();
        tasks.spawn(async move {
            sender
                .send(
                    RpcRequest::GetAccountInfo,
                    json!([key(n).to_string(),{"encoding":"base64","commitment":"processed"}]),
                )
                .await
        });
    }
    tokio::time::timeout(
        Duration::from_secs(1),
        r.test_transport().entered.notified(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while s.state.calls.load(Ordering::Relaxed) < 32 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(sender
        .send(
            RpcRequest::GetAccountInfo,
            json!([key(99).to_string(),{"encoding":"base64"}])
        )
        .await
        .is_err());
    let error = s.finish(Ok(())).err().unwrap();
    assert!(!error
        .downcast_ref::<ParameterReadFailure>()
        .unwrap()
        .allows_lane_fallback());
    while let Some(result) = tasks.join_next().await {
        assert!(result.unwrap().is_err());
    }
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert!(r.parameter_slot().is_ok());
}

#[tokio::test]
async fn parameter_scope_timeout_and_unknown_endpoint_never_become_a_known_missing_account() {
    let r = runtime(vec![Mode::Hold]);
    let s = session(&r);
    let started = Instant::now();
    assert!(s.rpc().get_account(&key(1)).await.is_err());
    let error = s.finish(Ok(())).err().unwrap();
    assert!(error
        .downcast_ref::<ParameterReadFailure>()
        .unwrap()
        .allows_lane_fallback());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert!(r.parameter_slot().is_ok());
    let r = runtime(vec![]);
    let s = ParameterReadSession::new(
        r.clone(),
        "https://unknown.invalid/?key=fixture-secret".into(),
        CommitmentConfig::processed(),
        ParameterReadScope::new(Pool::Pump, Address::from_bytes([1; 32]), None),
    );
    assert!(s.rpc().get_account(&key(1)).await.is_err());
    assert!(!format!("{:?}", s.finish(Ok(())).err().unwrap()).contains("fixture-secret"));
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}

fn scan_query(mint: Address, size: u64, offset: usize) -> Value {
    use sol_trade_sdk::instruction::utils::{
        pumpswap::accounts::AMM_PROGRAM, pumpswap_types::POOL_DISCRIMINATOR,
    };
    use solana_client::rpc_filter::{Memcmp, RpcFilterType};
    json!([AMM_PROGRAM.to_string(),{"encoding":"base64","dataSlice":null,"commitment":"processed","minContextSlot":null,"withContext":null,"sortResults":null,"filters":[
        RpcFilterType::DataSize(size), RpcFilterType::Memcmp(Memcmp::new_base58_encoded(0,&POOL_DISCRIMINATOR)),RpcFilterType::Memcmp(Memcmp::new_base58_encoded(offset,&mint.to_bytes()))
    ]}])
}

#[test]
fn parameter_query_pump_scan_requires_the_original_scope_program_discriminator_mint_and_size() {
    let mint = Address::from_bytes([1; 32]);
    let scope = ParameterReadScope::new(Pool::PumpAmm, mint, None);
    for size in [252, 261, 270, 271, 300, 643] {
        for offset in [43, 75] {
            let query = request::ParameterQuery::parse(
                RpcRequest::GetProgramAccounts,
                scan_query(mint, size, offset),
                scope,
            )
            .unwrap();
            assert_eq!(query.max_body(), 1_048_576);
            query.validate_result(&json!([])).unwrap();
        }
    }
    let valid = scan_query(mint, 643, 43);
    for damage in 0..8 {
        let mut params = valid.clone();
        match damage {
            0 => params[0] = json!(key(99).to_string()),
            1 => params[1]["filters"][0] = json!({"dataSize":100000}),
            2 => params[1]["filters"][2]["memcmp"]["bytes"] = json!(key(99).to_string()),
            3 => params[1]["filters"]
                .as_array_mut()
                .unwrap()
                .pop()
                .map(|_| ())
                .unwrap(),
            4 => params[1]["withContext"] = json!(true),
            5 => params[1]["dataSlice"] = json!({"offset":0,"length":1}),
            6 => params[1]["extra"] = json!(true),
            _ => params[1]["filters"][1]["memcmp"]["bytes"] = json!(key(99).to_string()),
        }
        assert!(
            request::ParameterQuery::parse(RpcRequest::GetProgramAccounts, params, scope).is_err()
        );
    }
    for scope in [
        ParameterReadScope::new(Pool::PumpAmm, mint, Some(mint)),
        ParameterReadScope::new(Pool::MeteoraDammV2, mint, None),
    ] {
        assert!(request::ParameterQuery::parse(
            RpcRequest::GetProgramAccounts,
            valid.clone(),
            scope
        )
        .is_err());
    }
}

#[test]
fn parameter_query_binary_limits_and_sdk_result_shape_are_checked_before_decoding() {
    let scope = ParameterReadScope::new(Pool::MeteoraDammV2, Address::from_bytes([1; 32]), None);
    let query = request::ParameterQuery::parse(
        RpcRequest::GetAccountInfo,
        json!([key(1).to_string(),{"encoding":"base64+zstd","commitment":"processed"}]),
        scope,
    )
    .unwrap();
    assert_eq!(query.body(1)["params"][1]["encoding"], "base64");
    let valid = json!({"context":{"slot":42},"value":account()});
    query.validate_result(&valid).unwrap();
    query
        .validate_result(&json!({"context":{"slot":42},"value":null}))
        .unwrap();
    for damage in 0..7 {
        let mut value = valid.clone();
        match damage {
            0 => value["context"]["slot"] = json!(0),
            1 => {
                value.as_object_mut().unwrap().remove("value");
            }
            2 => value["value"]["owner"] = json!("not-a-pubkey"),
            3 => {
                value["value"]["data"] = json!(["compressed-data-is-never-decoded", "base64+zstd"])
            }
            4 => value["value"]["data"] = json!(["invalid$$$", "base64"]),
            5 => value["value"]["space"] = json!(4),
            _ => {
                value["value"]["data"] = json!([
                    base64::engine::general_purpose::STANDARD.encode(vec![0u8; 65537]),
                    "base64"
                ]);
                value["value"]["space"] = json!(65537);
            }
        }
        assert!(query.validate_result(&value).is_err());
    }
    let batch = request::ParameterQuery::parse(
        RpcRequest::GetMultipleAccounts,
        json!([[key(1).to_string(),key(2).to_string()],{"encoding":"base64"}]),
        scope,
    )
    .unwrap();
    assert!(batch
        .validate_result(&json!({"context":{"slot":42},"value":[account()]}))
        .is_err());
    for config in [
        json!({"encoding":"jsonParsed"}),
        json!({"encoding":"base64","dataSlice":{"offset":0,"length":1}}),
        json!({"encoding":"base64","minContextSlot":1}),
        json!({"encoding":"base64","extra":true}),
    ] {
        assert!(request::ParameterQuery::parse(
            RpcRequest::GetAccountInfo,
            json!([key(1).to_string(), config]),
            scope
        )
        .is_err());
    }

    let scope = ParameterReadScope::new(
        Pool::PumpAmm,
        Address::from_bytes([4; 32]),
        Some(Address::from_bytes([1; 32])),
    );
    let config = json!({"encoding":"base64","commitment":"processed"});
    let batch = request::ParameterQuery::parse(
        RpcRequest::GetMultipleAccounts,
        json!([
            [
                key(1).to_string(),
                key(2).to_string(),
                key(3).to_string(),
                key(4).to_string()
            ],
            config
        ]),
        scope,
    )
    .unwrap();
    assert!(batch
        .with_pump_configs(ParameterReadScope::new(
            Pool::MeteoraDammV2,
            scope.mint,
            scope.pool_id
        ))
        .is_none());
    assert!(batch
        .with_pump_configs(ParameterReadScope::new(
            Pool::PumpAmm,
            scope.mint,
            Some(Address::from_bytes([99; 32]))
        ))
        .is_none());
    let expanded = batch.with_pump_configs(scope).unwrap();
    let keys = expanded.body(1)["params"][0].as_array().unwrap().clone();
    assert_eq!(keys.len(), 6);
    let value = json!({"context":{"slot":42},"value":[account(),account(),account(),account(),account(),null]});
    expanded.validate_result(&value).unwrap();
    let (original, snapshot) = expanded.split_pump_configs(value);
    batch.validate_result(&original).unwrap();
    for (index, value) in [(4, account()), (5, Value::Null)] {
        let query = request::ParameterQuery::parse(
            RpcRequest::GetAccountInfo,
            json!([keys[index], config]),
            scope,
        )
        .unwrap();
        assert_eq!(
            query.pump_config_reply(&snapshot),
            Some(json!({"context":{"slot":42},"value":value}))
        );
        let other_commitment = request::ParameterQuery::parse(
            RpcRequest::GetAccountInfo,
            json!([keys[index],{"encoding":"base64","commitment":"finalized"}]),
            scope,
        )
        .unwrap();
        assert!(other_commitment.pump_config_reply(&snapshot).is_none());
    }
    assert!(query.pump_config_reply(&snapshot).is_none());
}
