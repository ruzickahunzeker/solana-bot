use super::*;
use crate::adapters::rpc::{
    damm_sell_simulation::tests::MemorySimulation,
    read_http::{HttpReply, ReadRequest},
};
use reqwest::{StatusCode, Url};
use solana_sdk::{instruction::Instruction, message::Message};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub(crate) const URL: &str = "https://damm.example.invalid/?key=fixture-secret";
const BACKUP: &str = "https://backup.example.invalid/";

pub(crate) fn transaction() -> VersionedTransaction {
    let payer = Pubkey::new_from_array([1; 32]);
    VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::Legacy(Message::new_with_blockhash(
            &[Instruction {
                program_id: Pubkey::new_from_array([2; 32]),
                accounts: vec![],
                data: vec![7],
            }],
            Some(&payer),
            &Hash::new_from_array([9; 32]),
        )),
    }
}
pub(crate) fn result() -> Value {
    json!({"context":{"slot":123},"value":{"err":null,"logs":[],"accounts":null,"unitsConsumed":1000,"returnData":null,"innerInstructions":[]}})
}

#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Good,
    WrongId,
    WrongVersion,
    Both,
    Missing,
    InvalidJson,
    Oversize,
    Http,
    Rate,
    Error,
    Hold,
    Delay(Duration),
    Late,
}
pub(crate) struct Fake {
    response: Value,
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
pub(crate) fn runtime(response: Value, mode: Mode) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &[URL.into(), BACKUP.into()],
        Fake {
            response,
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
        if matches!(request, ReadRequest::LatestBlockhash { .. }) {
            return Ok(HttpReply::for_test(
                StatusCode::OK,
                serde_json::to_vec(&json!({
                    "jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":123},"value":{
                    "blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":456}}
                }))?,
            ));
        }
        assert!(matches!(
            request,
            ReadRequest::DammCandidateSimulation { .. }
        ));
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.entered.notify_one();
        match self.mode {
            Mode::Hold => std::future::pending::<()>().await,
            Mode::Delay(delay) => tokio::time::sleep(delay).await,
            Mode::Late => std::thread::sleep(Duration::from_millis(4100)),
            Mode::Error => anyhow::bail!("private transport diagnostic"),
            _ => {}
        }
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":self.response});
        match self.mode {
            Mode::WrongId => envelope["id"] = json!(999),
            Mode::WrongVersion => envelope["jsonrpc"] = json!("1.0"),
            Mode::Both => {
                envelope["error"] = json!({"code":-32005,"message":"private provider diagnostic"})
            }
            Mode::Missing => {
                envelope.as_object_mut().unwrap().remove("result");
            }
            _ => {}
        }
        let mut bytes = serde_json::to_vec(&envelope)?;
        if matches!(self.mode, Mode::InvalidJson) {
            bytes = b"private invalid JSON".to_vec();
        }
        if matches!(self.mode, Mode::Oversize) {
            bytes.resize(524_289, b' ');
        }
        Ok(HttpReply::for_test(
            match self.mode {
                Mode::Rate => StatusCode::TOO_MANY_REQUESTS,
                Mode::Http => StatusCode::BAD_GATEWAY,
                _ => StatusCode::OK,
            },
            bytes,
        ))
    }
}
async fn lookup(r: &SharedReadRuntime<Fake>) -> Result<DammSimulationResult> {
    simulate_unsigned_at_slot(
        &DammSimulationReader::new(r, URL.into()),
        &transaction(),
        123,
    )
    .await
}

#[tokio::test]
async fn damm_candidate_shared_exact_unsigned_packet_config_slot_and_program_outcome_match_oracle()
{
    for error in [Value::Null, json!({"InstructionError":[2,{"Custom":6001}]})] {
        let mut response = result();
        response["value"]["err"] = error;
        let mut old = MemorySimulation::new();
        old.response = response.clone();
        let r = runtime(response, Mode::Good);
        let expected = super::super::simulate_unsigned_at_slot(&old.client(), &transaction(), 123)
            .await
            .unwrap();
        let actual = lookup(&r).await.unwrap();
        assert_eq!(actual.error, expected.error);
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(*old.calls.lock().unwrap(), [calls[0].1["params"].clone()]);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(calls[0].1["params"][0].as_str().unwrap())
            .unwrap();
        assert_eq!(bytes, bincode::serialize(&transaction()).unwrap());
        assert_eq!(calls[0].0, URL);
    }
}

#[tokio::test]
async fn damm_candidate_shared_invalid_packet_and_zero_quote_slot_reject_before_any_io() {
    for case in 0..9 {
        let mut tx = transaction();
        match case {
            0 => tx.signatures.clear(),
            1 => tx.signatures.push(Signature::default()),
            2 => tx.signatures[0] = Signature::from([7; 64]),
            3..=7 => {
                let VersionedMessage::Legacy(m) = &mut tx.message else {
                    unreachable!()
                };
                match case {
                    3 => m.recent_blockhash = Hash::default(),
                    4 => m.instructions[0].data.resize(1400, 0),
                    5 => m.header.num_required_signatures = 2,
                    6 => m.account_keys[0] = Pubkey::default(),
                    _ => m.account_keys.clear(),
                }
            }
            _ => {
                let VersionedMessage::Legacy(m) = tx.message else {
                    unreachable!()
                };
                tx.message = VersionedMessage::V0(solana_sdk::message::v0::Message {
                    header: m.header,
                    account_keys: m.account_keys,
                    recent_blockhash: m.recent_blockhash,
                    instructions: m.instructions,
                    address_table_lookups: vec![],
                });
            }
        }
        let r = runtime(result(), Mode::Good);
        let reader = DammSimulationReader::new(r.as_ref(), URL.into());
        assert!(simulate_unsigned_at_slot(&reader, &tx, 123).await.is_err());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
    }
    let r = runtime(result(), Mode::Good);
    assert!(simulate_unsigned_at_slot(
        &DammSimulationReader::new(r.as_ref(), URL.into()),
        &transaction(),
        0
    )
    .await
    .is_err());
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn damm_candidate_shared_context_mutations_and_bad_scalar_match_oracle_rejection() {
    for case in 0..7 {
        let mut response = result();
        match case {
            0 => response["context"]["slot"] = json!(0),
            1 => response["context"]["slot"] = json!(122),
            2 => {
                response["value"]["replacementBlockhash"] = json!({"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":999})
            }
            3 => {
                response["value"]["loadedAddresses"] =
                    json!({"writable":[Pubkey::new_from_array([8;32]).to_string()],"readonly":[]})
            }
            4 => {
                response["value"]["loadedAddresses"] =
                    json!({"writable":[],"readonly":[Pubkey::new_from_array([8;32]).to_string()]})
            }
            5 => response["value"]["unitsConsumed"] = json!("invalid"),
            _ => response["value"]["err"] = json!("invalid"),
        }
        let mut old = MemorySimulation::new();
        old.response = response.clone();
        assert!(
            super::super::simulate_unsigned_at_slot(&old.client(), &transaction(), 123)
                .await
                .is_err()
        );
        let r = runtime(response, Mode::Good);
        assert!(lookup(&r).await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn damm_candidate_shared_absent_empty_loaded_addresses_and_later_slot_remain_valid() {
    for loaded in [Value::Null, json!({"writable":[],"readonly":[]})] {
        let mut response = result();
        response["context"]["slot"] = json!(u64::MAX);
        response["value"]["loadedAddresses"] = loaded;
        assert!(lookup(&runtime(response, Mode::Good))
            .await
            .unwrap()
            .error
            .is_none());
    }
}

#[tokio::test]
async fn damm_candidate_shared_bad_envelopes_transport_and_oversize_never_retry_or_leak() {
    for mode in [
        Mode::WrongId,
        Mode::WrongVersion,
        Mode::Both,
        Mode::Missing,
        Mode::InvalidJson,
        Mode::Oversize,
        Mode::Http,
        Mode::Error,
    ] {
        let r = runtime(result(), mode);
        let error = lookup(&r).await.err().unwrap();
        assert!(!format!("{error:?}").contains("private"));
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, URL);
    }
}

#[tokio::test]
async fn damm_candidate_shared_missing_explicit_err_tightens_only_malformed_success() {
    let mut response = result();
    response["value"].as_object_mut().unwrap().remove("err");
    let mut old = MemorySimulation::new();
    old.response = response.clone();
    assert!(
        super::super::simulate_unsigned_at_slot(&old.client(), &transaction(), 123)
            .await
            .unwrap()
            .error
            .is_none()
    );
    let error = lookup(&runtime(response, Mode::Good)).await.err().unwrap();
    assert!(error.to_string().contains("omitted explicit error outcome"));
}

#[tokio::test]
async fn damm_candidate_shared_unknown_endpoint_or_busy_existing_simulation_permit_send_nothing() {
    let r = runtime(result(), Mode::Good);
    let reader = DammSimulationReader::new(r.as_ref(), "https://unknown.example.invalid".into());
    assert!(simulate_unsigned_at_slot(&reader, &transaction(), 123)
        .await
        .is_err());
    let permit = r.aggregator_simulation_slot(URL).unwrap();
    assert!(lookup(&r).await.is_err());
    drop(permit);
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn damm_candidate_shared_rate_cools_other_profiles_without_failover_or_retry() {
    let r = runtime(result(), Mode::Rate);
    assert!(lookup(&r).await.is_err());
    assert!(lookup(&r).await.is_err());
    assert!(r.wsol_quote_slot(URL).is_err());
    assert!(r.aggregator_simulation_slot(URL).is_err());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [8; 32]);
    assert_eq!(r.test_transport().calls.lock().unwrap()[1].0, BACKUP);
}

#[tokio::test]
async fn damm_candidate_shared_cancel_releases_permit_and_keeps_hash_progress_independent() {
    let r = runtime(result(), Mode::Hold);
    {
        let pending = lookup(&r);
        tokio::pin!(pending);
        tokio::select! { _=&mut pending=>panic!("held simulation completed"), _=r.test_transport().entered.notified()=>{} }
        assert!(lookup(&r).await.is_err());
        assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [8; 32]);
    }
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    assert!(r.aggregator_simulation_slot(URL).is_ok());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn damm_candidate_shared_original_four_second_deadline_and_late_ready_response_fail_closed() {
    for mode in [Mode::Hold, Mode::Late] {
        let r = runtime(result(), mode);
        let start = Instant::now();
        assert!(lookup(&r).await.is_err());
        assert!(start.elapsed() >= Duration::from_secs(4));
        assert!(start.elapsed() < Duration::from_secs(6));
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert!(r.aggregator_simulation_slot(URL).is_ok());
    }
}

#[tokio::test]
async fn damm_candidate_shared_explicit_second_query_reuses_pacing_and_unique_ids() {
    let r = runtime(result(), Mode::Good);
    lookup(&r).await.unwrap();
    lookup(&r).await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_ne!(calls[0].1["id"], calls[1].1["id"]);
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(90));
}

#[tokio::test]
async fn damm_candidate_shared_response_after_two_seconds_preserves_original_four_second_budget() {
    let r = runtime(result(), Mode::Delay(Duration::from_millis(2100)));
    let start = Instant::now();
    assert!(lookup(&r).await.unwrap().error.is_none());
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}
