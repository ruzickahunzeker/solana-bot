use super::*;
use crate::adapters::rpc::read_http::{HttpReply, ReadRequest};
use reqwest::{StatusCode, Url};
use solana_client::{
    client_error::Result as RpcResult,
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Signature};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
pub(crate) const URL: &str = "https://primary.example.invalid/?key=fixture-secret";
const BACKUP: &str = "https://backup.example.invalid";
pub(crate) fn unsigned(partial: bool) -> UnsignedWsolTransaction {
    super::super::tests::unsigned_case(
        crate::application::wsol_submission_types::WsolDirection::Unwrap,
        if partial { 50 } else { 100 },
        100,
        true,
    )
}
pub(crate) fn quote(p: &UnsignedWsolTransaction) -> WsolQuoteSnapshot {
    p.authorization.quote.clone()
}
pub(crate) fn signed(partial: bool) -> SignedWsolTransaction {
    let p = unsigned(partial);
    let mut transaction = Transaction::new_unsigned(p.message.clone());
    transaction.signatures.fill(Signature::from([7; 64]));
    SignedWsolTransaction {
        owner: p.owner,
        authorized_message: p.message,
        signature: transaction.signatures[0].to_string(),
        signed_bytes: bincode::serialize(&transaction).unwrap(),
        transaction,
    }
}
pub(crate) fn result(error: Value) -> Value {
    json!({"err":error,"logs":["public fixture log"],"accounts":null,"unitsConsumed":999,"loadedAccountsDataSize":7,
        "returnData":{"programId":Pubkey::new_from_array([9;32]).to_string(),"data":["AQI=","base64"]},"innerInstructions":[],
        "replacementBlockhash":null,"fee":0,"preBalances":[u64::MAX,1],"postBalances":[u64::MAX-1,2],
        "preTokenBalances":[],"postTokenBalances":[],"loadedAddresses":{"writable":[],"readonly":[]}})
}
#[derive(Clone)]
pub(crate) struct Legacy {
    pub(crate) calls: Arc<Mutex<Vec<Value>>>,
    value: Value,
}
impl Legacy {
    pub(crate) fn new(value: Value) -> Self {
        Self {
            calls: Arc::default(),
            value,
        }
    }
    pub(crate) fn client(&self, commitment: CommitmentConfig) -> RpcClient {
        RpcClient::new_sender(
            self.clone(),
            RpcClientConfig {
                commitment_config: commitment,
                ..Default::default()
            },
        )
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
            assert_eq!(request, RpcRequest::SimulateTransaction);
            self.calls.lock().unwrap().push(params);
            Ok(json!({"context":{"slot":42},"value":self.value}))
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        URL.into()
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Good,
    WrongId,
    WrongVersion,
    Both,
    Omitted,
    RpcError,
    InvalidJson,
    Oversize,
    Rate,
    Hold,
    Late,
    Error,
    Http,
    ZeroContext,
}
pub(crate) struct Fake {
    value: Value,
    mode: Mode,
    pub(crate) calls: Mutex<Vec<(String, Value, Instant)>>,
    pub(crate) entered: Notify,
    pub(crate) active: AtomicUsize,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
pub(crate) fn runtime(value: Value, mode: Mode) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &[URL.into(), BACKUP.into()],
        Fake {
            value,
            mode,
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            active: AtomicUsize::new(0),
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
                serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":42},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":50}}}),
                )?,
            ));
        }
        assert!(matches!(request, ReadRequest::WsolSimulation { .. }));
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.entered.notify_one();
        match self.mode {
            Mode::Hold => std::future::pending::<()>().await,
            Mode::Late => std::thread::sleep(Duration::from_millis(2100)),
            Mode::Error => anyhow::bail!("private transport detail"),
            _ => {}
        }
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":42},"value":self.value}});
        match self.mode {
            Mode::WrongId => envelope["id"] = json!(999),
            Mode::WrongVersion => envelope["jsonrpc"] = json!("1.0"),
            Mode::Both => {
                envelope["error"] = json!({"code":-32005,"message":"private provider detail"})
            }
            Mode::Omitted => {
                envelope.as_object_mut().unwrap().remove("result");
            }
            Mode::RpcError => {
                envelope = json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32005,"message":"private provider detail"}})
            }
            Mode::ZeroContext => envelope["result"]["context"]["slot"] = json!(0),
            _ => {}
        }
        let mut bytes = serde_json::to_vec(&envelope)?;
        if matches!(self.mode, Mode::InvalidJson) {
            bytes = b"private invalid json".to_vec();
        }
        if matches!(self.mode, Mode::Oversize) {
            bytes.resize(524_289, b' ');
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
async fn lookup(
    r: &SharedReadRuntime<Fake>,
    signed_mode: bool,
) -> Result<RpcSimulateTransactionResult> {
    if signed_mode {
        simulate_signed(&signed(true), r, URL, CommitmentConfig::confirmed()).await
    } else {
        let p = unsigned(true);
        simulate_unsigned(&p, &quote(&p), r, URL, CommitmentConfig::confirmed()).await
    }
}
#[tokio::test]
async fn wsol_simulation_shared_unsigned_four_shapes_and_commitments_match_original_packet_and_full_result(
) {
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
            let p = super::super::tests::unsigned_case(direction, amount, balance, exists);
            let q = quote(&p);
            let old = Legacy::new(result(Value::Null));
            let r = runtime(result(Value::Null), Mode::Good);
            let expected = p.simulate(&old.client(commitment), &q).await.unwrap();
            let actual = simulate_unsigned(&p, &q, &r, URL, commitment)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                serde_json::to_value(actual).unwrap()
            );
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(*old.calls.lock().unwrap(), [calls[0].1["params"].clone()]);
            let tx: Transaction = bincode::deserialize(
                &base64::engine::general_purpose::STANDARD
                    .decode(calls[0].1["params"][0].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(tx.message, p.message);
            assert!(tx.signatures.iter().all(|s| *s == Signature::default()));
        }
    }
}
#[tokio::test]
async fn wsol_simulation_shared_signed_one_two_signers_keep_original_bytes_config_and_report_errors(
) {
    for partial in [false, true] {
        for commitment in [
            CommitmentConfig::processed(),
            CommitmentConfig::confirmed(),
            CommitmentConfig::finalized(),
        ] {
            for error in [Value::Null, json!({"InstructionError":[0,{"Custom":42}]})] {
                let p = signed(partial);
                let before = p.signed_bytes.clone();
                let old = Legacy::new(result(error.clone()));
                let r = runtime(result(error), Mode::Good);
                let expected = p.simulate(&old.client(commitment)).await.unwrap();
                let actual = simulate_signed(&p, &r, URL, commitment).await.unwrap();
                assert_eq!(
                    serde_json::to_value(expected).unwrap(),
                    serde_json::to_value(actual).unwrap()
                );
                let calls = r.test_transport().calls.lock().unwrap();
                assert_eq!(calls.len(), 1);
                assert_eq!(*old.calls.lock().unwrap(), [calls[0].1["params"].clone()]);
                assert_eq!(
                    base64::engine::general_purpose::STANDARD
                        .decode(calls[0].1["params"][0].as_str().unwrap())
                        .unwrap(),
                    before
                );
                assert_eq!(p.signed_bytes, before);
            }
        }
    }
}
#[tokio::test]
async fn wsol_simulation_shared_unsigned_program_rejection_keeps_original_text_and_kind() {
    let error = json!({"InstructionError":[0,{"Custom":42}]});
    let p = unsigned(true);
    let q = quote(&p);
    let old = p
        .simulate(
            &Legacy::new(result(error.clone())).client(CommitmentConfig::confirmed()),
            &q,
        )
        .await
        .unwrap_err();
    let r = runtime(result(error), Mode::Good);
    let new = simulate_unsigned(&p, &q, &r, URL, CommitmentConfig::confirmed())
        .await
        .unwrap_err();
    assert_eq!(old.to_string(), new.to_string());
    assert_eq!(
        new.downcast_ref::<WsolFailure>().unwrap().kind,
        crate::application::wsol_failure::WsolFailureKind::Rejected
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn wsol_simulation_shared_bad_unsigned_binding_expiry_message_and_temp_signer_precede_network(
) {
    for case in 0..7 {
        let mut p = unsigned(true);
        let mut q = quote(&p);
        match case {
            0 => q.amount_lamports += 1,
            1 => q.wallet_id.push('x'),
            2 => {
                q.expires_at_ms = 0;
                p.authorization.quote = q.clone();
            }
            3 => p.owner = Pubkey::new_from_array([99; 32]),
            4 => p.message.recent_blockhash = Hash::default(),
            5 => p.temporary_signer = None,
            _ => p.message.instructions[0].data.push(0),
        }
        let r = runtime(result(Value::Null), Mode::Good);
        let old = Legacy::new(result(Value::Null));
        let before = p
            .simulate(&old.client(CommitmentConfig::confirmed()), &q)
            .await
            .unwrap_err();
        let after = simulate_unsigned(
            &p,
            &q,
            &r,
            "https://unregistered.example.invalid",
            CommitmentConfig::confirmed(),
        )
        .await
        .unwrap_err();
        assert_eq!(before.to_string(), after.to_string());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
        assert!(old.calls.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn wsol_simulation_shared_signed_snapshot_mutations_reject_before_endpoint_resolution() {
    for case in 0..8 {
        let mut p = signed(false);
        match case {
            0 => p.owner = Pubkey::new_from_array([99; 32]),
            1 => p.authorized_message.account_keys[0] = Pubkey::new_from_array([99; 32]),
            2 => p.transaction.message.instructions[0].data.push(0),
            3 => p.transaction.signatures.clear(),
            4 => p.transaction.signatures.push(Signature::default()),
            5 => p.signature = "wrong".into(),
            6 => p.signed_bytes.push(0),
            _ => p.transaction.signatures[0] = Signature::from([8; 64]),
        }
        let r = runtime(result(Value::Null), Mode::Good);
        let old = Legacy::new(result(Value::Null));
        let before = p
            .simulate(&old.client(CommitmentConfig::confirmed()))
            .await
            .unwrap_err();
        let after = simulate_signed(
            &p,
            &r,
            "https://unregistered.example.invalid",
            CommitmentConfig::confirmed(),
        )
        .await
        .unwrap_err();
        assert_eq!(before.to_string(), after.to_string());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
        assert!(old.calls.lock().unwrap().is_empty());
    }
}
#[test]
fn wsol_simulation_shared_packet_checks_length_and_signer_count_before_allocation() {
    for case in 0..3 {
        let mut tx = Transaction::new_unsigned(unsigned(false).message);
        match case {
            0 => tx.message.header.num_required_signatures = 0,
            1 => tx.signatures.clear(),
            _ => tx.message.instructions[0].data = vec![0; 1233],
        }
        assert!(WsolSimulationPacket::encode(&tx, CommitmentConfig::confirmed()).is_err());
    }
}
#[tokio::test]
async fn wsol_simulation_shared_rich_optional_report_fields_and_raw_u64_are_not_rewritten() {
    let mut value = result(Value::Null);
    value["unitsConsumed"] = json!(u64::MAX);
    value["fee"] = json!(0);
    value["replacementBlockhash"] =
        json!({"blockhash":Hash::new_from_array([9;32]).to_string(),"lastValidBlockHeight":100});
    let r = runtime(value.clone(), Mode::Good);
    let old = Legacy::new(value);
    let p = signed(false);
    let actual = simulate_signed(&p, &r, URL, CommitmentConfig::confirmed())
        .await
        .unwrap();
    let expected = p
        .simulate(&old.client(CommitmentConfig::confirmed()))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(
        p.transaction.message.recent_blockhash,
        Hash::new_from_array([8; 32])
    );
}
#[tokio::test]
async fn wsol_simulation_shared_bad_envelopes_context_body_or_transport_never_succeed_or_fallback()
{
    for signed_mode in [false, true] {
        for mode in [
            Mode::WrongId,
            Mode::WrongVersion,
            Mode::Both,
            Mode::Omitted,
            Mode::RpcError,
            Mode::InvalidJson,
            Mode::Oversize,
            Mode::Error,
            Mode::Http,
            Mode::ZeroContext,
        ] {
            let r = runtime(result(Value::Null), mode);
            let error = lookup(&r, signed_mode).await.unwrap_err();
            assert!(!format!("{error:?}").contains("private"));
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, URL);
        }
    }
}
#[tokio::test]
async fn wsol_simulation_shared_unknown_endpoint_and_existing_aggregator_quota_send_nothing() {
    let r = runtime(result(Value::Null), Mode::Good);
    let p = signed(true);
    assert!(simulate_signed(
        &p,
        &r,
        "https://unregistered.example.invalid",
        CommitmentConfig::confirmed()
    )
    .await
    .is_err());
    let permit = r.aggregator_simulation_slot(URL).unwrap();
    assert!(lookup(&r, false).await.is_err());
    assert!(lookup(&r, true).await.is_err());
    drop(permit);
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
    assert!(r.wsol_quote_slot(URL).is_ok());
}
#[tokio::test]
async fn wsol_simulation_shared_429_cools_original_endpoint_across_profiles_without_retry() {
    for signed_mode in [false, true] {
        let r = runtime(result(Value::Null), Mode::Rate);
        assert!(lookup(&r, signed_mode).await.is_err());
        assert!(lookup(&r, signed_mode).await.is_err());
        assert!(r.wsol_quote_slot(URL).is_err());
        assert!(r.aggregator_simulation_slot(URL).is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn wsol_simulation_shared_cancel_both_paths_releases_permit_and_preserves_critical_hash() {
    for signed_mode in [false, true] {
        let r = runtime(result(Value::Null), Mode::Hold);
        {
            let pending = lookup(&r, signed_mode);
            tokio::pin!(pending);
            tokio::select! {_=&mut pending=>panic!("held simulation completed"),_=r.test_transport().entered.notified()=>{}}
            assert!(r.aggregator_simulation_slot(URL).is_err());
            assert!(lookup(&r, !signed_mode).await.is_err());
            r.latest_blockhash().await.unwrap();
        }
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        assert!(r.aggregator_simulation_slot(URL).is_ok());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}
#[tokio::test]
async fn wsol_simulation_shared_timeout_and_late_poll_do_not_retry_or_return_result() {
    for signed_mode in [false, true] {
        for mode in [Mode::Hold, Mode::Late] {
            let r = runtime(result(Value::Null), mode);
            let start = Instant::now();
            assert!(lookup(&r, signed_mode).await.is_err());
            assert!(start.elapsed() < Duration::from_secs(4));
            assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
            assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
            assert!(r.aggregator_simulation_slot(URL).is_ok());
        }
    }
}
#[tokio::test]
async fn wsol_simulation_shared_explicit_later_queries_reuse_one_budget_and_unique_ids() {
    let r = runtime(result(Value::Null), Mode::Good);
    lookup(&r, false).await.unwrap();
    lookup(&r, true).await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].1["id"], calls[1].1["id"]);
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(90));
}
#[tokio::test]
async fn wsol_simulation_shared_missing_units_remain_none_and_invalid_scalar_is_not_zero() {
    for units in [Value::Null, json!("bad")] {
        let mut value = result(Value::Null);
        value["unitsConsumed"] = units.clone();
        let r = runtime(value.clone(), Mode::Good);
        let old = Legacy::new(value);
        let p = unsigned(false);
        let q = quote(&p);
        let expected = p
            .simulate(&old.client(CommitmentConfig::confirmed()), &q)
            .await;
        let actual = simulate_unsigned(&p, &q, &r, URL, CommitmentConfig::confirmed()).await;
        assert_eq!(expected.is_ok(), actual.is_ok());
        if let Ok(actual) = actual {
            assert!(units.is_null());
            assert_eq!(actual.units_consumed, None);
            assert_eq!(actual.fee, Some(0));
        }
    }
}
