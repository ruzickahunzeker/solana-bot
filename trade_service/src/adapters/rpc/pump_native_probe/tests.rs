use super::*;
use crate::adapters::rpc::{
    read_http::{HttpReply, ReadHttpTransport, ReadRequest},
    read_runtime::SharedReadRuntime,
};
use reqwest::{StatusCode, Url};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::Message,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use std::time::Duration;
use tokio::sync::Notify;

const URL: &str = "http://127.0.0.1:12345";
const BACKUP: &str = "http://127.0.0.1:12346";
mod wire;

pub(crate) fn fixture() -> (Transaction, [Pubkey; EVIDENCE_ACCOUNTS]) {
    let wallet = Pubkey::new_from_array([1; 32]);
    let mut data = BUY_EXACT_SOL_IN_DISCRIMINATOR.to_vec();
    data.extend(1_000_000u64.to_le_bytes());
    data.extend(1u64.to_le_bytes());
    data.push(1);
    let ix = Instruction {
        program_id: accounts::PUMPFUN,
        accounts: vec![AccountMeta::new(wallet, true)],
        data,
    };
    (
        Transaction::new_unsigned(Message::new_with_blockhash(
            &[ix],
            Some(&wallet),
            &Hash::new_from_array([10; 32]),
        )),
        std::array::from_fn(|i| Pubkey::new_from_array([20 + i as u8; 32])),
    )
}

fn packet(tx: &Transaction, keys: &[Pubkey; EVIDENCE_ACCOUNTS]) -> PumpNativeProbePacket {
    PumpNativeProbePacket::new(tx, keys, 175, Instant::now() + Duration::from_secs(2)).unwrap()
}

#[test]
fn pump_native_read_packet_freezes_exact_zero_signature_bytes_and_fixed_config() {
    let (mut tx, mut keys) = fixture();
    let frozen = bincode::serialize(&tx).unwrap();
    let plan = packet(&tx, &keys);
    let body = plan.body(17);
    assert_eq!(
        body,
        json!({"jsonrpc":"2.0","id":17,"method":"simulateTransaction","params":[
        base64::engine::general_purpose::STANDARD.encode(&frozen),
        {"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
            "accounts":{"encoding":"base64","addresses":keys.map(|key|key.to_string())},
            "minContextSlot":175,"innerInstructions":true}]})
    );
    tx.message.recent_blockhash = Hash::new_from_array([11; 32]);
    keys.swap(0, 1);
    assert_eq!(plan.body(17), body);
    let reply = plan
        .binding()
        .complete(json!({"fixture":"opaque"}))
        .unwrap();
    assert!(reply.for_probe(&tx, &keys, 175).is_err());
}

#[test]
fn pump_native_read_packet_rejects_signed_bad_context_and_non_discovery_messages() {
    for damage in 0..16 {
        let (mut tx, mut keys) = fixture();
        let mut minimum = 175;
        let mut until = Instant::now() + Duration::from_secs(2);
        match damage {
            0 => minimum = 0,
            1 => until = Instant::now(),
            2 => until = Instant::now() + Duration::from_secs(10),
            3 => tx.signatures[0] = Signature::from([2; 64]),
            4 => tx
                .message
                .instructions
                .push(tx.message.instructions[0].clone()),
            5 => tx.message.instructions[0].data[16..24].copy_from_slice(&2u64.to_le_bytes()),
            6 => tx.message.instructions[0].data[0] = 0,
            7 => tx.message.instructions[0].data.truncate(16),
            8 => tx.message.instructions[0].program_id_index = 255,
            9 => keys[1] = keys[0],
            10 => tx.message.recent_blockhash = Hash::default(),
            11 => keys[0] = Pubkey::default(),
            12 => tx.message.instructions[0].data[24] = 2,
            13 => tx.signatures.push(Signature::default()),
            14 => tx.message.instructions[0].data.resize(2_048, 0),
            _ => {
                let program = usize::from(tx.message.instructions[0].program_id_index);
                tx.message.account_keys[program] = solana_system_interface::program::ID;
            }
        }
        assert!(
            PumpNativeProbePacket::new(&tx, &keys, minimum, until).is_err(),
            "damage {damage}"
        );
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Good,
    WrongId,
    WrongVersion,
    MissingErr,
    OldSlot,
    RpcError,
    Both,
    WrongCount,
    Oversize,
    Rate,
    Hold,
    Transport,
}
struct Fake {
    mode: Mode,
    calls: Mutex<Vec<(String, Value)>>,
    entered: Notify,
    dropped: AtomicUsize,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        assert!(matches!(request, ReadRequest::PumpNativeProbe { .. }));
        let body = request.test_body();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone()));
        self.entered.notify_one();
        let _active = Active(&self.dropped);
        match self.mode {
            Mode::Hold => std::future::pending::<()>().await,
            Mode::Transport => anyhow::bail!("https://fixture.invalid/?key=fixture-secret"),
            Mode::Rate => return Ok(HttpReply::for_test(StatusCode::TOO_MANY_REQUESTS, vec![])),
            Mode::Oversize => return Ok(HttpReply::for_test(StatusCode::OK, vec![b' '; 524_289])),
            _ => (),
        }
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":175},"value":{
            "err":null,"accounts":vec![Value::Null;EVIDENCE_ACCOUNTS]}}});
        match self.mode {
            Mode::WrongId => envelope["id"] = json!(999),
            Mode::WrongVersion => envelope["jsonrpc"] = json!("1.0"),
            Mode::MissingErr => {
                envelope["result"]["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("err");
            }
            Mode::OldSlot => envelope["result"]["context"]["slot"] = json!(174),
            Mode::RpcError | Mode::Both => {
                envelope["error"] =
                    json!({"code":-32005,"message":"https://fixture.invalid/?key=fixture-secret"});
                if matches!(self.mode, Mode::RpcError) {
                    envelope.as_object_mut().unwrap().remove("result");
                }
            }
            Mode::WrongCount => {
                envelope["result"]["value"]["accounts"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }
            _ => (),
        }
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&envelope)?,
        ))
    }
}
fn runtime(mode: Mode) -> SharedReadRuntime<Fake> {
    SharedReadRuntime::with_test_transport(
        &[URL.into(), BACKUP.into()],
        Fake {
            mode,
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            dropped: AtomicUsize::new(0),
        },
    )
}

#[tokio::test]
async fn pump_native_read_reply_is_bound_to_packet_roles_slot_and_original_shared_runtime() {
    let (tx, keys) = fixture();
    for damage in 0..4 {
        let runtime = runtime(Mode::Good);
        let plan = packet(&tx, &keys);
        let body = plan.body(1);
        let reply = runtime.pump_native_probe(URL, plan).await.unwrap();
        let mut actual = tx.clone();
        let mut actual_keys = keys;
        let minimum = if damage == 3 { 176 } else { 175 };
        if damage == 1 {
            actual.message.recent_blockhash = Hash::new_from_array([11; 32]);
        }
        if damage == 2 {
            actual_keys.swap(0, 1);
        }
        let result = reply.for_probe(&actual, &actual_keys, minimum);
        assert_eq!(result.is_ok(), damage == 0);
        assert_eq!(
            *runtime.test_transport().calls.lock().unwrap(),
            [(Url::parse(URL).unwrap().to_string(), body)]
        );
        assert!(runtime.aggregator_simulation_slot(URL).is_ok());
    }
}

#[tokio::test]
async fn pump_native_read_rejects_invalid_envelopes_and_never_queries_backup() {
    let (tx, keys) = fixture();
    for mode in [
        Mode::WrongId,
        Mode::WrongVersion,
        Mode::MissingErr,
        Mode::OldSlot,
        Mode::RpcError,
        Mode::Both,
        Mode::WrongCount,
        Mode::Oversize,
        Mode::Transport,
    ] {
        let runtime = runtime(mode);
        let error = runtime
            .pump_native_probe(URL, packet(&tx, &keys))
            .await
            .err()
            .expect("bad reply accepted");
        assert!(!format!("{error:#}").contains("fixture-secret"));
        assert_eq!(runtime.test_transport().calls.lock().unwrap().len(), 1);
        assert!(runtime.aggregator_simulation_slot(URL).is_ok());
    }
}

#[tokio::test]
async fn pump_native_read_429_uses_existing_endpoint_cooldown_without_fallback() {
    let (tx, keys) = fixture();
    let runtime = runtime(Mode::Rate);
    assert!(runtime
        .pump_native_probe(URL, packet(&tx, &keys))
        .await
        .is_err());
    assert!(runtime
        .pump_native_probe(URL, packet(&tx, &keys))
        .await
        .is_err());
    assert!(runtime.aggregator_simulation_slot(URL).is_err());
    assert!(runtime.aggregator_simulation_slot(BACKUP).is_ok());
    assert_eq!(runtime.test_transport().calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn pump_native_read_cancel_deadline_and_unknown_endpoint_release_original_quota() {
    let (tx, keys) = fixture();
    let runtime = runtime(Mode::Hold);
    assert!(runtime
        .pump_native_probe("http://127.0.0.1:12347", packet(&tx, &keys))
        .await
        .is_err());
    assert!(runtime.test_transport().calls.lock().unwrap().is_empty());
    {
        let read = runtime.pump_native_probe(URL, packet(&tx, &keys));
        tokio::pin!(read);
        tokio::select! {
            result=&mut read => panic!("held request completed: {}",result.is_ok()),
            entered=tokio::time::timeout(Duration::from_secs(1),runtime.test_transport().entered.notified()) => entered.unwrap(),
        }
        assert!(runtime.aggregator_simulation_slot(URL).is_err());
    }
    assert!(runtime.aggregator_simulation_slot(URL).is_ok());
    assert_eq!(runtime.test_transport().dropped.load(Ordering::SeqCst), 1);
    let short = Instant::now() + Duration::from_millis(150);
    let plan = PumpNativeProbePacket::new(&tx, &keys, 175, short).unwrap();
    assert!(runtime.pump_native_probe(URL, plan).await.is_err());
    assert!(Instant::now() >= short);
    assert!(runtime.aggregator_simulation_slot(URL).is_ok());
}

#[tokio::test]
async fn pump_native_read_packet_and_completed_reply_expire_without_extending_origin() {
    let (tx, keys) = fixture();
    let runtime = runtime(Mode::Good);
    let mut plan = packet(&tx, &keys);
    plan.deadline = Instant::now() - Duration::from_secs(1);
    assert!(runtime.pump_native_probe(URL, plan).await.is_err());
    assert!(runtime.test_transport().calls.lock().unwrap().is_empty());
    let mut reply = runtime
        .pump_native_probe(URL, packet(&tx, &keys))
        .await
        .unwrap();
    reply.binding.deadline = Instant::now() - Duration::from_secs(1);
    assert!(reply.for_probe(&tx, &keys, 175).is_err());
}
