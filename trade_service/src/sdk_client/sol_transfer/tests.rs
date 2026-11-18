use super::*;
use base64::Engine;
use serde_json::{json, Value};
use solana_client::{
    client_error::{ClientErrorKind, Result as RpcResult},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::{future::Future, pin::Pin, sync::Mutex};

mod shared_confirmation;
mod shared_reads;

#[derive(Clone)]
struct MemoryRpc {
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    violations: Arc<Mutex<Vec<RpcRequest>>>,
    balance: u64,
    fail: Option<RpcRequest>,
    pending: Option<RpcRequest>,
    allow_funds: bool,
    confirmed: bool,
    receipt: Signature,
}

impl Default for MemoryRpc {
    fn default() -> Self {
        Self {
            calls: Arc::default(),
            violations: Arc::default(),
            balance: 1_000_000,
            fail: None,
            pending: None,
            allow_funds: false,
            confirmed: true,
            receipt: Signature::from([7; 64]),
        }
    }
}

impl MemoryRpc {
    fn client(&self) -> Arc<RpcClient> {
        self.client_with_commitment(solana_commitment_config::CommitmentConfig::processed())
    }
    fn client_with_commitment(
        &self,
        commitment: solana_commitment_config::CommitmentConfig,
    ) -> Arc<RpcClient> {
        Arc::new(RpcClient::new_sender(
            self.clone(),
            RpcClientConfig {
                commitment_config: commitment,
                ..Default::default()
            },
        ))
    }
    fn methods(&self) -> Vec<RpcRequest> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(method, _)| *method)
            .filter(|method| *method != RpcRequest::GetVersion)
            .collect()
    }
    fn read_only(&self) {
        assert!(self.violations.lock().unwrap().is_empty());
        assert!(self.methods().iter().all(|method| matches!(
            method,
            RpcRequest::GetBalance
                | RpcRequest::GetLatestBlockhash
                | RpcRequest::GetFeeForMessage
                | RpcRequest::GetMinimumBalanceForRentExemption
        )));
    }
}

impl RpcSender for MemoryRpc {
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
            self.calls.lock().unwrap().push((request, params));
            if self.pending == Some(request) {
                return std::future::pending().await;
            }
            if self.fail == Some(request) {
                return Err(ClientErrorKind::Custom(format!("synthetic {request} failure")).into());
            }
            let value = match request {
                RpcRequest::GetVersion => json!({"solana-core":"3.0.0","feature-set":1}),
                RpcRequest::GetBalance => json!({"context":{"slot":123},"value":self.balance}),
                RpcRequest::GetLatestBlockhash => {
                    json!({"context":{"slot":123},"value":{"blockhash":Hash::new_from_array([6;32]).to_string(),"lastValidBlockHeight":999}})
                }
                RpcRequest::GetFeeForMessage => json!({"context":{"slot":123},"value":5000}),
                RpcRequest::GetMinimumBalanceForRentExemption => json!(890_880),
                RpcRequest::SendTransaction if self.allow_funds => json!(self.receipt.to_string()),
                RpcRequest::GetSignatureStatuses if self.allow_funds => {
                    json!({"context":{"slot":123},"value":[if self.confirmed { json!({"slot":123,"confirmations":1,"status":{"Ok":null},"err":null,"confirmationStatus":"confirmed"}) } else { Value::Null }]})
                }
                _ => {
                    self.violations.lock().unwrap().push(request);
                    return Err(ClientErrorKind::Custom(format!(
                        "forbidden memory RPC: {request}"
                    ))
                    .into());
                }
            };
            Ok(value)
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats {
            request_count: self.calls.lock().unwrap().len(),
            ..Default::default()
        }
    }
    fn url(&self) -> String {
        "memory://native-sol-transfer".into()
    }
}

fn source() -> Pubkey {
    Pubkey::new_from_array([1; 32])
}
fn destination() -> Pubkey {
    Pubkey::new_from_array([2; 32])
}
async fn prepare(rpc: &MemoryRpc, fixed: Option<u64>, percent: Option<u64>) -> PreparedSolTransfer {
    prepare_with_rpcs(
        source(),
        destination(),
        fixed,
        percent,
        std::iter::once(rpc.client()),
        PREPARE_TIMEOUT,
    )
    .await
    .unwrap()
}

// Deliberately invalid signature bytes; the transport cannot connect to a network.
fn synthetic_signed(prepared: PreparedSolTransfer) -> SignedSolTransfer {
    let mut transaction = Transaction::new_unsigned(prepared.message.clone());
    transaction.signatures[0] = Signature::from([7; 64]);
    SignedSolTransfer::new(prepared, transaction).unwrap()
}

#[tokio::test]
async fn native_sol_prepare_is_unsigned_and_matches_legacy_message_and_estimated_fee_order() {
    for (fixed, percent, amount, reserve) in [
        (Some(94_120), None, 94_120, 900_880),
        (None, Some(50), 47_060, 900_880),
        (None, Some(100), 995_000, 0),
    ] {
        let rpc = MemoryRpc::default();
        let prepared = prepare(&rpc, fixed, percent).await;
        let expected = Message::new_with_blockhash(
            &[system_instruction::transfer(
                &source(),
                &destination(),
                amount,
            )],
            Some(&source()),
            &Hash::new_from_array([6; 32]),
        );
        assert_eq!(prepared.message, expected);
        assert_eq!(
            Transaction::new_unsigned(prepared.message.clone()).signatures,
            vec![Signature::default()]
        );
        assert_eq!(
            prepared.snapshot,
            SolTransferSnapshot {
                source: Address::from_bytes(source().to_bytes()),
                destination: Address::from_bytes(destination().to_bytes()),
                amount_lamports: amount,
                fee_lamports: 5000,
                balance_before_lamports: 1_000_000,
                reserve_lamports: reserve,
            }
        );
        assert_eq!(
            rpc.methods(),
            [
                RpcRequest::GetBalance,
                RpcRequest::GetLatestBlockhash,
                RpcRequest::GetFeeForMessage,
                RpcRequest::GetMinimumBalanceForRentExemption
            ]
        );
        let calls = rpc.calls.lock().unwrap();
        let fee = calls
            .iter()
            .find(|(method, _)| *method == RpcRequest::GetFeeForMessage)
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(fee.1[0].as_str().unwrap())
            .unwrap();
        let message: Message = bincode::deserialize(&bytes).unwrap();
        assert_eq!(
            message,
            Message::new_with_blockhash(
                &[system_instruction::transfer(&source(), &destination(), 1)],
                Some(&source()),
                &prepared.blockhash
            )
        );
        drop(calls);
        rpc.read_only();
    }
}

#[tokio::test]
async fn native_sol_prepare_falls_back_only_for_rpc_errors_not_amount_errors() {
    for method in [
        RpcRequest::GetBalance,
        RpcRequest::GetLatestBlockhash,
        RpcRequest::GetFeeForMessage,
        RpcRequest::GetMinimumBalanceForRentExemption,
    ] {
        let primary = MemoryRpc {
            fail: Some(method),
            ..Default::default()
        };
        let backup = MemoryRpc::default();
        let prepared = prepare_with_rpcs(
            source(),
            destination(),
            Some(1),
            None,
            vec![primary.client(), backup.client()].into_iter(),
            PREPARE_TIMEOUT,
        )
        .await
        .unwrap();
        assert_eq!(prepared.snapshot.amount_lamports, 1);
        assert!(primary.methods().contains(&method));
        assert_eq!(backup.methods().len(), 4);
        primary.read_only();
        backup.read_only();
    }
    let primary = MemoryRpc {
        balance: 0,
        ..Default::default()
    };
    let backup = MemoryRpc::default();
    let error = prepare_with_rpcs(
        source(),
        destination(),
        Some(1),
        None,
        vec![primary.client(), backup.client()].into_iter(),
        PREPARE_TIMEOUT,
    )
    .await
    .err()
    .unwrap();
    assert!(error
        .to_string()
        .starts_with("insufficient SOL balance for fee and rent reserve"));
    assert!(backup.methods().is_empty());
    primary.read_only();
}

#[tokio::test]
async fn native_sol_prepare_keeps_self_destination_empty_lane_and_rpc_timeout_errors() {
    let rpc = MemoryRpc::default();
    assert_eq!(
        prepare_with_rpcs(
            source(),
            source(),
            Some(1),
            None,
            std::iter::once(rpc.client()),
            PREPARE_TIMEOUT
        )
        .await
        .err()
        .unwrap()
        .to_string(),
        "source and destination wallets must be different"
    );
    assert!(rpc.methods().is_empty());
    assert_eq!(
        prepare_with_rpcs(
            source(),
            destination(),
            Some(1),
            None,
            std::iter::empty(),
            PREPARE_TIMEOUT
        )
        .await
        .err()
        .unwrap()
        .to_string(),
        "unable to prepare SOL transfer: no Default RPC lane configured"
    );
    let pending = MemoryRpc {
        pending: Some(RpcRequest::GetBalance),
        ..Default::default()
    };
    assert_eq!(
        prepare_with_rpcs(
            source(),
            destination(),
            Some(1),
            None,
            std::iter::once(pending.client()),
            Duration::from_millis(2)
        )
        .await
        .err()
        .unwrap()
        .to_string(),
        "unable to prepare SOL transfer: RPC transfer preparation timed out"
    );
    let backup = MemoryRpc::default();
    assert!(prepare_with_rpcs(
        source(),
        destination(),
        Some(1),
        None,
        vec![pending.client(), backup.client()].into_iter(),
        Duration::from_millis(2)
    )
    .await
    .is_ok());
    pending.read_only();
    backup.read_only();
    assert_eq!(PREPARE_TIMEOUT, Duration::from_secs(5));
    assert_eq!(SEND_TIMEOUT, Duration::from_secs(12));
    assert_eq!(CONFIRM_TIMEOUT, Duration::from_secs(15));
}

#[tokio::test]
async fn native_sol_prepared_handle_rejects_message_identity_amount_and_fee_tampering() {
    for mode in 0..10 {
        let rpc = MemoryRpc::default();
        let mut prepared = prepare(&rpc, None, Some(50)).await;
        match mode {
            0 => prepared.snapshot.source = Address::from_bytes([9; 32]),
            1 => prepared.snapshot.destination = Address::from_bytes([9; 32]),
            2 => prepared.snapshot.amount_lamports += 1,
            3 => prepared.snapshot.reserve_lamports += 1,
            4 => prepared.snapshot.fee_lamports += 1,
            5 => prepared.snapshot.balance_before_lamports += 2,
            6 => prepared
                .message
                .instructions
                .push(prepared.message.instructions[0].clone()),
            7 => prepared.message.account_keys[0] = Pubkey::new_from_array([9; 32]),
            8 => prepared.message.recent_blockhash = Hash::new_from_array([9; 32]),
            9 => prepared.message.header.num_required_signatures = 2,
            _ => unreachable!(),
        }
        assert!(prepared.ensure_source(&source()).is_err(), "mode {mode}");
        rpc.read_only();
    }
}

#[tokio::test]
async fn native_sol_synthetic_submission_bytes_match_transport_and_confirmation() {
    let rpc = MemoryRpc {
        allow_funds: true,
        ..Default::default()
    };
    let signed = synthetic_signed(prepare(&rpc, Some(1), None).await);
    let bytes = signed.submission().bytes.clone();
    let signature = signed.submission().signature.clone();
    let receipt = broadcast_signed(&signed, &source(), SEND_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(receipt, rpc.receipt);
    assert_eq!(receipt.to_string(), signature);
    assert_eq!(signed.submission().signature, signature);
    assert_eq!(signed.submission().bytes, bytes);
    let tx: Transaction = bincode::deserialize(&bytes).unwrap();
    assert_eq!(tx.signatures[0].to_string(), signature);
    {
        let calls = rpc.calls.lock().unwrap();
        let call = calls
            .iter()
            .find(|(method, _)| *method == RpcRequest::SendTransaction)
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(call.1[0].as_str().unwrap())
                .unwrap(),
            bytes
        );
    }
    assert!(confirm_signed(&signed, &receipt, CONFIRM_TIMEOUT).await);
    let calls = rpc.calls.lock().unwrap();
    let call = calls
        .iter()
        .find(|(method, _)| *method == RpcRequest::GetSignatureStatuses)
        .unwrap();
    assert_eq!(call.1[0][0], receipt.to_string());
    assert!(rpc.violations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_sol_signed_handle_rejects_tampering_before_any_send() {
    for mode in 0..5 {
        let rpc = MemoryRpc::default();
        let mut signed = synthetic_signed(prepare(&rpc, Some(1), None).await);
        match mode {
            0 => signed.submission.bytes.push(0),
            1 => signed.submission.signature = "different-signature".into(),
            2 => signed.transaction.signatures[0] = Signature::default(),
            3 => signed.transaction.message.instructions[0].data.push(0),
            4 => signed.prepared.snapshot.destination = Address::from_bytes([9; 32]),
            _ => unreachable!(),
        }
        assert!(broadcast_signed(&signed, &source(), SEND_TIMEOUT)
            .await
            .is_err());
        rpc.read_only();
    }
    let rpc = MemoryRpc::default();
    let prepared = prepare(&rpc, Some(1), None).await;
    let tx = Transaction::new_unsigned(prepared.message.clone());
    assert!(SignedSolTransfer::new(prepared, tx).is_err());
    rpc.read_only();
}

#[tokio::test]
async fn native_sol_mismatched_rpc_receipt_is_rejected_without_losing_original_signature() {
    let rpc = MemoryRpc {
        allow_funds: true,
        receipt: Signature::from([9; 64]),
        ..Default::default()
    };
    let signed = synthetic_signed(prepare(&rpc, Some(1), None).await);
    let original_bytes = signed.submission().bytes.clone();
    let original_signature = signed.submission().signature.clone();
    let error = broadcast_signed(&signed, &source(), SEND_TIMEOUT)
        .await
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("RPC node returned mismatched signature"));
    assert_eq!(signed.submission().bytes, original_bytes);
    assert_eq!(signed.submission().signature, original_signature);
    assert_eq!(
        rpc.methods()
            .iter()
            .filter(|&&method| method == RpcRequest::SendTransaction)
            .count(),
        1
    );
    assert!(!rpc.methods().contains(&RpcRequest::GetSignatureStatuses));
}

#[tokio::test]
async fn native_sol_confirmation_uses_selected_rpc_commitment_without_forcing_finality() {
    use solana_commitment_config::CommitmentConfig;
    for (commitment, expected) in [
        (CommitmentConfig::processed(), true),
        (CommitmentConfig::finalized(), false),
    ] {
        let rpc = MemoryRpc {
            allow_funds: true,
            ..Default::default()
        };
        let prepared = prepare_with_rpcs(
            source(),
            destination(),
            Some(1),
            None,
            std::iter::once(rpc.client_with_commitment(commitment)),
            PREPARE_TIMEOUT,
        )
        .await
        .unwrap();
        let signed = synthetic_signed(prepared);
        assert_eq!(
            confirm_signed(&signed, &Signature::from([7; 64]), CONFIRM_TIMEOUT).await,
            expected
        );
        assert!(!rpc.methods().contains(&RpcRequest::SendTransaction));
    }
}

#[tokio::test]
async fn native_sol_send_error_timeout_and_confirmation_miss_keep_original_artifact() {
    for pending in [false, true] {
        let rpc = MemoryRpc {
            allow_funds: true,
            fail: (!pending).then_some(RpcRequest::SendTransaction),
            pending: pending.then_some(RpcRequest::SendTransaction),
            ..Default::default()
        };
        let signed = synthetic_signed(prepare(&rpc, Some(1), None).await);
        let original = signed.submission().bytes.clone();
        let error = broadcast_signed(&signed, &source(), Duration::from_millis(2))
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            if pending {
                "SOL transfer submission timed out"
            } else {
                "SOL transfer submission failed: balance=1000000, amount=1, fee=5000, reserve=900880 lamports"
            }
        );
        assert_eq!(signed.submission().bytes, original);
        assert_eq!(
            rpc.methods()
                .iter()
                .filter(|&&method| method == RpcRequest::SendTransaction)
                .count(),
            1
        );
        assert!(!rpc.methods().contains(&RpcRequest::GetSignatureStatuses));
    }
    for mode in 0..3 {
        let rpc = MemoryRpc {
            allow_funds: true,
            confirmed: false,
            fail: (mode == 1).then_some(RpcRequest::GetSignatureStatuses),
            pending: (mode == 2).then_some(RpcRequest::GetSignatureStatuses),
            ..Default::default()
        };
        let signed = synthetic_signed(prepare(&rpc, Some(1), None).await);
        assert!(
            !confirm_signed(&signed, &Signature::from([7; 64]), Duration::from_millis(2)).await
        );
        assert!(!rpc.methods().contains(&RpcRequest::SendTransaction));
    }
}

#[tokio::test]
async fn native_sol_stage_guards_precede_wallet_and_rpc_access_without_sdk_initialization() {
    let read_rpc =
        Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
    let mut client = SdkTradeClient {
        lanes: vec![],
        sol_hop_cache: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        swqos_lanes: vec![],
        mev_protection: false,
        require_mev_submit: false,
        rpc_count: 0,
        broadcast_policy: crate::domain::execution_policy::BroadcastPolicy::Disabled,
    };
    let expected = crate::domain::execution_policy::BROADCAST_DISABLED_ERROR;
    assert_eq!(
        client
            .prepare_sol_transfer(read_rpc.as_ref(), &destination(), Some(1), None)
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
    let rpc = MemoryRpc::default();
    assert_eq!(
        client
            .sign_sol_transfer(prepare(&rpc, Some(1), None).await)
            .err()
            .unwrap()
            .to_string(),
        expected
    );
    let signed = synthetic_signed(prepare(&rpc, Some(1), None).await);
    assert_eq!(
        client
            .send_signed_sol_transfer(&signed)
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
    rpc.read_only();
    client.broadcast_policy = crate::domain::execution_policy::BroadcastPolicy::Enabled;
    assert_eq!(
        client
            .prepare_sol_transfer(read_rpc.as_ref(), &destination(), Some(1), None)
            .await
            .err()
            .unwrap()
            .to_string(),
        "wallet has no execution lane"
    );
}
