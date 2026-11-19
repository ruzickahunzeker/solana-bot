use super::super::tests::fixture_from_probe;
use super::*;
use crate::adapters::{
    rpc::{
        pump_native_final::PumpNativeFinalPacket,
        pump_native_probe::PumpNativeProbePacket,
        read_http::{HttpReply, ReadRequest},
    },
    sdk::pump_curve_evidence::native_funding::probe::{
        observation::{post_bank::tests::response_with_bank, tests::observed},
        tests::fixture as original_bank,
    },
};
use crate::domain::preparation_balance::MAX_BALANCE_AGE;
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use sol_trade_sdk::swqos::TradeType;
use solana_sdk::signature::Signature;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Notify;

const PRIMARY: &str = "https://primary.invalid/rpc?api-key=fixture-secret";
const BACKUP: &str = "https://backup.invalid/rpc";

struct Case {
    bank: NativeFundingSnapshot,
    input: NonZeroU64,
    buyback: Pubkey,
    tip: Option<Pubkey>,
    expected: [Value; 2],
    results: [Value; 2],
    frozen: Vec<u8>,
    started: Instant,
}

async fn case(side: TradeType, provider: SwqosType) -> Case {
    // The unchanged SDK fixtures are only an oracle; the path under test consumes public bank data.
    let (probe, response) = observed(side).await;
    let started = probe.funding.started;
    let buyback = probe.instruction.accounts.last().unwrap().pubkey;
    let quote = response_with_bank(&probe, response.clone());
    let expected_quote = ReadRequest::PumpNativeProbe {
        id: 1,
        packet: PumpNativeProbePacket::new(
            probe.packet().unwrap(),
            &probe.evidence_accounts(),
            175,
            started + MAX_BALANCE_AGE,
        )
        .unwrap(),
    }
    .test_body();
    let (template, final_reply) = fixture_from_probe(probe, response, side, provider, false);
    let expected_final = ReadRequest::PumpNativeFinal {
        id: 2,
        packet: PumpNativeFinalPacket::new(
            template.packet().unwrap(),
            &template.evidence_accounts(),
            176,
            started + MAX_BALANCE_AGE,
        )
        .unwrap(),
    }
    .test_body();
    let (mut bank, _, input) = original_bank(side).await;
    assert_eq!(
        bank.keys,
        template.source.source.observation.probe.funding.keys
    );
    bank.started = started;
    Case {
        bank,
        input,
        buyback,
        tip: template.destination,
        expected: [expected_quote, expected_final],
        results: [quote, final_reply],
        frozen: bincode::serialize(template.packet().unwrap()).unwrap(),
        started,
    }
}

struct Tape {
    expected: [Value; 2],
    results: [Value; 2],
    requests: Mutex<Vec<(String, Value)>>,
    fail_at: Option<usize>,
    hold_at: Option<usize>,
    reached: Notify,
    release: Notify,
}

impl Tape {
    fn new(expected: [Value; 2], results: [Value; 2]) -> Self {
        Self {
            expected,
            results,
            requests: Mutex::default(),
            fail_at: None,
            hold_at: None,
            reached: Notify::new(),
            release: Notify::new(),
        }
    }
}

impl ReadHttpTransport for Tape {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let phase = match request {
            ReadRequest::PumpNativeProbe { .. } => 0,
            ReadRequest::PumpNativeFinal { .. } => 1,
            _ => anyhow::bail!("readonly bank path attempted an unrelated operation"),
        };
        let body = request.test_body();
        {
            let mut requests = self.requests.lock().unwrap();
            let position = requests.len();
            assert_eq!(position, phase);
            assert_eq!(url.as_str(), PRIMARY);
            assert_eq!(body, self.expected[position]);
            requests.push((url.to_string(), body.clone()));
        }
        if self.hold_at == Some(phase) {
            self.reached.notify_one();
            self.release.notified().await;
        }
        if self.fail_at == Some(phase) {
            return Ok(HttpReply::for_test(StatusCode::SERVICE_UNAVAILABLE, vec![]));
        }
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0", "id":body["id"], "result":self.results[phase]
            }))
            .unwrap(),
        ))
    }
}

fn runtime(tape: Tape) -> SharedReadRuntime<Tape> {
    SharedReadRuntime::with_test_transport(&[PRIMARY.into(), BACKUP.into()], tape)
}

#[tokio::test]
async fn pump_bank_flow_uses_original_quote_and_complete_simulation_with_one_frozen_unsigned_message(
) {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let c = case(side, provider).await;
            let expected = c.expected.clone();
            let runtime = runtime(Tape::new(c.expected, c.results));
            let result = c
                .bank
                .observe_complete(
                    &runtime,
                    PRIMARY,
                    c.input,
                    c.buyback,
                    Hash::new_from_array([10; 32]),
                    1_500,
                    provider,
                    0.0001,
                    c.tip,
                )
                .await
                .unwrap();
            assert_eq!(
                bincode::serialize(result.packet().unwrap()).unwrap(),
                c.frozen
            );
            assert_eq!(result.packet().unwrap().signatures, [Signature::default()]);
            assert_eq!(result.bank.snapshot.started, c.started);
            assert_eq!(
                (result.simulation_slot, result.bank.snapshot.epoch),
                (177, 21)
            );
            assert_eq!(result.network_fee.get(), 5_300);
            assert_eq!(
                result.output.get(),
                if matches!(side, TradeType::Buy) {
                    251
                } else {
                    500_000
                }
            );
            assert_eq!(
                result.template.source.minimum.get(),
                if matches!(side, TradeType::Buy) {
                    213
                } else {
                    425_000
                }
            );
            assert_ne!(result.template.source.minimum.get(), 1);
            assert_eq!(
                result.template.source.source.observation.simulation_slot,
                176
            );
            assert_eq!(
                result
                    .template
                    .source
                    .source
                    .observation
                    .probe
                    .funding
                    .curve
                    .slot,
                175
            );
            assert_eq!(
                *runtime.test_transport().requests.lock().unwrap(),
                [
                    (PRIMARY.into(), expected[0].clone()),
                    (PRIMARY.into(), expected[1].clone())
                ]
            );
            assert!(runtime.aggregator_simulation_slot(PRIMARY).is_ok());
        }
    }
}

#[tokio::test]
async fn pump_bank_flow_failure_is_terminal_at_original_phase_without_backup_or_final_retry() {
    for damage in 0..7 {
        let mut c = case(TradeType::Buy, SwqosType::Default).await;
        let mut slippage = 1_500;
        let mut hash = Hash::new_from_array([10; 32]);
        let mut fail_at = None;
        let expected_calls = match damage {
            0 => {
                c.results[0]["value"].as_object_mut().unwrap().remove("fee");
                1
            }
            1 => {
                fail_at = Some(0);
                1
            }
            2 => {
                c.results[1]["value"]["fee"] = json!(5_000);
                2
            }
            3 => {
                fail_at = Some(1);
                2
            }
            4 => {
                slippage = 10_000;
                1
            }
            5 => {
                c.bank.started = Instant::now() - MAX_BALANCE_AGE - Duration::from_secs(1);
                0
            }
            _ => {
                hash = Hash::default();
                0
            }
        };
        let mut tape = Tape::new(c.expected, c.results);
        tape.fail_at = fail_at;
        let runtime = runtime(tape);
        let error = c
            .bank
            .observe_complete(
                &runtime,
                PRIMARY,
                c.input,
                c.buyback,
                hash,
                slippage,
                SwqosType::Default,
                0.0001,
                None,
            )
            .await
            .err()
            .expect("damaged bank flow accepted");
        assert!(!format!("{error:#}").contains("fixture-secret"));
        assert_eq!(
            runtime.test_transport().requests.lock().unwrap().len(),
            expected_calls
        );
        assert!(runtime.aggregator_simulation_slot(PRIMARY).is_ok());
    }
}

#[tokio::test]
async fn pump_bank_flow_abort_joins_both_read_boundaries_and_releases_original_slot_without_continuation(
) {
    for phase in [0, 1] {
        let c = case(TradeType::Buy, SwqosType::Temporal).await;
        let mut tape = Tape::new(c.expected, c.results);
        tape.hold_at = Some(phase);
        let runtime = Arc::new(runtime(tape));
        let active = runtime.clone();
        let task = tokio::spawn(async move {
            c.bank
                .observe_complete(
                    &active,
                    PRIMARY,
                    c.input,
                    c.buyback,
                    Hash::new_from_array([10; 32]),
                    1_500,
                    SwqosType::Temporal,
                    0.0001,
                    c.tip,
                )
                .await
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            runtime.test_transport().reached.notified(),
        )
        .await
        .unwrap();
        assert!(runtime.aggregator_simulation_slot(PRIMARY).is_err());
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert!(runtime.aggregator_simulation_slot(PRIMARY).is_ok());
        let before = runtime.test_transport().requests.lock().unwrap().clone();
        assert_eq!(before.len(), phase + 1);
        runtime.test_transport().release.notify_one();
        tokio::task::yield_now().await;
        assert_eq!(*runtime.test_transport().requests.lock().unwrap(), before);
    }
}
