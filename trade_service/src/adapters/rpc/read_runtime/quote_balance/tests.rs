use super::*;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

fn addr(byte: u8) -> Address {
    Address::from_bytes([byte; 32])
}
fn token_amount(units: u64, decimals: u8) -> Value {
    json!({"amount":units.to_string(),"decimals":decimals,"uiAmount":null,"uiAmountString":units.to_string()})
}
fn token_account(byte: u8, units: u64) -> Value {
    json!({"pubkey":addr(byte).to_string(),"account":{"owner":spl_token_interface::ID.to_string(),"executable":false,"lamports":1,"rentEpoch":0,"space":165,
        "data":{"program":"spl-token","space":165,"parsed":{"type":"account","info":{"mint":addr(9).to_string(),"owner":addr(1).to_string(),"state":"initialized","tokenAmount":token_amount(units,6)}}}}})
}
fn accounts() -> Value {
    json!([token_account(3, 9007199254740993), token_account(4, 7)])
}
fn envelope(id: u64, value: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":value}})
}
#[derive(Clone, Copy)]
enum Mode {
    Success,
    Empty,
    Unparsed,
    ScanError,
    FirstAtaError,
    LastAtaError,
    BothAtaZero,
    InvalidMint,
    Mixed,
    Hold,
    Slow,
    SlowFallback,
    RateScan,
    PrimaryFailed,
    Oversize,
}
struct Fake {
    mode: Mode,
    calls: Mutex<Vec<(String, Value)>>,
    entered: Notify,
    dropped: Arc<AtomicUsize>,
}
struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        self.calls
            .lock()
            .unwrap()
            .push((url.host_str().unwrap().into(), request.body()));
        let _drop = DropCount(self.dropped.clone());
        self.entered.notify_one();
        if matches!(self.mode, Mode::Slow | Mode::SlowFallback) {
            tokio::time::sleep(Duration::from_millis(1800)).await;
        }
        if matches!(self.mode, Mode::PrimaryFailed)
            && url.host_str() == Some("primary.example.invalid")
        {
            return Ok(HttpReply {
                status: StatusCode::SERVICE_UNAVAILABLE,
                retry_after: None,
                body: vec![],
            });
        }
        let (id,value) = match request {
            ReadRequest::PreparationAccounts { .. } | ReadRequest::Parameters { .. } | ReadRequest::AggregatorAccount { .. } | ReadRequest::AggregatorSimulation { .. } | ReadRequest::WsolQuote { .. } => panic!("quote fixture must not read preparation accounts or protocol parameters or aggregator accounts/simulation/WSOL quote"),
            ReadRequest::TokenSupply{id,..} => (*id,token_amount(u64::MAX,6)),
            ReadRequest::TokenAccounts{id,..} => {
                if matches!(self.mode,Mode::Hold) { std::future::pending::<()>().await; }
                if matches!(self.mode,Mode::RateScan) { return Ok(HttpReply{status:StatusCode::TOO_MANY_REQUESTS,retry_after:Some(60),body:vec![]}); }
                if matches!(self.mode,Mode::ScanError|Mode::FirstAtaError|Mode::LastAtaError|Mode::BothAtaZero|Mode::SlowFallback) {
                    return Ok(HttpReply{status:StatusCode::OK,retry_after:None,body:serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"error":{"code":-32005,"message":"fixture-secret"}})).unwrap()});
                }
                let mut value=accounts();
                match self.mode {
                    Mode::Empty => value=json!([]),
                    Mode::Unparsed => value[0]["account"]["data"]=json!(["","base64"]),
                    Mode::InvalidMint => value[0]["account"]["data"]["parsed"]["info"]["mint"]=json!(addr(99).to_string()),
                    Mode::Mixed => value[1]["account"]["data"]=json!(["","base64"]),
                    _ => {}
                }
                if matches!(self.mode,Mode::Unparsed) { value.as_array_mut().unwrap().pop(); }
                if matches!(self.mode,Mode::Oversize) { let mut bytes=serde_json::to_vec(&envelope(*id,value)).unwrap();bytes.resize(super::super::super::read_http::MAX_TOKEN_BODY+1,b' ');return Ok(HttpReply{status:StatusCode::OK,retry_after:None,body:bytes}); }
                (*id,value)
            }
            ReadRequest::TokenAccountBalance{id,account} => {
                let first=*account==atas(addr(1),addr(9))[0];
                if (matches!(self.mode,Mode::FirstAtaError)&&first)||(matches!(self.mode,Mode::LastAtaError)&&!first) {
                    return Ok(HttpReply{status:StatusCode::SERVICE_UNAVAILABLE,retry_after:None,body:vec![]});
                }
                (*id,token_amount(if matches!(self.mode,Mode::BothAtaZero|Mode::LastAtaError|Mode::SlowFallback){0}else{23},6))
            }
            ReadRequest::Balance{id,..} => (*id,json!(123456789)),
            ReadRequest::LatestBlockhash{id} => return Ok(HttpReply{status:StatusCode::OK,retry_after:None,body:serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":{"blockhash":bs58::encode([7u8;32]).into_string(),"lastValidBlockHeight":456}}})).unwrap()}),
            ReadRequest::SignatureStatus{id,..} => (*id,json!([null])),
            ReadRequest::NativeBalances{id,wallets} => (*id,Value::Array(wallets.iter().map(|_|json!({"owner":addr(0).to_string(),"lamports":7,"data":["","base64"],"executable":false,"rentEpoch":0})).collect())),
        };
        Ok(HttpReply {
            status: StatusCode::OK,
            retry_after: None,
            body: serde_json::to_vec(&envelope(id, value)).unwrap(),
        })
    }
}
fn urls() -> Vec<String> {
    vec![
        "https://primary.example.invalid/?key=fixture-secret".into(),
        "https://backup.example.invalid".into(),
    ]
}
fn runtime(mode: Mode) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_transport(
        &urls(),
        Fake {
            mode,
            calls: Mutex::new(vec![]),
            entered: Notify::new(),
            dropped: Arc::new(AtomicUsize::new(0)),
        },
    ))
}
fn methods(runtime: &SharedReadRuntime<Fake>) -> Vec<String> {
    runtime
        .transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|(_, r)| r["method"].as_str().unwrap().into())
        .collect()
}

#[test]
fn quote_balance_codec_request_matches_sdk_processed_config_and_body_limits() {
    use solana_client::rpc_config::RpcAccountInfoConfig;
    let config = RpcAccountInfoConfig {
        encoding: Some(solana_account_decoder::UiAccountEncoding::JsonParsed),
        commitment: Some(solana_commitment_config::CommitmentConfig::processed()),
        data_slice: None,
        min_context_slot: None,
    };
    let request = ReadRequest::TokenAccounts {
        id: 7,
        wallet: addr(1),
        mint: addr(9),
    };
    assert_eq!(
        request.body(),
        json!({"jsonrpc":"2.0","id":7,"method":"getTokenAccountsByOwner","params":[addr(1).to_string(),{"mint":addr(9).to_string()},config]})
    );
    assert_eq!(request.max_body(), 524288);
    assert_eq!(
        ReadRequest::TokenSupply {
            id: 1,
            mint: addr(9)
        }
        .max_body(),
        65536
    );
    assert_eq!(
        ReadRequest::TokenAccountBalance {
            id: 1,
            account: addr(3)
        }
        .max_body(),
        65536
    );
    assert_eq!(
        ReadRequest::Balance {
            id: 1,
            wallet: addr(1)
        }
        .max_body(),
        65536
    );
}

#[test]
fn quote_balance_codec_preserves_exact_integers_saturation_empty_and_token2022() {
    assert_eq!(
        scan_balance(accounts(), addr(1), addr(9), 6).unwrap(),
        Some(9007199254741000)
    );
    assert_eq!(
        scan_balance(json!([]), addr(1), addr(9), 6).unwrap(),
        Some(0)
    );
    assert_eq!(
        scan_balance(
            json!([token_account(3, u64::MAX), token_account(4, 7)]),
            addr(1),
            addr(9),
            6
        )
        .unwrap(),
        Some(u64::MAX)
    );
    let mut value = json!([token_account(3, 0)]);
    value[0]["account"]["owner"] = json!(spl_token_2022_interface::ID.to_string());
    value[0]["account"]["data"]["program"] = json!("spl-token-2022");
    value[0]["account"]["data"]["parsed"]["info"]["state"] = json!("frozen");
    assert_eq!(scan_balance(value, addr(1), addr(9), 6).unwrap(), Some(0));
    for decimals in [0, 6, 9, 255] {
        assert_eq!(
            amount(token_amount(u64::MAX, decimals)).unwrap().decimals,
            decimals
        );
    }
}

#[test]
fn quote_balance_codec_rejects_wrong_wallet_mint_program_type_precision_and_amount() {
    for (pointer, value) in [
        ("/0/pubkey", json!("0")),
        ("/0/account/owner", json!(addr(0).to_string())),
        ("/0/account/executable", json!(true)),
        ("/0/account/data/program", json!("spl-token-2022")),
        ("/0/account/data/parsed/type", json!("mint")),
        (
            "/0/account/data/parsed/info/mint",
            json!(addr(99).to_string()),
        ),
        (
            "/0/account/data/parsed/info/owner",
            json!(addr(99).to_string()),
        ),
        ("/0/account/data/parsed/info/state", json!("uninitialized")),
        ("/0/account/data/parsed/info/tokenAmount/decimals", json!(9)),
        (
            "/0/account/data/parsed/info/tokenAmount/amount",
            json!("-1"),
        ),
        ("/0/account/data/parsed/info/tokenAmount/amount", json!(1)),
        (
            "/0/account/data/parsed/info/tokenAmount/amount",
            json!("18446744073709551616"),
        ),
    ] {
        let mut accounts = accounts();
        *accounts.pointer_mut(pointer).unwrap() = value;
        assert!(scan_balance(accounts, addr(1), addr(9), 6).is_err());
    }
}

#[test]
fn quote_balance_codec_rejects_duplicate_mixed_incomplete_and_excessive_scans() {
    let mut value = accounts();
    value[1]["pubkey"] = value[0]["pubkey"].clone();
    assert!(scan_balance(value, addr(1), addr(9), 6).is_err());
    let mut value = accounts();
    value[1]["account"]["data"] = json!(["", "base64"]);
    assert!(scan_balance(value, addr(1), addr(9), 6).is_err());
    let mut value = json!([token_account(3, 1)]);
    value[0]["account"]["data"] = json!(["", "base64"]);
    assert_eq!(scan_balance(value, addr(1), addr(9), 6).unwrap(), None);
    let mut value = json!([token_account(3, 1)]);
    value[0]["account"]["data"]["program"] = json!("unknown-program");
    assert_eq!(scan_balance(value, addr(1), addr(9), 6).unwrap(), None);
    assert!(scan_balance(json!([null]), addr(1), addr(9), 6).is_err());
    assert!(scan_balance(Value::Array(vec![json!({}); 1001]), addr(1), addr(9), 6).is_err());
}

#[test]
fn quote_balance_codec_rpc_identity_and_error_classes_never_expose_provider_body() {
    let good = envelope(1, json!(7));
    for (pointer, value) in [
        ("/id", json!(2)),
        ("/jsonrpc", json!("1.0")),
        ("/result/context/slot", json!(-1)),
    ] {
        let mut reply = good.clone();
        *reply.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            decode_envelope(1, &serde_json::to_vec(&reply).unwrap()),
            Err(StepError::Invalid)
        );
    }
    let error = json!({"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"fixture-secret"}});
    assert_eq!(
        decode_envelope(1, &serde_json::to_vec(&error).unwrap()),
        Err(StepError::Unavailable)
    );
    assert!(!StepError::Unavailable
        .error()
        .to_string()
        .contains("fixture-secret"));
    let mut both = good;
    both["error"] = error["error"].clone();
    assert_eq!(
        decode_envelope(1, &serde_json::to_vec(&both).unwrap()),
        Err(StepError::Invalid)
    );
    assert_eq!(
        decode_envelope(1, b"fixture-secret"),
        Err(StepError::Invalid)
    );
    for raw in [
        json!({"jsonrpc":"2.0","id":1,"result":null}),
        json!({"jsonrpc":"2.0","id":1,"error":"fixture-secret"}),
    ] {
        assert_eq!(
            decode_envelope(1, &serde_json::to_vec(&raw).unwrap()),
            Err(StepError::Invalid)
        );
    }
}

#[test]
fn quote_balance_codec_ata_derivation_matches_existing_library_without_signers() {
    for wallet in [0, 1, 42, 255] {
        for mint in [0, 9, 99, 255] {
            let expected=[spl_token_interface::ID,spl_token_2022_interface::ID].map(|program| Address::from_bytes(sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(&solana_sdk::pubkey::Pubkey::new_from_array([wallet;32]),&solana_sdk::pubkey::Pubkey::new_from_array([mint;32]),&program).to_bytes()));
            assert_eq!(atas(addr(wallet), addr(mint)), expected);
        }
    }
}

#[tokio::test]
async fn quote_balance_shared_read_success_uses_three_fixed_methods_and_exact_units() {
    let runtime = runtime(Mode::Success);
    assert_eq!(
        runtime
            .quote_balance(&urls()[0], addr(1), addr(9))
            .await
            .unwrap(),
        QuoteBalance {
            decimals: 6,
            balance_units: 9007199254741000,
            balance_lamports: 123456789
        }
    );
    assert_eq!(
        methods(&runtime),
        ["getTokenSupply", "getTokenAccountsByOwner", "getBalance"]
    );
    assert_eq!(runtime.slots[4].available_permits(), 1);
}

#[tokio::test]
async fn quote_balance_shared_read_empty_and_unparsed_preserve_distinct_fallback_policy() {
    for (mode, expected, requests) in [
        (Mode::Empty, 0, 3),
        (Mode::Unparsed, 23, 4),
        (Mode::ScanError, 23, 4),
    ] {
        let runtime = runtime(mode);
        assert_eq!(
            runtime
                .quote_balance(&urls()[0], addr(1), addr(9))
                .await
                .unwrap()
                .balance_units,
            expected
        );
        assert_eq!(methods(&runtime).len(), requests);
        if requests == 4 {
            assert_eq!(
                runtime.transport.calls.lock().unwrap()[2].1["params"][0],
                atas(addr(1), addr(9))[0].to_string()
            );
        }
    }
}

#[tokio::test]
async fn quote_balance_shared_read_ata_order_zero_and_error_semantics_match_legacy() {
    for (mode, expected) in [
        (Mode::FirstAtaError, Some(23)),
        (Mode::BothAtaZero, Some(0)),
        (Mode::LastAtaError, None),
    ] {
        let runtime = runtime(mode);
        let result = runtime.quote_balance(&urls()[0], addr(1), addr(9)).await;
        if let Some(units) = expected {
            assert_eq!(result.unwrap().balance_units, units);
            assert_eq!(methods(&runtime).len(), 5);
        } else {
            assert!(result.is_err());
            assert_eq!(methods(&runtime).len(), 4);
        }
        let calls = runtime.transport.calls.lock().unwrap();
        assert_eq!(
            calls[2].1["params"][0],
            atas(addr(1), addr(9))[0].to_string()
        );
        assert_eq!(
            calls[3].1["params"][0],
            atas(addr(1), addr(9))[1].to_string()
        );
    }
}

#[tokio::test]
async fn quote_balance_shared_read_bad_identity_partial_or_oversized_scan_never_falls_back() {
    for mode in [Mode::InvalidMint, Mode::Mixed, Mode::Oversize] {
        let runtime = runtime(mode);
        assert!(runtime
            .quote_balance(&urls()[0], addr(1), addr(9))
            .await
            .is_err());
        assert_eq!(
            methods(&runtime),
            ["getTokenSupply", "getTokenAccountsByOwner"]
        );
    }
}

#[tokio::test]
async fn quote_balance_shared_read_never_widens_first_default_node_or_credentials() {
    let runtime = runtime(Mode::PrimaryFailed);
    assert!(runtime
        .quote_balance(&urls()[0], addr(1), addr(9))
        .await
        .is_err());
    assert_eq!(methods(&runtime), ["getTokenSupply"]);
    assert!(runtime
        .quote_balance(
            "https://primary.example.invalid/?key=other",
            addr(1),
            addr(9)
        )
        .await
        .is_err());
    assert_eq!(methods(&runtime).len(), 1);
    assert!(runtime
        .quote_balance(&urls()[1], addr(1), addr(9))
        .await
        .is_ok());
    assert!(runtime.transport.calls.lock().unwrap()[1..]
        .iter()
        .all(|(host, _)| host == "backup.example.invalid"));
}

#[tokio::test]
async fn quote_balance_shared_read_429_cools_every_profile_and_no_ata_request_is_sent() {
    let runtime = runtime(Mode::RateScan);
    assert!(runtime
        .quote_balance(&urls()[0], addr(1), addr(9))
        .await
        .is_err());
    let cooldowns = runtime.budget.lock().unwrap().cooldowns.clone();
    assert!(!runtime.readiness_ping(&[urls()[0].clone()]).await);
    assert!(runtime
        .native_balances(&[urls()[0].clone()], &[addr(1)])
        .await
        .is_err());
    assert!(runtime
        .signature_status(
            &Url::parse(&urls()[0]).unwrap(),
            9,
            &bs58::encode([1u8; 64]).into_string()
        )
        .await
        .is_err());
    assert_eq!(methods(&runtime).len(), 2);
    runtime.latest_blockhash().await.unwrap();
    assert_eq!(methods(&runtime).len(), 3);
    assert_eq!(runtime.budget.lock().unwrap().cooldowns, cooldowns);
}

#[tokio::test]
async fn quote_balance_shared_read_busy_cancel_and_other_four_reservations_are_independent() {
    let runtime = runtime(Mode::Hold);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.quote_balance(&urls()[0], addr(1), addr(9)).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.transport.calls.lock().unwrap().len() < 2 {
            runtime.transport.entered.notified().await;
        }
    })
    .await
    .unwrap();
    let next = runtime.budget.lock().unwrap().next_start[4];
    assert!(tokio::time::timeout(
        Duration::from_millis(200),
        runtime.quote_balance(&urls()[0], addr(1), addr(9))
    )
    .await
    .unwrap()
    .is_err());
    runtime.latest_blockhash().await.unwrap();
    assert!(runtime.readiness_ping(&urls()).await);
    runtime.native_balances(&urls(), &[addr(1)]).await.unwrap();
    runtime
        .signature_status(
            &Url::parse(&urls()[0]).unwrap(),
            99,
            &bs58::encode([1u8; 64]).into_string(),
        )
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.budget.lock().unwrap().next_start[4], next);
    assert_eq!(
        runtime
            .slots
            .each_ref()
            .map(|slot| slot.available_permits()),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!(methods(&runtime).len(), 6);
    assert_eq!(runtime.transport.dropped.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn quote_balance_shared_read_pacing_rejection_sends_nothing_and_does_not_refund() {
    let runtime = runtime(Mode::Success);
    let next = Instant::now() + Duration::from_secs(60);
    runtime.budget.lock().unwrap().next_start[4] = next;
    assert!(runtime
        .quote_balance(&urls()[0], addr(1), addr(9))
        .await
        .is_err());
    assert!(methods(&runtime).is_empty());
    assert_eq!(runtime.budget.lock().unwrap().next_start[4], next);
    assert_eq!(runtime.slots[4].available_permits(), 1);
    runtime.latest_blockhash().await.unwrap();
}

#[tokio::test]
async fn quote_balance_shared_read_three_slow_steps_can_succeed_within_total_budget() {
    let runtime = runtime(Mode::Slow);
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(9),
        runtime.quote_balance(&urls()[0], addr(1), addr(9)),
    )
    .await
    .unwrap();
    assert!(result.is_ok());
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(methods(&runtime).len(), 3);
}

#[tokio::test]
async fn quote_balance_shared_read_total_deadline_cancels_fifth_step_without_partial_snapshot() {
    let runtime = runtime(Mode::SlowFallback);
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(9),
        runtime.quote_balance(&urls()[0], addr(1), addr(9)),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(started.elapsed() >= Duration::from_millis(7500));
    assert_eq!(methods(&runtime).len(), 5);
    assert_eq!(runtime.transport.dropped.load(Ordering::SeqCst), 5);
    assert_eq!(runtime.slots[4].available_permits(), 1);
}

#[tokio::test]
async fn quote_balance_legacy_oracle_exposes_unchecked_identity_without_changing_trade_helper() {
    use solana_client::rpc_request::RpcRequest;
    use std::collections::HashMap;
    let mut bad = json!([token_account(3, 23)]);
    bad[0]["account"]["data"]["parsed"]["info"]["mint"] = json!(addr(99).to_string());
    let rpc = solana_client::nonblocking::rpc_client::RpcClient::new_mock_with_mocks(
        "fails".into(),
        HashMap::from([(
            RpcRequest::GetTokenAccountsByOwner,
            json!({"context":{"slot":123},"value":bad.clone()}),
        )]),
    );
    let old = crate::adapters::rpc::wallet_reads::token_balance(
        &rpc,
        &solana_sdk::pubkey::Pubkey::new_from_array(addr(1).to_bytes()),
        &solana_sdk::pubkey::Pubkey::new_from_array(addr(9).to_bytes()),
    )
    .await
    .unwrap();
    assert_eq!(old, 23);
    assert!(scan_balance(bad, addr(1), addr(9), 6).is_err());
}
