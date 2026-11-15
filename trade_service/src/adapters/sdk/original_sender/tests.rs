use super::*;
use crate::domain::reconciliation::ChainObservation;
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::swqos::SwqosClientTrait;
use solana_client::{
    client_error::ClientError,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_sdk::{
    hash::Hash,
    message::Message,
    signature::{Keypair, Signer},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

#[derive(Default)]
struct Trace {
    rpc: Mutex<Vec<(RpcRequest, Value)>>,
    packets: Mutex<Vec<Vec<u8>>>,
    waits: Mutex<Vec<bool>>,
    observations: Mutex<Vec<String>>,
    tips: AtomicUsize,
    dropped: AtomicBool,
}
#[derive(Clone)]
struct MemoryRpc {
    trace: Arc<Trace>,
    reply: String,
}
impl RpcSender for MemoryRpc {
    fn send<'a, 'f>(
        &'a self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ClientError>> + Send + 'f>>
    where
        'a: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.trace.rpc.lock().unwrap().push((request, params));
            assert_eq!(request, RpcRequest::SendTransaction);
            Ok(json!(self.reply))
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://original-sender".into()
    }
}

struct Relay {
    provider: SwqosType,
    minimum: Option<u64>,
    builtin: f64,
    fail: bool,
    block: bool,
    trace: Arc<Trace>,
}
struct DropFlag(Arc<Trace>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}
impl SwqosClientTrait for Relay {
    fn configured_min_tip_lamports(&self) -> Option<u64> {
        self.minimum
    }
    fn get_swqos_type(&self) -> SwqosType {
        self.provider
    }
    fn min_tip_sol(&self) -> f64 {
        self.builtin
    }
    fn get_tip_account(&self) -> Result<String> {
        self.trace.tips.fetch_add(1, Ordering::SeqCst);
        Ok(Pubkey::new_from_array([7; 32]).to_string())
    }
    fn send_transaction<'a, 'b, 'f>(
        &'a self,
        trade: TradeType,
        tx: &'b VersionedTransaction,
        wait: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            assert!(matches!(trade, TradeType::Sell));
            self.trace
                .packets
                .lock()
                .unwrap()
                .push(bincode::serialize(tx).unwrap());
            self.trace.waits.lock().unwrap().push(wait);
            if self.block {
                let _flag = DropFlag(self.trace.clone());
                std::future::pending::<()>().await;
            }
            ensure!(!self.fail, "synthetic provider error");
            Ok(())
        })
    }
    fn send_transactions<'a, 'b, 'f>(
        &'a self,
        _trade: TradeType,
        _txs: &'b Vec<VersionedTransaction>,
        _wait: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { anyhow::bail!("batch sending is forbidden in original sender tests") })
    }
}
struct Observer {
    trace: Arc<Trace>,
    values: Mutex<Vec<Option<ChainObservation>>>,
}
impl SignatureStatusReader for Observer {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        self.trace
            .observations
            .lock()
            .unwrap()
            .push(signature.into());
        let mut values = self.values.lock().unwrap();
        Ok(if values.len() > 1 {
            values.remove(0)
        } else {
            values.first().cloned().flatten()
        })
    }
}
fn packet() -> SignedSubmission {
    let key = Keypair::new();
    let message = VersionedMessage::Legacy(Message::new_with_blockhash(
        &[],
        Some(&key.pubkey()),
        &Hash::new_unique(),
    ));
    let tx = VersionedTransaction::try_new(message, &[&key]).unwrap();
    SignedSubmission {
        signature: tx.signatures[0].to_string(),
        bytes: bincode::serialize(&tx).unwrap(),
    }
}
fn observed(s: &SignedSubmission, commitment: ChainCommitment) -> Option<ChainObservation> {
    Some(ChainObservation {
        signature: s.signature.clone(),
        commitment,
        slot: 42,
        error: None,
    })
}
fn sender(
    provider: SwqosType,
    s: &SignedSubmission,
    values: Vec<Option<ChainObservation>>,
    fail: bool,
    block: bool,
) -> (OriginalSdkSender<Observer>, Arc<Trace>) {
    let trace = Arc::new(Trace::default());
    let rpc = Arc::new(RpcClient::new_sender(
        MemoryRpc {
            trace: trace.clone(),
            reply: s.signature.clone(),
        },
        RpcClientConfig::default(),
    ));
    (
        OriginalSdkSender {
            sender: Arc::new(Relay {
                provider,
                minimum: None,
                builtin: 0.001,
                fail,
                block,
                trace: trace.clone(),
            }),
            rpc,
            observer: Observer {
                trace: trace.clone(),
                values: Mutex::new(values),
            },
            provider,
            check_min_tip: false,
        },
        trace,
    )
}

#[tokio::test]
async fn sdk_original_sender_default_retains_original_packet_and_sdk_rpc_options() {
    let s = packet();
    let (sender, trace) = sender(SwqosType::Default, &s, vec![None], false, false);
    assert_eq!(
        sender.send_packet(&s, false, 100).await.unwrap(),
        s.signature
    );
    let rpc = trace.rpc.lock().unwrap();
    assert_eq!(rpc.len(), 1);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(rpc[0].1[0].as_str().unwrap())
            .unwrap(),
        s.bytes
    );
    assert_eq!(
        rpc[0].1[1],
        json!({"encoding":"base64","skipPreflight":true,"preflightCommitment":"processed","maxRetries":3,"minContextSlot":0})
    );
    assert!(trace.packets.lock().unwrap().is_empty());
    assert!(trace.observations.lock().unwrap().is_empty());
}

#[test]
fn sdk_original_sender_provider_serializers_keep_the_same_original_damm_packet() {
    let key = Keypair::new();
    let candidate =
        crate::adapters::sdk::damm_sell_message::candidate_fixture_for_wallet(key.pubkey());
    let message: VersionedMessage = bincode::deserialize(&candidate.message_bytes()).unwrap();
    let tx = VersionedTransaction::try_new(message, &[&key]).unwrap();
    let expected = bincode::serialize(&tx).unwrap();
    let (binary, signature) =
        sol_trade_sdk::swqos::serialization::serialize_transaction_bincode_sync(&tx).unwrap();
    assert_eq!(binary.as_ref(), expected.as_slice());
    assert_eq!(signature, tx.signatures[0]);
    let (encoded, signature) = sol_trade_sdk::swqos::common::serialize_transaction_and_encode(
        &tx,
        UiTransactionEncoding::Base64,
    )
    .unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
        expected
    );
    assert_eq!(signature, tx.signatures[0]);
}

#[tokio::test]
async fn sdk_original_sender_default_mismatched_receipt_is_not_accepted() {
    let s = packet();
    let (mut sender, trace) = sender(
        SwqosType::Default,
        &s,
        vec![observed(&s, ChainCommitment::Finalized)],
        false,
        false,
    );
    sender.rpc = Arc::new(RpcClient::new_sender(
        MemoryRpc {
            trace: trace.clone(),
            reply: Signature::new_unique().to_string(),
        },
        RpcClientConfig::default(),
    ));
    assert!(sender.send_packet(&s, false, 100).await.is_err());
    assert_eq!(trace.rpc.lock().unwrap().len(), 1);
    assert!(trace.observations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sdk_original_sender_provider_ok_without_receipt_or_observation_is_unknown() {
    for budget in [0, 5] {
        let s = packet();
        let (sender, trace) = sender(SwqosType::Temporal, &s, vec![None], false, false);
        assert!(sender.send_packet(&s, false, budget).await.is_err());
        assert_eq!(*trace.packets.lock().unwrap(), vec![s.bytes.clone()]);
        assert!(trace.rpc.lock().unwrap().is_empty());
        assert_eq!(trace.observations.lock().unwrap().is_empty(), budget == 0);
        assert_eq!(*trace.waits.lock().unwrap(), vec![false]);
    }
}

#[tokio::test]
async fn sdk_original_sender_preserves_transport_error_but_accepts_evidenced_original_signature() {
    let s = packet();
    let (sender, trace) = sender(
        SwqosType::Temporal,
        &s,
        vec![observed(&s, ChainCommitment::Processed)],
        true,
        false,
    );
    assert_eq!(
        sender.send_packet(&s, false, 200).await.unwrap(),
        s.signature
    );
    assert_eq!(trace.packets.lock().unwrap().len(), 1);
    assert_eq!(
        *trace.observations.lock().unwrap(),
        vec![s.signature.clone()]
    );
    assert!(trace.rpc.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sdk_original_sender_bad_status_identity_slot_or_error_is_never_success() {
    for change in 0..3 {
        let s = packet();
        let mut observation = observed(&s, ChainCommitment::Confirmed).unwrap();
        match change {
            0 => observation.signature = Signature::new_unique().to_string(),
            1 => observation.slot = 0,
            _ => observation.error = Some("synthetic chain error".into()),
        }
        let (sender, trace) = sender(
            SwqosType::Temporal,
            &s,
            vec![Some(observation)],
            false,
            false,
        );
        assert!(sender.send_packet(&s, false, 200).await.is_err());
        assert_eq!(trace.packets.lock().unwrap().len(), 1);
        assert_eq!(
            *trace.observations.lock().unwrap(),
            vec![s.signature.clone()]
        );
    }
}

#[tokio::test]
async fn sdk_original_sender_wait_policy_is_checked_by_original_signature_not_provider_flag() {
    let s = packet();
    let (sender, trace) = sender(
        SwqosType::Temporal,
        &s,
        vec![
            observed(&s, ChainCommitment::Processed),
            observed(&s, ChainCommitment::Confirmed),
        ],
        false,
        false,
    );
    assert_eq!(sender.send_packet(&s, true, 0).await.unwrap(), s.signature);
    assert_eq!(trace.observations.lock().unwrap().len(), 2);
    assert_eq!(*trace.waits.lock().unwrap(), vec![false]);
    let (sender, _) = self::sender(
        SwqosType::Temporal,
        &s,
        vec![observed(&s, ChainCommitment::Processed)],
        false,
        false,
    );
    assert!(sender.verify_original(&s.signature, true, 5).await.is_err());
}

#[tokio::test]
async fn sdk_original_sender_invalid_packets_never_enter_any_transport() {
    let good = packet();
    for change in 0..4 {
        let mut s = SignedSubmission {
            signature: good.signature.clone(),
            bytes: good.bytes.clone(),
        };
        match change {
            0 => s.bytes.push(0),
            1 => s.signature = Signature::default().to_string(),
            2 => s.bytes = vec![0; 1233],
            _ => s.bytes[5] ^= 1,
        }
        let (sender, trace) = sender(SwqosType::Temporal, &s, vec![None], false, false);
        assert!(sender.send_packet(&s, false, 0).await.is_err());
        assert!(trace.packets.lock().unwrap().is_empty());
        assert!(trace.rpc.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn sdk_original_sender_tip_minimum_and_fixed_recipient_are_checked_before_building() {
    let s = packet();
    let (mut sender, trace) = sender(SwqosType::Temporal, &s, vec![None], false, false);
    sender.sender = Arc::new(Relay {
        provider: SwqosType::Temporal,
        minimum: Some(2),
        builtin: 0.001,
        fail: false,
        block: false,
        trace: trace.clone(),
    });
    assert!(sender.freeze_tip(0.000000001).is_err());
    assert!(sender.freeze_tip(0.0000000016).unwrap().is_some());
    assert_eq!(trace.tips.load(Ordering::SeqCst), 1);
    sender.check_min_tip = true;
    assert!(sender.freeze_tip(0.0001).is_err());
    assert!(sender.freeze_tip(0.001).unwrap().is_some());
    for value in [f64::NAN, f64::INFINITY, -0.1] {
        assert!(sender.freeze_tip(value).is_err());
    }
    assert!(trace.packets.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sdk_original_sender_cancellation_drops_the_single_send_without_background_retry() {
    let s = packet();
    let (sender, trace) = sender(SwqosType::Temporal, &s, vec![None], false, true);
    let task = tokio::spawn(async move { sender.send_packet(&s, false, 0).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while trace.packets.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert_eq!(trace.packets.lock().unwrap().len(), 1);
    assert!(trace.observations.lock().unwrap().is_empty());
}
