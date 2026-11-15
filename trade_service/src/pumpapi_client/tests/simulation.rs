//! Fixed public messages and a socket-free sender; no Keypair or Signer is constructed.
use super::*;
use crate::application::trade_execution::classify_result;
use crate::domain::trade::TradeStatus;
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}

#[derive(Clone)]
struct PreviewRpc {
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    response: Value,
    error: Option<String>,
    gate: Option<Arc<Gate>>,
}
impl PreviewRpc {
    fn new(err: Value) -> Self {
        Self {
            calls: Arc::new(Mutex::new(vec![])),
            response: json!({"context":{"slot":42}, "value": {
                "err":err, "logs":[], "accounts":null, "unitsConsumed":10,
                "returnData":null, "innerInstructions":[], "replacementBlockhash":null
            }}),
            error: None,
            gate: None,
        }
    }
    fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
    }
    fn assert_calls(&self, count: usize) -> Vec<Value> {
        let calls = self.calls.lock().unwrap();
        assert_eq!(calls.len(), count);
        assert!(calls
            .iter()
            .all(|(method, _)| *method == RpcRequest::SimulateTransaction));
        calls.iter().map(|(_, params)| params.clone()).collect()
    }
}
impl RpcSender for PreviewRpc {
    fn send<'life0, 'async_trait>(
        &'life0 self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ClientError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.calls.lock().unwrap().push((request, params));
            if request != RpcRequest::SimulateTransaction {
                return Err(ClientErrorKind::Custom("forbidden preview RPC".into()).into());
            }
            if let Some(gate) = &self.gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            if let Some(error) = &self.error {
                return Err(ClientErrorKind::Custom(error.clone()).into());
            }
            Ok(self.response.clone())
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats {
            request_count: self.calls.lock().unwrap().len(),
            ..Default::default()
        }
    }
    fn url(&self) -> String {
        "memory://pumpapi-preview".into()
    }
}

fn wallet() -> Address {
    Address::from_bytes([1; 32])
}

#[tokio::test]
async fn pumpapi_h2a_direct_unsigned_rpc_boundary_does_not_bypass_packet_limit() {
    for versioned in [false, true] {
        let transport = PreviewRpc::new(Value::Null);
        let mut tx = transaction(versioned, 7);
        match &mut tx.message {
            VersionedMessage::Legacy(message) => message.instructions[0].data = vec![7; 1232],
            VersionedMessage::V0(message) => message.instructions[0].data = vec![7; 1232],
            _ => unreachable!(),
        }
        assert_eq!(
            simulate_unsigned(&transport.client(), wallet(), tx)
                .await
                .err()
                .unwrap()
                .to_string(),
            "PumpAPI transaction exceeds 1232 bytes"
        );
        transport.assert_calls(0);
    }
}

#[tokio::test]
async fn pumpapi_h2a_unsigned_rpc_keeps_exact_message_zero_slots_and_original_config() {
    for versioned in [false, true] {
        let transport = PreviewRpc::new(Value::Null);
        let original = transaction(versioned, b' ');
        let bytes = bincode::serialize(&original).unwrap();
        let decoded = decode_pumpapi_transaction(&bytes).unwrap();
        let result = simulate_unsigned(&transport.client(), wallet(), decoded)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.signature.is_none());
        assert!(result.error.is_none());
        let calls = transport.assert_calls(1);
        let submitted = base64::engine::general_purpose::STANDARD
            .decode(calls[0][0].as_str().unwrap())
            .unwrap();
        assert_eq!(submitted, bytes);
        let sent: VersionedTransaction = bincode::deserialize(&submitted).unwrap();
        assert_eq!(sent.message, original.message);
        assert_eq!(sent.signatures, [Signature::default()]);
        assert_eq!(
            calls[0][1],
            json!({
                "sigVerify":false, "replaceRecentBlockhash":false, "commitment":"processed",
                "encoding":"base64", "accounts":null, "minContextSlot":null, "innerInstructions":true
            })
        );
        let (status, signature, error) = classify_result(Ok(result), true);
        assert_eq!(status, TradeStatus::Simulated);
        assert!(signature.is_none());
        assert!(error.is_none());
    }
}

#[tokio::test]
async fn pumpapi_h2a_program_failure_is_a_failed_preview_without_submission_signature() {
    let transport = PreviewRpc::new(json!({"InstructionError":[0,{"Custom":7}]}));
    let result = simulate_unsigned(&transport.client(), wallet(), transaction(false, 7))
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.signature.is_none());
    assert_eq!(
        result.error.as_deref(),
        Some("PumpAPI simulation failed: UiTransactionError(InstructionError(0, Custom(7)))")
    );
    let (status, signature, _) = classify_result(Ok(result), true);
    assert_eq!(status, TradeStatus::Failed);
    assert!(signature.is_none());
    transport.assert_calls(1);
}

#[tokio::test]
async fn pumpapi_h2a_wrong_wallet_nonzero_or_extra_signers_and_bad_shapes_stop_before_rpc() {
    for versioned in [false, true] {
        for mutation in 0..7 {
            let transport = PreviewRpc::new(Value::Null);
            let mut tx = transaction(versioned, 7);
            let mut expected_wallet = wallet();
            match mutation {
                0 => expected_wallet = Address::from_bytes([2; 32]),
                1 => tx.signatures[0] = Signature::from([8; 64]),
                2 => tx.signatures.clear(),
                3 => tx.signatures.push(Signature::default()),
                4 => {
                    tx.signatures.push(Signature::default());
                    match &mut tx.message {
                        VersionedMessage::Legacy(message) => {
                            message.header.num_required_signatures = 2
                        }
                        VersionedMessage::V0(message) => message.header.num_required_signatures = 2,
                        _ => unreachable!(),
                    }
                }
                5 => match &mut tx.message {
                    VersionedMessage::Legacy(message) => message.instructions[0].accounts.push(3),
                    VersionedMessage::V0(message) => message.instructions[0].accounts.push(3),
                    _ => unreachable!(),
                },
                6 => tx.message = VersionedMessage::V1(solana_sdk::message::v1::Message::default()),
                _ => unreachable!(),
            }
            let error = simulate_unsigned(&transport.client(), expected_wallet, tx)
                .await
                .err()
                .unwrap()
                .to_string();
            match mutation {
                0 => assert_eq!(
                    error,
                    "PumpAPI simulation payer does not match request wallet"
                ),
                1 | 4 => assert_eq!(
                    error,
                    "PumpAPI simulation requires exactly one unsigned wallet signature slot"
                ),
                6 => assert_eq!(
                    error,
                    "PumpAPI message version is not supported for account audit"
                ),
                _ => assert_eq!(error, "PumpAPI transaction failed Solana sanitization"),
            }
            transport.assert_calls(0);
        }
    }
}

#[tokio::test]
async fn pumpapi_h2a_rpc_error_is_redacted_without_retry_or_signature() {
    let mut transport = PreviewRpc::new(Value::Null);
    transport.error = Some("RPC unavailable https://fixture.invalid/private?token=secret".into());
    let result = simulate_unsigned(&transport.client(), wallet(), transaction(false, 7)).await;
    let error = format!("{:#}", result.as_ref().err().unwrap());
    assert!(error.contains("<redacted-url>"));
    assert!(!error.contains("fixture.invalid"));
    assert!(!error.contains("private"));
    assert!(!error.contains("secret"));
    let (status, signature, _) = classify_result(result, true);
    assert_eq!(status, TradeStatus::Failed);
    assert!(signature.is_none());
    transport.assert_calls(1);
}

#[tokio::test]
async fn pumpapi_h2a_malformed_rpc_result_is_not_success_or_retried() {
    let mut transport = PreviewRpc::new(Value::Null);
    transport.response = json!({"context":{"slot":42}, "value":"not-a-simulation-result"});
    let result = simulate_unsigned(&transport.client(), wallet(), transaction(true, 7)).await;
    assert!(result.is_err());
    transport.assert_calls(1);
}

#[tokio::test]
async fn pumpapi_h2a_entry_mode_config_and_request_errors_precede_http_and_rpc() {
    use crate::domain::trade::TradeAmount;
    for (simulate, enabled, lightning, amount, expected) in [
        (false, false, true, TradeAmount::SellPercent{percent:99}, "PumpAPI unsigned simulation requires simulate=true"),
        (true, false, true, TradeAmount::SellPercent{percent:99}, "ENABLE_PUMPAPI_EXECUTOR=true is required for execution_method=pumpapi"),
        (true, true, true, TradeAmount::SellPercent{percent:99}, "PUMPAPI_LIGHTNING_ENABLED is reserved; this service only supports PumpAPI Local Transaction mode"),
        (true, true, false, TradeAmount::SellPercent{percent:99}, "PumpAPI first version only supports sell_percent=100"),
        (true, true, false, TradeAmount::BuySol{lamports:0}, "PumpAPI buy amount must be greater than 0"),
    ] {
        let mut command = command();
        command.execution.mode = ExecutionMode::resolve(Some(simulate), false);
        command.amount = amount;
        let mut config = config();
        config.enabled = enabled;
        config.lightning_enabled = lightning;
        let input = PumpApiTrade::new(&command, config, true);
        let transport = PreviewRpc::new(Value::Null);
        assert_eq!(simulate_trade(&transport.client(), wallet(), input).await.err().unwrap().to_string(), expected);
        transport.assert_calls(0);
    }
}

#[tokio::test]
async fn pumpapi_h2a_cancelling_unsigned_rpc_does_not_submit_retry_or_produce_signature() {
    let gate = Arc::new(Gate::default());
    let mut transport = PreviewRpc::new(Value::Null);
    transport.gate = Some(gate.clone());
    let rpc = transport.client();
    let handle =
        tokio::spawn(async move { simulate_unsigned(&rpc, wallet(), transaction(false, 7)).await });
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    handle.abort();
    assert!(handle.await.err().unwrap().is_cancelled());
    gate.release.notify_one();
    transport.assert_calls(1);
}
