use super::*;
use crate::adapters::rpc::read_http::{HttpReply, ReadRequest};
use anyhow::Result;
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub(crate) const URL: &str = "https://primary.example.invalid/?key=fixture-secret";
const BACKUP: &str = "https://backup.example.invalid";
pub(crate) fn signature() -> Signature {
    Signature::from([7; 64])
}
pub(crate) fn status(commitment: &str, confirmations: Value, error: Value) -> Value {
    json!({"slot":42,"confirmations":confirmations,"confirmationStatus":commitment,"err":error,
        "status":if error.is_null() {json!({"Ok":null})}else{json!({"Err":error})}})
}
pub(crate) fn result() -> Value {
    json!({"context":{"slot":43},"value":[status("finalized", json!(null), json!(null))]})
}
#[derive(Clone, Copy)]
enum Mode {
    Normal,
    WrongId,
    WrongVersion,
    Both,
    Omitted,
    Error,
    InvalidJson,
    Oversize,
    Rate,
    Pending,
    Late,
    Fail,
    Http,
}
pub(crate) struct Fixture {
    result: Value,
    mode: Mode,
    pub(crate) calls: Mutex<Vec<(String, Value)>>,
    entered: Notify,
    dropped: AtomicUsize,
}
struct Dropped<'a>(&'a AtomicUsize);
impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl ReadHttpTransport for Fixture {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let body = request.test_body();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone()));
        self.entered.notify_one();
        let _dropped = Dropped(&self.dropped);
        if body["method"] == "getLatestBlockhash" {
            return Ok(HttpReply::for_test(
                StatusCode::OK,
                serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":43},"value":{"blockhash":bs58::encode([8;32]).into_string(),"lastValidBlockHeight":50}}}),
                )?,
            ));
        }
        assert_eq!(body["method"], "getSignatureStatuses");
        match self.mode {
            Mode::Pending => std::future::pending::<()>().await,
            Mode::Late => std::thread::sleep(Duration::from_millis(2100)),
            Mode::Fail => anyhow::bail!("private transport detail"),
            _ => {}
        }
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":self.result});
        match self.mode {
            Mode::WrongId => envelope["id"] = json!(999),
            Mode::WrongVersion => envelope["jsonrpc"] = json!("1.0"),
            Mode::Both => envelope["error"] = json!({"code":-32005,"message":"private RPC detail"}),
            Mode::Omitted => {
                envelope.as_object_mut().unwrap().remove("result");
            }
            Mode::Error => {
                envelope = json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32005,"message":"private RPC detail"}})
            }
            _ => {}
        }
        let mut bytes = serde_json::to_vec(&envelope)?;
        if matches!(self.mode, Mode::InvalidJson) {
            bytes = b"private invalid JSON".to_vec();
        }
        if matches!(self.mode, Mode::Oversize) {
            bytes.resize(MAX_BODY + 1, b' ');
        }
        Ok(HttpReply {
            status: match self.mode {
                Mode::Rate => StatusCode::TOO_MANY_REQUESTS,
                Mode::Http => StatusCode::BAD_GATEWAY,
                _ => StatusCode::OK,
            },
            retry_after: Some(60),
            body: bytes,
        })
    }
}
pub(crate) fn runtime(value: Value) -> Arc<SharedReadRuntime<Fixture>> {
    runtime_mode(value, Mode::Normal)
}
fn runtime_mode(value: Value, mode: Mode) -> Arc<SharedReadRuntime<Fixture>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &[URL.into(), BACKUP.into()],
        Fixture {
            result: value,
            mode,
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            dropped: AtomicUsize::new(0),
        },
    ))
}
fn reader(r: &Arc<SharedReadRuntime<Fixture>>) -> WsolSignatureReader<Fixture> {
    WsolSignatureReader::new(r.clone(), URL.into(), signature())
}
fn oracle(value: Value) -> original::WsolSignatureReader {
    use solana_client::{nonblocking::rpc_client::RpcClient, rpc_request::RpcRequest};
    let mocks = [(RpcRequest::GetSignatureStatuses, value)]
        .into_iter()
        .collect();
    original::WsolSignatureReader::new(
        Arc::new(RpcClient::new_mock_with_mocks("fails".into(), mocks)),
        signature(),
    )
}
fn same(a: &Option<WsolSignatureObservation>, b: &Option<WsolSignatureObservation>) {
    match (a, b) {
        (Some(a), Some(b)) => {
            assert_eq!(a.chain.signature, b.chain.signature);
            assert_eq!(a.chain.commitment, b.chain.commitment);
            assert_eq!(a.chain.slot, b.chain.slot);
            assert_eq!(a.chain.error, b.chain.error);
            assert_eq!(a.context_slot, b.context_slot);
            assert_eq!(
                a.outcome_for(&signature().to_string()).unwrap(),
                b.outcome_for(&signature().to_string()).unwrap()
            );
        }
        (None, None) => {}
        _ => panic!("status presence differs from original RpcClient"),
    }
}
#[tokio::test]
async fn wsol_status_shared_all_finalities_and_error_text_match_original_rpc() {
    for (commitment, confirmations) in [
        ("processed", json!(0)),
        ("confirmed", json!(1)),
        ("finalized", json!(null)),
    ] {
        for err in [
            json!(null),
            json!({"InstructionError":[0,{"Custom":42}]}),
            json!("AccountNotFound"),
        ] {
            let value = json!({"context":{"slot":43},"value":[status(commitment, confirmations.clone(), err)]});
            let r = runtime(value.clone());
            let actual = reader(&r).lookup().await.unwrap();
            let expected = oracle(value).lookup().await.unwrap();
            same(&actual, &expected);
            assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        }
    }
}
#[tokio::test]
async fn wsol_status_shared_null_and_strict_invalid_metadata_match_original_stop_points() {
    let mut candidates = vec![json!({"context":{"slot":43},"value":[null]})];
    for value in [json!([]), json!([null, null])] {
        candidates.push(json!({"context":{"slot":43},"value":value}));
    }
    candidates.push(json!({"context":{"slot":0},"value":[null]}));
    for (slot, context) in [(0, 43), (44, 43), (42, 0)] {
        let mut value = result();
        value["context"]["slot"] = json!(context);
        value["value"][0]["slot"] = json!(slot);
        candidates.push(value);
    }
    for (commitment, confirmations) in [
        (json!(null), json!(null)),
        (json!("unknown"), json!(1)),
        (json!("processed"), json!(null)),
        (json!("confirmed"), json!(null)),
        (json!("finalized"), json!(0)),
        (json!("finalized"), json!("bad")),
    ] {
        let mut value = result();
        value["value"][0]["confirmationStatus"] = commitment;
        value["value"][0]["confirmations"] = confirmations;
        candidates.push(value);
    }
    for key in ["confirmationStatus", "status", "slot"] {
        let mut value = result();
        value["value"][0].as_object_mut().unwrap().remove(key);
        candidates.push(value);
    }
    for (err, legacy) in [
        (json!(null), json!({"Err":"AccountNotFound"})),
        (json!("AccountNotFound"), json!({"Ok":null})),
        (json!("AccountNotFound"), json!({"Err":"BlockhashNotFound"})),
    ] {
        let mut value = result();
        value["value"][0]["err"] = err;
        value["value"][0]["status"] = legacy;
        candidates.push(value);
    }
    for value in candidates {
        let r = runtime(value.clone());
        let actual = reader(&r).lookup().await;
        let expected = oracle(value.clone()).lookup().await;
        assert_eq!(actual.is_ok(), expected.is_ok(), "{value}");
        if let (Ok(actual), Ok(expected)) = (actual, expected) {
            same(&actual, &expected);
        }
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn wsol_status_shared_one_bound_signature_history_and_unique_ids_allow_later_poll_only() {
    let r = runtime(result());
    let a = reader(&r);
    assert!(Arc::ptr_eq(&a.runtime, &r));
    a.lookup().await.unwrap();
    a.lookup().await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    for (n, (url, body)) in calls.iter().enumerate() {
        assert_eq!(url, URL);
        assert_eq!(
            *body,
            json!({"jsonrpc":"2.0","id":n+1,"method":"getSignatureStatuses","params":[[signature().to_string()],{"searchTransactionHistory":true}]})
        );
    }
}
#[tokio::test]
async fn wsol_status_shared_envelope_body_and_transport_failures_never_fallback() {
    for mode in [
        Mode::WrongId,
        Mode::WrongVersion,
        Mode::Both,
        Mode::Omitted,
        Mode::Error,
        Mode::InvalidJson,
        Mode::Oversize,
        Mode::Fail,
        Mode::Http,
    ] {
        let r = runtime_mode(result(), mode);
        let error = reader(&r).lookup().await.unwrap_err();
        assert!(!format!("{error:?}").contains("private RPC detail"));
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn wsol_status_shared_unknown_endpoint_sends_nothing() {
    let r = runtime(result());
    let a = WsolSignatureReader::new(
        r.clone(),
        "https://unregistered.example.invalid/".into(),
        signature(),
    );
    assert!(a.lookup().await.is_err());
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn wsol_status_shared_429_cools_same_endpoint_for_every_read_without_backup() {
    let r = runtime_mode(result(), Mode::Rate);
    assert!(reader(&r).lookup().await.is_err());
    assert!(r.wsol_quote_slot(URL).is_err());
    assert!(reader(&r).lookup().await.is_err());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn wsol_status_shared_busy_cancel_uses_existing_observation_permit_not_hash() {
    let r = runtime_mode(result(), Mode::Pending);
    let a = reader(&r);
    let lookup = a.lookup();
    tokio::pin!(lookup);
    tokio::select! { _ = &mut lookup => panic!("pending fixture completed"), _ = r.test_transport().entered.notified() => {} }
    assert!(reader(&r).lookup().await.is_err());
    assert!(r
        .signature_status(&Url::parse(URL).unwrap(), 99, &signature().to_string())
        .await
        .is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [8; 32]);
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn wsol_status_shared_cancel_releases_permit_without_retry_or_hidden_task() {
    let r = runtime_mode(result(), Mode::Pending);
    for expected in 1..=2 {
        {
            let a = reader(&r);
            let lookup = a.lookup();
            tokio::pin!(lookup);
            tokio::select! { _ = &mut lookup => panic!("pending fixture completed"), _ = r.test_transport().entered.notified() => {} }
        }
        assert_eq!(r.test_transport().dropped.load(Ordering::SeqCst), expected);
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), expected);
    }
}
#[tokio::test]
async fn wsol_status_shared_timeout_and_late_polling_never_return_evidence() {
    for mode in [Mode::Pending, Mode::Late] {
        let r = runtime_mode(result(), mode);
        let start = Instant::now();
        assert!(reader(&r).lookup().await.is_err());
        assert!(start.elapsed() < Duration::from_secs(4));
        assert_eq!(r.test_transport().dropped.load(Ordering::SeqCst), 1);
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [8; 32]);
    }
}
#[tokio::test]
async fn wsol_status_shared_reader_owns_only_runtime_endpoint_and_original_signature() {
    let r = runtime(result());
    let weak = Arc::downgrade(&r);
    let a = reader(&r);
    drop(r);
    assert!(weak.upgrade().is_some());
    assert_eq!(a.url, URL);
    assert_eq!(a.signature, signature());
    a.lookup().await.unwrap();
    drop(a);
    assert!(weak.upgrade().is_none());
}
#[tokio::test]
async fn wsol_status_shared_request_witness_matches_original_real_rpc_sender_without_normalization()
{
    use solana_client::{
        client_error::Result as RpcResult,
        nonblocking::rpc_client::RpcClient,
        rpc_client::RpcClientConfig,
        rpc_request::RpcRequest,
        rpc_sender::{RpcSender, RpcTransportStats},
    };
    use std::{future::Future, pin::Pin};
    #[derive(Clone)]
    struct Sender(Arc<Mutex<Vec<Value>>>);
    impl RpcSender for Sender {
        fn send<'life0, 'async_trait>(
            &'life0 self,
            request: RpcRequest,
            params: Value,
        ) -> Pin<Box<dyn Future<Output = RpcResult<Value>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                assert_eq!(request, RpcRequest::GetSignatureStatuses);
                self.0.lock().unwrap().push(params);
                Ok(result())
            })
        }
        fn get_transport_stats(&self) -> RpcTransportStats {
            RpcTransportStats::default()
        }
        fn url(&self) -> String {
            "memory://original-wsol-status".into()
        }
    }
    let sender = Sender(Arc::default());
    let original = original::WsolSignatureReader::new(
        Arc::new(RpcClient::new_sender(
            sender.clone(),
            RpcClientConfig::default(),
        )),
        signature(),
    );
    let r = runtime(result());
    same(
        &reader(&r).lookup().await.unwrap(),
        &original.lookup().await.unwrap(),
    );
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1["method"], "getSignatureStatuses");
    assert_eq!(*sender.0.lock().unwrap(), [calls[0].1["params"].clone()]);
}
