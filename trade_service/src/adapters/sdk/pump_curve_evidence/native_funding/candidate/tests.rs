use super::*;
use crate::{
    adapters::{
        rpc::read_http::{HttpReply, ReadRequest},
        sdk::prepared_candidate::{validate_candidate, validate_signed_candidate},
    },
    application::{
        ports::ordinary_claims::OrdinaryExecutionClaimer,
        sdk_preparation::{validate_candidate_intent, validate_origin},
    },
    domain::{
        preparation_balance::MAX_BALANCE_AGE, pump_native_candidate::PumpNativeCandidateRecord,
        sdk_prepared_record::SdkPreparedRecord, submission::SignedSubmission,
    },
};
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use sol_trade_sdk::swqos::TradeType;
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;

const PRIMARY: &str = "https://producer.invalid/rpc?api-key=fixture-secret";
const BACKUP: &str = "https://backup.invalid/rpc";

struct Tape {
    results: [Value; 4],
    expected_requests: [Value; 4],
    requests: Mutex<Vec<Value>>,
    fail_at: Option<usize>,
    hold_at: Option<usize>,
    reached: Notify,
    release: Notify,
}

impl ReadHttpTransport for Tape {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        assert_eq!(url.as_str(), PRIMARY);
        let body = request.test_body();
        let phase = {
            let mut requests = self.requests.lock().unwrap();
            let phase = requests.len();
            assert!(phase < 4);
            assert_eq!(body["id"], json!(phase + 1));
            match phase {
                0 | 1 => {
                    assert!(matches!(request, ReadRequest::Parameters { .. }));
                    assert_eq!(body["method"], "getMultipleAccounts");
                    let count = if phase == 0 { 4 } else { 8 };
                    assert_eq!(body["params"][0].as_array().unwrap().len(), count);
                    assert_eq!(body["params"][1]["minContextSlot"], json!(173 + phase));
                }
                2 => {
                    assert!(matches!(request, ReadRequest::PumpNativeProbe { .. }));
                }
                _ => {
                    assert!(matches!(request, ReadRequest::PumpNativeFinal { .. }));
                }
            }
            assert_eq!(body, self.expected_requests[phase]);
            requests.push(body.clone());
            phase
        };
        if self.hold_at == Some(phase) {
            self.reached.notify_one();
            self.release.notified().await;
        }
        if self.fail_at == Some(phase) {
            return Ok(HttpReply::for_test(StatusCode::SERVICE_UNAVAILABLE, vec![]));
        }
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":body["id"],"result":self.results[phase]}),
            )
            .unwrap(),
        ))
    }
}

async fn tape(side: TradeType, provider: SwqosType) -> (Tape, NativeEvidenceContext, Vec<u8>) {
    let (results, expected_requests, frozen, buyback, tip_destination) =
        NativeBoundIntent::producer_fixture(side, provider).await;
    (
        Tape {
            results,
            expected_requests,
            requests: Mutex::default(),
            fail_at: None,
            hold_at: None,
            reached: Notify::new(),
            release: Notify::new(),
        },
        NativeEvidenceContext {
            minimum_slot: 173,
            blockhash: Hash::new_from_array([10; 32]),
            provider,
            buyback,
            tip_destination,
            deadline: Instant::now() + Duration::from_secs(3),
        },
        frozen,
    )
}

fn runtime(tape: Tape) -> SharedReadRuntime<Tape> {
    SharedReadRuntime::with_test_transport(&[PRIMARY.into(), BACKUP.into()], tape)
}

fn released(runtime: &SharedReadRuntime<Tape>) {
    assert!(runtime.parameter_slot().is_ok());
    assert!(runtime.aggregator_simulation_slot(PRIMARY).is_ok());
}

#[tokio::test]
async fn pump_bank_producer_reads_original_accounts_and_simulations_and_retains_only_unsigned_evidence(
) {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (f, _, _) = PumpNativeCandidateRecord::history_fixture(side, provider).await;
            let (tape, context, frozen) = tape(side, provider).await;
            let runtime = runtime(tape);
            let proof = prepare_native_evidence(&f.execution, &runtime, PRIMARY, context)
                .await
                .unwrap();
            assert_eq!(bincode::serialize(proof.packet().unwrap()).unwrap(), frozen);
            assert_eq!(proof.packet().unwrap().signatures, [Signature::default()]);
            let record = proof.record().unwrap();
            let message = proof.packet().unwrap().message.serialize();
            proof.check_record(&record, &message).unwrap();
            assert_eq!(
                record.details().request_id,
                f.execution.reservation().request_id()
            );
            let metadata = SdkPreparedRecord::PumpNative(record);
            validate_candidate(&metadata, &message).unwrap();
            validate_candidate_intent(&f.execution, &metadata, &message).unwrap();
            assert!(validate_origin(&f.execution, &metadata, &message).is_err());
            assert!(f.claims().claim(&f.execution).await.is_err());
            assert!(validate_signed_candidate(
                &metadata,
                &message,
                &SignedSubmission {
                    signature: String::new(),
                    bytes: vec![],
                }
            )
            .unwrap_err()
            .to_string()
            .contains("historical Pump candidate cannot enter signing"));
            let requests = runtime.test_transport().requests.lock().unwrap();
            assert_eq!(requests.len(), 4);
            assert_eq!(
                requests[0]["params"][0],
                json!(&requests[1]["params"][0].as_array().unwrap()[..4])
            );
            drop(requests);
            released(&runtime);
        }
    }
}

#[tokio::test]
async fn pump_bank_producer_rejects_changed_scope_context_and_provider_before_any_read() {
    let (f, _, _) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    for field in [
        "wallet",
        "key",
        "pool",
        "pool_id",
        "quote",
        "mode",
        "method",
        "aggregate",
        "hop",
        "hop_pool",
        "reservation",
        "zero",
        "close",
        "slot",
        "hash",
        "deadline",
        "provider",
    ] {
        let (tape, mut context, _) = tape(TradeType::Buy, SwqosType::Default).await;
        let runtime = runtime(tape);
        let mut command = f.execution.command().clone();
        match field {
            "wallet" => command.wallet_id.push('x'),
            "key" => command.idempotency_key = command.idempotency_key.trim().into(),
            "pool" => command.route.pool = Pool::PumpAmm,
            "pool_id" => command.route.pool_id = Some(Address::from_bytes([91; 32])),
            "quote" => command.route.quote_mint = Address::from_bytes([91; 32]),
            "mode" => command.execution.mode = ExecutionMode::Simulate,
            "method" => command.execution.method = ExecutionMethod::Pumpapi,
            "aggregate" => command.aggregator_sol = true,
            "hop" => command.route.sol_hop_pool_id = Some(Address::from_bytes([91; 32])),
            "hop_pool" => command.route.sol_hop_pool = Some(Pool::RaydiumCpmm),
            "reservation" => {}
            "zero" => command.amount = TradeAmount::BuySol { lamports: 0 },
            "close" => command.amount = TradeAmount::SellPercent { percent: 100 },
            "slot" => context.minimum_slot = 0,
            "hash" => context.blockhash = Hash::default(),
            "deadline" => context.deadline = Instant::now(),
            "provider" => context.provider = SwqosType::Temporal,
            _ => unreachable!(),
        }
        let changed = if field == "reservation" {
            AdmittedTrade::unreserved_fixture(&f.record, command)
        } else {
            f.execution.with_command_for_test(command)
        };
        assert!(
            prepare_native_evidence(&changed, &runtime, PRIMARY, context)
                .await
                .is_err(),
            "{field}"
        );
        assert!(
            runtime.test_transport().requests.lock().unwrap().is_empty(),
            "{field}"
        );
        released(&runtime);
    }
}

#[tokio::test]
async fn pump_bank_producer_stops_at_failed_bank_or_simulation_without_backup_or_owner() {
    let (f, _, _) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    for phase in 0..4 {
        let (mut tape, context, _) = tape(TradeType::Buy, SwqosType::Default).await;
        tape.fail_at = Some(phase);
        let runtime = runtime(tape);
        let error = prepare_native_evidence(&f.execution, &runtime, PRIMARY, context)
            .await
            .err()
            .expect("failed phase accepted");
        assert!(!format!("{error:#}").contains("fixture-secret"));
        assert_eq!(
            runtime.test_transport().requests.lock().unwrap().len(),
            phase + 1
        );
        released(&runtime);
        assert!(f.claims().claim(&f.execution).await.is_err());
    }
    // A missing existing ATA cannot silently become account setup in this profile.
    let (mut tape, context, _) = tape(TradeType::Buy, SwqosType::Default).await;
    tape.results[1]["value"][5] = Value::Null;
    let runtime = runtime(tape);
    assert!(
        prepare_native_evidence(&f.execution, &runtime, PRIMARY, context)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("existing user ATA")
    );
    assert_eq!(runtime.test_transport().requests.lock().unwrap().len(), 2);
    released(&runtime);
}

#[tokio::test]
async fn pump_bank_producer_cancellation_and_whole_deadline_release_all_original_read_permits() {
    for phase in 0..4 {
        let (f, _, _) =
            PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Temporal).await;
        let (mut tape, context, _) = tape(TradeType::Buy, SwqosType::Temporal).await;
        tape.hold_at = Some(phase);
        let runtime = Arc::new(runtime(tape));
        let active = runtime.clone();
        let task = tokio::spawn(async move {
            prepare_native_evidence(&f.execution, &active, PRIMARY, context).await?;
            Ok::<_, anyhow::Error>(())
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            runtime.test_transport().reached.notified(),
        )
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        released(&runtime);
        let before = runtime.test_transport().requests.lock().unwrap().clone();
        assert_eq!(before.len(), phase + 1);
        runtime.test_transport().release.notify_one();
        tokio::task::yield_now().await;
        assert_eq!(*runtime.test_transport().requests.lock().unwrap(), before);
    }
    let (f, _, _) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let (mut tape, mut context, _) = tape(TradeType::Buy, SwqosType::Default).await;
    tape.hold_at = Some(3);
    context.deadline = Instant::now() + Duration::from_secs(1);
    let runtime = runtime(tape);
    let error = prepare_native_evidence(&f.execution, &runtime, PRIMARY, context)
        .await
        .err()
        .expect("whole deadline ignored");
    assert!(error
        .to_string()
        .contains("complete evidence deadline expired"));
    assert_eq!(runtime.test_transport().requests.lock().unwrap().len(), 4);
    released(&runtime);
}

#[tokio::test]
async fn pump_bank_producer_frozen_evidence_expires_and_coherent_history_cannot_replace_its_bank() {
    let (f, _, _) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let (tape, context, _) = tape(TradeType::Buy, SwqosType::Default).await;
    let runtime = runtime(tape);
    let proof = prepare_native_evidence(&f.execution, &runtime, PRIMARY, context)
        .await
        .unwrap();
    let original = proof.record().unwrap();
    let mut details = original.details().clone();
    details.protocol_fee = [81; 32];
    let unsigned = PumpNativeCandidateRecord::new(
        details.clone(),
        &proof.packet().unwrap().message.serialize(),
    )
    .unwrap();
    let changed: VersionedTransaction =
        crate::adapters::sdk::pump_native_message::reconstruct(&unsigned).unwrap();
    let message = changed.message.serialize();
    let nominal = PumpNativeCandidateRecord::new(details, &message).unwrap();
    validate_candidate(&SdkPreparedRecord::PumpNative(nominal), &message).unwrap();
    let foreign = PumpNativeCandidateRecord::new(unsigned.details().clone(), &message).unwrap();
    assert!(proof.check_record(&foreign, &message).is_err());
    assert_eq!(proof.record().unwrap(), original);
    tokio::time::sleep(MAX_BALANCE_AGE + Duration::from_millis(20)).await;
    assert!(proof.packet().is_err());
    assert!(proof.record().is_err());
    assert!(proof.check_record(&original, &message).is_err());
    released(&runtime);
}
