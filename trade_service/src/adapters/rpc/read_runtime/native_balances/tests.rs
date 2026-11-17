use super::*;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum Mode {
    Success,
    RatePrimary,
    PartialPrimary,
    HoldNative,
    LateNative,
    LateRatePrimary,
}
struct Fake {
    mode: Mode,
    calls: Mutex<Vec<(String, Value)>>,
    primary_calls: AtomicUsize,
    entered: Notify,
    dropped: Arc<AtomicUsize>,
}
struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn account(lamports: u64) -> Value {
    json!({"lamports":lamports,"owner":"11111111111111111111111111111111","data":["","base64"],"executable":false,"rentEpoch":u64::MAX,"space":0})
}
fn envelope(id: u64, values: Vec<Value>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":values}})
}
fn amount(address: &Address) -> u64 {
    let byte = address.to_bytes()[0];
    if byte == 1 {
        u64::MAX
    } else {
        u64::from(byte)
    }
}
impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let host = url.host_str().unwrap().to_string();
        self.calls
            .lock()
            .unwrap()
            .push((host.clone(), request.body()));
        let _drop = Dropped(self.dropped.clone());
        self.entered.notify_one();
        let primary = host == "primary.example.invalid";
        if matches!(self.mode, Mode::LateRatePrimary) && primary {
            std::thread::sleep(Duration::from_millis(350));
        }
        if matches!(self.mode, Mode::RatePrimary | Mode::LateRatePrimary) && primary {
            return Ok(HttpReply {
                status: StatusCode::TOO_MANY_REQUESTS,
                retry_after: Some(60),
                body: vec![],
            });
        }
        let body = match request {
            ReadRequest::NativeBalances { id, wallets } => {
                if matches!(self.mode, Mode::LateNative) {
                    std::thread::sleep(Duration::from_millis(350));
                }
                if matches!(self.mode, Mode::HoldNative) {
                    std::future::pending::<()>().await;
                }
                let nth = if primary {
                    self.primary_calls.fetch_add(1, Ordering::SeqCst)
                } else {
                    0
                };
                if matches!(self.mode, Mode::PartialPrimary) && primary && nth == 1 {
                    return Ok(HttpReply {
                        status: StatusCode::SERVICE_UNAVAILABLE,
                        retry_after: None,
                        body: vec![],
                    });
                }
                envelope(
                    *id,
                    wallets
                        .iter()
                        .map(|wallet| {
                            if wallet.to_bytes()[0] == 0 {
                                Value::Null
                            } else {
                                account(if primary && matches!(self.mode, Mode::PartialPrimary) {
                                    999
                                } else {
                                    amount(wallet)
                                })
                            }
                        })
                        .collect(),
                )
            }
            ReadRequest::LatestBlockhash { id } => {
                json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":{"blockhash":bs58::encode([7u8;32]).into_string(),"lastValidBlockHeight":456}}})
            }
            ReadRequest::SignatureStatus { id, .. } => envelope(*id, vec![Value::Null]),
            _ => panic!("quote tests use their own typed fixtures"),
        };
        Ok(HttpReply {
            status: StatusCode::OK,
            retry_after: None,
            body: serde_json::to_vec(&body).unwrap(),
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
    runtime_with_urls(&urls(), mode)
}
fn runtime_with_urls(urls: &[String], mode: Mode) -> Arc<SharedReadRuntime<Fake>> {
    Arc::new(SharedReadRuntime::with_transport(
        urls,
        Fake {
            mode,
            calls: Mutex::new(vec![]),
            primary_calls: AtomicUsize::new(0),
            entered: Notify::new(),
            dropped: Arc::new(AtomicUsize::new(0)),
        },
    ))
}
fn addresses(count: usize) -> Vec<Address> {
    (0..count)
        .map(|index| Address::from_bytes([(index % 255) as u8; 32]))
        .collect()
}
fn parse(value: Value, count: usize) -> Result<Vec<u64>> {
    parse_balances(1, count, &serde_json::to_vec(&value).unwrap())
}

#[test]
fn native_balance_codec_preserves_u64_order_duplicates_and_only_null_is_missing() {
    assert_eq!(
        parse(
            envelope(
                1,
                vec![
                    account(u64::MAX),
                    Value::Null,
                    account(9),
                    account(u64::MAX)
                ]
            ),
            4
        )
        .unwrap(),
        vec![u64::MAX, 0, 9, u64::MAX]
    );
    for values in [vec![], vec![account(1), account(2)]] {
        assert!(parse(envelope(1, values), 1).is_err());
    }
    assert!(parse(envelope(1, vec![json!({})]), 1).is_err());
}

#[test]
fn native_balance_codec_rejects_bad_identity_types_owner_and_ignored_data_slice() {
    let good = envelope(1, vec![account(1)]);
    for (pointer, bad) in [
        ("/id", json!(2)),
        ("/jsonrpc", json!("1.0")),
        ("/result/context/slot", json!(-1)),
        ("/result/value/0/lamports", json!("1")),
        ("/result/value/0/lamports", json!(-1)),
        ("/result/value/0/lamports", json!(1.5)),
        ("/result/value/0/owner", json!("0")),
        ("/result/value/0/owner", json!("1".repeat(60_000))),
        ("/result/value/0/executable", json!(1)),
        ("/result/value/0/rentEpoch", json!(null)),
        ("/result/value/0/data/0", json!("AA==")),
        ("/result/value/0/data/1", json!("base64+zstd")),
    ] {
        let mut value = good.clone();
        *value.pointer_mut(pointer).unwrap() = bad;
        let error = parse(value, 1).unwrap_err().to_string();
        assert!(!error.contains("fixture-secret"));
    }
    let mut both = good;
    both["error"] = json!({"message":"fixture-secret"});
    assert!(!parse(both, 1)
        .unwrap_err()
        .to_string()
        .contains("fixture-secret"));
    assert!(parse(json!({"jsonrpc":"2.0","id":1,"result":null}), 1).is_err());
}

#[test]
fn native_balance_codec_has_exact_body_boundary_and_rejects_unsigned_overflow() {
    let mut bytes = serde_json::to_vec(&envelope(1, vec![account(1)])).unwrap();
    bytes.resize(super::super::super::read_http::MAX_BODY, b' ');
    assert_eq!(parse_balances(1, 1, &bytes).unwrap(), vec![1]);
    bytes.push(b' ');
    assert!(parse_balances(1, 1, &bytes).is_err());
    let raw = serde_json::to_string(&envelope(1, vec![account(u64::MAX)]))
        .unwrap()
        .replacen(&u64::MAX.to_string(), "18446744073709551616", 1);
    assert!(parse_balances(1, 1, raw.as_bytes()).is_err());
}

#[tokio::test]
async fn native_balance_shared_read_splits_exactly_100_and_preserves_order_and_duplicates() {
    let runtime = runtime(Mode::Success);
    let wallets = addresses(201);
    let expected = wallets.iter().map(amount).collect::<Vec<_>>();
    assert_eq!(
        runtime.native_balances(&urls(), &wallets).await.unwrap(),
        expected
    );
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls
            .iter()
            .map(|(_, request)| request["params"][0].as_array().unwrap().len())
            .collect::<Vec<_>>(),
        vec![100, 100, 1]
    );
    for (index, (_, request)) in calls.iter().enumerate() {
        assert_eq!(request["method"], "getMultipleAccounts");
        assert_eq!(
            request["params"][1],
            json!({"commitment":"processed","encoding":"base64","dataSlice":{"offset":0,"length":0}})
        );
        assert_eq!(
            request["params"][0],
            json!(wallets
                .chunks(100)
                .nth(index)
                .unwrap()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>())
        );
    }
}

#[tokio::test]
async fn native_balance_shared_read_partial_failure_restarts_whole_batch_on_allowed_backup() {
    let runtime = runtime(Mode::PartialPrimary);
    let wallets = addresses(201);
    assert_eq!(
        runtime.native_balances(&urls(), &wallets).await.unwrap(),
        wallets.iter().map(amount).collect::<Vec<_>>()
    );
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert!(calls[..2]
        .iter()
        .all(|(host, _)| host == "primary.example.invalid"));
    assert!(calls[2..]
        .iter()
        .all(|(host, _)| host == "backup.example.invalid"));
    assert_eq!(calls[0].1["params"][0], calls[2].1["params"][0]);
}

#[tokio::test]
async fn native_balance_shared_read_empty_and_oversized_queries_send_nothing() {
    let runtime = runtime(Mode::Success);
    assert!(runtime.native_balances(&[], &[]).await.unwrap().is_empty());
    assert!(runtime
        .native_balances(&urls(), &addresses(1001))
        .await
        .is_err());
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
    assert_eq!(runtime.slots[3].available_permits(), 1);
}

#[tokio::test]
async fn native_balance_shared_read_full_limit_fits_bounded_profile_and_keeps_duplicate_rows() {
    let runtime = runtime(Mode::Success);
    let wallets = addresses(1000);
    assert_eq!(
        runtime.native_balances(&urls(), &wallets).await.unwrap(),
        wallets.iter().map(amount).collect::<Vec<_>>()
    );
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 10);
}

#[tokio::test]
async fn native_balance_shared_read_candidate_order_and_unregistered_keys_are_not_widened() {
    let runtime = runtime(Mode::Success);
    assert!(runtime
        .native_balances(
            &["https://primary.example.invalid/?key=other".into()],
            &addresses(1)
        )
        .await
        .is_err());
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
    assert!(runtime
        .native_balances(&[urls()[1].clone(), urls()[0].clone()], &addresses(1))
        .await
        .is_ok());
    assert_eq!(
        runtime.transport.calls.lock().unwrap()[0].0,
        "backup.example.invalid"
    );
}

#[tokio::test]
async fn native_balance_shared_read_429_is_shared_without_retry_or_extension() {
    let runtime = runtime(Mode::RatePrimary);
    runtime
        .native_balances(&urls(), &addresses(1))
        .await
        .unwrap();
    let cooldowns = runtime.budget.lock().unwrap().cooldowns.clone();
    runtime.latest_blockhash().await.unwrap();
    assert!(!runtime.readiness_ping(&[urls()[0].clone()]).await);
    assert!(runtime
        .signature_status(
            &Url::parse(&urls()[0]).unwrap(),
            99,
            &bs58::encode([1u8; 64]).into_string()
        )
        .await
        .is_err());
    assert!(runtime
        .native_balances(&[urls()[0].clone()], &addresses(1))
        .await
        .is_err());
    assert_eq!(runtime.budget.lock().unwrap().cooldowns, cooldowns);
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn native_balance_shared_read_critical_cooldown_is_respected_without_default_scope_escape() {
    let runtime = runtime(Mode::RatePrimary);
    runtime.latest_blockhash().await.unwrap();
    let before = runtime.transport.calls.lock().unwrap().len();
    assert!(runtime
        .native_balances(&[urls()[0].clone()], &addresses(1))
        .await
        .is_err());
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), before);
}

#[tokio::test]
async fn native_balance_shared_read_busy_and_cancel_do_not_take_other_reserved_capacity() {
    let runtime = runtime(Mode::HoldNative);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.native_balances(&urls(), &addresses(1)).await })
    };
    tokio::time::timeout(Duration::from_secs(1), runtime.transport.entered.notified())
        .await
        .unwrap();
    let next_start = runtime.budget.lock().unwrap().next_start[3];
    assert!(tokio::time::timeout(
        Duration::from_millis(200),
        runtime.native_balances(&urls(), &addresses(1))
    )
    .await
    .unwrap()
    .is_err());
    runtime.latest_blockhash().await.unwrap();
    assert!(runtime.readiness_ping(&urls()).await);
    assert!(runtime
        .signature_status(
            &Url::parse(&urls()[0]).unwrap(),
            42,
            &bs58::encode([1u8; 64]).into_string()
        )
        .await
        .is_ok());
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 4);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.budget.lock().unwrap().next_start[3], next_start);
    assert_eq!(
        runtime
            .slots
            .each_ref()
            .map(|slot| slot.available_permits()),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!(runtime.transport.dropped.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn native_balance_shared_read_rejected_pacing_never_spends_or_refunds_budget() {
    let runtime = runtime(Mode::Success);
    let reserved = Instant::now() + Duration::from_secs(60);
    runtime.budget.lock().unwrap().next_start[3] = reserved;
    assert!(runtime
        .native_balances(&urls(), &addresses(1))
        .await
        .is_err());
    assert_eq!(runtime.budget.lock().unwrap().next_start[3], reserved);
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
    runtime.latest_blockhash().await.unwrap();
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn native_balance_shared_read_late_success_and_429_cannot_escape_response_deadline() {
    for (mode, cooled) in [(Mode::LateNative, false), (Mode::LateRatePrimary, true)] {
        let runtime = runtime(mode);
        let deadline = Instant::now() + Duration::from_millis(250);
        let result = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            runtime.read(
                Class::NativeBalances,
                0,
                ReadRequest::NativeBalances {
                    id: 1,
                    wallets: addresses(1),
                },
                deadline,
            ),
        )
        .await
        .unwrap();
        assert!(
            result.is_err(),
            "a ready response future must not bypass its expired deadline"
        );
        assert_eq!(runtime.transport.calls.lock().unwrap().len(), 1);
        assert_eq!(
            runtime.budget.lock().unwrap().cooldowns[0].is_some(),
            cooled
        );
    }
}

#[tokio::test]
async fn native_balance_shared_read_total_deadline_cancels_without_visiting_later_nodes() {
    let mut candidates = urls();
    candidates.push("https://unvisited.example.invalid".into());
    let runtime = runtime_with_urls(&candidates, Mode::HoldNative);
    let started = Instant::now();
    assert!(tokio::time::timeout(
        Duration::from_secs(9),
        runtime.native_balances(&candidates, &addresses(1))
    )
    .await
    .unwrap()
    .is_err());
    assert!(started.elapsed() >= Duration::from_millis(7500));
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 2);
    assert!(runtime
        .transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(host, _)| host != "unvisited.example.invalid"));
    assert_eq!(runtime.transport.dropped.load(Ordering::SeqCst), 2);
    assert_eq!(runtime.slots[3].available_permits(), 1);
}
