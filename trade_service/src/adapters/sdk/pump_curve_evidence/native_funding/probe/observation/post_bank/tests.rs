use super::super::tests::observed;
use super::*;
use crate::adapters::sdk::pump_curve_evidence::{
    native_funding::destinations::tests::account as global_account,
    tests::{snapshot, ui},
};
use base64::Engine;
use serde_json::json;
use sol_trade_sdk::{instruction::utils::pumpfun::global_constants, swqos::TradeType};
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_program_pack::Pack;
use solana_sdk::{clock::Clock, sysvar};
use spl_token_interface::state::{Account, AccountState, Mint};
use std::time::{Duration, Instant};

pub(in crate::adapters::sdk::pump_curve_evidence::native_funding::probe::observation) fn accounts(
    probe: &NativeQuoteProbe,
    response: &Value,
) -> Vec<Option<UiAccount>> {
    let keys = &probe.transaction.message.account_keys;
    let index = |key| keys.iter().position(|actual| *actual == key).unwrap();
    let native = response["value"]["postBalances"].as_array().unwrap();
    let balance = |key| native[index(key)].as_u64().unwrap();
    let tokens = response["value"]["postTokenBalances"].as_array().unwrap();
    let units = |key| {
        tokens
            .iter()
            .find(|row| row["accountIndex"].as_u64() == Some(index(key) as u64))
            .unwrap()["uiTokenAmount"]["amount"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let token = |key, owner| {
        let mut bytes = [0; Account::LEN];
        Account::pack(
            Account {
                mint: probe.funding.keys[1],
                owner,
                amount: units(key),
                state: AccountState::Initialized,
                ..Account::default()
            },
            &mut bytes,
        )
        .unwrap();
        let mut account = ui(spl_token_interface::ID, &bytes);
        account.lamports = balance(key);
        Some(account)
    };
    let mut values = snapshot(None, spl_token_interface::ID).to_vec();
    values[2] = Some(global_account(global_constants::FEE_RECIPIENT));
    for i in [0, 1, 2] {
        values[i].as_mut().unwrap().lamports = balance(probe.funding.keys[i]);
    }
    values.push(token(probe.funding.keys[4], probe.funding.keys[0]));
    values.push(token(probe.funding.keys[5], probe.funding.keys[6]));
    let mut wallet = ui(solana_system_interface::program::ID, &[]);
    wallet.lamports = balance(probe.funding.keys[6]);
    values.push(Some(wallet));
    values.push(Some(ui(
        sysvar::id(),
        &bincode::serialize(&Clock {
            slot: response["context"]["slot"].as_u64().unwrap(),
            epoch: 20,
            ..Clock::default()
        })
        .unwrap(),
    )));
    let buy = probe.funding.side == super::super::Side::Buy;
    for position in [
        1,
        if buy { 9 } else { 8 },
        probe.instruction.accounts.len() - 1,
    ] {
        let key = probe.instruction.accounts[position].pubkey;
        // Synthetic metadata only: an observed owner is not a permitted-recipient proof.
        let mut account = ui(Pubkey::new_from_array([88; 32]), &[1, 2]);
        account.lamports = balance(key);
        values.push(Some(account));
    }
    assert_eq!(values.len(), 11);
    values
}

pub(in crate::adapters::sdk::pump_curve_evidence::native_funding::probe::observation) fn response_with_bank(
    probe: &NativeQuoteProbe,
    mut response: Value,
) -> Value {
    response["value"]["accounts"] = json!(accounts(probe, &response));
    response
}

fn bytes(account: &UiAccount) -> Vec<u8> {
    let UiAccountData::Binary(encoded, UiAccountEncoding::Base64) = &account.data else {
        panic!("bad fixture")
    };
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap()
}

fn with_bytes(account: &mut UiAccount, data: &[u8]) {
    account.data = UiAccountData::Binary(
        base64::engine::general_purpose::STANDARD.encode(data),
        UiAccountEncoding::Base64,
    );
    account.space = Some(data.len() as u64);
}

fn reject(probe: &NativeQuoteProbe, response: &Value, reason: &str) {
    let error = probe
        .inspect_post_bank(response)
        .err()
        .expect("invalid post bank accepted");
    assert!(
        format!("{error:#}").contains(reason),
        "expected {reason}: {error:#}"
    );
}

#[tokio::test]
async fn native_post_bank_reuses_canonical_decoder_and_retains_original_age_with_new_epoch() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (probe, response) = observed(side).await;
        let response = response_with_bank(&probe, response);
        let bank = probe.inspect_post_bank(&response).unwrap();
        assert_eq!(bank.snapshot.started, probe.funding.started);
        assert_eq!(bank.snapshot.keys, probe.funding.keys);
        assert_eq!((bank.snapshot.curve.slot, bank.snapshot.epoch), (176, 20));
        assert_ne!(bank.snapshot.native_lamports, probe.funding.native_lamports);
        assert_eq!(
            bank.snapshot.user_base_units,
            Some(if matches!(side, TradeType::Buy) {
                9_251
            } else {
                8_000
            })
        );
        assert_eq!(
            bank.snapshot.curve_base_units,
            if matches!(side, TradeType::Buy) {
                9_749
            } else {
                11_000
            }
        );
        assert!(bank.recipients.iter().all(|recipient| recipient.owner
            == Pubkey::new_from_array([88; 32])
            && recipient.data_len == 2
            && recipient.lamports > 0));
        assert_eq!(bank.recipients[0].key, probe.instruction.accounts[1].pubkey);
    }
}

#[tokio::test]
async fn native_post_bank_rejects_wrong_owners_ata_authorities_clock_and_frozen_destinations() {
    for damage in 0..11 {
        let (probe, response) = observed(TradeType::Buy).await;
        let mut values = accounts(&probe, &response);
        let reason = match damage {
            0 => {
                values[1] = None;
                "mint is missing"
            }
            1 => {
                values[1].as_mut().unwrap().owner =
                    solana_system_interface::program::ID.to_string();
                "token program is unsupported"
            }
            2 => {
                values[5] = None;
                "message account is missing"
            }
            3 => {
                let account = values[5].as_mut().unwrap();
                let mut state = Account::unpack(&bytes(account)).unwrap();
                state.owner = probe.funding.keys[0];
                let mut data = [0; Account::LEN];
                Account::pack(state, &mut data).unwrap();
                with_bytes(account, &data);
                "user base ATA"
            }
            4 => {
                let account = values[4].as_mut().unwrap();
                let mut state = Account::unpack(&bytes(account)).unwrap();
                state.delegate = Some(Pubkey::new_from_array([89; 32])).into();
                let mut data = [0; Account::LEN];
                Account::pack(state, &mut data).unwrap();
                with_bytes(account, &data);
                "curve base ATA"
            }
            5 => {
                values[7] = Some(ui(
                    sysvar::id(),
                    &bincode::serialize(&Clock {
                        slot: 175,
                        epoch: 20,
                        ..Clock::default()
                    })
                    .unwrap(),
                ));
                "Clock slot differs"
            }
            6 => {
                values[7].as_mut().unwrap().owner =
                    solana_system_interface::program::ID.to_string();
                "Clock account owner"
            }
            7 => {
                values[2] = Some(global_account(Pubkey::new_from_array([90; 32])));
                "differs from the bank"
            }
            8 => {
                let account = values[0].as_mut().unwrap();
                let mut data = bytes(account);
                data[49..81].copy_from_slice(Pubkey::new_from_array([91; 32]).as_ref());
                with_bytes(account, &data);
                "differs from the bank"
            }
            9 => {
                values.pop();
                "account count differs"
            }
            _ => {
                let account = values[1].as_mut().unwrap();
                let mut state = Mint::unpack(&bytes(account)).unwrap();
                state.supply += 1;
                let mut data = [0; Mint::LEN];
                Mint::pack(state, &mut data).unwrap();
                with_bytes(account, &data);
                "mint precision or supply changed"
            }
        };
        let mut response = response;
        response["value"]["accounts"] = json!(values);
        reject(&probe, &response, reason);
    }
}

#[tokio::test]
async fn native_post_bank_cross_checks_all_returned_message_lamports_token_data_and_recipient_metadata(
) {
    for damage in 0..8 {
        let (probe, response) = observed(TradeType::Buy).await;
        let mut values = accounts(&probe, &response);
        let reason = match damage {
            0 => {
                values[6].as_mut().unwrap().lamports += 1;
                "lamports contradict"
            }
            1 => {
                values[8].as_mut().unwrap().lamports += 1;
                "recipient lamports contradict"
            }
            2 => {
                values[9] = None;
                "recipient is missing"
            }
            3 => {
                values[10].as_mut().unwrap().executable = true;
                "recipient metadata account owner or state"
            }
            4 => {
                values[9].as_mut().unwrap().owner = "invalid owner".into();
                "owner is malformed"
            }
            5 => {
                with_bytes(
                    values[10].as_mut().unwrap(),
                    &vec![0; MAX_ACCOUNT_BYTES + 1],
                );
                "data exceeds size limit"
            }
            6 => {
                let account = values[5].as_mut().unwrap();
                let mut state = Account::unpack(&bytes(account)).unwrap();
                state.amount += 1;
                let mut data = [0; Account::LEN];
                Account::pack(state, &mut data).unwrap();
                with_bytes(account, &data);
                "token data contradict"
            }
            _ => {
                values[4].as_mut().unwrap().data =
                    UiAccountData::Binary("!".into(), UiAccountEncoding::Base64);
                "base64 is invalid"
            }
        };
        let mut response = response;
        response["value"]["accounts"] = json!(values);
        reject(&probe, &response, reason);
    }
}

#[tokio::test]
async fn native_post_bank_allows_terminal_after_simulation_but_not_preparation_or_remaining_real_tokens(
) {
    let (probe, response) = observed(TradeType::Buy).await;
    let mut values = accounts(&probe, &response);
    let account = values[0].as_mut().unwrap();
    let mut data = bytes(account);
    data[48] = 1;
    data[24..32].copy_from_slice(&0u64.to_le_bytes());
    with_bytes(account, &data);
    let mut response = response;
    response["value"]["accounts"] = json!(values);
    assert!(
        probe
            .inspect_post_bank(&response)
            .unwrap()
            .snapshot
            .curve
            .curve
            .complete
    );
    let plan = NativeFundingPlan::new(
        PumpCurveAccounts::new(probe.funding.keys[1], None, probe.funding.curve.quote).unwrap(),
        probe.funding.keys[6],
        spl_token_interface::ID,
        probe.funding.side,
    )
    .unwrap();
    assert!(plan
        .decode(plan.keys(), 176, 175, &values[..8])
        .unwrap_err()
        .to_string()
        .contains("curve is complete"));
    let account = values[0].as_mut().unwrap();
    let mut data = bytes(account);
    data[24..32].copy_from_slice(&1u64.to_le_bytes());
    with_bytes(account, &data);
    response["value"]["accounts"] = json!(values);
    reject(&probe, &response, "remaining real tokens");
}

#[tokio::test]
async fn native_post_bank_rejects_missing_native_token_fields_stale_origin_and_instruction_drift() {
    for damage in 0..5 {
        let (mut probe, response) = observed(TradeType::Buy).await;
        let mut response = response_with_bank(&probe, response);
        let reason = match damage {
            0 => {
                response["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("postBalances");
                "omitted native balances"
            }
            1 => {
                response["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("postTokenBalances");
                "omitted token balances"
            }
            2 => {
                response["value"]["postBalances"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
                "native index count"
            }
            3 => {
                probe.funding.started =
                    Instant::now() - super::super::super::MAX_BALANCE_AGE - Duration::from_secs(1);
                "expired"
            }
            _ => {
                probe.instruction.data[16..24].copy_from_slice(&2u64.to_le_bytes());
                "instruction and packet differ"
            }
        };
        reject(&probe, &response, reason);
    }
}
