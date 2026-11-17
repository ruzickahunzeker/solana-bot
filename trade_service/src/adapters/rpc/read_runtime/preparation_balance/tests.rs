use super::*;
use crate::{
    adapters::{
        rpc::preparation_balance::RpcPreparationBalanceRead,
        sdk::preparation_assets::tests::raw_accounts,
    },
    application::ports::preparation_balance::PreparationBalanceRead,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

mod presence;

pub(crate) fn a(n: u8) -> Address {
    Address::from_bytes([n; 32])
}
pub(crate) fn scan(token2022: bool, units: u64) -> Value {
    let raw = preparation_assets::input_keys(a(1), a(9));
    let program = if token2022 {
        spl_token_2022_interface::ID
    } else {
        spl_token_interface::ID
    };
    json!([{"pubkey":raw[if token2022{2}else{1}].to_string(),"account":{"owner":program.to_string(),"lamports":1,"rentEpoch":0,"executable":false,"space":165,
        "data":{"program":if token2022{"spl-token-2022"}else{"spl-token"},"space":165,"parsed":{"type":"account","info":{"owner":a(1).to_string(),"mint":a(9).to_string(),"state":"initialized","tokenAmount":{"amount":units.to_string(),"decimals":6,"uiAmount":null,"uiAmountString":units.to_string()}}}}}}])
}
enum Answer {
    Value(u64, Value),
    Unavailable,
    Limit,
    Hold,
    Rate,
    Delay(Duration, Value),
}
struct Fake {
    answers: Mutex<VecDeque<Answer>>,
    calls: Mutex<Vec<(String, Value, Instant)>>,
    entered: Notify,
    dropped: AtomicUsize,
}
impl Fake {
    fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            dropped: AtomicUsize::new(0),
        }
    }
}
struct DropRead<'a>(&'a AtomicUsize);
impl Drop for DropRead<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let body = request.body();
        let id = body["id"].as_u64().unwrap();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body, Instant::now()));
        if matches!(request, ReadRequest::LatestBlockhash { .. }) {
            return Ok(HttpReply{status:StatusCode::OK,retry_after:None,body:serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":42},"value":{"blockhash":bs58::encode([7u8;32]).into_string(),"lastValidBlockHeight":45}}})).unwrap()});
        }
        assert!(matches!(
            request,
            ReadRequest::TokenAccounts { .. } | ReadRequest::PreparationAccounts { .. }
        ));
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("no unbudgeted/extra preparation read");
        let _drop = DropRead(&self.dropped);
        self.entered.notify_one();
        let (slot, value) = match answer {
            Answer::Value(slot, value) => (slot, value),
            Answer::Delay(delay, value) => {
                tokio::time::sleep(delay).await;
                (42, value)
            }
            Answer::Hold => std::future::pending::<(u64, Value)>().await,
            Answer::Unavailable => anyhow::bail!("https://fixture.invalid/?key=fixture-secret"),
            Answer::Rate => {
                return Ok(HttpReply {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    retry_after: Some(60),
                    body: vec![],
                })
            }
            Answer::Limit => {
                return Ok(HttpReply {
                    status: StatusCode::OK,
                    retry_after: None,
                    body: vec![b' '; request.max_body() + 1],
                })
            }
        };
        Ok(HttpReply {
            status: StatusCode::OK,
            retry_after: None,
            body: serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":slot},"value":value}}),
            )
            .unwrap(),
        })
    }
}
fn urls() -> Vec<String> {
    vec![
        "https://one.invalid/?key=fixture-secret".into(),
        "https://two.invalid".into(),
    ]
}
fn runtime(answers: Vec<Answer>) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_transport(
        &urls(),
        Fake::new(answers),
    ))
}
fn reader(r: &Arc<SharedReadRuntime<Fake>>) -> RpcPreparationBalanceRead<Fake> {
    RpcPreparationBalanceRead::new(a(1), urls()[0].clone(), r.clone())
}
fn fatal(error: &anyhow::Error) -> bool {
    !retryable(error)
}

#[tokio::test]
async fn preparation_shared_read_real_port_preserves_units_identity_exact_config_and_pacing() {
    for token2022 in [false, true] {
        let r = runtime(vec![
            Answer::Value(42, scan(token2022, 9007199254740993)),
            Answer::Value(43, raw_accounts(token2022, 9007199254740993)),
        ]);
        let snapshot = reader(&r).snapshot(a(9)).await.unwrap();
        assert_eq!(
            snapshot.checked_units(a(1), a(9)).unwrap(),
            9007199254740993
        );
        let input = snapshot.checked_input(a(1), a(9)).unwrap();
        assert_eq!(
            input.account.to_bytes(),
            preparation_assets::input_keys(a(1), a(9))[if token2022 { 2 } else { 1 }].to_bytes()
        );
        let calls = r.transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].1,
            json!({"jsonrpc":"2.0","id":1,"method":"getTokenAccountsByOwner","params":[a(1).to_string(),{"mint":a(9).to_string()},{"encoding":"jsonParsed","dataSlice":null,"commitment":"processed","minContextSlot":null}]})
        );
        let config = solana_client::rpc_config::RpcAccountInfoConfig {
            encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
            commitment: Some(solana_commitment_config::CommitmentConfig::processed()),
            data_slice: None,
            min_context_slot: Some(42),
        };
        assert_eq!(
            calls[1].1["params"],
            json!([
                preparation_assets::input_keys(a(1), a(9)).map(|key| key.to_string()),
                serde_json::to_value(config).unwrap()
            ])
        );
        assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(240));
        assert_eq!(r.slots[5].available_permits(), 1);
    }
}

#[tokio::test]
async fn preparation_shared_read_empty_unparsed_and_unavailable_scan_require_raw_accounts() {
    let mut unparsed = scan(false, 1000);
    unparsed[0]["account"]["data"] = json!(["", "base64"]);
    for (answer, units, min) in [
        (Answer::Value(42, json!([])), 0, Some(42)),
        (Answer::Value(42, unparsed), 1000, Some(42)),
        (Answer::Unavailable, 1000, None),
    ] {
        let r = runtime(vec![answer, Answer::Value(42, raw_accounts(false, units))]);
        assert_eq!(
            reader(&r)
                .snapshot(a(9))
                .await
                .unwrap()
                .checked_units(a(1), a(9))
                .unwrap(),
            units
        );
        assert_eq!(
            r.transport.calls.lock().unwrap()[1].1["params"][1]["minContextSlot"],
            json!(min)
        );
    }
}

#[tokio::test]
async fn preparation_shared_read_bad_scan_never_reaches_raw_fallback() {
    let mut wrong = scan(false, 1000);
    wrong[0]["account"]["data"]["parsed"]["info"]["mint"] = json!(a(99).to_string());
    let duplicate = json!([scan(false, 1000)[0].clone(), scan(false, 1000)[0].clone()]);
    let mut mixed = duplicate.clone();
    mixed[1]["pubkey"] = json!(a(99).to_string());
    mixed[1]["account"]["data"] = json!(["", "base64"]);
    for answer in [
        Answer::Value(42, wrong),
        Answer::Value(42, duplicate),
        Answer::Value(42, mixed),
        Answer::Value(42, json!(vec![Value::Null; 1001])),
        Answer::Limit,
    ] {
        let r = runtime(vec![answer]);
        assert!(fatal(&reader(&r).snapshot(a(9)).await.err().unwrap()));
        assert_eq!(r.transport.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn preparation_shared_read_raw_mismatch_missing_bad_program_and_limits_are_fatal() {
    for raw in [
        raw_accounts(false, 999),
        raw_accounts(true, 1000),
        json!([null, null, null]),
        json!([]),
    ] {
        let r = runtime(vec![
            Answer::Value(42, scan(false, 1000)),
            Answer::Value(42, raw),
        ]);
        assert!(fatal(&reader(&r).snapshot(a(9)).await.err().unwrap()));
        assert_eq!(r.transport.calls.lock().unwrap().len(), 2);
    }
    let r = runtime(vec![Answer::Value(42, scan(false, 1000)), Answer::Limit]);
    assert!(fatal(&reader(&r).snapshot(a(9)).await.err().unwrap()));
}

#[test]
fn preparation_shared_read_envelope_classifies_invalid_identity_context_and_rpc_errors() {
    let good = json!({"jsonrpc":"2.0","id":7,"result":{"context":{"slot":42},"value":[]}});
    for damage in 0..7 {
        let mut value = good.clone();
        match damage {
            0 => value["id"] = json!(8),
            1 => value["jsonrpc"] = json!("1.0"),
            2 => value["result"]["context"]["slot"] = json!(0),
            3 => value["result"]["context"]["slot"] = json!(-1),
            4 => value["error"] = json!({"code":-1,"message":"fixture-secret"}),
            5 => {
                value["result"].as_object_mut().unwrap().remove("value");
            }
            _ => {
                value["result"].as_object_mut().unwrap().remove("context");
            }
        }
        let error = decode(7, &serde_json::to_vec(&value).unwrap())
            .err()
            .unwrap();
        assert!(fatal(&error));
        assert!(!format!("{error:?}").contains("fixture-secret"));
    }
    assert!(retryable(
        &decode(
            7,
            &serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":7,"error":{"code":-32005,"message":"fixture-secret"}})
            )
            .unwrap()
        )
        .err()
        .unwrap()
    ));
    assert!(decode(7, b"fixture-secret").is_err());
}

#[tokio::test]
async fn preparation_shared_read_context_cannot_regress_and_unknown_nodes_do_not_widen_scope() {
    for slot in [0, 41] {
        let r = runtime(vec![
            Answer::Value(42, scan(false, 1000)),
            Answer::Value(slot, raw_accounts(false, 1000)),
        ]);
        assert!(fatal(&reader(&r).snapshot(a(9)).await.err().unwrap()));
    }
    let r = runtime(vec![]);
    let error = r
        .preparation_balance(
            "https://unregistered.invalid/?key=fixture-secret",
            a(1),
            a(9),
        )
        .await
        .err()
        .unwrap();
    assert!(retryable(&error));
    assert!(!format!("{error:?}").contains("fixture-secret"));
    assert!(r.transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn preparation_shared_read_429_cools_all_profiles_without_raw_or_alternate_retry() {
    let r = runtime(vec![Answer::Rate]);
    assert!(retryable(&reader(&r).snapshot(a(9)).await.err().unwrap()));
    assert!(reader(&r).snapshot(a(9)).await.is_err());
    assert_eq!(r.transport.calls.lock().unwrap().len(), 1);
    r.latest_blockhash().await.unwrap();
    let calls = r.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, Url::parse(&urls()[1]).unwrap().to_string());
}

#[tokio::test]
async fn preparation_shared_read_busy_cancel_and_pacing_do_not_occupy_hash_capacity() {
    let r = runtime(vec![Answer::Hold]);
    let port = reader(&r);
    let task = tokio::spawn(async move { port.snapshot(a(9)).await });
    tokio::time::timeout(Duration::from_secs(1), r.transport.entered.notified())
        .await
        .unwrap();
    assert!(reader(&r)
        .snapshot(a(9))
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("busy"));
    let before = r.budget.lock().unwrap().next_start[5];
    r.latest_blockhash().await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(r.budget.lock().unwrap().next_start[5], before);
    assert_eq!(r.slots[5].available_permits(), 1);
    assert_eq!(r.transport.dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn preparation_shared_read_cancel_while_pacing_sends_nothing_and_refunds_no_start() {
    use std::{future::Future, task::Poll};
    let r = runtime(vec![]);
    r.budget.lock().unwrap().next_start[5] = Instant::now() + Duration::from_secs(1);
    let before = r.budget.lock().unwrap().next_start[5];
    let port = reader(&r);
    let mut pending = Box::pin(port.snapshot(a(9)));
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(pending);
    assert_eq!(r.budget.lock().unwrap().next_start[5], before);
    assert_eq!(r.slots[5].available_permits(), 1);
    assert!(r.transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn preparation_shared_read_scan_timeout_can_recover_raw_within_fresh_total_budget() {
    let r = runtime(vec![
        Answer::Hold,
        Answer::Value(42, raw_accounts(false, 1000)),
    ]);
    assert_eq!(
        reader(&r)
            .snapshot(a(9))
            .await
            .unwrap()
            .checked_units(a(1), a(9))
            .unwrap(),
        1000
    );
    assert_eq!(r.transport.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn preparation_shared_read_two_step_total_timeout_and_slow_evidence_do_not_renew_timestamp() {
    let r = runtime(vec![Answer::Hold, Answer::Hold]);
    let started = Instant::now();
    assert!(reader(&r).snapshot(a(9)).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(r.transport.calls.lock().unwrap().len(), 2);
    assert_eq!(r.slots[5].available_permits(), 1);
    let r = runtime(vec![
        Answer::Delay(Duration::from_millis(1600), scan(false, 1000)),
        Answer::Delay(Duration::from_millis(1600), raw_accounts(false, 1000)),
    ]);
    let error = reader(&r).snapshot(a(9)).await.err().unwrap();
    assert!(fatal(&error));
    assert!(error.to_string().contains("expired"));
}

#[tokio::test]
async fn preparation_shared_read_distinct_credentials_are_separate_but_alias_cooldowns_shared() {
    let configured = vec![
        "https://ONE.invalid:443/?key=one".into(),
        "https://one.invalid/?key=two".into(),
    ];
    let r = Arc::new(SharedReadRuntime::with_transport(
        &configured,
        Fake::new(vec![
            Answer::Rate,
            Answer::Value(42, scan(false, 1000)),
            Answer::Value(42, raw_accounts(false, 1000)),
        ]),
    ));
    assert!(r
        .preparation_balance("https://one.invalid/?key=one", a(1), a(9))
        .await
        .is_err());
    assert!(r
        .preparation_balance(&configured[0], a(1), a(9))
        .await
        .is_err());
    assert!(r
        .preparation_balance(&configured[1], a(1), a(9))
        .await
        .is_ok());
    assert_eq!(r.transport.calls.lock().unwrap().len(), 3);
}
