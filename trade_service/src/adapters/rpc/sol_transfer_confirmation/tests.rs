use super::*;
use crate::adapters::rpc::read_http::{HttpReply, ReadRequest};
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use solana_client::{
    client_error::Result as RpcResult,
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::Notify;

const URL: &str = "http://native-confirm.invalid/primary";
const BACKUP: &str = "http://native-confirm.invalid/backup";
const BUDGET: Duration = Duration::from_secs(15);

fn signature() -> Signature {
    Signature::from([7; 64])
}

fn status(
    commitment: Option<&str>,
    confirmations: Value,
    legacy_error: bool,
    explicit_error: bool,
    slot: u64,
) -> Value {
    let error = json!({"InstructionError":[0,{"Custom":7}]});
    json!({"slot":slot,"confirmations":confirmations,"confirmationStatus":commitment,
        "err":if explicit_error {error.clone()} else {Value::Null},
        "status":if legacy_error {json!({"Err":error})} else {json!({"Ok":null})}})
}

fn result(value: Value, context: u64) -> Value {
    json!({"context":{"slot":context},"value":value})
}

#[derive(Default)]
struct Trace {
    calls: Mutex<Vec<(String, Value, Instant)>>,
    raw: Mutex<Vec<(RpcRequest, Value)>>,
    entered: Notify,
    dropped: AtomicUsize,
}
struct DropRead(Arc<Trace>);
impl Drop for DropRead {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct Fake {
    result: Value,
    fault: u8,
    trace: Arc<Trace>,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            result: result(
                json!([status(Some("confirmed"), json!(1), false, false, 42)]),
                42,
            ),
            fault: 0,
            trace: Arc::default(),
        }
    }
}
impl Fake {
    fn runtime(&self) -> SharedReadRuntime<Self> {
        SharedReadRuntime::with_test_transport(&[URL.into(), BACKUP.into()], self.clone())
    }
    fn reader<'a>(
        &self,
        runtime: &'a SharedReadRuntime<Self>,
        commitment: CommitmentConfig,
    ) -> SolTransferConfirmationReader<'a, Self> {
        SolTransferConfirmationReader::new(runtime, URL.into(), signature(), commitment)
    }
    fn calls(&self) -> Vec<(String, Value, Instant)> {
        self.trace.calls.lock().unwrap().clone()
    }
    async fn original(&self, commitment: CommitmentConfig) -> bool {
        RpcClient::new_sender(
            self.clone(),
            RpcClientConfig {
                commitment_config: commitment,
                ..Default::default()
            },
        )
        .confirm_transaction(&signature())
        .await
        .unwrap_or(false)
    }
}
impl RpcSender for Fake {
    fn send<'life0, 'async_trait>(
        &'life0 self,
        method: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = RpcResult<Value>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            assert_eq!(method, RpcRequest::GetSignatureStatuses);
            self.trace.raw.lock().unwrap().push((method, params));
            Ok(self.result.clone())
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats {
            request_count: self.trace.raw.lock().unwrap().len(),
            ..Default::default()
        }
    }
    fn url(&self) -> String {
        URL.into()
    }
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        assert!(matches!(
            request,
            ReadRequest::SolTransferConfirmation { .. }
                | ReadRequest::SignatureStatus { .. }
                | ReadRequest::LatestBlockhash { .. }
        ));
        let body = request.body();
        self.trace
            .calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone(), Instant::now()));
        let _drop = DropRead(self.trace.clone());
        self.trace.entered.notify_one();
        let native = matches!(request, ReadRequest::SolTransferConfirmation { .. });
        let fault = if native { self.fault } else { 0 };
        match fault {
            1 => anyhow::bail!("http://provider.invalid/private-fixture-error"),
            2 => return Ok(HttpReply::for_test(StatusCode::FOUND, vec![])),
            3 => return Ok(HttpReply::for_test(StatusCode::SERVICE_UNAVAILABLE, vec![])),
            4 => {
                return Ok(HttpReply::for_test(
                    StatusCode::OK,
                    vec![b' '; MAX_BODY + 1],
                ))
            }
            5 => return Ok(HttpReply::for_test(StatusCode::OK, b"not JSON".to_vec())),
            9 => {
                return Ok(HttpReply {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    retry_after: Some(60),
                    body: vec![],
                })
            }
            10 => return std::future::pending().await,
            11 => std::thread::sleep(Duration::from_millis(250)),
            _ => {}
        }
        let value = if matches!(request, ReadRequest::LatestBlockhash { .. }) {
            json!({"context":{"slot":42},"value":{"blockhash":bs58::encode([6;32]).into_string(),"lastValidBlockHeight":999}})
        } else {
            self.result.clone()
        };
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":value});
        match fault {
            6 => envelope["id"] = json!("wrong"),
            7 => envelope["jsonrpc"] = json!("1.0"),
            8 => envelope["error"] = json!({"code":-1,"message":"public fault"}),
            _ => {}
        }
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&envelope).unwrap(),
        ))
    }
}

#[tokio::test]
async fn sol_transfer_confirm_read_single_original_signature_request_has_no_history_flag() {
    let fake = Fake::default();
    let runtime = fake.runtime();
    assert!(fake
        .reader(&runtime, CommitmentConfig::processed())
        .confirm(BUDGET)
        .await
        .unwrap());
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, URL);
    assert_eq!(
        calls[0].1,
        json!({"jsonrpc":"2.0","id":1,"method":"getSignatureStatuses","params":[[signature().to_string()]]})
    );
    assert!(fake.original(CommitmentConfig::processed()).await);
    assert_eq!(
        *fake.trace.raw.lock().unwrap(),
        [(
            RpcRequest::GetSignatureStatuses,
            calls[0].1["params"].clone()
        )]
    );
}

#[tokio::test]
async fn sol_transfer_confirm_read_legacy_commitment_status_matrix_matches_pinned_rpc() {
    let mut checked = 0;
    for commitment in [
        CommitmentConfig::processed(),
        CommitmentConfig::confirmed(),
        CommitmentConfig::finalized(),
    ] {
        for level in [
            None,
            Some("processed"),
            Some("confirmed"),
            Some("finalized"),
        ] {
            for confirmations in [json!(0), json!(1), json!(2), Value::Null] {
                for legacy_error in [false, true] {
                    for explicit_error in [false, true] {
                        for context in [0, 42] {
                            for slot in [0, 42] {
                                let fake = Fake {
                                    result: result(
                                        json!([status(
                                            level,
                                            confirmations.clone(),
                                            legacy_error,
                                            explicit_error,
                                            slot
                                        )]),
                                        context,
                                    ),
                                    ..Default::default()
                                };
                                let expected = fake.original(commitment).await;
                                let runtime = fake.runtime();
                                let actual = fake
                                    .reader(&runtime, commitment)
                                    .confirm(BUDGET)
                                    .await
                                    .unwrap();
                                assert_eq!(actual,expected,"{commitment:?}/{level:?}/{confirmations}/{legacy_error}/{explicit_error}/{context}/{slot}");
                                assert_eq!(fake.calls().len(), 1);
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 768);
}

#[tokio::test]
async fn sol_transfer_confirm_read_missing_optional_legacy_metadata_is_not_forced_to_wsol_policy() {
    for commitment in [
        CommitmentConfig::processed(),
        CommitmentConfig::confirmed(),
        CommitmentConfig::finalized(),
    ] {
        let mut legacy = status(None, Value::Null, false, false, 0);
        legacy.as_object_mut().unwrap().remove("confirmationStatus");
        legacy.as_object_mut().unwrap().remove("err");
        let fake = Fake {
            result: result(json!([legacy]), 0),
            ..Default::default()
        };
        let expected = fake.original(commitment).await;
        let runtime = fake.runtime();
        assert_eq!(
            fake.reader(&runtime, commitment)
                .confirm(BUDGET)
                .await
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn sol_transfer_confirm_read_bad_rpc_identity_body_transport_or_redirect_never_fallback() {
    for fault in 1..=8 {
        let fake = Fake {
            fault,
            ..Default::default()
        };
        let runtime = fake.runtime();
        let error = fake
            .reader(&runtime, CommitmentConfig::processed())
            .confirm(BUDGET)
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("provider.invalid"));
        let calls = fake.calls();
        assert_eq!(calls.len(), 1, "fault {fault}");
        assert_eq!(calls[0].0, URL);
    }
}

#[tokio::test]
async fn sol_transfer_confirm_read_empty_or_extra_results_fail_closed_without_sdk_index_panic() {
    for values in [
        json!([]),
        json!([null, null]),
        json!([status(Some("confirmed"), json!(1), false, false, 42), null]),
    ] {
        let fake = Fake {
            result: result(values, 42),
            ..Default::default()
        };
        let runtime = fake.runtime();
        assert!(fake
            .reader(&runtime, CommitmentConfig::processed())
            .confirm(BUDGET)
            .await
            .is_err());
        assert_eq!(fake.calls().len(), 1);
    }
}

#[tokio::test]
async fn sol_transfer_confirm_read_invalid_signature_budget_and_unregistered_endpoint_do_no_io() {
    let fake = Fake::default();
    let runtime = fake.runtime();
    assert!(SolTransferConfirmationReader::new(
        &runtime,
        URL.into(),
        Signature::default(),
        CommitmentConfig::processed()
    )
    .confirm(BUDGET)
    .await
    .is_err());
    for budget in [Duration::ZERO, Duration::from_secs(16)] {
        assert!(fake
            .reader(&runtime, CommitmentConfig::processed())
            .confirm(budget)
            .await
            .is_err());
    }
    for url in ["invalid", "http://unregistered.invalid/"] {
        assert!(SolTransferConfirmationReader::new(
            &runtime,
            url.into(),
            signature(),
            CommitmentConfig::processed()
        )
        .confirm(BUDGET)
        .await
        .is_err());
    }
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn sol_transfer_confirm_read_cancel_shares_observation_permit_but_hash_remains_independent() {
    let fake = Fake {
        fault: 10,
        ..Default::default()
    };
    let runtime = fake.runtime();
    let mut work = Box::pin(
        fake.reader(&runtime, CommitmentConfig::processed())
            .confirm(BUDGET),
    );
    tokio::select! { value=&mut work=>panic!("pending confirmation completed: {value:?}"),_ = fake.trace.entered.notified()=>{} }
    assert!(runtime
        .wsol_signature_status(URL, &signature())
        .await
        .is_err());
    assert!(runtime.latest_blockhash().await.is_ok());
    drop(work);
    assert!(runtime
        .wsol_signature_status(URL, &signature())
        .await
        .unwrap()
        .is_some());
    assert_eq!(fake.calls().len(), 3);
    assert_eq!(fake.trace.dropped.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn sol_transfer_confirm_read_rate_cools_existing_status_profiles_without_retry() {
    let fake = Fake {
        fault: 9,
        ..Default::default()
    };
    let runtime = fake.runtime();
    assert!(fake
        .reader(&runtime, CommitmentConfig::processed())
        .confirm(BUDGET)
        .await
        .is_err());
    assert!(runtime
        .wsol_signature_status(URL, &signature())
        .await
        .is_err());
    assert!(fake
        .reader(&runtime, CommitmentConfig::processed())
        .confirm(BUDGET)
        .await
        .is_err());
    assert_eq!(fake.calls().len(), 1);
    assert!(runtime.latest_blockhash().await.is_ok());
    assert_eq!(fake.calls()[1].0, BACKUP);
}

#[tokio::test]
async fn sol_transfer_confirm_read_pacing_and_ids_reuse_existing_observation_start_budget() {
    let fake = Fake::default();
    let runtime = fake.runtime();
    let started = Instant::now();
    for _ in 0..2 {
        assert!(fake
            .reader(&runtime, CommitmentConfig::processed())
            .confirm(BUDGET)
            .await
            .unwrap());
    }
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1["id"], 1);
    assert_eq!(calls[1].1["id"], 2);
    assert!(calls[1].2.duration_since(started) >= Duration::from_secs(1));
}

#[tokio::test]
async fn sol_transfer_confirm_read_original_fifteen_second_budget_cancels_one_read_not_polling() {
    let fake = Fake {
        fault: 10,
        ..Default::default()
    };
    let runtime = fake.runtime();
    let started = Instant::now();
    let error = fake
        .reader(&runtime, CommitmentConfig::processed())
        .confirm(BUDGET)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() >= BUDGET);
    assert!(started.elapsed() < Duration::from_secs(18));
    assert_eq!(fake.calls().len(), 1);
    assert_eq!(fake.trace.dropped.load(Ordering::SeqCst), 1);
    assert!(runtime
        .wsol_signature_status(URL, &signature())
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn sol_transfer_confirm_read_late_ready_response_cannot_escape_original_caller_budget() {
    let fake = Fake {
        fault: 11,
        ..Default::default()
    };
    let runtime = fake.runtime();
    assert!(fake
        .reader(&runtime, CommitmentConfig::processed())
        .confirm(Duration::from_millis(100))
        .await
        .is_err());
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn sol_transfer_confirm_read_missing_or_invalid_typed_status_is_never_fabricated_success() {
    for values in [
        json!([null]),
        json!([{"slot":42,"confirmations":1,"confirmationStatus":"unknown","status":{"Ok":null},"err":null}]),
        json!([{"slot":42,"confirmations":1,"confirmationStatus":"confirmed","err":null}]),
    ] {
        let fake = Fake {
            result: result(values, 42),
            ..Default::default()
        };
        let expected = fake.original(CommitmentConfig::processed()).await;
        let runtime = fake.runtime();
        let actual = fake
            .reader(&runtime, CommitmentConfig::processed())
            .confirm(BUDGET)
            .await
            .unwrap_or(false);
        assert_eq!(actual, expected);
        assert!(!actual);
        assert_eq!(fake.calls().len(), 1);
    }
}
