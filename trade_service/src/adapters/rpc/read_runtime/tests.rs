use super::*;
use crate::adapters::rpc::{
    blockhash::RpcBlockhashReader, ordinary_observation::SingleShotStatusReader,
};
use crate::application::ports::blockhash::LatestBlockhashReader;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum Answer {
    Success,
    RateLimited,
    Hold,
    Invalid,
}
struct Fake {
    limited_primary: bool,
    hash: Answer,
    status: Answer,
    calls: Mutex<Vec<(String, Value)>>,
    entered: Notify,
    dropped: Arc<AtomicUsize>,
}
impl Fake {
    fn new(hash: Answer, status: Answer) -> Self {
        Self {
            limited_primary: false,
            hash,
            status,
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            dropped: Arc::new(AtomicUsize::new(0)),
        }
    }
}
struct DropCounter(Arc<AtomicUsize>);
impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn hash_response(id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":{"blockhash":bs58::encode([7u8;32]).into_string(),"lastValidBlockHeight":456}}})
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        self.calls
            .lock()
            .unwrap()
            .push((url.host_str().unwrap().into(), request.body()));
        let _drop = DropCounter(self.dropped.clone());
        self.entered.notify_one();
        let answer = if self.limited_primary && url.host_str() == Some("primary.example.invalid") {
            Answer::RateLimited
        } else {
            match request {
                ReadRequest::LatestBlockhash { .. } => self.hash,
                ReadRequest::SignatureStatus { .. } => self.status,
                ReadRequest::NativeBalances { .. } => self.hash,
                _ => panic!("quote tests use their own typed fixtures"),
            }
        };
        match answer {
            Answer::Hold => std::future::pending::<()>().await,
            Answer::RateLimited => {
                return Ok(HttpReply {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    retry_after: Some(60),
                    body: vec![],
                })
            }
            Answer::Success => {}
            Answer::Invalid => {
                return Ok(HttpReply {
                    status: StatusCode::OK,
                    retry_after: None,
                    body: b"{\"jsonrpc\":\"2.0\",\"id\":999,\"error\":\"fixture-secret\"}".to_vec(),
                })
            }
        }
        let body = match request {
            ReadRequest::LatestBlockhash { id } => hash_response(*id),
            ReadRequest::SignatureStatus { id, .. } => {
                json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":[null]}})
            }
            ReadRequest::NativeBalances { .. } => {
                panic!("native balance tests use their own typed fixtures")
            }
            _ => panic!("quote tests use their own typed fixtures"),
        };
        Ok(HttpReply {
            status: StatusCode::OK,
            retry_after: None,
            body: serde_json::to_vec(&body).unwrap(),
        })
    }
}

fn urls() -> Vec<String> {
    vec![
        "https://primary.example.invalid/?key=fixture-secret".into(),
        "https://backup.example.invalid".into(),
    ]
}
fn signature() -> String {
    bs58::encode([1u8; 64]).into_string()
}

#[tokio::test]
async fn shared_read_primary_processed_success_stops_before_backup_and_uses_project_port() {
    let runtime = Arc::new(SharedReadRuntime::with_transport(
        &urls(),
        Fake::new(Answer::Success, Answer::Success),
    ));
    let reader = RpcBlockhashReader::new(runtime.clone());
    assert_eq!(reader.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        (
            "primary.example.invalid".into(),
            json!({"jsonrpc":"2.0","id":1,"method":"getLatestBlockhash","params":[{"commitment":"processed"}]})
        )
    );
    assert_eq!(runtime.slots[0].available_permits(), 1);
}

#[tokio::test]
async fn shared_read_429_has_one_attempt_and_serial_backup_then_future_refresh_skips_cooled_key() {
    let mut fake = Fake::new(Answer::Success, Answer::Success);
    fake.limited_primary = true;
    let runtime = SharedReadRuntime::with_transport(&urls(), fake);
    assert_eq!(
        runtime.latest_blockhash().await.unwrap().to_bytes(),
        [7; 32]
    );
    assert_eq!(
        runtime.latest_blockhash().await.unwrap().to_bytes(),
        [7; 32]
    );
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(
        calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
        vec![
            "primary.example.invalid",
            "backup.example.invalid",
            "backup.example.invalid"
        ]
    );
}

#[tokio::test]
async fn shared_read_observation_429_is_visible_to_critical_refresh_without_another_primary_request(
) {
    let mut fake = Fake::new(Answer::Success, Answer::Success);
    fake.limited_primary = true;
    let runtime = SharedReadRuntime::with_transport(&urls(), fake);
    let reply = runtime
        .signature_status(&Url::parse(&urls()[0]).unwrap(), 1, &signature())
        .await
        .unwrap();
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        runtime.latest_blockhash().await.unwrap().to_bytes(),
        [7; 32]
    );
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1["method"], "getSignatureStatuses");
    assert_eq!(calls[1].0, "backup.example.invalid");
}

#[tokio::test]
async fn shared_read_critical_429_is_visible_to_observation_without_extending_cooldown() {
    let mut fake = Fake::new(Answer::Success, Answer::Success);
    fake.limited_primary = true;
    let runtime = SharedReadRuntime::with_transport(&urls(), fake);
    runtime.latest_blockhash().await.unwrap();
    let before = runtime.budget.lock().unwrap().cooldowns[0];
    let error = runtime
        .signature_status(&Url::parse(&urls()[0]).unwrap(), 99, &signature())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("cooling down"));
    assert!(!error.contains("fixture-secret"));
    assert_eq!(runtime.budget.lock().unwrap().cooldowns[0], before);
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn shared_read_pending_observation_cannot_occupy_reserved_blockhash_capacity() {
    let runtime = Arc::new(SharedReadRuntime::with_transport(
        &urls(),
        Fake::new(Answer::Success, Answer::Hold),
    ));
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .signature_status(&Url::parse(&urls()[0]).unwrap(), 1, &signature())
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), runtime.transport.entered.notified())
        .await
        .unwrap();
    assert_eq!(runtime.slots[1].available_permits(), 0);
    let next = runtime.budget.lock().unwrap().next_start[1];
    let hash = tokio::time::timeout(Duration::from_secs(1), runtime.latest_blockhash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hash.to_bytes(), [7; 32]);
    assert_eq!(runtime.budget.lock().unwrap().next_start[1], next);
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert_eq!(runtime.slots[1].available_permits(), 1);
}

#[tokio::test]
async fn shared_read_long_observation_spacing_does_not_delay_critical_refresh() {
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    let reserved = Instant::now() + Duration::from_secs(60);
    runtime.budget.lock().unwrap().next_start[1] = reserved;
    let url = Url::parse(&urls()[0]).unwrap();
    let sig = signature();
    assert!(runtime
        .signature_status(&url, 1, &sig)
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("pacing exceeds deadline"));
    assert_eq!(runtime.slots[1].available_permits(), 1);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), runtime.latest_blockhash())
            .await
            .unwrap()
            .unwrap()
            .to_bytes(),
        [7; 32]
    );
    assert_eq!(runtime.budget.lock().unwrap().next_start[1], reserved);
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn shared_read_cancel_critical_releases_capacity_but_does_not_refund_started_request() {
    let runtime = Arc::new(SharedReadRuntime::with_transport(
        &urls(),
        Fake::new(Answer::Hold, Answer::Success),
    ));
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.latest_blockhash().await })
    };
    tokio::time::timeout(Duration::from_secs(1), runtime.transport.entered.notified())
        .await
        .unwrap();
    let reserved = runtime.budget.lock().unwrap().next_start[0];
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.slots[0].available_permits(), 1);
    assert_eq!(runtime.budget.lock().unwrap().next_start[0], reserved);
    assert_eq!(runtime.transport.dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shared_read_total_deadline_bounds_many_slow_nodes_without_detached_retries() {
    let endpoints = (0..10)
        .map(|index| format!("https://node-{index}.example.invalid"))
        .collect::<Vec<_>>();
    let runtime =
        SharedReadRuntime::with_transport(&endpoints, Fake::new(Answer::Hold, Answer::Success));
    let started = Instant::now();
    assert!(runtime.latest_blockhash().await.is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    let calls = runtime.transport.calls.lock().unwrap().len();
    assert!((1..=3).contains(&calls));
    assert_eq!(runtime.transport.dropped.load(Ordering::SeqCst), calls);
    assert_eq!(runtime.slots[0].available_permits(), 1);
    tokio::task::yield_now().await;
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), calls);
}

#[tokio::test]
async fn shared_read_all_cooling_or_invalid_endpoints_emit_no_requests() {
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    runtime
        .budget
        .lock()
        .unwrap()
        .cooldowns
        .fill(Some(Instant::now() + Duration::from_secs(60)));
    assert!(runtime.latest_blockhash().await.is_err());
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
    let runtime = SharedReadRuntime::with_transport(
        &["file:///tmp/fixture-secret".into(), "bad-url".into()],
        Fake::new(Answer::Success, Answer::Success),
    );
    let error = runtime.latest_blockhash().await.err().unwrap().to_string();
    assert!(!error.contains("fixture-secret"));
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn shared_read_expired_wire_admission_never_sends_or_consumes_start_budget() {
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    let before = runtime.budget.lock().unwrap().next_start;
    let expired = Instant::now() - Duration::from_secs(1);
    let result = runtime
        .read(
            Class::Blockhash,
            0,
            ReadRequest::LatestBlockhash { id: 1 },
            expired,
        )
        .await;
    assert!(result.is_err());
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
    assert_eq!(runtime.budget.lock().unwrap().next_start, before);
}

#[tokio::test]
async fn shared_read_deadline_is_rechecked_after_wait_before_consuming_or_sending() {
    use std::{future::Future, task::Poll};
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    runtime.budget.lock().unwrap().next_start[0] = Instant::now() + Duration::from_millis(10);
    let before = runtime.budget.lock().unwrap().next_start;
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut pending = Box::pin(runtime.read(
        Class::Blockhash,
        0,
        ReadRequest::LatestBlockhash { id: 1 },
        deadline,
    ));
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    // Simulate delayed executor polling: both the pacing timer and admission deadline have passed.
    std::thread::sleep(Duration::from_millis(1100));
    assert!(pending.await.is_err());
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
    assert_eq!(runtime.budget.lock().unwrap().next_start, before);
}

#[test]
fn shared_read_canonical_url_aliases_share_state_but_distinct_keys_do_not() {
    let configured = vec![
        "https://PRIMARY.example.invalid:443/?key=one".into(),
        "https://primary.example.invalid/?key=one".into(),
        "https://primary.example.invalid/?key=two".into(),
    ];
    let runtime =
        SharedReadRuntime::with_transport(&configured, Fake::new(Answer::Success, Answer::Success));
    assert_eq!(runtime.endpoints.len(), 2);
    assert_eq!(
        runtime.urls(),
        &[configured[0].clone(), configured[2].clone()]
    );
}

#[test]
fn shared_read_hash_decoder_checks_envelope_and_preserves_required_sdk_field_shapes() {
    assert_eq!(
        parse_blockhash(1, &serde_json::to_vec(&hash_response(1)).unwrap())
            .unwrap()
            .to_bytes(),
        [7; 32]
    );
    for case in [
        "id",
        "version",
        "error",
        "hash",
        "length",
        "missing_context",
        "missing_height",
        "height_type",
        "slot_type",
    ] {
        let mut value = hash_response(1);
        match case {
            "id" => value["id"] = json!(2),
            "version" => value["jsonrpc"] = json!("1.0"),
            "error" => value["error"] = json!({"code":-1,"message":"fixture-secret"}),
            "hash" => value["result"]["value"]["blockhash"] = json!("not-base58-0"),
            "length" => value["result"]["value"]["blockhash"] = json!("111"),
            "missing_context" => {
                value["result"].as_object_mut().unwrap().remove("context");
            }
            "missing_height" => {
                value["result"]["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("lastValidBlockHeight");
            }
            "height_type" => value["result"]["value"]["lastValidBlockHeight"] = json!(-1),
            "slot_type" => value["result"]["context"]["slot"] = json!("123"),
            _ => unreachable!(),
        }
        let error = parse_blockhash(1, &serde_json::to_vec(&value).unwrap())
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("fixture-secret"), "{case}");
    }
}

#[test]
fn shared_read_blockhash_codec_matches_sdk_with_fixed_output_and_input_bounds() {
    let mut samples = (0u8..=255)
        .map(|byte| bs58::encode([byte; 32]).into_string())
        .collect::<Vec<_>>();
    samples.extend([
        String::new(),
        "1".repeat(31),
        "1".repeat(33),
        "z".repeat(44),
        "z".repeat(45),
        "0".repeat(44),
        "1".repeat(60_000),
    ]);
    for encoded in samples {
        let expected = encoded
            .parse::<solana_sdk::hash::Hash>()
            .map(|hash| hash.to_bytes())
            .ok();
        let mut envelope = hash_response(1);
        envelope["result"]["value"]["blockhash"] = json!(encoded);
        let actual = parse_blockhash(1, &serde_json::to_vec(&envelope).unwrap())
            .map(|hash| hash.to_bytes())
            .ok();
        assert_eq!(actual, expected);
    }
}

#[path = "wire_tests.rs"]
mod wire_tests;

#[path = "readiness_tests.rs"]
mod readiness_tests;
