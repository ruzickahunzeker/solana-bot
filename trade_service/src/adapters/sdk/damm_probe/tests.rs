use super::*;
use base64::Engine;
use serde_json::{json, Value};
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_sdk::{instruction::AccountMeta, signature::Signature};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone)]
struct MemoryRpc {
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    response: Value,
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
    fn new() -> Self {
        Self {
            calls: Arc::default(),
            response: json!({"context":{"slot":55},"value":{"err":null,"logs":["synthetic log"],"accounts":null,"unitsConsumed":123}}),
            fail: false,
            block: false,
            dropped: Arc::default(),
        }
    }
    fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
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
            if request != RpcRequest::SimulateTransaction {
                return Err(
                    ClientErrorKind::Custom(format!("forbidden probe RPC: {request}")).into(),
                );
            }
            if self.block {
                let _drop = DropFlag(self.dropped.clone());
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(
                    ClientErrorKind::Custom("synthetic probe transport failure".into()).into(),
                );
            }
            Ok(self.response.clone())
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://unsigned-damm-probe".into()
    }
}

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn prepared_quote() -> PreparedDammProbe {
    let pool = sol_trade_sdk::trading::core::params::MeteoraDammV2Params::new(
        key(10),
        key(11),
        key(12),
        key(13),
        sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT,
        spl_token_2022_interface::ID,
        spl_token_interface::ID,
    );
    prepare_damm_probe(
        key(1),
        &pool,
        key(13),
        sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT,
        123,
        Hash::new_from_array([2; 32]),
        false,
    )
    .unwrap()
}

#[tokio::test]
async fn prepared_public_quote_serializes_the_immutable_zero_signature_message() {
    let source = MemoryRpc::new();
    let prepared = prepared_quote();
    simulate_prepared_damm_probe(&source.client(), &prepared, 41)
        .await
        .unwrap();
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, RpcRequest::SimulateTransaction);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(calls[0].1[0].as_str().unwrap())
        .unwrap();
    let actual: Transaction = bincode::deserialize(&bytes).unwrap();
    assert_eq!(&actual, prepared.transaction());
    assert_eq!(actual.signatures, [Signature::default()]);
    assert_eq!(
        calls[0].1[1],
        json!({
            "sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed",
            "encoding":"base64","accounts":{"encoding":"base64","addresses":[prepared.output_ata().to_string()]},
            "minContextSlot":41,"innerInstructions":false,
        })
    );
}

#[tokio::test]
async fn cancelling_prepared_public_quote_drops_only_the_readonly_rpc_future() {
    let mut source = MemoryRpc::new();
    source.block = true;
    let rpc = source.client();
    let prepared = prepared_quote();
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(20),
        simulate_prepared_damm_probe(&rpc, &prepared, 41)
    )
    .await
    .is_err());
    assert!(source.dropped.load(Ordering::SeqCst));
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, RpcRequest::SimulateTransaction);
}
fn instructions(wallet: Pubkey) -> Vec<Instruction> {
    vec![Instruction {
        program_id: key(4),
        accounts: vec![
            AccountMeta::new_readonly(wallet, true),
            AccountMeta::new(key(5), false),
        ],
        data: vec![1, 2, 3, 4],
    }]
}

#[test]
fn unsigned_packet_keeps_legacy_message_compilation_without_a_signer() {
    let wallet = key(1);
    let hash = Hash::new_from_array([2; 32]);
    let mut cases = vec![vec![], instructions(wallet)];
    let mut two = instructions(wallet);
    two.push(solana_system_interface::instruction::transfer(
        &wallet,
        &key(6),
        17,
    ));
    cases.push(two);
    for instructions in cases {
        let expected = Message::new_with_blockhash(&instructions, Some(&wallet), &hash);
        let transaction = unsigned_probe(wallet, &instructions, hash).unwrap();
        assert_eq!(transaction.message, expected);
        assert_eq!(transaction.signatures, [Signature::default()]);
        let round_trip: Transaction =
            bincode::deserialize(&bincode::serialize(&transaction).unwrap()).unwrap();
        assert_eq!(round_trip, transaction);
    }
}

#[tokio::test]
async fn actual_rpc_serialization_contains_only_zero_signature_and_unchanged_probe_options() {
    let source = MemoryRpc::new();
    let wallet = key(1);
    let hash = Hash::new_from_array([2; 32]);
    let instructions = instructions(wallet);
    let result = simulate_damm_probe(&source.client(), wallet, &instructions, hash, key(3), 41)
        .await
        .unwrap();
    assert_eq!(result.context.slot, 55);
    assert_eq!(result.value.units_consumed, Some(123));
    assert_eq!(result.value.logs, Some(vec!["synthetic log".into()]));
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, RpcRequest::SimulateTransaction);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(calls[0].1[0].as_str().unwrap())
        .unwrap();
    let transaction: Transaction = bincode::deserialize(&bytes).unwrap();
    assert_eq!(transaction.signatures, [Signature::default()]);
    assert_eq!(
        transaction.message,
        Message::new_with_blockhash(&instructions, Some(&wallet), &hash)
    );
    let config = &calls[0].1[1];
    assert_eq!(config["sigVerify"], false);
    assert_eq!(config["replaceRecentBlockhash"], false);
    assert_eq!(config["encoding"], "base64");
    assert_eq!(config["commitment"], "processed");
    assert_eq!(config["minContextSlot"], 41);
    assert_eq!(config["innerInstructions"], false);
    assert_eq!(
        config["accounts"],
        json!({"encoding":"base64","addresses":[key(3).to_string()]})
    );
}

#[tokio::test]
async fn additional_signer_is_rejected_before_any_rpc_or_key_access() {
    let source = MemoryRpc::new();
    let wallet = key(1);
    let mut instructions = instructions(wallet);
    instructions[0]
        .accounts
        .push(AccountMeta::new_readonly(key(7), true));
    let error = simulate_damm_probe(
        &source.client(),
        wallet,
        &instructions,
        Hash::default(),
        key(3),
        41,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "DAMM v2 quote probe requires exactly one wallet signer"
    );
    assert!(source.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn transport_failure_is_returned_without_an_additional_submit_or_retry() {
    let mut source = MemoryRpc::new();
    source.fail = true;
    let error = simulate_damm_probe(
        &source.client(),
        key(1),
        &instructions(key(1)),
        Hash::default(),
        key(3),
        41,
    )
    .await
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("synthetic probe transport failure"));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn program_failure_is_preserved_for_the_existing_quote_result_classifier() {
    let mut source = MemoryRpc::new();
    source.response["value"]["err"] = json!({"InstructionError":[0,{"Custom":6001}]});
    let result = simulate_damm_probe(
        &source.client(),
        key(1),
        &instructions(key(1)),
        Hash::default(),
        key(3),
        41,
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(result.value.err).unwrap(),
        json!({"InstructionError":[0,{"Custom":6001}]})
    );
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancelling_probe_drops_the_simulation_future_without_background_send_or_resign() {
    let mut source = MemoryRpc::new();
    source.block = true;
    let rpc = source.client();
    let instructions = instructions(key(1));
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(20),
        simulate_damm_probe(&rpc, key(1), &instructions, Hash::default(), key(3), 41)
    )
    .await
    .is_err());
    assert!(source.dropped.load(Ordering::SeqCst));
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, RpcRequest::SimulateTransaction);
}
