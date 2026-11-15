use super::*;
use crate::domain::reconciliation::ChainCommitment;
use serde_json::{json, Value};
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_sdk::signature::Signature;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

#[derive(Clone)]
struct MemoryRpc {
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    values: Arc<Vec<Value>>,
    index: Arc<AtomicUsize>,
    fail: bool,
    block: bool,
    dropped: Arc<AtomicBool>,
}
struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl MemoryRpc {
    fn new(values: Vec<Value>) -> Self {
        Self {
            calls: Arc::default(),
            values: Arc::new(values),
            index: Arc::default(),
            fail: false,
            block: false,
            dropped: Arc::default(),
        }
    }
    fn client(&self) -> Arc<RpcClient> {
        Arc::new(RpcClient::new_sender(
            self.clone(),
            RpcClientConfig::default(),
        ))
    }
}
impl RpcSender for MemoryRpc {
    fn send<'life0, 'async_trait>(
        &'life0 self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ClientError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.calls.lock().unwrap().push((request, params));
            if request != RpcRequest::GetSignatureStatuses {
                return Err(ClientErrorKind::Custom(format!(
                    "forbidden SDK observer RPC: {request}"
                ))
                .into());
            }
            if self.block {
                let _drop = DropFlag(self.dropped.clone());
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(ClientErrorKind::Custom(
                    "RPC https://fixture.invalid/?key=secret failed".into(),
                )
                .into());
            }
            let index = self
                .index
                .fetch_add(1, Ordering::SeqCst)
                .min(self.values.len() - 1);
            Ok(json!({"context":{"slot":55},"value":self.values[index]}))
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://sdk-signature-observer".into()
    }
}
fn signature() -> String {
    Signature::from([11; 64]).to_string()
}
fn status(commitment: &str) -> Value {
    json!({"slot":55,"confirmations":1,"status":{"Ok":null},"err":null,"confirmationStatus":commitment})
}
fn assert_readonly(rpc: &MemoryRpc) {
    for (request, params) in rpc.calls.lock().unwrap().iter() {
        assert_eq!(*request, RpcRequest::GetSignatureStatuses);
        assert_eq!(
            *params,
            json!([[signature()], {"searchTransactionHistory":true}])
        );
    }
}

#[tokio::test]
async fn observes_original_signature_with_explicit_commitment_and_history_only() {
    for (name, expected) in [
        ("processed", ChainCommitment::Processed),
        ("confirmed", ChainCommitment::Confirmed),
        ("finalized", ChainCommitment::Finalized),
    ] {
        let rpc = MemoryRpc::new(vec![json!([status(name)])]);
        let observation = observe_sdk_signature(rpc.client(), &signature(), 1_000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observation.signature, signature());
        assert_eq!(observation.slot, 55);
        assert_eq!(observation.commitment, expected);
        assert_eq!(observation.error, None);
        assert_readonly(&rpc);
        assert_eq!(rpc.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn missing_status_polls_same_signature_then_returns_observation() {
    let rpc = MemoryRpc::new(vec![json!([null]), json!([status("confirmed")])]);
    assert!(observe_sdk_signature(rpc.client(), &signature(), 1_000)
        .await
        .unwrap()
        .is_some());
    assert_eq!(rpc.calls.lock().unwrap().len(), 2);
    assert_readonly(&rpc);
}

#[tokio::test]
async fn absent_or_blocked_status_uses_total_deadline_without_send_or_failover() {
    for block in [false, true] {
        let mut rpc = MemoryRpc::new(vec![json!([null])]);
        rpc.block = block;
        assert!(observe_sdk_signature(rpc.client(), &signature(), 20)
            .await
            .unwrap()
            .is_none());
        assert_eq!(rpc.calls.lock().unwrap().len(), 1);
        assert_eq!(rpc.dropped.load(Ordering::SeqCst), block);
        assert_readonly(&rpc);
    }
}

#[tokio::test]
async fn zero_budget_and_invalid_signature_make_no_rpc_call() {
    let rpc = MemoryRpc::new(vec![json!([null])]);
    assert!(observe_sdk_signature(rpc.client(), "invalid", 0)
        .await
        .unwrap()
        .is_none());
    assert!(observe_sdk_signature(rpc.client(), "invalid", 500)
        .await
        .is_err());
    assert!(rpc.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn malformed_metadata_zero_slot_and_transport_errors_fail_closed_and_redact() {
    let mut missing_commitment = status("confirmed");
    missing_commitment
        .as_object_mut()
        .unwrap()
        .remove("confirmationStatus");
    let mut zero_slot = status("confirmed");
    zero_slot["slot"] = json!(0);
    let mut inconsistent = status("confirmed");
    inconsistent["err"] = json!({"InstructionError":[0,{"Custom":42}]});
    for value in [
        json!([]),
        json!([null, null]),
        json!([missing_commitment]),
        json!([zero_slot]),
        json!([inconsistent]),
    ] {
        let rpc = MemoryRpc::new(vec![value]);
        assert!(observe_sdk_signature(rpc.client(), &signature(), 500)
            .await
            .is_err());
        assert_eq!(rpc.calls.lock().unwrap().len(), 1);
        assert_readonly(&rpc);
    }
    let mut rpc = MemoryRpc::new(vec![json!([null])]);
    rpc.fail = true;
    let error = observe_sdk_signature(rpc.client(), &signature(), 500)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("secret"));
    assert_eq!(rpc.calls.lock().unwrap().len(), 1);
    assert_readonly(&rpc);
}

#[tokio::test]
async fn explicit_onchain_error_is_preserved_as_observation_not_an_automatic_retry() {
    let mut value = status("processed");
    value["err"] = json!({"InstructionError":[0,{"Custom":42}]});
    value["status"] = json!({"Err":value["err"]});
    let rpc = MemoryRpc::new(vec![json!([value])]);
    let observation = observe_sdk_signature(rpc.client(), &signature(), 500)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observation.signature, signature());
    assert!(observation.error.unwrap().contains("42"));
    assert_readonly(&rpc);
}

#[tokio::test]
async fn caller_cancellation_drops_only_the_local_read_and_does_not_send() {
    let mut rpc = MemoryRpc::new(vec![json!([null])]);
    rpc.block = true;
    assert!(tokio::time::timeout(
        Duration::from_millis(20),
        observe_sdk_signature(rpc.client(), &signature(), 10_000)
    )
    .await
    .is_err());
    assert!(rpc.dropped.load(Ordering::SeqCst));
    assert_eq!(rpc.calls.lock().unwrap().len(), 1);
    assert_readonly(&rpc);
}
