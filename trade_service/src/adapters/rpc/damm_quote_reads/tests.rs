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
use solana_sdk::{instruction::Instruction, message::Message};
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub(crate) const URL: &str = "https://damm-quote.example.invalid/?key=fixture-secret";
const BACKUP: &str = "https://backup.example.invalid/";
pub(crate) fn keys(count: u8) -> Vec<Pubkey> {
    (1..=count)
        .map(|byte| Pubkey::new_from_array([byte; 32]))
        .collect()
}
pub(crate) fn transaction() -> Transaction {
    Transaction::new_unsigned(Message::new_with_blockhash(
        &[Instruction {
            program_id: Pubkey::new_from_array([10; 32]),
            accounts: vec![],
            data: vec![7],
        }],
        Some(&keys(1)[0]),
        &Hash::new_from_array([9; 32]),
    ))
}
pub(crate) fn accounts(count: usize) -> Value {
    let mut value = vec![Value::Null; count];
    value[0] = json!({"owner":Pubkey::new_from_array([2;32]).to_string(),"lamports":u64::MAX,
        "data":["AQID","base64"],"executable":false,"rentEpoch":0,"space":3});
    json!({"context":{"slot":123},"value":value})
}
pub(crate) fn simulation(count: usize) -> Value {
    json!({"context":{"slot":124},"value":{"err":null,"logs":["public fixture log"],
        "accounts":vec![Value::Null;count],"unitsConsumed":999,"innerInstructions":[],
        "fee":5000,"preBalances":[u64::MAX,1],"postBalances":[u64::MAX-1,2],
        "preTokenBalances":[],"postTokenBalances":[],"loadedAddresses":{"writable":[],"readonly":[]}}})
}

#[derive(Clone)]
pub(crate) struct Legacy {
    pub(crate) calls: Arc<Mutex<Vec<(String, Value)>>>,
    accounts: Value,
    simulation: Value,
}
impl Legacy {
    pub(crate) fn new(accounts: Value, simulation: Value) -> Self {
        Self {
            calls: Arc::default(),
            accounts,
            simulation,
        }
    }
    pub(crate) fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
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
            self.calls
                .lock()
                .unwrap()
                .push((request.to_string(), params));
            Ok(match request {
                RpcRequest::GetMultipleAccounts => self.accounts.clone(),
                RpcRequest::SimulateTransaction => self.simulation.clone(),
                _ => panic!("quote oracle must not send or query another method"),
            })
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
    Missing,
    InvalidJson,
    Oversize,
    Http,
    Rate,
    Error,
    Hold,
    HoldProbe,
    Delay(Duration),
    DelayProbe(Duration),
    Late,
}
pub(crate) struct Fake {
    accounts: Value,
    simulation: Value,
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
pub(crate) fn runtime(
    accounts: Value,
    simulation: Value,
    mode: Mode,
) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &[URL.into(), BACKUP.into()],
        Fake {
            accounts,
            simulation,
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
                serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":{
                "context":{"slot":123},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":456}}}))?,
            ));
        }
        let response = match request {
            ReadRequest::DammQuoteAccounts { .. } => &self.accounts,
            ReadRequest::DammQuoteProbe { .. } => &self.simulation,
            _ => panic!("quote fake does not accept other profiles or sending"),
        };
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.entered.notify_one();
        match self.mode {
            Mode::Hold => std::future::pending::<()>().await,
            Mode::HoldProbe if body["method"] == "simulateTransaction" => {
                std::future::pending::<()>().await
            }
            Mode::Delay(delay) => tokio::time::sleep(delay).await,
            Mode::DelayProbe(delay) if body["method"] == "simulateTransaction" => {
                tokio::time::sleep(delay).await
            }
            Mode::Late => std::thread::sleep(Duration::from_millis(5100)),
            Mode::Error => anyhow::bail!("private transport diagnostic"),
            _ => {}
        }
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":response});
        match self.mode {
            Mode::WrongId => envelope["id"] = json!(999),
            Mode::WrongVersion => envelope["jsonrpc"] = json!("1.0"),
            Mode::Both => {
                envelope["error"] = json!({"code":-32005,"message":"private provider detail"})
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

async fn read(r: &SharedReadRuntime<Fake>, probe: bool) -> Result<Value> {
    let reader = DammQuoteReader::new(r, URL.into());
    if probe {
        Ok(serde_json::to_value(
            reader
                .damm_probe(&transaction(), &keys(8), 123, true)
                .await?,
        )?)
    } else {
        Ok(serde_json::to_value(
            reader.damm_accounts(&keys(8), Some(123)).await?,
        )?)
    }
}

#[tokio::test]
async fn damm_quote_read_exact_eight_nine_account_requests_and_full_responses_match_original() {
    for count in [8, 9] {
        for minimum in [None, Some(123)] {
            let old = Legacy::new(accounts(count), simulation(count));
            let r = runtime(accounts(count), simulation(count), Mode::Good);
            let expected = old
                .client()
                .damm_accounts(&keys(count as u8), minimum)
                .await
                .unwrap();
            let actual = DammQuoteReader::new(r.as_ref(), URL.into())
                .damm_accounts(&keys(count as u8), minimum)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                serde_json::to_value(actual).unwrap()
            );
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(old.calls.lock().unwrap()[0].1, calls[0].1["params"]);
            assert_eq!(calls[0].0, URL);
        }
    }
}

#[tokio::test]
async fn damm_quote_read_probe_bytes_config_and_rich_success_or_chain_error_match_original() {
    for count in [8, 9] {
        for error in [Value::Null, json!({"InstructionError":[2,{"Custom":6001}]})] {
            let mut response = simulation(count);
            response["value"]["err"] = error;
            let old = Legacy::new(accounts(count), response.clone());
            let r = runtime(accounts(count), response, Mode::Good);
            let tx = transaction();
            let expected = old
                .client()
                .damm_probe(&tx, &keys(count as u8), 123, true)
                .await
                .unwrap();
            let actual = DammQuoteReader::new(r.as_ref(), URL.into())
                .damm_probe(&tx, &keys(count as u8), 123, true)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                serde_json::to_value(actual).unwrap()
            );
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(old.calls.lock().unwrap()[0].1, calls[0].1["params"]);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(calls[0].1["params"][0].as_str().unwrap())
                .unwrap();
            assert_eq!(bytes, bincode::serialize(&tx).unwrap());
        }
    }
}

#[tokio::test]
async fn damm_quote_read_invalid_account_shape_alias_and_zero_minimum_reject_before_io() {
    for case in 0..4 {
        let r = runtime(accounts(8), simulation(8), Mode::Good);
        let mut k = keys(if case == 0 {
            7
        } else if case == 1 {
            10
        } else {
            8
        });
        if case == 2 {
            k[1] = k[0];
        }
        assert!(DammQuoteReader::new(r.as_ref(), URL.into())
            .damm_accounts(&k, Some(if case == 3 { 0 } else { 123 }))
            .await
            .is_err());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn damm_quote_read_invalid_probe_signatures_hash_payer_size_or_scope_reject_before_io() {
    for case in 0..9 {
        let mut tx = transaction();
        let mut k = keys(8);
        match case {
            0 => tx.signatures[0] = Signature::from([7; 64]),
            1 => tx.signatures.push(Signature::default()),
            2 => tx.message.recent_blockhash = Hash::default(),
            3 => tx.message.account_keys[0] = Pubkey::default(),
            4 => tx.message.instructions[0].data.resize(1400, 0),
            5 => tx.message.header.num_required_signatures = 2,
            6 => k[1] = k[0],
            _ => {}
        }
        let r = runtime(accounts(8), simulation(8), Mode::Good);
        assert!(DammQuoteReader::new(r.as_ref(), URL.into())
            .damm_probe(&tx, &k, if case == 7 { 0 } else { 123 }, case != 8)
            .await
            .is_err());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn damm_quote_read_bad_identity_schema_body_or_transport_never_fallback_or_leak() {
    for probe in [false, true] {
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
            let r = runtime(accounts(8), simulation(8), mode);
            let error = read(&r, probe).await.unwrap_err();
            assert!(!format!("{error:?}").contains("private"));
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, URL);
        }
    }
}

#[tokio::test]
async fn damm_quote_read_missing_explicit_probe_err_is_not_a_success() {
    let mut response = simulation(8);
    response["value"].as_object_mut().unwrap().remove("err");
    let old = Legacy::new(accounts(8), response.clone());
    assert!(old
        .client()
        .damm_probe(&transaction(), &keys(8), 123, true)
        .await
        .unwrap()
        .value
        .err
        .is_none());
    let r = runtime(accounts(8), response, Mode::Good);
    assert!(read(&r, true)
        .await
        .unwrap_err()
        .to_string()
        .contains("omitted explicit error outcome"));
}

#[tokio::test]
async fn damm_quote_read_semantic_bank_and_count_values_are_forwarded_for_original_typed_validators(
) {
    for slot in [0, 122, 123] {
        let mut a = accounts(7);
        a["context"]["slot"] = json!(slot);
        let r = runtime(a.clone(), simulation(8), Mode::Good);
        let old = Legacy::new(a, simulation(8));
        let expected = serde_json::to_value(
            old.client()
                .damm_accounts(&keys(8), Some(123))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(read(&r, false).await.unwrap(), expected);
    }
}

#[tokio::test]
async fn damm_quote_read_unregistered_endpoint_and_existing_profile_permits_send_nothing() {
    let r = runtime(accounts(8), simulation(8), Mode::Good);
    let reader = DammQuoteReader::new(r.as_ref(), "https://unregistered.example.invalid/".into());
    assert!(reader.damm_accounts(&keys(8), None).await.is_err());
    assert!(reader
        .damm_probe(&transaction(), &keys(8), 123, true)
        .await
        .is_err());
    let accounts = r.parameter_slot().unwrap();
    let simulation = r.aggregator_simulation_slot(URL).unwrap();
    assert!(read(&r, false).await.is_err());
    assert!(read(&r, true).await.is_err());
    drop((accounts, simulation));
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn damm_quote_read_rate_cools_account_probe_and_wsol_profiles_without_resending() {
    for first_probe in [false, true] {
        let r = runtime(accounts(8), simulation(8), Mode::Rate);
        assert!(read(&r, first_probe).await.is_err());
        assert!(read(&r, !first_probe).await.is_err());
        assert!(r.wsol_quote_slot(URL).is_err());
        assert!(r.aggregator_simulation_slot(URL).is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [8; 32]);
        assert_eq!(r.test_transport().calls.lock().unwrap()[1].0, BACKUP);
    }
}

#[tokio::test]
async fn damm_quote_read_cancel_releases_original_permits_and_preserves_hash_progress() {
    for probe in [false, true] {
        let r = runtime(accounts(8), simulation(8), Mode::Hold);
        {
            let pending = read(&r, probe);
            tokio::pin!(pending);
            tokio::select! {_=&mut pending=>panic!("held read completed"),_=r.test_transport().entered.notified()=>{}}
            assert!(read(&r, probe).await.is_err());
            assert!(r.latest_blockhash().await.is_ok());
        }
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        assert!(r.parameter_slot().is_ok());
        assert!(r.aggregator_simulation_slot(URL).is_ok());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn damm_quote_read_explicit_queries_share_existing_pacing_and_unique_ids() {
    let r = runtime(accounts(8), simulation(8), Mode::Good);
    for probe in [false, false, true, true] {
        read(&r, probe).await.unwrap();
    }
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .map(|c| c.1["id"].as_u64().unwrap())
            .collect::<HashSet<_>>()
            .len(),
        4
    );
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(240));
    assert!(calls[3].2.duration_since(calls[2].2) >= Duration::from_millis(90));
}

#[tokio::test]
async fn damm_quote_read_original_five_second_accounts_and_thirty_second_probe_deadlines_cancel() {
    for (probe, budget) in [(false, 5), (true, 30)] {
        let r = runtime(accounts(8), simulation(8), Mode::Hold);
        let start = Instant::now();
        assert!(read(&r, probe).await.is_err());
        assert!(start.elapsed() >= Duration::from_secs(budget));
        assert!(start.elapsed() < Duration::from_secs(budget + 3));
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert!(r.parameter_slot().is_ok() && r.aggregator_simulation_slot(URL).is_ok());
    }
}

#[tokio::test]
async fn damm_quote_read_late_ready_account_response_does_not_escape_its_deadline() {
    let r = runtime(accounts(8), simulation(8), Mode::Late);
    assert!(read(&r, false).await.is_err());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert!(r.parameter_slot().is_ok());
}

#[tokio::test]
async fn damm_quote_read_two_second_default_does_not_shrink_fixed_account_or_probe_profile() {
    for probe in [false, true] {
        let r = runtime(
            accounts(8),
            simulation(8),
            Mode::Delay(Duration::from_millis(2100)),
        );
        let start = Instant::now();
        read(&r, probe).await.unwrap();
        assert!(start.elapsed() >= Duration::from_secs(2));
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}
