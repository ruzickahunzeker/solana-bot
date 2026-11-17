use super::*;
use crate::{
    adapters::rpc::read_http::{HttpReply, ReadRequest},
    application::{wsol_queries::WsolQuoteInput, wsol_submission_types::WsolDirection},
};
use reqwest::{StatusCode, Url};
use solana_client::{
    client_error::Result as ClientResult,
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;
pub(crate) fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
#[derive(Clone)]
pub(crate) struct Fixture {
    pub(crate) owner: Pubkey,
    pub(crate) sol: u64,
    pub(crate) token: Option<u64>,
    pub(crate) rent: u64,
    pub(crate) fee: Option<u64>,
    pub(crate) corrupt: u8,
    pub(crate) overrides: HashMap<String, Value>,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            owner: key(1),
            sol: 10_000_000,
            token: Some(2_000_000),
            rent: 2_039_280,
            fee: Some(5000),
            corrupt: 0,
            overrides: HashMap::new(),
        }
    }
}
impl Fixture {
    pub(crate) fn ata(&self) -> Pubkey {
        spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&self.owner,&crate::domain::trade::WSOL_MINT.parse().unwrap(),&spl_token_interface::ID)
    }
    pub(crate) fn account(&self) -> Value {
        let Some(amount) = self.token else {
            return Value::Null;
        };
        let token = spl_token_interface::state::Account {
            mint: if self.corrupt == 2 {
                key(99)
            } else {
                crate::domain::trade::WSOL_MINT.parse().unwrap()
            },
            owner: if self.corrupt == 3 {
                key(99)
            } else {
                self.owner
            },
            amount,
            state: if self.corrupt == 5 {
                spl_token_interface::state::AccountState::Uninitialized
            } else {
                spl_token_interface::state::AccountState::Initialized
            },
            is_native: if self.corrupt == 4 {
                None.into()
            } else {
                Some(self.rent).into()
            },
            ..Default::default()
        };
        let mut data = vec![0; spl_token_interface::state::Account::LEN];
        spl_token_interface::state::Account::pack(token, &mut data).unwrap();
        let owner = if self.corrupt == 1 {
            key(99)
        } else {
            spl_token_interface::ID
        };
        json!({"lamports":amount.saturating_add(self.rent),"owner":owner.to_string(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"],"executable":false,"rentEpoch":0,"space":165})
    }
    pub(crate) fn result(&self, method: &str) -> Value {
        if let Some(value) = self.overrides.get(method) {
            return value.clone();
        }
        match method {
            "getBalance" => json!({"context":{"slot":42},"value":self.sol}),
            "getAccountInfo" => json!({"context":{"slot":42},"value":self.account()}),
            "getMinimumBalanceForRentExemption" => json!(self.rent),
            "getLatestBlockhash" => {
                json!({"context":{"slot":42},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":100}})
            }
            "getFeeForMessage" => json!({"context":{"slot":42},"value":self.fee}),
            _ => panic!("forbidden WSOL quote fixture method"),
        }
    }
    pub(crate) fn message(&self) -> Message {
        let (mut m, _) = crate::sdk_client::wsol_conversion::quote_message(
            &self.owner,
            WsolDirection::Wrap,
            1,
            0,
            self.token.is_some(),
            self.rent,
        )
        .unwrap();
        m.recent_blockhash = Hash::new_from_array([8; 32]);
        m
    }
    pub(crate) fn input(
        direction: WsolDirection,
        amount: Option<u64>,
        all: bool,
    ) -> WsolQuoteInput {
        WsolQuoteInput {
            wallet_id: "fixture-wallet".into(),
            owner_type: "telegram".into(),
            owner_id: "42".into(),
            direction,
            amount_lamports: amount,
            all,
        }
    }
    pub(crate) fn legacy(&self) -> Legacy {
        Legacy {
            fixture: self.clone(),
            calls: Arc::new(Mutex::new(vec![])),
        }
    }
}
#[derive(Clone)]
pub(crate) struct Legacy {
    fixture: Fixture,
    pub(crate) calls: Arc<Mutex<Vec<(String, Value)>>>,
}
impl Legacy {
    pub(crate) fn client(&self) -> RpcClient {
        RpcClient::new_sender(
            self.clone(),
            RpcClientConfig {
                commitment_config: CommitmentConfig::confirmed(),
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
    ) -> Pin<Box<dyn Future<Output = ClientResult<Value>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((request.to_string(), params));
            Ok(self.fixture.result(&request.to_string()))
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://wsol-quote-fixture".into()
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Good,
    Hold(usize),
    Rate(usize),
    WrongId(usize),
    Oversize(usize),
    Error(usize),
    Late(usize),
}
pub(crate) struct Fake {
    fixture: Fixture,
    mode: Mode,
    pub(crate) calls: Mutex<Vec<(String, Value, Instant)>>,
    count: AtomicUsize,
    pub(crate) active: AtomicUsize,
    pub(crate) entered: Notify,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
pub(crate) fn urls() -> Vec<String> {
    vec![
        "https://primary.example.invalid/?key=fixture-secret".into(),
        "https://backup.example.invalid".into(),
    ]
}
pub(crate) fn runtime(fixture: Fixture, mode: Mode) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &urls(),
        Fake {
            fixture,
            mode,
            calls: Mutex::new(vec![]),
            count: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            entered: Notify::new(),
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
            return Ok(HttpReply::for_test(StatusCode::OK,serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":Fixture::default().result("getLatestBlockhash")})).unwrap()));
        }
        assert!(matches!(request, ReadRequest::WsolQuote { .. }));
        let index = self.count.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.entered.notify_one();
        match self.mode {
            Mode::Hold(n) if n == index => std::future::pending::<()>().await,
            Mode::Rate(n) if n == index => {
                return Ok(HttpReply::for_test(StatusCode::TOO_MANY_REQUESTS, vec![]))
            }
            Mode::Oversize(n) if n == index => {
                return Ok(HttpReply::for_test(StatusCode::OK, vec![b' '; 65_537]))
            }
            Mode::Error(n) if n == index => anyhow::bail!("private transport detail"),
            Mode::Late(n) if n == index => std::thread::sleep(Duration::from_millis(2100)),
            _ => {}
        }
        let id = if matches!(self.mode,Mode::WrongId(n) if n==index) {
            json!(999)
        } else {
            body["id"].clone()
        };
        Ok(HttpReply::for_test(StatusCode::OK,serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":self.fixture.result(body["method"].as_str().unwrap())})).unwrap()))
    }
}
async fn stage(s: &mut WsolQuoteReadSession<'_, Fake>, f: &Fixture, n: usize) -> Result<()> {
    match n {
        0 => {
            s.balance().await?;
        }
        1 => {
            s.ata().await?;
        }
        2 => {
            s.rent().await?;
        }
        3 => {
            s.blockhash().await?;
        }
        _ => {
            s.fee(&f.message()).await?;
        }
    }
    Ok(())
}
pub(crate) async fn prepare(s: &mut WsolQuoteReadSession<'_, Fake>, f: &Fixture, n: usize) {
    for k in 0..n {
        stage(s, f, k).await.unwrap();
    }
}

#[tokio::test]
async fn wsol_quote_read_exact_five_methods_keep_commitment_rent_and_message() {
    let f = Fixture::default();
    let r = runtime(f.clone(), Mode::Good);
    let mut s =
        WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner).unwrap();
    prepare(&mut s, &f, 5).await;
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert_eq!(
        calls
            .iter()
            .map(|(_, v, _)| v["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "getBalance",
            "getAccountInfo",
            "getMinimumBalanceForRentExemption",
            "getLatestBlockhash",
            "getFeeForMessage"
        ]
    );
    assert_eq!(
        calls[0].1["params"],
        json!([f.owner.to_string(),{"commitment":"confirmed"}])
    );
    assert_eq!(calls[1].1["params"][0], f.ata().to_string());
    assert_eq!(calls[1].1["params"][1]["commitment"], "processed");
    assert_eq!(calls[2].1["params"], json!([165]));
    assert_eq!(calls[3].1["params"], json!([{"commitment":"confirmed"}]));
    assert_eq!(calls[4].1["params"][1], json!({"commitment":"confirmed"}));
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(calls[4].1["params"][0].as_str().unwrap())
            .unwrap(),
        f.message().serialize()
    );
}
#[tokio::test]
async fn wsol_quote_read_order_and_unknown_endpoint_cannot_add_requests() {
    let f = Fixture::default();
    let r = runtime(f.clone(), Mode::Good);
    assert!(WsolQuoteReadSession::new(
        &r,
        "https://unknown.example.invalid",
        CommitmentConfig::confirmed(),
        f.owner
    )
    .is_err());
    let mut s =
        WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner).unwrap();
    assert!(s.rent().await.is_err());
    assert!(s.fee(&f.message()).await.is_err());
    s.balance().await.unwrap();
    assert!(s.balance().await.is_err());
    prepare_after_balance(&mut s, &f).await;
    assert!(s.balance().await.is_err());
    assert!(s.fee(&f.message()).await.is_err());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 5);
}
async fn prepare_after_balance(s: &mut WsolQuoteReadSession<'_, Fake>, f: &Fixture) {
    for k in 1..5 {
        stage(s, f, k).await.unwrap();
    }
}
#[tokio::test]
async fn wsol_quote_read_missing_ata_is_explicit_but_bad_metadata_closes_scope() {
    for damage in 0..4 {
        let mut f = Fixture::default();
        f.token = None;
        if damage > 0 {
            let mut v = Fixture::default().result("getAccountInfo");
            match damage {
                1 => v["value"]["owner"] = json!("bad"),
                2 => v["value"]["data"] = json!(["bad!", "base64"]),
                _ => v["value"]["space"] = json!(99),
            };
            f.overrides.insert("getAccountInfo".into(), v);
        }
        let r = runtime(f.clone(), Mode::Good);
        let mut s =
            WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner)
                .unwrap();
        s.balance().await.unwrap();
        if damage == 0 {
            assert!(s.ata().await.unwrap().is_none());
        } else {
            assert!(s.ata().await.is_err());
            assert!(s.rent().await.is_err());
        }
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}
#[tokio::test]
async fn wsol_quote_read_fee_rejects_changed_payer_hash_signers_or_oversized_message() {
    let f = Fixture::default();
    for damage in 0..4 {
        let r = runtime(f.clone(), Mode::Good);
        let mut s =
            WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner)
                .unwrap();
        prepare(&mut s, &f, 4).await;
        let mut message = f.message();
        match damage {
            0 => message.account_keys[0] = key(99),
            1 => message.recent_blockhash = Hash::default(),
            2 => message.header.num_required_signatures = 3,
            _ => message.instructions[0].data = vec![0; 1233],
        };
        assert!(s.fee(&message).await.is_err());
        assert!(s.fee(&f.message()).await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 4);
    }
}
#[tokio::test]
async fn wsol_quote_read_scalar_context_and_hash_schema_faults_are_terminal() {
    for (n, method, bad) in [
        (0, "getBalance", json!({"context":{"slot":0},"value":1})),
        (0, "getBalance", json!({"context":{"slot":42},"value":-1})),
        (2, "getMinimumBalanceForRentExemption", json!(-1)),
        (
            3,
            "getLatestBlockhash",
            json!({"context":{"slot":42},"value":{"blockhash":"bad","lastValidBlockHeight":100}}),
        ),
        (
            4,
            "getFeeForMessage",
            json!({"context":{"slot":42},"value":null}),
        ),
    ] {
        let mut f = Fixture::default();
        f.overrides.insert(method.into(), bad);
        let r = runtime(f.clone(), Mode::Good);
        let mut s =
            WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner)
                .unwrap();
        prepare(&mut s, &f, n).await;
        assert!(stage(&mut s, &f, n).await.is_err());
        assert!(stage(&mut s, &f, n).await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), n + 1);
    }
}
#[tokio::test]
async fn wsol_quote_read_429_at_each_stage_is_shared_without_node_fallback() {
    let f = Fixture::default();
    for n in 0..5 {
        let r = runtime(f.clone(), Mode::Rate(n));
        let mut s =
            WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner)
                .unwrap();
        prepare(&mut s, &f, n).await;
        assert!(stage(&mut s, &f, n).await.is_err());
        assert!(
            WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner)
                .is_err()
        );
        r.latest_blockhash().await.unwrap();
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(calls.len(), n + 2);
        assert_eq!(
            calls.last().unwrap().0,
            Url::parse(&urls()[1]).unwrap().to_string()
        );
    }
}
#[tokio::test]
async fn wsol_quote_read_busy_cancel_at_each_stage_releases_quote_not_critical_capacity() {
    for n in 0..5 {
        let f = Fixture::default();
        let r = runtime(f.clone(), Mode::Hold(n));
        let task = {
            let r = r.clone();
            tokio::spawn(async move {
                let mut s = WsolQuoteReadSession::new(
                    &r,
                    &urls()[0],
                    CommitmentConfig::confirmed(),
                    f.owner,
                )
                .unwrap();
                for k in 0..5 {
                    stage(&mut s, &f, k).await?;
                }
                Ok::<(), anyhow::Error>(())
            })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if r.test_transport().count.load(Ordering::SeqCst) == n + 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(r.wsol_quote_slot(&urls()[0]).is_err());
        assert!(r.parameter_slot().is_ok());
        assert!(r.aggregator_simulation_slot(&urls()[0]).is_ok());
        r.latest_blockhash().await.unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
    }
}
#[tokio::test]
async fn wsol_quote_read_timeouts_in_first_and_last_stage_do_not_retry() {
    let f = Fixture::default();
    for n in [0, 4] {
        let r = runtime(f.clone(), Mode::Hold(n));
        let mut s =
            WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner)
                .unwrap();
        prepare(&mut s, &f, n).await;
        let start = Instant::now();
        assert!(stage(&mut s, &f, n).await.is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(stage(&mut s, &f, n).await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), n + 1);
        drop(s);
        assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
    }
}
#[tokio::test]
async fn wsol_quote_read_total_deadline_and_pacing_are_not_renewed() {
    let f = Fixture::default();
    let r = runtime(f.clone(), Mode::Good);
    let mut s =
        WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner).unwrap();
    prepare(&mut s, &f, 3).await;
    s.deadline = Instant::now();
    assert!(s.blockhash().await.is_err());
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls
        .windows(2)
        .all(|w| w[1].2.duration_since(w[0].2) >= Duration::from_millis(90)));
}
#[tokio::test]
async fn wsol_quote_read_late_identity_body_and_transport_faults_never_become_values() {
    let f = Fixture::default();
    for n in [0, 4] {
        for mode in [
            Mode::WrongId(n),
            Mode::Oversize(n),
            Mode::Error(n),
            Mode::Late(n),
        ] {
            let r = runtime(f.clone(), mode);
            let mut s =
                WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner)
                    .unwrap();
            prepare(&mut s, &f, n).await;
            let err = stage(&mut s, &f, n).await.unwrap_err();
            assert!(!format!("{err:?}").contains("private transport detail"));
            assert!(stage(&mut s, &f, n).await.is_err());
            assert_eq!(r.test_transport().calls.lock().unwrap().len(), n + 1);
            drop(s);
            assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
        }
    }
}
