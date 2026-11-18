use super::*;
use crate::{
    adapters::rpc::{
        quote_status::RpcQuoteStatusReader, read_http::HttpReply,
        signature_status::RpcSignatureStatusReader,
    },
    application::ports::reconciliation::SignatureStatusReader,
    domain::reconciliation::ChainCommitment,
};
use serde_json::json;
use solana_client::{
    client_error::Result as RpcResult,
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

pub(crate) const PRIMARY: &str = "https://primary.example.invalid/?key=fixture-secret";
pub(crate) const BACKUP: &str = "https://primary.example.invalid/?key=other-fixture-secret";
pub(crate) const THIRD: &str = "https://third.example.invalid/";
pub(crate) fn signature() -> String {
    Signature::from([1; 64]).to_string()
}
pub(crate) fn status(commitment: &str, error: Value) -> Value {
    json!({"slot":123,"confirmations":1,"confirmationStatus":commitment,
        "err":error,"status":if error.is_null() {json!({"Ok":null})} else {json!({"Err":error})}})
}
pub(crate) fn value(entries: Value) -> Value {
    json!({"context":{"slot":123},"value":entries})
}
pub(crate) fn found() -> Value {
    value(json!([status("confirmed", Value::Null)]))
}
fn fields(
    observation: Option<ChainObservation>,
) -> Option<(String, ChainCommitment, u64, Option<String>)> {
    observation.map(|v| (v.signature, v.commitment, v.slot, v.error))
}

#[derive(Clone)]
pub(crate) struct Legacy {
    url: String,
    result: Value,
    pub(crate) calls: Arc<Mutex<Vec<Value>>>,
}
impl Legacy {
    pub(crate) fn new(url: &str, result: Value) -> Self {
        Self {
            url: url.into(),
            result,
            calls: Arc::default(),
        }
    }
    pub(crate) fn client(&self) -> Arc<RpcClient> {
        Arc::new(RpcClient::new_sender(
            self.clone(),
            RpcClientConfig::default(),
        ))
    }
}
impl RpcSender for Legacy {
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
            self.calls.lock().unwrap().push(params);
            Ok(self.result.clone())
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        self.url.clone()
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Fault {
    Id,
    Version,
    Both,
    Missing,
    Json,
    Oversize,
    Rpc,
    Transport,
}
pub(crate) enum Reply {
    Good(Value),
    Wait(u64, Value),
    Hold,
    Late(u64, Value),
    Rate,
    Fault(Fault),
}
pub(crate) struct Fake {
    replies: Mutex<HashMap<String, VecDeque<Reply>>>,
    pub(crate) calls: Mutex<Vec<(String, Value, Instant)>>,
    pub(crate) active: AtomicUsize,
    pub(crate) entered: Notify,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
pub(crate) fn runtime(scripts: Vec<(&str, Vec<Reply>)>) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &[PRIMARY.into(), BACKUP.into(), THIRD.into()],
        Fake {
            replies: Mutex::new(
                scripts
                    .into_iter()
                    .map(|(url, replies)| (url.into(), replies.into()))
                    .collect(),
            ),
            calls: Mutex::new(vec![]),
            active: AtomicUsize::new(0),
            entered: Notify::new(),
        },
    ))
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let body = request.test_body();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone(), Instant::now()));
        if body["method"] == "getLatestBlockhash" {
            return Ok(HttpReply::for_test(
                StatusCode::OK,
                serde_json::to_vec(&json!({
                    "jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":123},
                    "value":{"blockhash":bs58::encode([7;32]).into_string(),"lastValidBlockHeight":456}}
                }))?,
            ));
        }
        assert!(matches!(request, ReadRequest::SignatureStatus { .. }));
        let reply = self
            .replies
            .lock()
            .unwrap()
            .get_mut(url.as_str())
            .expect("unscripted endpoint")
            .pop_front()
            .expect("extra status request or retry");
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.entered.notify_one();
        let (result, fault) = match reply {
            Reply::Good(v) => (v, None),
            Reply::Wait(ms, v) => {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                (v, None)
            }
            Reply::Hold => {
                std::future::pending::<()>().await;
                unreachable!()
            }
            Reply::Late(ms, v) => {
                std::thread::sleep(Duration::from_millis(ms));
                (v, None)
            }
            Reply::Rate => return Ok(HttpReply::for_test(StatusCode::TOO_MANY_REQUESTS, vec![])),
            Reply::Fault(Fault::Transport) => bail!("private provider detail fixture-secret"),
            Reply::Fault(fault) => (found(), Some(fault)),
        };
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":result});
        match fault {
            Some(Fault::Id) => envelope["id"] = json!(999),
            Some(Fault::Version) => envelope["jsonrpc"] = json!("1.0"),
            Some(Fault::Both) => {
                envelope["error"] = json!({"code":-32005,"message":"private fixture-secret"})
            }
            Some(Fault::Missing) => {
                envelope.as_object_mut().unwrap().remove("result");
            }
            Some(Fault::Rpc) => {
                envelope = json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32005,"message":"private fixture-secret"}})
            }
            _ => {}
        }
        let mut bytes = serde_json::to_vec(&envelope)?;
        if matches!(fault, Some(Fault::Json)) {
            bytes = b"private fixture-secret".to_vec();
        }
        if matches!(fault, Some(Fault::Oversize)) {
            bytes.resize(65_537, b' ');
        }
        Ok(HttpReply::for_test(StatusCode::OK, bytes))
    }
}
fn lookup(r: Arc<SharedReadRuntime<Fake>>, urls: &[&str]) -> RpcQuoteStatusReader<Fake> {
    RpcQuoteStatusReader::new(r, urls.iter().map(|url| (*url).into()).collect())
}
pub(crate) fn released(r: &SharedReadRuntime<Fake>) {
    assert_eq!(
        r.slots
            .iter()
            .map(|s| s.available_permits())
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn quote_status_shared_original_observations_and_exact_params_all_finalities_errors_and_slots(
) {
    for commitment in ["processed", "confirmed", "finalized"] {
        for error in [Value::Null, json!({"InstructionError":[0,{"Custom":42}]})] {
            for slot in [0, 123, u64::MAX] {
                let mut item = status(commitment, error.clone());
                item["slot"] = json!(slot);
                let mut response = value(json!([item]));
                response["context"]["slot"] = json!(0);
                let old = Legacy::new(PRIMARY, response.clone());
                let original = RpcSignatureStatusReader {
                    rpcs: vec![old.client()],
                };
                let r = runtime(vec![(PRIMARY, vec![Reply::Good(response)])]);
                let actual = lookup(r.clone(), &[PRIMARY])
                    .lookup_signature(&signature())
                    .await
                    .unwrap();
                assert_eq!(
                    fields(actual),
                    fields(original.lookup_signature(&signature()).await.unwrap())
                );
                let calls = r.test_transport().calls.lock().unwrap();
                assert_eq!(calls.len(), 1);
                assert_eq!(*old.calls.lock().unwrap(), [calls[0].1["params"].clone()]);
                assert_eq!(calls[0].1["method"], "getSignatureStatuses");
                assert_eq!(
                    calls[0].1["params"],
                    json!([[signature()],{"searchTransactionHistory":true}])
                );
                drop(calls);
                released(&r);
            }
        }
    }
}

#[tokio::test]
async fn quote_status_shared_original_terminal_parser_errors_do_not_consult_backup() {
    for missing_commitment in [false, true] {
        let mut item = status("confirmed", Value::Null);
        if missing_commitment {
            item.as_object_mut().unwrap().remove("confirmationStatus");
        } else {
            item["err"] = json!({"InstructionError":[0,{"Custom":42}]});
        }
        let response = value(json!([item]));
        let old = Legacy::new(PRIMARY, response.clone());
        let original = RpcSignatureStatusReader {
            rpcs: vec![old.client()],
        };
        let r = runtime(vec![(PRIMARY, vec![Reply::Good(response)])]);
        assert_eq!(
            lookup(r.clone(), &[PRIMARY, BACKUP])
                .lookup_signature(&signature())
                .await
                .unwrap_err()
                .to_string(),
            original
                .lookup_signature(&signature())
                .await
                .unwrap_err()
                .to_string()
        );
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        released(&r);
    }
}

#[tokio::test]
async fn quote_status_shared_compatible_optional_err_and_nonrooted_finalized_fields_match_original()
{
    let mut item = status("finalized", Value::Null);
    item.as_object_mut().unwrap().remove("err");
    let response = value(json!([item]));
    let old = Legacy::new(PRIMARY, response.clone());
    let r = runtime(vec![(PRIMARY, vec![Reply::Good(response)])]);
    assert_eq!(
        fields(
            lookup(r, &[PRIMARY])
                .lookup_signature(&signature())
                .await
                .unwrap()
        ),
        fields(
            RpcSignatureStatusReader {
                rpcs: vec![old.client()]
            }
            .lookup_signature(&signature())
            .await
            .unwrap()
        )
    );
}

#[tokio::test]
async fn quote_status_shared_missing_and_invalid_count_preserve_original_backup_order_and_limit() {
    for first in [
        value(json!([null])),
        value(json!([])),
        value(json!([null, null])),
    ] {
        let old_first = Legacy::new(PRIMARY, first.clone());
        let old_second = Legacy::new(BACKUP, found());
        let old_third = Legacy::new(THIRD, found());
        let r = runtime(vec![
            (PRIMARY, vec![Reply::Good(first)]),
            (BACKUP, vec![Reply::Good(found())]),
        ]);
        let original = RpcSignatureStatusReader {
            rpcs: vec![old_first.client(), old_second.client(), old_third.client()],
        };
        assert_eq!(
            fields(
                lookup(r.clone(), &[PRIMARY, BACKUP, THIRD])
                    .lookup_signature(&signature())
                    .await
                    .unwrap()
            ),
            fields(original.lookup_signature(&signature()).await.unwrap())
        );
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(
            calls.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
            [PRIMARY, BACKUP]
        );
        assert_eq!(
            *old_first.calls.lock().unwrap(),
            [calls[0].1["params"].clone()]
        );
        assert_eq!(
            *old_second.calls.lock().unwrap(),
            [calls[1].1["params"].clone()]
        );
        assert!(old_third.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn quote_status_shared_any_missing_wins_over_later_transport_or_invalid_count_without_a_third_attempt(
) {
    for reverse in [false, true] {
        let responses = if reverse {
            [value(json!([])), value(json!([null]))]
        } else {
            [value(json!([null])), value(json!([]))]
        };
        let r = runtime(vec![
            (PRIMARY, vec![Reply::Good(responses[0].clone())]),
            (BACKUP, vec![Reply::Good(responses[1].clone())]),
        ]);
        let old_a = Legacy::new(PRIMARY, responses[0].clone());
        let old_b = Legacy::new(BACKUP, responses[1].clone());
        assert!(lookup(r.clone(), &[PRIMARY, BACKUP, THIRD])
            .lookup_signature(&signature())
            .await
            .unwrap()
            .is_none());
        assert!(RpcSignatureStatusReader {
            rpcs: vec![old_a.client(), old_b.client()]
        }
        .lookup_signature(&signature())
        .await
        .unwrap()
        .is_none());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
    let r = runtime(vec![
        (PRIMARY, vec![Reply::Good(value(json!([null])))]),
        (BACKUP, vec![Reply::Fault(Fault::Transport)]),
    ]);
    assert!(lookup(r.clone(), &[PRIMARY, BACKUP])
        .lookup_signature(&signature())
        .await
        .unwrap()
        .is_none());
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_invalid_signature_empty_lanes_and_unregistered_endpoints_before_io() {
    let r = runtime(vec![]);
    let original = RpcSignatureStatusReader { rpcs: vec![] };
    for input in ["invalid", &signature()] {
        assert_eq!(
            lookup(r.clone(), &[])
                .lookup_signature(input)
                .await
                .unwrap_err()
                .to_string(),
            original
                .lookup_signature(input)
                .await
                .unwrap_err()
                .to_string()
        );
    }
    for endpoint in [
        "bad-url",
        "file:///etc/hosts",
        "https://unregistered.example.invalid",
    ] {
        assert!(lookup(r.clone(), &[endpoint])
            .lookup_signature(&signature())
            .await
            .is_err());
    }
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_last_invalid_count_error_and_nonmissing_faults_fail_closed_and_redacted(
) {
    for fault in [
        Fault::Id,
        Fault::Version,
        Fault::Both,
        Fault::Missing,
        Fault::Json,
        Fault::Oversize,
        Fault::Rpc,
        Fault::Transport,
    ] {
        let r = runtime(vec![(PRIMARY, vec![Reply::Fault(fault)])]);
        let error = lookup(r.clone(), &[PRIMARY])
            .lookup_signature(&signature())
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        released(&r);
    }
    let r = runtime(vec![
        (PRIMARY, vec![Reply::Fault(Fault::Transport)]),
        (BACKUP, vec![Reply::Good(value(json!([])))]),
    ]);
    assert_eq!(
        lookup(r.clone(), &[PRIMARY, BACKUP])
            .lookup_signature(&signature())
            .await
            .unwrap_err()
            .to_string(),
        "signature status RPC returned an invalid result count"
    );
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_fast_missing_primary_keeps_backup_full_io_budget_after_shared_pacing()
{
    let r = runtime(vec![
        (PRIMARY, vec![Reply::Good(value(json!([null])))]),
        (BACKUP, vec![Reply::Wait(700, found())]),
    ]);
    let start = Instant::now();
    assert!(lookup(r.clone(), &[PRIMARY, BACKUP])
        .lookup_signature(&signature())
        .await
        .unwrap()
        .is_some());
    let calls = r.test_transport().calls.lock().unwrap();
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(990));
    assert!(
        start.elapsed() >= Duration::from_millis(1600)
            && start.elapsed() < Duration::from_millis(2700)
    );
    drop(calls);
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_primary_timeout_keeps_backup_full_io_budget_without_hidden_retries() {
    let r = runtime(vec![
        (PRIMARY, vec![Reply::Hold]),
        (BACKUP, vec![Reply::Wait(700, found())]),
    ]);
    let start = Instant::now();
    assert!(lookup(r.clone(), &[PRIMARY, BACKUP, THIRD])
        .lookup_signature(&signature())
        .await
        .unwrap()
        .is_some());
    assert!(
        start.elapsed() >= Duration::from_millis(1600)
            && start.elapsed() < Duration::from_millis(2700)
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_quota_is_global_with_wsol_and_ordinary_observation_but_not_hash() {
    let r = runtime(vec![(PRIMARY, vec![Reply::Hold])]);
    let reader = lookup(r.clone(), &[PRIMARY, BACKUP]);
    let sig = signature();
    let mut pending = Box::pin(reader.lookup_signature(&sig));
    tokio::select! { result=&mut pending=>panic!("unexpected {result:?}"),_=r.test_transport().entered.notified()=>{} }
    assert!(lookup(r.clone(), &[BACKUP])
        .lookup_signature(&sig)
        .await
        .is_err());
    assert!(r
        .wsol_signature_status(PRIMARY, &sig.parse().unwrap())
        .await
        .is_err());
    assert!(r
        .signature_status(&Url::parse(PRIMARY).unwrap(), 99, &sig)
        .await
        .is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    drop(pending);
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_cancel_unpolled_and_during_io_or_pacing_releases_all_original_slots() {
    let r = runtime(vec![(PRIMARY, vec![Reply::Hold, Reply::Good(found())])]);
    let reader = lookup(r.clone(), &[PRIMARY]);
    let sig = signature();
    drop(reader.lookup_signature(&sig));
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
    let mut pending = Box::pin(reader.lookup_signature(&sig));
    tokio::select! { result=&mut pending=>panic!("unexpected {result:?}"),_=r.test_transport().entered.notified()=>{} }
    drop(pending);
    released(&r);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), reader.lookup_signature(&sig))
            .await
            .is_err()
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    released(&r);
    assert!(reader.lookup_signature(&sig).await.unwrap().is_some());
    let calls = r.test_transport().calls.lock().unwrap();
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(990));
    drop(calls);
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_late_response_rejected_and_expired_operation_never_queries_backup() {
    for ms in [1100, 3100] {
        let mut backup = found();
        backup["value"][0]["slot"] = json!(999);
        let r = runtime(vec![
            (PRIMARY, vec![Reply::Late(ms, found())]),
            (BACKUP, vec![Reply::Good(backup)]),
        ]);
        let outcome = lookup(r.clone(), &[PRIMARY, BACKUP])
            .lookup_signature(&signature())
            .await;
        if ms == 1100 {
            assert_eq!(outcome.unwrap().unwrap().slot, 999);
            assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
        } else {
            assert_eq!(
                outcome.unwrap_err().to_string(),
                "signature status RPC timed out"
            );
            assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        }
        released(&r);
    }
}

#[tokio::test]
async fn quote_status_shared_rate_limit_falls_back_and_cools_other_profiles_without_new_clients() {
    let r = runtime(vec![
        (PRIMARY, vec![Reply::Rate]),
        (BACKUP, vec![Reply::Good(found())]),
    ]);
    assert!(lookup(r.clone(), &[PRIMARY, BACKUP])
        .lookup_signature(&signature())
        .await
        .unwrap()
        .is_some());
    assert!(r
        .wsol_signature_status(PRIMARY, &signature().parse().unwrap())
        .await
        .is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(
        calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        [PRIMARY, BACKUP, BACKUP]
    );
    drop(calls);
    released(&r);
}

#[tokio::test]
async fn quote_status_shared_alias_order_and_unknown_primary_preserve_exact_original_two_positions()
{
    let alias = "https://PRIMARY.example.invalid:443/?key=fixture-secret";
    let r = runtime(vec![(
        PRIMARY,
        vec![Reply::Good(value(json!([null]))), Reply::Good(found())],
    )]);
    assert!(lookup(r.clone(), &[alias, PRIMARY, THIRD])
        .lookup_signature(&signature())
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        r.test_transport()
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.0.as_str())
            .collect::<Vec<_>>(),
        [PRIMARY, PRIMARY]
    );
    let r = runtime(vec![(BACKUP, vec![Reply::Good(found())])]);
    assert!(lookup(
        r.clone(),
        &["https://unknown.example.invalid", BACKUP, THIRD]
    )
    .lookup_signature(&signature())
    .await
    .unwrap()
    .is_some());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    released(&r);
}
