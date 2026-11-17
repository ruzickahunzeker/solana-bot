use super::*;
use base64::Engine;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Good,
    Rate,
    Hold,
    Late,
    Oversize,
    Error,
}
pub(crate) struct Fake {
    pub(crate) value: Value,
    mode: Mode,
    pub(crate) calls: Mutex<Vec<(String, Value, Instant)>>,
    pub(crate) entered: Notify,
    dropped: AtomicUsize,
}
struct DropRead<'a>(&'a AtomicUsize);
impl Drop for DropRead<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
pub(crate) fn ui(owner: solana_sdk::pubkey::Pubkey, data: &[u8]) -> Value {
    json!({"lamports":u64::MAX,"owner":owner.to_string(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"],"executable":false,"rentEpoch":0,"space":data.len()})
}
pub(crate) fn urls() -> Vec<String> {
    vec![
        "https://primary.example.invalid/?key=fixture-secret".into(),
        "https://backup.example.invalid".into(),
    ]
}
pub(crate) fn address(n: u8) -> Address {
    Address::from_bytes([n; 32])
}
pub(crate) fn runtime(value: Value, mode: Mode) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &urls(),
        Fake {
            value,
            mode,
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            dropped: AtomicUsize::new(0),
        },
    ))
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let body = request.body();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone(), Instant::now()));
        if matches!(request, ReadRequest::LatestBlockhash { .. }) {
            return Ok(HttpReply::for_test(StatusCode::OK, serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":42},"value":{"blockhash":address(7).to_string(),"lastValidBlockHeight":456}}})).unwrap()));
        }
        assert!(matches!(request, ReadRequest::AggregatorAccount { .. }));
        let _read = DropRead(&self.dropped);
        self.entered.notify_one();
        match self.mode {
            Mode::Good => {}
            Mode::Rate => {
                return Ok(HttpReply {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    retry_after: Some(60),
                    body: vec![],
                })
            }
            Mode::Hold => std::future::pending::<()>().await,
            Mode::Late => std::thread::sleep(Duration::from_millis(2100)),
            Mode::Oversize => return Ok(HttpReply::for_test(StatusCode::OK, vec![b' '; 65_537])),
            Mode::Error => anyhow::bail!("private transport detail"),
        }
        Ok(HttpReply::for_test(StatusCode::OK, serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":42},"value":self.value}})).unwrap()))
    }
}
fn value() -> Value {
    ui(
        solana_sdk::pubkey::Pubkey::new_from_array([8; 32]),
        &[1, 2, 3],
    )
}

#[test]
fn aggregator_account_read_decoder_keeps_integer_and_raw_data_and_rejects_invalid_evidence() {
    let good = json!({"jsonrpc":"2.0","id":7,"result":{"context":{"slot":42},"value":value()}});
    let raw = decode(7, &serde_json::to_vec(&good).unwrap()).unwrap();
    assert_eq!((raw.lamports, raw.data), (u64::MAX, vec![1, 2, 3]));
    for damage in 0..15 {
        let mut broken = good.clone();
        match damage {
            0 => broken["id"] = json!(8),
            1 => broken["jsonrpc"] = json!("1.0"),
            2 => broken["result"]["context"]["slot"] = json!(0),
            3 => broken["result"]["value"] = Value::Null,
            4 => broken["result"]["value"]["owner"] = json!("bad"),
            5 => broken["result"]["value"]["data"] = json!(["not base64!", "base64"]),
            6 => broken["result"]["value"]["data"] = json!(["", "base64+zstd"]),
            7 => broken["result"]["value"]["data"] = json!({"parsed":{}}),
            8 => broken["result"]["value"]["lamports"] = json!(-1),
            9 => broken["result"]["value"]["executable"] = json!("false"),
            10 => broken["error"] = json!({"code":-32005,"message":"fixture-secret"}),
            11 => {
                broken["result"] = Value::Null;
                broken["error"] = json!({"code":-32005,"message":"fixture-secret"});
            }
            12 => {
                broken.as_object_mut().unwrap().remove("result");
            }
            13 => broken["result"]["value"]["space"] = json!(99),
            _ => broken["result"]["value"]["owner"] = json!("x".repeat(45)),
        }
        let error = decode(7, &serde_json::to_vec(&broken).unwrap()).unwrap_err();
        assert!(
            !format!("{error:?}").contains("fixture-secret"),
            "damage={damage}"
        );
    }
}

#[tokio::test]
async fn aggregator_account_read_fixed_request_preserves_commitment_address_and_pacing() {
    let r = runtime(value(), Mode::Good);
    for commitment in [CommitmentConfig::confirmed(), CommitmentConfig::finalized()] {
        let account = r
            .aggregator_account(&urls()[0], address(3), commitment)
            .await
            .unwrap();
        assert_eq!(account.data, vec![1, 2, 3]);
    }
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, Url::parse(&urls()[0]).unwrap().to_string());
    assert_eq!(
        calls[0].1,
        json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[address(3).to_string(),{"encoding":"base64","commitment":"confirmed"}]})
    );
    assert_eq!(calls[1].1["params"][1]["commitment"], json!("finalized"));
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(90));
}

#[tokio::test]
async fn aggregator_account_read_unknown_lane_and_pacing_deadline_never_send() {
    let r = runtime(value(), Mode::Good);
    assert!(r
        .aggregator_account(
            "https://unknown.example.invalid",
            address(3),
            CommitmentConfig::processed()
        )
        .await
        .is_err());
    r.budget.lock().unwrap().next_start[Class::AggregatorAccounts.index()] =
        Instant::now() + Duration::from_secs(30);
    assert!(r
        .aggregator_account(&urls()[0], address(3), CommitmentConfig::processed())
        .await
        .is_err());
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
    assert_eq!(r.slots[9].available_permits(), 1);
}

#[tokio::test]
async fn aggregator_account_read_429_shares_cooldown_without_switching_current_node() {
    let r = runtime(value(), Mode::Rate);
    assert!(r
        .aggregator_account(&urls()[0], address(3), CommitmentConfig::processed())
        .await
        .is_err());
    let cooldown = r.budget.lock().unwrap().cooldowns.clone();
    assert!(r
        .aggregator_account(&urls()[0], address(4), CommitmentConfig::processed())
        .await
        .is_err());
    r.latest_blockhash().await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, Url::parse(&urls()[1]).unwrap().to_string());
    assert_eq!(r.budget.lock().unwrap().cooldowns, cooldown);
}

#[tokio::test]
async fn aggregator_account_read_busy_and_cancel_leave_all_other_capacities_intact() {
    let r = runtime(value(), Mode::Hold);
    let task = {
        let r = r.clone();
        tokio::spawn(async move {
            r.aggregator_account(&urls()[0], address(3), CommitmentConfig::processed())
                .await
        })
    };
    tokio::time::timeout(
        Duration::from_secs(1),
        r.test_transport().entered.notified(),
    )
    .await
    .unwrap();
    assert!(r
        .aggregator_account(&urls()[0], address(4), CommitmentConfig::processed())
        .await
        .is_err());
    assert_eq!(
        r.slots
            .iter()
            .map(|s| s.available_permits())
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1, 1, 1, 1, 2, 1, 0]
    );
    r.latest_blockhash().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(r.test_transport().dropped.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.slots
            .iter()
            .map(|s| s.available_permits())
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1, 1, 1, 1, 2, 1, 1]
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn aggregator_account_read_timeout_releases_permit_without_hidden_retry() {
    let r = runtime(value(), Mode::Hold);
    let started = Instant::now();
    assert!(r
        .aggregator_account(&urls()[0], address(3), CommitmentConfig::processed())
        .await
        .is_err());
    assert!(
        started.elapsed() >= Duration::from_millis(1900)
            && started.elapsed() < Duration::from_secs(3)
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert_eq!(r.test_transport().dropped.load(Ordering::SeqCst), 1);
    assert_eq!(r.slots[9].available_permits(), 1);
}

#[tokio::test]
async fn aggregator_account_read_late_response_cannot_be_accepted_after_deadline() {
    let r = runtime(value(), Mode::Late);
    assert!(r
        .aggregator_account(&urls()[0], address(3), CommitmentConfig::processed())
        .await
        .is_err());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert_eq!(r.slots[9].available_permits(), 1);
}

#[tokio::test]
async fn aggregator_account_read_body_and_transport_errors_never_return_default_accounts() {
    for mode in [Mode::Oversize, Mode::Error] {
        let r = runtime(value(), mode);
        let error = r
            .aggregator_account(&urls()[0], address(3), CommitmentConfig::processed())
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("private transport detail"));
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert_eq!(r.slots[9].available_permits(), 1);
    }
}
