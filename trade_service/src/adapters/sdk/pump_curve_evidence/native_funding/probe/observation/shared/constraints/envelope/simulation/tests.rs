use super::*;
use crate::adapters::{
    rpc::read_http::ReadRequest,
    sdk::pump_curve_evidence::{
        native_funding::probe::observation::{
            post_bank::tests::{accounts, response_with_bank},
            shared::NativeBankObservation,
            tests::observed,
        },
        tests::ui,
    },
    test_support::read_rpc_server::{LocalServer, Reply},
};
use base64::Engine;
use serde_json::json;
use sol_trade_sdk::{
    constants::WSOL_TOKEN_ACCOUNT,
    swqos::{SwqosType, TradeType},
};
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_program_pack::Pack;
use solana_sdk::{clock::Clock, sysvar};
use spl_token_interface::state::Account;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

async fn fixture(
    side: TradeType,
    provider: SwqosType,
    small_sell: bool,
) -> (NativeEnvelopeTemplate, Value) {
    let (probe, response) = observed(side).await;
    fixture_from_probe(probe, response, side, provider, small_sell)
}

pub(super) fn fixture_from_probe(
    probe: crate::adapters::sdk::pump_curve_evidence::native_funding::probe::NativeQuoteProbe,
    mut response: Value,
    side: TradeType,
    provider: SwqosType,
    small_sell: bool,
) -> (NativeEnvelopeTemplate, Value) {
    if small_sell {
        assert!(matches!(side, TradeType::Sell));
        let curve = probe
            .transaction
            .message
            .account_keys
            .iter()
            .position(|key| *key == probe.funding.keys[0])
            .unwrap();
        response["value"]["postBalances"][0] = json!(49_997_000u64);
        response["value"]["postBalances"][curve] = json!(59_992_000u64);
    }
    let mut bank_accounts = accounts(&probe, &response);
    let old_keys = probe.transaction.message.account_keys.clone();
    let original = response.clone();
    let response = response_with_bank(&probe, response);
    let bank = probe.inspect_post_bank(&response).unwrap();
    let observation = probe.inspect_observation(response).unwrap();
    let destination = (provider != SwqosType::Default)
        .then(|| crate::adapters::sdk::damm_instructions::tip_accounts(provider)[0]);
    let template = NativeBankObservation { observation, bank }
        .swap_constraints(1_500)
        .unwrap()
        .prepare_envelope(provider, 0.0001, destination)
        .unwrap();
    let VersionedMessage::Legacy(message) = &template.packet().unwrap().message else {
        unreachable!()
    };
    let index = |key| {
        message
            .account_keys
            .iter()
            .position(|actual| *actual == key)
            .unwrap()
    };
    let remap_native = |name: &str| {
        message
            .account_keys
            .iter()
            .map(|key| {
                old_keys
                    .iter()
                    .position(|actual| actual == key)
                    .map(|position| original["value"][name][position].as_u64().unwrap())
                    .unwrap_or(1_000_000)
            })
            .collect::<Vec<_>>()
    };
    let mut before = remap_native("preBalances");
    let mut after = remap_native("postBalances");
    // Final bank has70M, not the previous quote's50M or the preparation RPC's20M.
    before[0] += 20_000_000;
    after[0] += 20_000_000;
    after[0] -= PRIORITY_LAMPORTS;
    if let Some(destination) = destination {
        after[index(destination)] += 100_000;
        after[0] -= 100_000;
    }
    let remap_tokens = |name: &str| {
        let mut tokens = original["value"][name].as_array().unwrap().clone();
        for row in &mut tokens {
            let old = row["accountIndex"].as_u64().unwrap() as usize;
            row["accountIndex"] = json!(index(old_keys[old]));
        }
        tokens
    };
    bank_accounts[6].as_mut().unwrap().lamports = after[0];
    bank_accounts[7] = Some(ui(
        sysvar::id(),
        &bincode::serialize(&Clock {
            slot: 177,
            epoch: 21,
            ..Clock::default()
        })
        .unwrap(),
    ));
    if let Some(destination) = destination {
        let mut tip = ui(solana_system_interface::program::ID, &[]);
        tip.lamports = after[index(destination)];
        bank_accounts.push(Some(tip));
    }
    let result = json!({"context":{"slot":177},"value":{"err":null,"logs":[],"fee":5_300,
        "preBalances":before,"postBalances":after,"accounts":bank_accounts,
        "preTokenBalances":remap_tokens("preTokenBalances"),"postTokenBalances":remap_tokens("postTokenBalances")}});
    (template, result)
}

fn reply(result: Value, mode: Reply) -> Reply {
    Reply::PumpNativeFinal {
        result,
        mode: Box::new(mode),
    }
}

fn token_units(result: &mut Value, account_position: usize, units: u64) {
    let mut account: UiAccount =
        serde_json::from_value(result["value"]["accounts"][account_position].clone()).unwrap();
    let UiAccountData::Binary(encoded, UiAccountEncoding::Base64) = &account.data else {
        panic!("bad fixture")
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let mut state = Account::unpack(&bytes).unwrap();
    state.amount = units;
    let mut bytes = [0; Account::LEN];
    Account::pack(state, &mut bytes).unwrap();
    account.data = UiAccountData::Binary(
        base64::engine::general_purpose::STANDARD.encode(bytes),
        UiAccountEncoding::Base64,
    );
    result["value"]["accounts"][account_position] = json!(account);
}

#[tokio::test]
async fn native_final_adapter_binds_actual_complete_packet_new_bank_and_same_response_output() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (template, response) = fixture(side, provider, false).await;
            let until = template.source.source.observation.probe.funding.started;
            let frozen = bincode::serialize(template.packet().unwrap()).unwrap();
            let keys = template.evidence_accounts();
            let plan = PumpNativeFinalPacket::new(
                template.packet().unwrap(),
                &keys,
                176,
                until + crate::domain::preparation_balance::MAX_BALANCE_AGE,
            )
            .unwrap();
            let expected = ReadRequest::PumpNativeFinal {
                id: 1,
                packet: plan,
            }
            .test_body();
            let post = response["value"]["postBalances"][0].as_u64().unwrap();
            let server = LocalServer::new(reply(response, Reply::Success)).await;
            let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
            let result = template.simulate(&runtime, &server.url).await.unwrap();
            assert_eq!(
                bincode::serialize(result.packet().unwrap()).unwrap(),
                frozen
            );
            assert_eq!(
                result.output.get(),
                if matches!(side, TradeType::Buy) {
                    251
                } else {
                    500_000
                }
            );
            assert_eq!(result.network_fee.get(), 5_300);
            assert_eq!(
                (result.simulation_slot, result.bank.snapshot.epoch),
                (177, 21)
            );
            assert_eq!(result.bank.snapshot.started, until);
            assert_eq!(result.bank.snapshot.native_lamports, post);
            assert_ne!(
                post,
                result.template.source.source.bank.snapshot.native_lamports
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
                    .quote,
                WSOL_TOKEN_ACCOUNT
            );
            assert_eq!(*server.requests.lock().unwrap(), [expected]);
            assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
            server.finish().await;
        }
    }
}

#[tokio::test]
async fn native_final_adapter_rejects_inconsistent_fee_minimum_tip_token_bank_and_context() {
    for damage in 0..16 {
        let (template, mut response) = fixture(TradeType::Buy, SwqosType::Temporal, false).await;
        let VersionedMessage::Legacy(message) = &template.packet().unwrap().message else {
            unreachable!()
        };
        let probe = &template.source.source.observation.probe;
        let index = |key| {
            message
                .account_keys
                .iter()
                .position(|actual| *actual == key)
                .unwrap()
        };
        let reason = match damage {
            0 => {
                response["value"].as_object_mut().unwrap().remove("err");
                "explicit success"
            }
            1 => {
                response["value"]["fee"] = json!(5_000);
                let value = response["value"]["postBalances"][0].as_u64().unwrap() + 300;
                response["value"]["postBalances"][0] = json!(value);
                "fee differs"
            }
            2 => {
                response["value"]["fee"] = Value::Null;
                "positive network fee"
            }
            3 => {
                let tip = index(template.destination.unwrap());
                let value = response["value"]["postBalances"][tip].as_u64().unwrap() + 1;
                response["value"]["postBalances"][tip] = json!(value);
                "tip is absent or differs"
            }
            4 => {
                let mint = index(probe.funding.keys[1]);
                let value = response["value"]["postBalances"][mint].as_u64().unwrap() + 1;
                response["value"]["postBalances"][mint] = json!(value);
                "outside its profile"
            }
            5 => {
                response["value"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] = json!("1.5");
                "not an unsigned integer"
            }
            6 => {
                response["value"]["postTokenBalances"][1] =
                    response["value"]["postTokenBalances"][0].clone();
                "token index set"
            }
            7 => {
                let output = template.source.minimum.get() - 1;
                response["value"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] =
                    json!((9_000 + output).to_string());
                response["value"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] =
                    json!((10_000 - output).to_string());
                "below the frozen minimum"
            }
            8 => {
                response["value"]["replacementBlockhash"] = json!({"blockhash":message.recent_blockhash.to_string(),"lastValidBlockHeight":999});
                "fixed message context"
            }
            9 => {
                response["value"]["accounts"][5]["owner"] =
                    json!(solana_system_interface::program::ID.to_string());
                "user base ATA"
            }
            10 => {
                response["value"]["accounts"][11]["owner"] =
                    json!(spl_token_interface::ID.to_string());
                "tip account owner or state"
            }
            11 => {
                let value = response["value"]["accounts"][11]["lamports"]
                    .as_u64()
                    .unwrap()
                    + 1;
                response["value"]["accounts"][11]["lamports"] = json!(value);
                "tip metadata contradicts"
            }
            12 => {
                response["value"]["accounts"][7] = json!(ui(
                    sysvar::id(),
                    &bincode::serialize(&Clock {
                        slot: 176,
                        epoch: 21,
                        ..Clock::default()
                    })
                    .unwrap()
                ));
                "Clock slot differs"
            }
            13 => {
                token_units(&mut response, 5, 9_252);
                "token data contradict"
            }
            14 => {
                response["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("postBalances");
                "omitted post balances"
            }
            _ => {
                response["context"]["slot"] = json!(175);
                "precedes"
            }
        };
        let server = LocalServer::new(reply(response, Reply::Success)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        let error = template
            .simulate(&runtime, &server.url)
            .await
            .err()
            .expect("invalid final result accepted");
        assert!(
            format!("{error:#}").contains(reason),
            "damage{damage}, expected{reason}: {error:#}"
        );
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        server.finish().await;
    }
}

#[tokio::test]
async fn native_final_adapter_handles_small_sell_output_and_rejects_proceeds_as_prefunding() {
    let (template, response) = fixture(TradeType::Sell, SwqosType::Default, true).await;
    let result = template.inspect_final(response).unwrap();
    assert_eq!(result.output.get(), 2_000);
    assert_eq!(result.network_fee.get(), 5_300);
    let (template, mut response) = fixture(TradeType::Sell, SwqosType::Temporal, false).await;
    response["value"]["preBalances"][0] = json!(0);
    response["value"]["postBalances"][0] = json!(394_700);
    response["value"]["accounts"][6]["lamports"] = json!(394_700);
    let error = template.inspect_final(response).err().unwrap();
    assert!(error.to_string().contains("cannot prefund"));
}

#[tokio::test]
async fn native_final_adapter_enforces_exact_net_minimum_with_matching_token_account_data() {
    for delta in [-1i64, 0, 1] {
        let (template, mut response) = fixture(TradeType::Buy, SwqosType::Temporal, false).await;
        let minimum = template.source.minimum.get();
        let output = u64::try_from(i128::from(minimum) + i128::from(delta)).unwrap();
        response["value"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] =
            json!((9_000 + output).to_string());
        response["value"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] =
            json!((10_000 - output).to_string());
        token_units(&mut response, 5, 9_000 + output);
        token_units(&mut response, 4, 10_000 - output);
        let server = LocalServer::new(reply(response, Reply::Success)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        let result = template.simulate(&runtime, &server.url).await;
        if delta < 0 {
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("below the frozen minimum"));
        } else {
            let mut result = result.unwrap();
            assert_eq!(result.output.get(), output);
            result.packet().unwrap();
            result.output = NonZeroU64::new(minimum - 1).unwrap();
            assert!(result.packet().is_err());
        }
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        server.finish().await;
    }
}

#[tokio::test]
async fn native_final_adapter_refuses_expired_changed_or_unsupported_templates_before_io() {
    for damage in 0..3 {
        let (mut template, response) = fixture(TradeType::Buy, SwqosType::Default, false).await;
        match damage {
            0 => {
                template.source.source.observation.probe.funding.started = Instant::now()
                    - crate::domain::preparation_balance::MAX_BALANCE_AGE
                    - Duration::from_secs(1)
            }
            1 => {
                template.transaction.signatures[0] = solana_sdk::signature::Signature::from([1; 64])
            }
            _ => {
                template
                    .source
                    .source
                    .observation
                    .probe
                    .funding
                    .curve
                    .base_mint_layout
                    .program = spl_token_2022_interface::ID
            }
        }
        let server = LocalServer::new(reply(response, Reply::Success)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        assert!(template.simulate(&runtime, &server.url).await.is_err());
        assert!(server.requests.lock().unwrap().is_empty());
        server.finish().await;
    }
}

#[tokio::test]
async fn native_final_adapter_cancellation_or_bad_rpc_never_rebuilds_or_queries_another_lane() {
    for mode in [
        Reply::ServerError,
        Reply::RateLimited(Some("60")),
        Reply::SlowHeaders,
    ] {
        let cancel = matches!(&mode, Reply::SlowHeaders);
        let (template, response) = fixture(TradeType::Sell, SwqosType::Temporal, false).await;
        let server = LocalServer::new(reply(response, mode)).await;
        let backup = LocalServer::new(Reply::Success).await;
        let runtime =
            Arc::new(SharedReadRuntime::new(&[server.url.clone(), backup.url.clone()]).unwrap());
        let task = {
            let runtime = runtime.clone();
            let url = server.url.clone();
            tokio::spawn(async move { template.simulate(&runtime, &url).await })
        };
        server.received.notified().await;
        if cancel {
            task.abort();
            assert!(matches!(task.await,Err(error) if error.is_cancelled()));
        } else {
            assert!(task.await.unwrap().is_err());
        }
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        assert!(backup.requests.lock().unwrap().is_empty());
        assert!(runtime.aggregator_simulation_slot(&backup.url).is_ok());
        server.finish().await;
        backup.finish().await;
    }
}
