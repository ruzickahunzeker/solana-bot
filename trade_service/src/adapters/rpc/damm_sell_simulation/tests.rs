use super::*;
use serde_json::{json, Value};
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;

#[derive(Clone)]
pub(crate) struct MemorySimulation {
    pub(crate) response: Value,
    pub(crate) fail: bool,
    pub(crate) block: bool,
    pub(crate) delay: Option<Duration>,
    pub(crate) calls: Arc<Mutex<Vec<Value>>>,
    pub(crate) entered: Arc<Notify>,
    pub(crate) dropped: Arc<AtomicBool>,
}

impl MemorySimulation {
    pub(crate) fn new() -> Self {
        Self {
            response: json!({"context":{"slot":123},"value":{"err":null,"logs":[],"accounts":null,"unitsConsumed":1000,"returnData":null,"innerInstructions":[]}}),
            fail: false,
            block: false,
            delay: None,
            calls: Arc::default(),
            entered: Arc::new(Notify::new()),
            dropped: Arc::default(),
        }
    }
    pub(crate) fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
    }
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl RpcSender for MemorySimulation {
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
            assert_eq!(
                request,
                RpcRequest::SimulateTransaction,
                "unsigned source must never submit, confirm or query another method"
            );
            self.calls.lock().unwrap().push(params);
            let _flag = DropFlag(self.dropped.clone());
            self.entered.notify_one();
            if self.block {
                std::future::pending::<()>().await;
            }
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            if self.fail {
                return Err(ClientErrorKind::Custom(
                    "https://fixture.invalid/?key=simulation-secret".into(),
                )
                .into());
            }
            Ok(self.response.clone())
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://unsigned-damm-simulation".into()
    }
}

fn transaction() -> VersionedTransaction {
    let wallet = Pubkey::new_from_array([1; 32]);
    VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::Legacy(Message::new_with_blockhash(
            &[Instruction {
                program_id: Pubkey::new_from_array([2; 32]),
                accounts: vec![AccountMeta::new_readonly(wallet, true)],
                data: vec![7],
            }],
            Some(&wallet),
            &Hash::new_from_array([9; 32]),
        )),
    }
}

#[tokio::test]
async fn damm_unsigned_simulation_preserves_original_rpc_config_with_only_zero_signature() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let source = MemorySimulation::new();
    let transaction = transaction();
    let result = simulate_unsigned(&source.client(), &transaction)
        .await
        .unwrap();
    assert!(result.error.is_none());
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let config = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(CommitmentConfig::processed()),
        encoding: Some(UiTransactionEncoding::Base64),
        accounts: None,
        min_context_slot: None,
        inner_instructions: true,
    };
    assert_eq!(calls[0][1], serde_json::to_value(config).unwrap());
    let bytes = STANDARD.decode(calls[0][0].as_str().unwrap()).unwrap();
    let decoded: VersionedTransaction = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded.signatures, [Signature::default()]);
    assert_eq!(decoded.message, transaction.message);
    assert_eq!(bytes, bincode::serialize(&transaction).unwrap());
}

#[tokio::test]
async fn damm_unsigned_simulation_rejects_signed_missing_oversize_or_invalid_candidates_before_rpc()
{
    for change in 0..6 {
        let source = MemorySimulation::new();
        let mut tx = transaction();
        match change {
            0 => tx.signatures = vec![Signature::from([7; 64])],
            1 => tx.signatures.clear(),
            2 => tx.signatures.push(Signature::default()),
            3 => {
                let VersionedMessage::Legacy(message) = &mut tx.message else {
                    unreachable!()
                };
                message.recent_blockhash = Hash::default();
            }
            4 => {
                let VersionedMessage::Legacy(message) = &mut tx.message else {
                    unreachable!()
                };
                message.instructions[0].data.resize(1400, 1);
            }
            5 => {
                let VersionedMessage::Legacy(message) = &mut tx.message else {
                    unreachable!()
                };
                message.header.num_required_signatures = 2;
            }
            _ => unreachable!(),
        }
        assert!(simulate_unsigned(&source.client(), &tx).await.is_err());
        assert!(source.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn damm_unsigned_simulation_chain_rejection_is_not_success_or_a_transaction_signature() {
    let mut source = MemorySimulation::new();
    source.response["value"]["err"] = json!({"InstructionError":[2,{"Custom":6001}]});
    let result = simulate_unsigned(&source.client(), &transaction())
        .await
        .unwrap();
    assert!(result.error.unwrap().contains("6001"));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_unsigned_simulation_rejects_zero_context_and_malformed_responses() {
    for response in [
        json!({"context":{"slot":0},"value":{"err":null,"logs":[]}}),
        json!({"unexpected":"response"}),
    ] {
        let mut source = MemorySimulation::new();
        source.response = response;
        assert!(simulate_unsigned(&source.client(), &transaction())
            .await
            .is_err());
        assert_eq!(source.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn damm_unsigned_simulation_rpc_failure_is_redacted_without_fallback() {
    let mut source = MemorySimulation::new();
    source.fail = true;
    let error = simulate_unsigned(&source.client(), &transaction())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("simulation-secret"));
    assert!(!error.contains("https://"));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_unsigned_simulation_real_four_second_timeout_cancels_the_logical_rpc() {
    let mut source = MemorySimulation::new();
    source.block = true;
    let start = std::time::Instant::now();
    let error = simulate_unsigned(&source.client(), &transaction())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("timed out"));
    assert!(start.elapsed() >= Duration::from_secs(4));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn damm_unsigned_simulation_cancellation_does_not_start_another_rpc() {
    let mut source = MemorySimulation::new();
    source.block = true;
    let rpc = source.client();
    let task = tokio::spawn(async move { simulate_unsigned(&rpc, &transaction()).await });
    source.entered.notified().await;
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}
