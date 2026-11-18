use super::super::tests::fixture;
use super::*;
use serde_json::json;
use sol_trade_sdk::swqos::TradeType;
use solana_sdk::hash::Hash;
use std::time::{Duration, Instant};

async fn observed(side: TradeType) -> (NativeQuoteProbe, Value) {
    let (bank, ix, input) = fixture(side).await;
    let probe = bank
        .prepare_quote_probe(ix, input, Hash::new_from_array([10; 32]))
        .unwrap();
    let keys = &probe.transaction.message.account_keys;
    let index = |key| keys.iter().position(|actual| *actual == key).unwrap();
    let wallet = index(probe.funding.keys[6]);
    let curve = index(probe.funding.keys[0]);
    let buy = matches!(side, TradeType::Buy);
    let compiled = &probe.transaction.message.instructions[0];
    let recipients = [
        usize::from(compiled.accounts[1]),
        usize::from(compiled.accounts[if buy { 9 } else { 8 }]),
        usize::from(*compiled.accounts.last().unwrap()),
    ];
    let mut before = vec![1_000_000u64; keys.len()];
    before[wallet] = 50_000_000; // Not the earlier RPC snapshot's 20M balance.
    before[curve] = 60_000_000;
    let mut after = before.clone();
    for (index, credit) in recipients.into_iter().zip([1_000u64, 2_000, 3_000]) {
        after[index] += credit;
    }
    if buy {
        after[wallet] -= input.get() + 5_000;
        after[curve] += input.get() - 6_000;
    } else {
        after[wallet] += 500_000 - 5_000;
        after[curve] -= 500_000 + 6_000;
    }
    let token = |key, owner, amount: u64| {
        json!({
            "accountIndex": index(key), "mint": probe.funding.keys[1].to_string(),
            "owner": owner, "programId":spl_token_interface::ID.to_string(),
            "uiTokenAmount":{"amount":amount.to_string(), "decimals":probe.funding.curve.base_mint_layout.decimals,
                "uiAmount":null,"uiAmountString":"0"}
        })
    };
    let user = probe.funding.keys[5];
    let vault = probe.funding.keys[4];
    let user_owner = keys[wallet].to_string();
    let curve_owner = keys[curve].to_string();
    let response = json!({"context":{"slot":176},"value":{
        "err":null,"logs":[],"accounts":null,"fee":5_000,
        "preBalances":before,"postBalances":after,
        "preTokenBalances":[token(user, &user_owner, 9_000), token(vault, &curve_owner, 10_000)],
        "postTokenBalances":[
            token(user, &user_owner, if buy {9_251} else {8_000}),
            token(vault, &curve_owner, if buy {9_749} else {11_000})]
    }});
    (probe, response)
}

fn reject(probe: NativeQuoteProbe, response: Value, reason: &str) {
    let error = probe
        .inspect_observation(response)
        .err()
        .expect("invalid observation accepted");
    assert!(
        format!("{error:#}").contains(reason),
        "expected {reason}: {error:#}"
    );
}

#[tokio::test]
async fn native_observation_uses_same_response_net_units_and_retains_frozen_probe() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (probe, mut response) = observed(side).await;
        let frozen = bincode::serialize(probe.packet().unwrap()).unwrap();
        // Row order and float display values have no authority over raw integer units.
        response["value"]["preTokenBalances"]
            .as_array_mut()
            .unwrap()
            .reverse();
        response["value"]["postTokenBalances"][0]["uiTokenAmount"]["uiAmount"] = json!(9999.5);
        let output = probe.inspect_observation(response).unwrap();
        assert_eq!(
            output.output.get(),
            if matches!(side, TradeType::Buy) {
                251
            } else {
                500_000
            }
        );
        assert_eq!(output.network_fee.get(), 5_000);
        assert_eq!(output.simulation_slot, 176);
        assert_eq!(
            bincode::serialize(output.probe.packet().unwrap()).unwrap(),
            frozen
        );
    }
}

#[tokio::test]
async fn native_observation_rejects_missing_fields_wrong_context_and_native_movements() {
    for damage in 0..15 {
        let (probe, mut response) = observed(TradeType::Buy).await;
        let wallet = 0;
        let curve = probe
            .transaction
            .message
            .account_keys
            .iter()
            .position(|key| *key == probe.funding.keys[0])
            .unwrap();
        let reason = match damage {
            0 => {
                response["value"].as_object_mut().unwrap().remove("err");
                "explicit success"
            }
            1 => {
                response["value"]["err"] = json!({"InstructionError":[0,{"Custom":1}]});
                "explicit success"
            }
            2 => {
                response["context"]["slot"] = json!(174);
                "precedes"
            }
            3 => {
                response["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("preBalances");
                "omitted pre balances"
            }
            4 => {
                response["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("postBalances");
                "omitted post balances"
            }
            5 => {
                response["value"]["fee"] = Value::Null;
                "positive network fee"
            }
            6 => {
                response["value"]["fee"] = json!(0);
                "positive network fee"
            }
            7 => {
                response["value"]["postBalances"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
                "index count"
            }
            8 => {
                response["value"]["postBalances"][curve] = json!(1);
                "do not conserve"
            }
            9 => {
                let post = response["value"]["postBalances"][wallet].as_u64().unwrap();
                response["value"]["postBalances"][wallet] = json!(post - 1);
                let post = response["value"]["postBalances"][curve].as_u64().unwrap();
                response["value"]["postBalances"][curve] = json!(post + 1);
                "buy spend differs"
            }
            10 => {
                response["value"]["loadedAddresses"] =
                    json!({"writable":[probe.funding.keys[1].to_string()],"readonly":[]});
                "changed the fixed legacy context"
            }
            11 => {
                response["value"]["replacementBlockhash"] = json!({"blockhash":Hash::new_from_array([20;32]).to_string(),"lastValidBlockHeight":1000});
                "changed the fixed legacy context"
            }
            12 => {
                let index = probe
                    .transaction
                    .message
                    .account_keys
                    .iter()
                    .position(|key| *key == probe.funding.keys[5])
                    .unwrap();
                response["value"]["postBalances"][index] = json!(2_000_000);
                "outside its profile"
            }
            13 => {
                let recipient = usize::from(probe.transaction.message.instructions[0].accounts[1]);
                response["value"]["preBalances"][recipient] = json!(0);
                "recipient is absent or debited"
            }
            _ => {
                let recipient = usize::from(probe.transaction.message.instructions[0].accounts[1]);
                let pre = response["value"]["preBalances"][recipient]
                    .as_u64()
                    .unwrap();
                response["value"]["postBalances"][recipient] = json!(pre - 1);
                "recipient is absent or debited"
            }
        };
        reject(probe, response, reason);
    }
}

#[tokio::test]
async fn native_observation_handles_fee_exceeding_sell_output_and_large_native_totals() {
    for small_output in [true, false] {
        let (probe, mut response) = observed(TradeType::Sell).await;
        let wallet = 0;
        let curve = probe
            .transaction
            .message
            .account_keys
            .iter()
            .position(|key| *key == probe.funding.keys[0])
            .unwrap();
        let output = if small_output { 2_000 } else { 500_000 };
        let before = if small_output {
            50_000_000
        } else {
            u64::MAX - 500_000
        };
        response["value"]["preBalances"][wallet] = json!(before);
        response["value"]["postBalances"][wallet] = json!(before + output - 5_000);
        response["value"]["postBalances"][curve] = json!(60_000_000 - output - 6_000);
        assert_eq!(
            probe.inspect_observation(response).unwrap().output.get(),
            output
        );
    }
}

#[tokio::test]
async fn native_observation_requires_unique_bound_integer_token_rows_and_conservation() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for damage in 0..12 {
            let (probe, mut response) = observed(side).await;
            let row = &mut response["value"]["postTokenBalances"][0];
            let reason = match damage {
                0 => {
                    row["owner"] = json!(Pubkey::new_from_array([77; 32]).to_string());
                    "identity differs"
                }
                1 => {
                    row.as_object_mut().unwrap().remove("programId");
                    "identity differs"
                }
                2 => {
                    row["mint"] = json!(Pubkey::new_from_array([78; 32]).to_string());
                    "identity differs"
                }
                3 => {
                    row["uiTokenAmount"]["decimals"] = json!(99);
                    "identity differs"
                }
                4 => {
                    row["uiTokenAmount"]["amount"] = json!("+9251");
                    "unsigned integer"
                }
                5 => {
                    row["uiTokenAmount"]["amount"] = json!("18446744073709551616");
                    "overflows"
                }
                6 => {
                    row["accountIndex"] = json!(255);
                    "token index set"
                }
                7 => {
                    response["value"]["postTokenBalances"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                    "token index set"
                }
                8 => {
                    let copy = response["value"]["postTokenBalances"][0].clone();
                    response["value"]["postTokenBalances"][1] = copy;
                    "token index set"
                }
                9 => {
                    response["value"]
                        .as_object_mut()
                        .unwrap()
                        .remove("preTokenBalances");
                    "omitted pre token"
                }
                10 => {
                    response["value"]
                        .as_object_mut()
                        .unwrap()
                        .remove("postTokenBalances");
                    "omitted post token"
                }
                _ => {
                    row["uiTokenAmount"]["amount"] = json!(if matches!(side, TradeType::Buy) {
                        "9252"
                    } else {
                        "8001"
                    });
                    if matches!(side, TradeType::Buy) {
                        "token conservation differs"
                    } else {
                        "sell token input differs"
                    }
                }
            };
            reject(probe, response, reason);
        }
    }
}

#[tokio::test]
async fn native_observation_rejects_zero_outputs_unfunded_fee_and_stale_or_extended_profile() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (probe, mut response) = observed(side).await;
        if matches!(side, TradeType::Buy) {
            response["value"]["postTokenBalances"] = response["value"]["preTokenBalances"].clone();
        } else {
            let wallet = 0;
            let curve = probe
                .transaction
                .message
                .account_keys
                .iter()
                .position(|key| *key == probe.funding.keys[0])
                .unwrap();
            let pre = response["value"]["preBalances"][wallet].as_u64().unwrap();
            response["value"]["postBalances"][wallet] = json!(pre - 5_000);
            let pre = response["value"]["preBalances"][curve].as_u64().unwrap();
            response["value"]["postBalances"][curve] = json!(pre - 6_000);
        }
        reject(probe, response, "output is zero");
    }
    let (mut probe, response) = observed(TradeType::Buy).await;
    probe.funding.curve.base_mint_layout.program = spl_token_2022_interface::ID;
    reject(probe, response, "requires classic SPL Token");
    let (mut probe, response) = observed(TradeType::Buy).await;
    probe.funding.started = Instant::now() - super::super::MAX_BALANCE_AGE - Duration::from_secs(1);
    reject(probe, response, "expired");
    let (probe, mut response) = observed(TradeType::Sell).await;
    response["value"]["preBalances"][0] = json!(0);
    reject(probe, response, "cannot prefund its fee");
}
