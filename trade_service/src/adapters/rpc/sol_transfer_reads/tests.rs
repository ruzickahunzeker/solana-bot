use super::*;
use crate::adapters::rpc::read_http::{HttpReply, ReadRequest};
use reqwest::{StatusCode, Url};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::sync::Notify;

const URL: &str = "http://transfer.invalid/fixture";
const BACKUP: &str = "http://transfer-backup.invalid/fixture";

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

#[derive(Clone, Default)]
enum Mode {
    #[default]
    Good,
    Fault(u8),
    Value(&'static str, Value),
    Rate,
    Pending,
    Delay(Duration),
    Block(Duration),
}

#[derive(Default)]
struct Trace {
    calls: Mutex<Vec<(String, Value, Instant)>>,
    entered: Notify,
    dropped: AtomicUsize,
}

struct DropCall(Arc<Trace>);
impl Drop for DropCall {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Default)]
struct Fake {
    mode: Mode,
    trace: Arc<Trace>,
}

impl Fake {
    fn runtime(&self) -> SharedReadRuntime<Self> {
        SharedReadRuntime::with_test_transport(&[URL.into(), BACKUP.into()], self.clone())
    }

    fn calls(&self) -> Vec<(String, Value, Instant)> {
        self.trace.calls.lock().unwrap().clone()
    }
}

impl ReadHttpTransport for Fake {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        assert!(matches!(
            request,
            ReadRequest::SolTransferPreparation { .. } | ReadRequest::LatestBlockhash { .. }
        ));
        let body = request.body();
        self.trace
            .calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone(), Instant::now()));
        let _call = DropCall(self.trace.clone());
        self.trace.entered.notify_one();
        let is_transfer = matches!(request, ReadRequest::SolTransferPreparation { .. });
        let method = body["method"].as_str().unwrap();
        let mode = if is_transfer {
            self.mode.clone()
        } else {
            Mode::Good
        };
        match &mode {
            Mode::Pending => return std::future::pending().await,
            Mode::Delay(delay) => tokio::time::sleep(*delay).await,
            Mode::Block(delay) => std::thread::sleep(*delay),
            Mode::Rate => {
                return Ok(HttpReply {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    retry_after: Some(60),
                    body: vec![],
                });
            }
            Mode::Fault(0) => anyhow::bail!("http://provider.invalid/private-fixture-error"),
            Mode::Fault(1) => {
                return Ok(HttpReply::for_test(StatusCode::FOUND, vec![]));
            }
            _ => {}
        }
        let result = match &mode {
            Mode::Value(selected, value) if *selected == method => value.clone(),
            _ => match method {
                "getBalance" => json!({"context":{"slot":123},"value":1_000_000}),
                "getLatestBlockhash" => {
                    json!({"context":{"slot":123},"value":{"blockhash":Hash::new_from_array([6;32]).to_string(),"lastValidBlockHeight":999}})
                }
                "getFeeForMessage" => json!({"context":{"slot":123},"value":5000}),
                "getMinimumBalanceForRentExemption" => json!(890_880),
                _ => panic!("non-read SOL transfer fixture request"),
            },
        };
        let mut envelope = json!({"jsonrpc":"2.0","id":body["id"],"result":result});
        match &mode {
            Mode::Fault(2) => envelope["id"] = json!("wrong"),
            Mode::Fault(3) => envelope["jsonrpc"] = json!("1.0"),
            Mode::Fault(4) => envelope["error"] = json!({"code":-1,"message":"public fixture"}),
            Mode::Fault(5) => {
                envelope.as_object_mut().unwrap().remove("result");
            }
            Mode::Fault(6) => {
                return Ok(HttpReply::for_test(StatusCode::OK, b"bad JSON".to_vec()));
            }
            Mode::Fault(7) => {
                return Ok(HttpReply::for_test(
                    StatusCode::OK,
                    vec![b' '; request.max_body() + 1],
                ));
            }
            _ => {}
        }
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&envelope).unwrap(),
        ))
    }
}

fn scope(runtime: &SharedReadRuntime<Fake>) -> SolTransferReadSession<'_, Fake> {
    SolTransferReadSession::new(runtime, URL, CommitmentConfig::processed(), key(1), key(2))
        .unwrap()
}

#[tokio::test]
async fn sol_transfer_read_original_four_requests_order_and_fee_message_are_exact() {
    for commitment in [
        CommitmentConfig::processed(),
        CommitmentConfig::confirmed(),
        CommitmentConfig::finalized(),
    ] {
        let fake = Fake::default();
        let runtime = fake.runtime();
        let session =
            SolTransferReadSession::new(&runtime, URL, commitment, key(1), key(2)).unwrap();
        let snapshot = session.snapshot().await.unwrap();
        assert_eq!(
            snapshot,
            (1_000_000, Hash::new_from_array([6; 32]), 5000, 890_880)
        );
        let message = Message::new_with_blockhash(
            &[solana_system_interface::instruction::transfer(
                &key(1),
                &key(2),
                1,
            )],
            Some(&key(1)),
            &snapshot.1,
        );
        let expected = [
            ("getBalance", json!([key(1).to_string(), commitment])),
            ("getLatestBlockhash", json!([commitment])),
            (
                "getFeeForMessage",
                json!([
                    base64::engine::general_purpose::STANDARD.encode(message.serialize()),
                    commitment
                ]),
            ),
            ("getMinimumBalanceForRentExemption", json!([0])),
        ];
        let calls = fake.calls();
        assert_eq!(calls.len(), 4);
        for (index, (method, params)) in expected.into_iter().enumerate() {
            assert_eq!(calls[index].0, URL);
            assert_eq!(
                calls[index].1,
                json!({"jsonrpc":"2.0","id":index+1,"method":method,"params":params})
            );
        }
    }
}

#[tokio::test]
async fn sol_transfer_read_order_repeats_and_consumed_rent_cannot_issue_extra_requests() {
    let fake = Fake::default();
    let runtime = fake.runtime();
    let mut reads = scope(&runtime);
    assert!(reads.blockhash().await.is_err());
    assert!(reads.fee().await.is_err());
    assert!(reads.rent().await.is_err());
    assert!(fake.calls().is_empty());
    reads.balance().await.unwrap();
    assert!(reads.balance().await.is_err());
    reads.blockhash().await.unwrap();
    assert!(reads.blockhash().await.is_err());
    reads.fee().await.unwrap();
    assert!(reads.fee().await.is_err());
    reads.rent().await.unwrap();
    assert!(reads.rent().await.is_err());
    assert_eq!(fake.calls().len(), 4);
}

#[tokio::test]
async fn sol_transfer_read_failed_stage_cannot_retry_or_advance() {
    let fake = Fake {
        mode: Mode::Fault(0),
        ..Default::default()
    };
    let runtime = fake.runtime();
    let mut reads = scope(&runtime);
    assert!(reads.balance().await.is_err());
    assert!(reads.balance().await.is_err());
    assert!(reads.blockhash().await.is_err());
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn sol_transfer_read_self_unregistered_and_existing_wsol_quota_reject_before_io() {
    let fake = Fake::default();
    let runtime = fake.runtime();
    assert!(SolTransferReadSession::new(
        &runtime,
        URL,
        CommitmentConfig::processed(),
        key(1),
        key(1)
    )
    .is_err());
    assert!(SolTransferReadSession::new(
        &runtime,
        "http://unregistered.invalid/",
        CommitmentConfig::processed(),
        key(1),
        key(2)
    )
    .is_err());
    let wsol = runtime.wsol_quote_slot(URL).unwrap();
    assert!(SolTransferReadSession::new(
        &runtime,
        URL,
        CommitmentConfig::processed(),
        key(1),
        key(2)
    )
    .is_err());
    drop(wsol);
    let transfer = scope(&runtime);
    assert!(runtime.wsol_quote_slot(BACKUP).is_err());
    assert!(fake.calls().is_empty());
    drop(transfer);
    assert!(runtime.wsol_quote_slot(BACKUP).is_ok());
}

#[tokio::test]
async fn sol_transfer_read_bad_identity_rpc_fault_body_and_transport_are_bounded_redacted() {
    for mode in 0..8 {
        let fake = Fake {
            mode: Mode::Fault(mode),
            ..Default::default()
        };
        let runtime = fake.runtime();
        let error = scope(&runtime).snapshot().await.unwrap_err().to_string();
        assert!(!error.contains("provider.invalid"), "mode {mode}");
        assert_eq!(fake.calls().len(), 1, "mode {mode}");
        assert!(runtime.wsol_quote_slot(URL).is_ok());
    }
}

#[tokio::test]
async fn sol_transfer_read_invalid_scalar_context_hash_and_missing_fee_fail_closed() {
    for (method, value, count) in [
        ("getBalance", json!({"value":1}), 1),
        ("getBalance", json!({"context":{"slot":123},"value":-1}), 1),
        ("getBalance", json!({"context":{"slot":123},"value":1.5}), 1),
        (
            "getLatestBlockhash",
            json!({"context":{"slot":123},"value":{"blockhash":"invalid","lastValidBlockHeight":999}}),
            2,
        ),
        (
            "getLatestBlockhash",
            json!({"context":{"slot":123},"value":{"blockhash":Hash::default().to_string()}}),
            2,
        ),
        (
            "getFeeForMessage",
            json!({"context":{"slot":123},"value":null}),
            3,
        ),
        (
            "getFeeForMessage",
            json!({"context":{"slot":123},"value":"5000"}),
            3,
        ),
        ("getMinimumBalanceForRentExemption", json!(-1), 4),
        ("getMinimumBalanceForRentExemption", json!("890880"), 4),
    ] {
        let fake = Fake {
            mode: Mode::Value(method, value),
            ..Default::default()
        };
        let runtime = fake.runtime();
        assert!(scope(&runtime).snapshot().await.is_err(), "{method}");
        assert_eq!(fake.calls().len(), count, "{method}");
    }
}

#[tokio::test]
async fn sol_transfer_read_rate_cools_existing_wsol_profile_without_retry_or_backup() {
    let fake = Fake {
        mode: Mode::Rate,
        ..Default::default()
    };
    let runtime = fake.runtime();
    assert!(scope(&runtime).snapshot().await.is_err());
    assert!(runtime.wsol_quote_slot(URL).is_err());
    assert!(SolTransferReadSession::new(
        &runtime,
        URL,
        CommitmentConfig::processed(),
        key(1),
        key(2)
    )
    .is_err());
    assert_eq!(fake.calls().len(), 1);
    assert!(runtime.latest_blockhash().await.is_ok());
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, BACKUP);
}

#[tokio::test]
async fn sol_transfer_read_cancel_releases_shared_quota_and_hash_has_separate_capacity() {
    let fake = Fake {
        mode: Mode::Pending,
        ..Default::default()
    };
    let runtime = fake.runtime();
    let mut work = Box::pin(scope(&runtime).snapshot());
    tokio::select! {
        result = &mut work => panic!("pending fixture completed: {result:?}"),
        _ = fake.trace.entered.notified() => {},
    }
    assert!(runtime.wsol_quote_slot(URL).is_err());
    assert!(runtime.latest_blockhash().await.is_ok());
    drop(work);
    assert_eq!(fake.trace.dropped.load(Ordering::SeqCst), 2);
    assert!(runtime.wsol_quote_slot(URL).is_ok());
    assert_eq!(fake.calls().len(), 2);
}

#[tokio::test]
async fn sol_transfer_read_pacing_and_ids_share_existing_wsol_start_budget() {
    let fake = Fake::default();
    let runtime = fake.runtime();
    let started = Instant::now();
    scope(&runtime).snapshot().await.unwrap();
    scope(&runtime).snapshot().await.unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 8);
    for (i, call) in calls.iter().enumerate() {
        assert_eq!(call.1["id"], i + 1);
        assert!(call.2.duration_since(started) >= Duration::from_millis((i * 100) as u64));
    }
}

#[tokio::test]
async fn sol_transfer_read_original_five_second_total_is_not_renewed_per_step() {
    let fake = Fake {
        mode: Mode::Delay(Duration::from_millis(2600)),
        ..Default::default()
    };
    let runtime = fake.runtime();
    let started = Instant::now();
    let error = scope(&runtime).snapshot().await.unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(started.elapsed() < Duration::from_secs(7));
    assert_eq!(fake.calls().len(), 2);
    assert!(runtime.wsol_quote_slot(URL).is_ok());
}

#[tokio::test]
async fn sol_transfer_read_late_ready_reply_cannot_escape_post_io_deadline() {
    let fake = Fake {
        mode: Mode::Block(Duration::from_millis(20)),
        ..Default::default()
    };
    let runtime = fake.runtime();
    let mut reads = scope(&runtime);
    reads.deadline = Instant::now() + Duration::from_millis(10);
    assert!(reads.balance().await.is_err());
    assert_eq!(fake.calls().len(), 1);
    assert!(reads.balance().await.is_err());
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn sol_transfer_read_zero_fee_rent_and_zero_context_match_original_scalar_acceptance() {
    for (method, value, expected) in [
        (
            "getBalance",
            json!({"context":{"slot":0},"value":0}),
            (0, 5000, 890_880),
        ),
        (
            "getFeeForMessage",
            json!({"context":{"slot":0},"value":0}),
            (1_000_000, 0, 890_880),
        ),
        (
            "getMinimumBalanceForRentExemption",
            json!(0),
            (1_000_000, 5000, 0),
        ),
    ] {
        let fake = Fake {
            mode: Mode::Value(method, value),
            ..Default::default()
        };
        let runtime = fake.runtime();
        let (balance, _, fee, rent) = scope(&runtime).snapshot().await.unwrap();
        assert_eq!((balance, fee, rent), expected);
        assert_eq!(fake.calls().len(), 4);
    }
}
