use super::*;
use crate::{
    adapters::rpc::read_http::{HttpReply, ReadRequest},
    domain::trade::Address,
};
use reqwest::{StatusCode, Url};
use solana_program_pack::Pack;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{AddressLookupTableAccount, Message},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::sync::Notify;

#[derive(Clone)]
pub(crate) struct Fixture {
    pub(crate) wallet: Pubkey,
    pub(crate) mint: Pubkey,
    pub(crate) target: Pubkey,
    pub(crate) program: Pubkey,
    pub(crate) transaction: VersionedTransaction,
    pub(crate) keys: Vec<Pubkey>,
    pub(crate) target_index: u8,
}
pub(crate) fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
pub(crate) fn fixture(token_program: Pubkey, v0: bool) -> Fixture {
    let (wallet, mint) = (key(1), key(2));
    let target = sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &wallet,
        &mint,
        &token_program,
    );
    let ix = Instruction {
        program_id: key(9),
        accounts: vec![
            AccountMeta::new(target, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![7],
    };
    let table = AddressLookupTableAccount {
        key: key(8),
        addresses: vec![target, mint],
    };
    let message = if v0 {
        VersionedMessage::V0(
            solana_sdk::message::v0::Message::try_compile(
                &wallet,
                &[ix],
                &[table.clone()],
                Hash::new_from_array([7; 32]),
            )
            .unwrap(),
        )
    } else {
        VersionedMessage::Legacy(Message::new_with_blockhash(
            &[ix],
            Some(&wallet),
            &Hash::new_from_array([7; 32]),
        ))
    };
    let mut keys = message.static_account_keys().to_vec();
    if let Some(lookups) = message.address_table_lookups() {
        for lookup in lookups {
            for index in &lookup.writable_indexes {
                keys.push(table.addresses[usize::from(*index)]);
            }
        }
        for lookup in lookups {
            for index in &lookup.readonly_indexes {
                keys.push(table.addresses[usize::from(*index)]);
            }
        }
    }
    let target_index = keys.iter().position(|key| *key == target).unwrap() as u8;
    Fixture {
        wallet,
        mint,
        target,
        program: token_program,
        transaction: VersionedTransaction {
            signatures: vec![Signature::default()],
            message,
        },
        keys,
        target_index,
    }
}
impl Fixture {
    pub(crate) fn loaded(&self) -> Value {
        let start = self.transaction.message.static_account_keys().len();
        let writable = self
            .transaction
            .message
            .address_table_lookups()
            .unwrap_or(&[])
            .iter()
            .map(|lookup| lookup.writable_indexes.len())
            .sum::<usize>();
        json!({"writable":self.keys[start..start+writable].iter().map(ToString::to_string).collect::<Vec<_>>(),"readonly":self.keys[start+writable..].iter().map(ToString::to_string).collect::<Vec<_>>()})
    }
    pub(crate) fn plan(&self) -> UnsignedAggregatorSimulation {
        UnsignedAggregatorSimulation::new(
            &self.transaction,
            self.wallet,
            self.target,
            self.target_index,
            &self.keys,
        )
        .unwrap()
    }
    pub(crate) fn token(&self, amount: u64) -> Value {
        let account = spl_token_interface::state::Account {
            mint: self.mint,
            owner: self.wallet,
            amount,
            delegate: Default::default(),
            state: spl_token_interface::state::AccountState::Initialized,
            is_native: Default::default(),
            delegated_amount: 0,
            close_authority: Default::default(),
        };
        let mut data = vec![0; spl_token_interface::state::Account::LEN];
        spl_token_interface::state::Account::pack(account, &mut data).unwrap();
        crate::adapters::rpc::read_runtime::aggregator_accounts::tests::ui(self.program, &data)
    }
    pub(crate) fn baseline(&self) -> Value {
        let mut payer = crate::adapters::rpc::read_runtime::aggregator_accounts::tests::ui(
            Pubkey::default(),
            &[],
        );
        payer["lamports"] = json!(1_000_000_000u64);
        json!({"context":{"slot":42},"value":[payer,self.token(10)]})
    }
    pub(crate) fn simulation(&self) -> Value {
        let mut payer = self.baseline()["value"][0].clone();
        payer["lamports"] = json!(999_990_000u64);
        let balance = |amount: &str| json!({"accountIndex":self.target_index,"mint":self.mint.to_string(),"owner":self.wallet.to_string(),"programId":self.program.to_string(),"uiTokenAmount":{"amount":amount,"decimals":6,"uiAmount":null,"uiAmountString":"unused"}});
        let mut pre = vec![0u64; self.keys.len()];
        pre[0] = 1_000_000_000;
        let mut post = pre.clone();
        post[0] = 999_990_000;
        json!({"context":{"slot":42},"value":{"err":null,"logs":[],"accounts":[payer,self.token(30)],"unitsConsumed":1234,"fee":5000,"preBalances":pre,"postBalances":post,"preTokenBalances":[balance("10")],"postTokenBalances":[balance("30")],"innerInstructions":[]}})
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Good,
    Hold(bool),
    Rate(bool),
    WrongId(bool),
    Oversize(bool),
    Error(bool),
    Late(bool),
}
pub(crate) struct Fake {
    baseline: Value,
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
pub(crate) fn urls() -> Vec<String> {
    vec![
        "https://primary.example.invalid/?key=fixture-secret".into(),
        "https://backup.example.invalid".into(),
    ]
}
pub(crate) fn runtime(
    baseline: Value,
    simulation: Value,
    mode: Mode,
) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &urls(),
        Fake {
            baseline,
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
        let id = body["id"].clone();
        if matches!(request, ReadRequest::LatestBlockhash { .. }) {
            return Ok(HttpReply::for_test(StatusCode::OK,serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":42},"value":{"blockhash":key(7).to_string(),"lastValidBlockHeight":456}}})).unwrap()));
        }
        let ReadRequest::AggregatorSimulation { minimum_slot, .. } = request else {
            panic!("simulation fixture cannot query other methods")
        };
        let simulation = minimum_slot.is_some();
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.entered.notify_one();
        match self.mode {
            Mode::Hold(stage) if stage == simulation => std::future::pending::<()>().await,
            Mode::Rate(stage) if stage == simulation => {
                return Ok(HttpReply::for_test(StatusCode::TOO_MANY_REQUESTS, vec![]))
            }
            Mode::Oversize(stage) if stage == simulation => {
                return Ok(HttpReply::for_test(StatusCode::OK, vec![b' '; 524_289]))
            }
            Mode::Error(stage) if stage == simulation => anyhow::bail!("private transport detail"),
            Mode::Late(stage) if stage == simulation => {
                std::thread::sleep(Duration::from_millis(2100))
            }
            _ => {}
        }
        let id = if matches!(self.mode,Mode::WrongId(stage) if stage==simulation) {
            json!(999)
        } else {
            id
        };
        Ok(HttpReply::for_test(StatusCode::OK,serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":if simulation {&self.simulation} else {&self.baseline}})).unwrap()))
    }
}
fn case() -> Fixture {
    fixture(spl_token_interface::ID, false)
}

#[test]
fn aggregator_sim_read_plan_binds_legacy_and_v0_unsigned_packet_and_loaded_keys() {
    for v0 in [false, true] {
        let f = fixture(spl_token_interface::ID, v0);
        let p = f.plan();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&p.packet)
                .unwrap(),
            bincode::serialize(&f.transaction).unwrap()
        );
        assert_eq!(
            p.baseline_body(7),
            json!({"jsonrpc":"2.0","id":7,"method":"getMultipleAccounts","params":[[f.wallet.to_string(),f.target.to_string()],{"encoding":"base64","commitment":"processed"}]})
        );
        let simulated = p.simulation_body(8, 42);
        assert_eq!(simulated["method"], "simulateTransaction");
        assert_eq!(
            simulated["params"][1],
            json!({"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64","accounts":{"encoding":"base64","addresses":[f.wallet.to_string(),f.target.to_string()]},"minContextSlot":42,"innerInstructions":true})
        );
        assert_eq!(
            p.loaded_writable.len() + p.loaded_readonly.len(),
            f.keys.len() - f.transaction.message.static_account_keys().len()
        );
    }
}
#[test]
fn aggregator_sim_read_plan_rejects_signed_mismatched_or_unbounded_candidates() {
    for damage in 0..8 {
        let mut f = case();
        match damage {
            0 => f.transaction.signatures[0] = Signature::from([1; 64]),
            1 => f.transaction.signatures.clear(),
            2 => f.wallet = key(99),
            3 => f.target_index = 255,
            4 => f.keys.push(f.wallet),
            5 => f.keys[0] = key(99),
            6 => {
                let VersionedMessage::Legacy(m) = &mut f.transaction.message else {
                    unreachable!()
                };
                m.instructions[0].data = vec![0; 1232];
            }
            _ => f.target = f.mint,
        }
        assert!(
            UnsignedAggregatorSimulation::new(
                &f.transaction,
                f.wallet,
                f.target,
                f.target_index,
                &f.keys
            )
            .is_err(),
            "damage={damage}"
        );
    }
}

#[tokio::test]
async fn aggregator_sim_read_session_is_one_shot_and_uses_its_baseline_slot() {
    let f = case();
    let r = runtime(f.baseline(), f.simulation(), Mode::Good);
    let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
    assert!(s.simulate().await.is_err());
    assert_eq!(s.baseline().await.unwrap().context.slot, 42);
    assert!(s.baseline().await.is_err());
    assert_eq!(s.simulate().await.unwrap().context.slot, 42);
    assert!(s.simulate().await.is_err());
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].1["params"][1]["minContextSlot"], 42);
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(90));
}
#[tokio::test]
async fn aggregator_sim_read_bad_baseline_closes_sequence_before_simulation() {
    let f = case();
    for damage in 0..5 {
        let mut baseline = f.baseline();
        match damage {
            0 => baseline["context"]["slot"] = json!(0),
            1 => baseline["value"] = json!([]),
            2 => baseline["value"][0]["owner"] = json!("bad"),
            3 => baseline["value"][0]["data"] = json!(["bad!", "base64"]),
            _ => baseline["value"][0]["space"] = json!(99),
        };
        let r = runtime(baseline, f.simulation(), Mode::Good);
        let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
        assert!(s.baseline().await.is_err());
        assert!(s.baseline().await.is_err());
        assert!(s.simulate().await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn aggregator_sim_read_bad_response_context_accounts_or_loaded_keys_never_succeed() {
    let f = fixture(spl_token_interface::ID, true);
    for damage in 0..6 {
        let mut simulation = f.simulation();
        match damage {
            0 => simulation["context"]["slot"] = json!(41),
            1 => simulation["value"]["accounts"] = json!([]),
            2 => simulation["value"]["accounts"][1]["data"] = json!(["bad!", "base64"]),
            3 => {
                simulation["value"]["replacementBlockhash"] =
                    json!({"blockhash":key(7).to_string(),"lastValidBlockHeight":456})
            }
            4 => simulation["value"]["loadedAddresses"] = json!({"writable":[],"readonly":[]}),
            _ => simulation["value"]["fee"] = json!(-1),
        }
        let r = runtime(f.baseline(), simulation, Mode::Good);
        let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
        s.baseline().await.unwrap();
        assert!(s.simulate().await.is_err());
        assert!(s.simulate().await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}
#[tokio::test]
async fn aggregator_sim_read_cancellation_in_either_stage_releases_only_its_scope() {
    for stage in [false, true] {
        let f = case();
        let r = runtime(f.baseline(), f.simulation(), Mode::Hold(stage));
        let task = {
            let r = r.clone();
            tokio::spawn(async move {
                let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
                s.baseline().await?;
                s.simulate().await
            })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if r.test_transport().calls.lock().unwrap().len() == usize::from(stage) + 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(r.aggregator_simulation_slot(&urls()[0]).is_err());
        assert!(r.parameter_slot().is_ok());
        assert!(r.sol_hop_slot().is_ok());
        r.latest_blockhash().await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        assert!(r.aggregator_simulation_slot(&urls()[0]).is_ok());
    }
}
#[tokio::test]
async fn aggregator_sim_read_timeout_in_either_stage_is_terminal_without_retry() {
    let f = case();
    for stage in [false, true] {
        let r = runtime(f.baseline(), f.simulation(), Mode::Hold(stage));
        let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
        if stage {
            s.baseline().await.unwrap();
        }
        let started = Instant::now();
        if stage {
            assert!(s.simulate().await.is_err());
        } else {
            assert!(s.baseline().await.is_err());
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(s.simulate().await.is_err());
        assert_eq!(
            r.test_transport().calls.lock().unwrap().len(),
            usize::from(stage) + 1
        );
        drop(s);
        assert!(r.aggregator_simulation_slot(&urls()[0]).is_ok());
    }
}
#[tokio::test]
async fn aggregator_sim_read_429_is_shared_and_does_not_switch_the_fixed_sequence() {
    let f = case();
    for stage in [false, true] {
        let r = runtime(f.baseline(), f.simulation(), Mode::Rate(stage));
        let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
        if stage {
            s.baseline().await.unwrap();
            assert!(s.simulate().await.is_err());
        } else {
            assert!(s.baseline().await.is_err());
        }
        assert!(r
            .aggregator_account(
                &urls()[0],
                Address::from_bytes([1; 32]),
                solana_commitment_config::CommitmentConfig::processed()
            )
            .await
            .is_err());
        r.latest_blockhash().await.unwrap();
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(calls.len(), usize::from(stage) + 2);
        assert_eq!(
            calls.last().unwrap().0,
            Url::parse(&urls()[1]).unwrap().to_string()
        );
    }
}
#[tokio::test]
async fn aggregator_sim_read_scope_deadline_is_not_renewed_between_stages() {
    let f = case();
    let r = runtime(f.baseline(), f.simulation(), Mode::Good);
    assert!(
        AggregatorSimulationReadSession::new(&r, "https://unknown.example.invalid", f.plan())
            .is_err()
    );
    let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
    assert!(s.deadline.saturating_duration_since(Instant::now()) <= Duration::from_secs(10));
    s.baseline().await.unwrap();
    s.deadline = Instant::now();
    assert!(s.simulate().await.is_err());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn aggregator_sim_read_late_identity_body_and_transport_faults_never_retry_or_leak() {
    let f = case();
    for stage in [false, true] {
        for mode in [
            Mode::WrongId(stage),
            Mode::Oversize(stage),
            Mode::Error(stage),
            Mode::Late(stage),
        ] {
            let r = runtime(f.baseline(), f.simulation(), mode);
            let mut s = AggregatorSimulationReadSession::new(&r, &urls()[0], f.plan()).unwrap();
            let error = if stage {
                s.baseline().await.unwrap();
                s.simulate().await.unwrap_err()
            } else {
                s.baseline().await.unwrap_err()
            };
            assert!(!format!("{error:?}").contains("private transport detail"));
            assert!(s.simulate().await.is_err());
            assert_eq!(
                r.test_transport().calls.lock().unwrap().len(),
                usize::from(stage) + 1
            );
            drop(s);
            assert!(r.aggregator_simulation_slot(&urls()[0]).is_ok());
        }
    }
}
