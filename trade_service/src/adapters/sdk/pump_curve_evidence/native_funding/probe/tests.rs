use super::*;
use crate::adapters::rpc::read_http::{HttpReply, ReadHttpTransport, ReadRequest};
use crate::adapters::rpc::read_runtime::SharedReadRuntime;
use crate::adapters::sdk::pump_curve_evidence::{
    instruction_amounts::tests::request,
    native_funding::{
        destinations::tests::account,
        instruction_tests::{build, funding_with_global},
        tests::fixture as funding_fixture,
    },
};
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use sol_trade_sdk::{
    constants::SOL_TOKEN_ACCOUNT, instruction::utils::pumpfun::global_constants, swqos::TradeType,
};
use solana_sdk::signature::Signature;
use std::{sync::Mutex, time::Duration};

pub(super) async fn fixture(side: TradeType) -> (NativeFundingSnapshot, Instruction, NonZeroU64) {
    let buy = matches!(side, TradeType::Buy);
    let mut params = request(SOL_TOKEN_ACCOUNT, side, (!buy).then_some(1), true);
    let amount = if buy { 1_000_000 } else { 1_000 };
    params.input_amount = Some(amount);
    let bank = funding_with_global(&params, Some(account(global_constants::FEE_RECIPIENT)));
    (bank, build(&params).await, NonZeroU64::new(amount).unwrap())
}

#[tokio::test]
async fn native_probe_freezes_one_zero_signature_swap_and_its_bank_context() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (bank, mut ix, input) = fixture(side).await;
        let hash = Hash::new_from_array([10; 32]);
        let probe = bank.prepare_quote_probe(ix.clone(), input, hash).unwrap();
        assert_eq!(probe.funding.curve.slot, 175);
        assert_eq!(probe.input, input);
        let tx = probe.packet().unwrap();
        let mut expected = ix.clone();
        expected.data[16..24].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(
            tx.message,
            Message::new_with_blockhash(&[expected], Some(&probe.funding.keys[6]), &hash)
        );
        assert_eq!(tx.signatures, [Signature::default()]);
        assert_eq!(tx.message.recent_blockhash, hash);
        assert_eq!(tx.message.header.num_required_signatures, 1);
        assert_eq!(tx.message.instructions.len(), 1);
        assert_eq!(
            &tx.message.instructions[0].data[8..16],
            &input.get().to_le_bytes()
        );
        assert_eq!(
            &tx.message.instructions[0].data[16..24],
            &1u64.to_le_bytes()
        );
        let frozen = bincode::serialize(tx).unwrap();
        ix.accounts[1].pubkey = solana_sdk::pubkey::Pubkey::new_from_array([80; 32]);
        ix.data[8..16].fill(0);
        assert_eq!(bincode::serialize(probe.packet().unwrap()).unwrap(), frozen);
    }
}

#[tokio::test]
async fn native_probe_rejects_wrong_input_unfunded_balance_and_missing_ata() {
    let hash = Hash::new_from_array([10; 32]);
    for side in [TradeType::Buy, TradeType::Sell] {
        for damage in 0..7 {
            let (mut bank, mut ix, input) = fixture(side).await;
            let expected = match damage {
                0 => NonZeroU64::new(input.get() + 1).unwrap(),
                1 => {
                    bank.user_base_units = None;
                    input
                }
                2 => {
                    if matches!(side, TradeType::Buy) {
                        bank.native_lamports = 1;
                    } else {
                        bank.user_base_units = Some(1);
                    }
                    input
                }
                _ => input,
            };
            let blockhash = if damage == 3 { Hash::default() } else { hash };
            let reason = match damage {
                0 => "differs from the frozen intent",
                1 => "requires an existing user ATA",
                2 if matches!(side, TradeType::Buy) => "cannot fund input",
                2 => "exceeds snapshot balance",
                3 => "blockhash is missing",
                4 => {
                    ix.data.truncate(16);
                    "wire length"
                }
                5 => {
                    ix.accounts[1].is_signer = true;
                    "unexpected signer"
                }
                _ => {
                    ix.accounts[1].pubkey = solana_sdk::pubkey::Pubkey::new_from_array([80; 32]);
                    "differs from the bank"
                }
            };
            let error = bank
                .prepare_quote_probe(ix, expected, blockhash)
                .err()
                .expect("invalid probe accepted");
            assert!(
                error.to_string().contains(reason),
                "damage {damage}: {error:#}"
            );
        }
    }
}

#[tokio::test]
async fn native_probe_checks_age_before_and_after_freezing() {
    let hash = Hash::new_from_array([10; 32]);
    for future in [false, true] {
        let (mut bank, ix, input) = fixture(TradeType::Buy).await;
        bank.started = if future {
            Instant::now() + Duration::from_secs(10)
        } else {
            Instant::now() - MAX_BALANCE_AGE - Duration::from_secs(1)
        };
        assert!(bank.prepare_quote_probe(ix, input, hash).is_err());
    }
    let (bank, ix, input) = fixture(TradeType::Buy).await;
    let mut probe = bank.prepare_quote_probe(ix, input, hash).unwrap();
    probe.funding.started = Instant::now() - MAX_BALANCE_AGE - Duration::from_secs(1);
    assert!(probe.packet().is_err());
}

struct AccountReply {
    result: Value,
    received: Mutex<Option<(Instant, Value)>>,
}

impl ReadHttpTransport for AccountReply {
    async fn fetch(&self, _url: &Url, request: &ReadRequest) -> anyhow::Result<HttpReply> {
        let body = request.test_body();
        assert!(matches!(request, ReadRequest::Parameters { .. }));
        *self.received.lock().unwrap() = Some((Instant::now(), body.clone()));
        tokio::task::yield_now().await;
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0", "id":body["id"], "result":self.result
            }))?,
        ))
    }
}

#[tokio::test]
async fn native_probe_account_age_starts_before_shared_rpc_not_after_decode() {
    let (plan, values) = funding_fixture(Side::Buy, spl_token_interface::ID);
    let url = "http://127.0.0.1:12345".to_owned();
    let runtime = SharedReadRuntime::with_test_transport(
        &[url.clone()],
        AccountReply {
            result: json!({"context":{"slot":175}, "value":values}),
            received: Mutex::new(None),
        },
    );
    let before = Instant::now();
    let snapshot = plan
        .read(&runtime, &url, 174, before + Duration::from_secs(4))
        .await
        .unwrap();
    let (received, body) = runtime
        .test_transport()
        .received
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert!(before <= snapshot.started && snapshot.started <= received);
    assert_eq!(snapshot.curve.slot, 175);
    assert_eq!(
        body,
        json!({"jsonrpc":"2.0", "id":1, "method":"getMultipleAccounts",
        "params":[plan.keys().map(|key|key.to_string()),
        {"encoding":"base64", "commitment":"processed", "dataSlice":null, "minContextSlot":174}]})
    );
    assert!(runtime.parameter_slot().is_ok());
}
