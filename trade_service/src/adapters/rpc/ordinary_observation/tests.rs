use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

mod transport;

#[test]
fn sdk_selected_observer_retains_only_the_selected_registered_endpoint_and_shared_runtime() {
    let urls = vec![
        "http://127.0.0.1:1/".to_string(),
        "http://127.0.0.1:2/".to_string(),
    ];
    let runtime = Arc::new(SharedReadRuntime::new(&urls).unwrap());
    let reader = SingleShotStatusReader::with_selected_runtime(runtime.clone(), &urls[1]).unwrap();
    assert_eq!(
        reader.urls.iter().map(Url::as_str).collect::<Vec<_>>(),
        [urls[1].as_str()]
    );
    assert!(Arc::ptr_eq(&runtime, &reader.transport.runtime));
    assert_eq!(reader.gate.state.lock().unwrap().cooldowns.len(), 1);
}

#[test]
fn sdk_selected_observer_rejects_unregistered_or_different_key_endpoints_without_io() {
    let runtime = Arc::new(
        SharedReadRuntime::new(&["https://rpc.example.invalid/?api-key=test-a".into()]).unwrap(),
    );
    for selected in [
        "https://rpc.example.invalid/?api-key=test-b",
        "http://127.0.0.1:3/",
        "not-a-url",
    ] {
        assert!(SingleShotStatusReader::with_selected_runtime(runtime.clone(), selected).is_err());
    }
}

fn signature() -> String {
    Signature::from([1; 64]).to_string()
}
fn response(id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":[{"slot":123,"confirmations":null,"status":{"Ok":null},"err":null,"confirmationStatus":"finalized"}]}})
}
struct FakeHttp {
    status: StatusCode,
    calls: AtomicUsize,
    urls: Mutex<Vec<String>>,
    entered: Notify,
    pending: bool,
}
impl FakeHttp {
    fn new(status: StatusCode) -> Self {
        Self {
            status,
            calls: AtomicUsize::new(0),
            urls: Mutex::new(vec![]),
            entered: Notify::new(),
            pending: false,
        }
    }
}
impl StatusTransport for FakeHttp {
    async fn fetch(&self, url: &Url, id: u64, signature: &str) -> Result<HttpReply> {
        assert_eq!(signature, self::signature());
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.urls.lock().unwrap().push(url.to_string());
        self.entered.notify_one();
        if self.pending {
            std::future::pending::<()>().await;
        }
        Ok(HttpReply {
            status: self.status,
            retry_after: Some(60),
            body: serde_json::to_vec(&response(id)).unwrap(),
        })
    }
}

#[test]
fn ordinary_scan_http_gate_is_serial_paced_and_does_not_refund_cancellation() {
    let gate = RequestGate::new(2, Duration::from_secs(1));
    let now = Instant::now();
    let first = gate.begin(now).unwrap();
    assert_eq!(first.index, 0);
    assert!(gate.begin(now + Duration::from_secs(2)).is_err());
    drop(first);
    assert!(gate.begin(now + Duration::from_millis(999)).is_err());
    let second = gate.begin(now + Duration::from_secs(1)).unwrap();
    assert_eq!(second.index, 1);
}

#[test]
fn ordinary_scan_http_429_cools_one_lane_and_bounds_retry_after() {
    let gate = RequestGate::new(2, Duration::from_secs(1));
    let now = Instant::now();
    let first = gate.begin(now).unwrap();
    first.rate_limited(Some(u64::MAX));
    drop(first);
    let deadline = gate.state.lock().unwrap().cooldowns[0].unwrap();
    assert!(deadline.duration_since(Instant::now()) <= Duration::from_secs(300));
    let next = gate.begin(now + Duration::from_secs(1)).unwrap();
    assert_eq!(next.index, 1);
}

#[tokio::test]
async fn ordinary_scan_http_uses_first_two_distinct_endpoints_before_applying_limit() {
    let urls = vec![
        "https://one.example.invalid/?api-key=fixture-secret".into(),
        "https://one.example.invalid/?api-key=fixture-secret".into(),
        "https://two.example.invalid/fixture-secret".into(),
        "https://third.example.invalid".into(),
    ];
    let reader = SingleShotStatusReader::with_transport(
        &urls,
        Duration::from_secs(1),
        FakeHttp::new(StatusCode::TOO_MANY_REQUESTS),
    )
    .unwrap();
    assert_eq!(reader.urls.len(), 2);
    assert!(reader.lookup_foreground(&signature()).await.is_err());
    reader.gate.state.lock().unwrap().next_allowed = Instant::now();
    assert!(reader.lookup_foreground(&signature()).await.is_err());
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        *reader.transport.urls.lock().unwrap(),
        vec![urls[0].clone(), urls[2].clone()]
    );
    // Both selected endpoints are cooling down: never reach the third distinct provider.
    reader.gate.state.lock().unwrap().next_allowed = Instant::now();
    assert!(reader.lookup_foreground(&signature()).await.is_err());
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ordinary_scan_http_sends_one_logical_request_and_never_retries_429() {
    let urls = vec![
        "https://one.example.invalid/?api-key=fixture-secret".into(),
        "https://two.example.invalid/fixture-secret".into(),
    ];
    let reader = SingleShotStatusReader::with_transport(
        &urls,
        Duration::from_secs(1),
        FakeHttp::new(StatusCode::TOO_MANY_REQUESTS),
    )
    .unwrap();
    let error = reader
        .lookup_signature(&signature())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("429"));
    assert!(!error.contains("fixture-secret"));
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 1);
    let reserved = reader.gate.state.lock().unwrap().next_allowed;
    assert!(reader
        .gate
        .begin(reserved - Duration::from_nanos(1))
        .is_err());
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 1);
    reader.gate.state.lock().unwrap().next_allowed = Instant::now();
    assert!(reader.lookup_foreground(&signature()).await.is_err());
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 2);
    assert_eq!(*reader.transport.urls.lock().unwrap(), urls);
}

#[tokio::test]
async fn ordinary_scan_http_success_uses_original_signature_and_invalid_signature_never_fetches() {
    let reader = SingleShotStatusReader::with_transport(
        &["https://node.example.invalid".into()],
        Duration::from_secs(1),
        FakeHttp::new(StatusCode::OK),
    )
    .unwrap();
    assert!(reader.lookup_signature("invalid").await.is_err());
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 0);
    let result = reader
        .lookup_signature(&signature())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.signature, signature());
    assert_eq!(
        result.commitment,
        crate::domain::reconciliation::ChainCommitment::Finalized
    );
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        request(7, &signature()),
        json!({"jsonrpc":"2.0","id":7,"method":"getSignatureStatuses","params":[[signature()],{"searchTransactionHistory":true}]})
    );
}

#[test]
fn ordinary_scan_http_rejects_bad_identity_finality_or_oversized_bodies_without_echoing_secrets() {
    for case in [
        "id",
        "version",
        "count",
        "context",
        "slot",
        "confirmations",
        "missing_level",
        "errors",
        "rpc_error",
    ] {
        let mut value = response(7);
        match case {
            "id" => value["id"] = json!(8),
            "version" => value["jsonrpc"] = json!("1.0"),
            "count" => value["result"]["value"] = json!([]),
            "context" => value["result"]["context"]["slot"] = json!(122),
            "slot" => value["result"]["value"][0]["slot"] = json!(0),
            "confirmations" => value["result"]["value"][0]["confirmations"] = json!(1),
            "missing_level" => {
                value["result"]["value"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("confirmationStatus");
            }
            "errors" => {
                value["result"]["value"][0]["err"] = json!({"InstructionError":[0,{"Custom":42}]})
            }
            "rpc_error" => {
                value.as_object_mut().unwrap().remove("result");
                value["error"] = json!({"code":-32005,"message":"fixture-secret"});
            }
            _ => unreachable!(),
        }
        let error = parse_reply(7, &signature(), &serde_json::to_vec(&value).unwrap())
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("fixture-secret"), "{case}");
    }
    assert!(parse_reply(7, &signature(), &vec![b' '; MAX_BODY + 1]).is_err());
    for url in [
        "file:///tmp/private",
        "https://user:password@node.example.invalid",
        "https://node.example.invalid/#fragment",
    ] {
        assert!(SingleShotStatusReader::with_transport(
            &[url.into()],
            Duration::from_secs(1),
            FakeHttp::new(StatusCode::OK)
        )
        .is_err());
    }
}

#[tokio::test]
async fn ordinary_scan_http_cancel_releases_inflight_but_not_request_spacing() {
    let mut fake = FakeHttp::new(StatusCode::OK);
    fake.pending = true;
    let reader = Arc::new(
        SingleShotStatusReader::with_transport(
            &["https://node.example.invalid".into()],
            Duration::from_secs(1),
            fake,
        )
        .unwrap(),
    );
    let task = {
        let r = reader.clone();
        tokio::spawn(async move { r.lookup_signature(&signature()).await })
    };
    tokio::time::timeout(Duration::from_secs(2), reader.transport.entered.notified())
        .await
        .unwrap();
    let reserved = reader.gate.state.lock().unwrap().next_allowed;
    assert!(reader.lookup_foreground(&signature()).await.is_err());
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(!reader.gate.state.lock().unwrap().in_flight);
    assert_eq!(reader.gate.state.lock().unwrap().next_allowed, reserved);
    assert!(reader
        .gate
        .begin(reserved - Duration::from_nanos(1))
        .is_err());
    assert_eq!(reader.transport.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ordinary_scan_http_background_waiter_cannot_be_overtaken_by_foreground_probes() {
    let gate = RequestGate::new(2, Duration::from_secs(1));
    let held = gate.foreground(Instant::now()).unwrap();
    let mut background = Box::pin(gate.background());
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(
            background.as_mut().poll(cx).is_pending()
        ))
        .await
    );
    gate.state.lock().unwrap().next_allowed = Instant::now();
    drop(held);
    assert!(gate.foreground(Instant::now()).is_err());
    let acquired = tokio::time::timeout(Duration::from_secs(2), background)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acquired.index, 1);
    assert!(gate.foreground(Instant::now()).is_err());
}
