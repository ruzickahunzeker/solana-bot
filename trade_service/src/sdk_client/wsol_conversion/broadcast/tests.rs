use super::*;
use crate::sdk_client::wsol_conversion::{simulation::tests as fixture, UnsignedWsolTransaction};
use base64::Engine;
use serde_json::{json, Value};
use solana_client::{
    client_error::Result as RpcResult,
    rpc_client::RpcClientConfig,
    rpc_request::{RpcError, RpcRequest, RpcResponseErrorData},
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Clone)]
enum Reply {
    Echo,
    Value(Value),
    Error,
    Hold,
}
#[derive(Clone)]
struct Memory {
    reply: Reply,
    calls: Arc<Mutex<Vec<Value>>>,
    active: Arc<AtomicUsize>,
    entered: Arc<Notify>,
}
impl Memory {
    fn new(reply: Reply) -> Self {
        Self {
            reply,
            calls: Arc::default(),
            active: Arc::default(),
            entered: Arc::default(),
        }
    }
    fn client(&self, commitment: CommitmentConfig) -> Arc<RpcClient> {
        Arc::new(RpcClient::new_sender(
            self.clone(),
            RpcClientConfig {
                commitment_config: commitment,
                ..Default::default()
            },
        ))
    }
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl RpcSender for Memory {
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
            assert_eq!(request, RpcRequest::SendTransaction);
            self.calls.lock().unwrap().push(params.clone());
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.entered.notify_one();
            match &self.reply {
                Reply::Echo => {
                    let transaction: Transaction = bincode::deserialize(
                        &base64::engine::general_purpose::STANDARD
                            .decode(params[0].as_str().unwrap())
                            .unwrap(),
                    )
                    .unwrap();
                    Ok(json!(transaction.signatures[0].to_string()))
                }
                Reply::Value(value) => Ok(value.clone()),
                Reply::Error => Err(RpcError::RpcResponseError {
                    code: -32002,
                    message: "public fixture preflight failure".into(),
                    data: RpcResponseErrorData::SendTransactionPreflightFailure(
                        serde_json::from_value(fixture::result(json!({
                            "InstructionError":[0,{"Custom":42}]
                        })))
                        .unwrap(),
                    ),
                }
                .into()),
                Reply::Hold => std::future::pending().await,
            }
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        fixture::URL.into()
    }
}

fn dummy_signed(unsigned: UnsignedWsolTransaction) -> SignedWsolTransaction {
    // Intentionally not signed: isolated transport fixtures do not prove chain acceptance.
    let mut transaction = Transaction::new_unsigned(unsigned.message.clone());
    transaction.signatures.fill(Signature::from([7; 64]));
    SignedWsolTransaction {
        owner: unsigned.owner,
        authorized_message: unsigned.message,
        signature: transaction.signatures[0].to_string(),
        signed_bytes: bincode::serialize(&transaction).unwrap(),
        transaction,
    }
}

#[tokio::test]
async fn wsol_broadcast_lane_four_shapes_all_commitments_match_original_exact_request_and_receipt()
{
    use crate::application::wsol_submission_types::WsolDirection::{Unwrap, Wrap};
    for commitment in [
        CommitmentConfig::processed(),
        CommitmentConfig::confirmed(),
        CommitmentConfig::finalized(),
    ] {
        for (direction, amount, balance, exists) in [
            (Wrap, 50, 0, false),
            (Wrap, 50, 100, true),
            (Unwrap, 100, 100, true),
            (Unwrap, 50, 100, true),
        ] {
            let signed = dummy_signed(super::super::tests::unsigned_case(
                direction, amount, balance, exists,
            ));
            let original = Memory::new(Reply::Echo);
            let actual = Memory::new(Reply::Echo);
            let lane = WsolBroadcastLane::from_selected(actual.client(commitment));
            let before = signed.signed_bytes().to_vec();
            let expected = original
                .client(commitment)
                .send_transaction(&signed.transaction)
                .await
                .unwrap();
            let packet = WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap();
            assert!(std::ptr::eq(packet.transaction, &signed.transaction));
            assert_eq!(lane.send(packet).await.unwrap(), expected);
            assert_eq!(signed.signature(), expected.to_string());
            assert_eq!(signed.signed_bytes(), before);
            let calls = actual.calls.lock().unwrap();
            assert_eq!(*calls, *original.calls.lock().unwrap());
            assert_eq!(calls.len(), 1);
            assert_eq!(
                calls[0][0],
                base64::engine::general_purpose::STANDARD.encode(&before)
            );
            assert_eq!(
                calls[0][1],
                json!({"encoding":"base64","skipPreflight":false,
                    "preflightCommitment":serde_json::to_value(commitment.commitment).unwrap(),
                    "maxRetries":null,"minContextSlot":null})
            );
            assert_eq!(lane.url(), fixture::URL);
            assert_eq!(lane.commitment(), commitment);
        }
    }
}

#[tokio::test]
async fn wsol_broadcast_lane_mismatched_and_malformed_receipts_keep_original_errors_and_packet() {
    for value in [
        json!(Signature::from([8; 64]).to_string()),
        json!("not-a-signature"),
        Value::Null,
        json!({"signature":"wrong-shape"}),
        json!(17),
    ] {
        let original = Memory::new(Reply::Value(value.clone()));
        let actual = Memory::new(Reply::Value(value));
        let signed = fixture::signed(true);
        let before = signed.signed_bytes().to_vec();
        let expected = original
            .client(CommitmentConfig::confirmed())
            .send_transaction(&signed.transaction)
            .await
            .unwrap_err();
        let lane = WsolBroadcastLane::from_selected(actual.client(CommitmentConfig::confirmed()));
        let error = lane
            .send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected.to_string());
        assert_eq!(signed.signed_bytes(), before);
        assert_eq!(
            *actual.calls.lock().unwrap(),
            *original.calls.lock().unwrap()
        );
        assert_eq!(actual.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn wsol_broadcast_lane_preflight_failure_keeps_original_error_without_fallback_or_resign() {
    let signed = fixture::signed(false);
    let before = signed.signed_bytes().to_vec();
    let original = Memory::new(Reply::Error);
    let actual = Memory::new(Reply::Error);
    let expected = original
        .client(CommitmentConfig::processed())
        .send_transaction(&signed.transaction)
        .await
        .unwrap_err();
    let lane = WsolBroadcastLane::from_selected(actual.client(CommitmentConfig::processed()));
    let error = lane
        .send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), expected.to_string());
    assert_eq!(signed.signed_bytes(), before);
    assert_eq!(
        *actual.calls.lock().unwrap(),
        *original.calls.lock().unwrap()
    );
    assert_eq!(actual.calls.lock().unwrap().len(), 1);
}

#[test]
fn wsol_broadcast_lane_snapshot_and_wallet_fail_before_a_packet_or_rpc_request() {
    let memory = Memory::new(Reply::Echo);
    let _lane = WsolBroadcastLane::from_selected(memory.client(CommitmentConfig::confirmed()));
    for mutation in 0..8 {
        let mut signed = fixture::signed(true);
        let mut wallet = signed.owner;
        match mutation {
            0 => wallet = Pubkey::from([9; 32]),
            1 => signed.owner = Pubkey::from([9; 32]),
            2 => signed.authorized_message.recent_blockhash = Default::default(),
            3 => signed.transaction.message.recent_blockhash = Default::default(),
            4 => signed.transaction.signatures.clear(),
            5 => signed.signature = Signature::from([8; 64]).to_string(),
            6 => signed.signed_bytes.push(0),
            7 => signed.transaction.signatures[0] = Signature::from([8; 64]),
            _ => unreachable!(),
        }
        let expected = signed.validate_snapshot(&wallet).unwrap_err().to_string();
        assert_eq!(
            WsolBroadcastPacket::checked(&signed, &wallet)
                .err()
                .unwrap()
                .to_string(),
            expected
        );
        assert!(memory.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn wsol_broadcast_lane_moves_original_arc_without_new_client_and_releases_it() {
    let memory = Memory::new(Reply::Echo);
    let rpc = memory.client(CommitmentConfig::finalized());
    let weak = Arc::downgrade(&rpc);
    let pointer = Arc::as_ptr(&rpc);
    let lane = WsolBroadcastLane::from_selected(rpc);
    assert_eq!(Arc::as_ptr(&lane.rpc), pointer);
    assert_eq!(Arc::strong_count(&lane.rpc), 1);
    assert_eq!(lane.commitment(), CommitmentConfig::finalized());
    assert_eq!(lane.url(), fixture::URL);
    let moved = lane;
    assert_eq!(Arc::as_ptr(&moved.rpc), pointer);
    assert!(memory.calls.lock().unwrap().is_empty());
    drop(moved);
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn wsol_broadcast_lane_drop_before_first_poll_has_no_send() {
    let memory = Memory::new(Reply::Echo);
    let lane = WsolBroadcastLane::from_selected(memory.client(CommitmentConfig::processed()));
    let signed = fixture::signed(false);
    let pending = lane.send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap());
    drop(pending);
    assert!(memory.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn wsol_broadcast_lane_cancellation_drops_original_io_no_adapter_retry() {
    let memory = Memory::new(Reply::Hold);
    let lane = WsolBroadcastLane::from_selected(memory.client(CommitmentConfig::processed()));
    let signed = fixture::signed(true);
    let before = signed.signed_bytes().to_vec();
    let mut pending =
        Box::pin(lane.send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap()));
    tokio::select! {
        result = &mut pending => panic!("unexpected completion: {result:?}"),
        _ = memory.entered.notified() => {}
    }
    assert_eq!(memory.active.load(Ordering::SeqCst), 1);
    drop(pending);
    tokio::task::yield_now().await;
    assert_eq!(memory.active.load(Ordering::SeqCst), 0);
    assert_eq!(memory.calls.lock().unwrap().len(), 1);
    assert_eq!(signed.signed_bytes(), before);
}

#[tokio::test]
async fn wsol_broadcast_lane_caller_timeout_retains_original_uncertain_packet_without_retry() {
    let memory = Memory::new(Reply::Hold);
    let lane = WsolBroadcastLane::from_selected(memory.client(CommitmentConfig::confirmed()));
    let signed = fixture::signed(false);
    let before = signed.signed_bytes().to_vec();
    assert!(tokio::time::timeout(
        Duration::from_millis(30),
        lane.send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap())
    )
    .await
    .is_err());
    assert_eq!(memory.active.load(Ordering::SeqCst), 0);
    assert_eq!(memory.calls.lock().unwrap().len(), 1);
    assert_eq!(signed.signed_bytes(), before);
    assert_eq!(
        signed.signature(),
        signed.transaction.signatures[0].to_string()
    );
}

#[tokio::test]
async fn wsol_broadcast_lane_production_policy_guard_precedes_wallet_and_snapshot_and_io() {
    let client = disabled_client();
    let memory = Memory::new(Reply::Echo);
    let lane = WsolBroadcastLane::from_selected(memory.client(CommitmentConfig::confirmed()));
    let mut signed = fixture::signed(true);
    signed.signed_bytes.clear();
    assert_eq!(
        client
            .broadcast_signed_wsol(&signed, &lane)
            .await
            .unwrap_err()
            .to_string(),
        crate::domain::execution_policy::BROADCAST_DISABLED_ERROR
    );
    assert!(memory.calls.lock().unwrap().is_empty());
}

pub(crate) fn disabled_client() -> SdkTradeClient {
    SdkTradeClient {
        lanes: vec![],
        sol_hop_cache: Default::default(),
        swqos_lanes: vec![],
        mev_protection: false,
        require_mev_submit: false,
        rpc_count: 0,
        broadcast_policy: crate::domain::execution_policy::BroadcastPolicy::Disabled,
    }
}

mod wire;
