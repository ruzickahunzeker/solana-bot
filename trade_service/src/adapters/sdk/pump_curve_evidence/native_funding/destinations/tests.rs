use super::*;
use crate::adapters::sdk::pump_curve_evidence::{
    instruction_amounts::tests::request,
    native_funding::instruction_tests::{build, funding_with_global},
    tests::ui,
};
use crate::domain::trade::Side;
use serde_json::Value;
use sol_trade_sdk::{
    constants::SOL_TOKEN_ACCOUNT,
    instruction::utils::pumpfun::{accounts, global_constants},
    swqos::TradeType,
};
use solana_account_decoder::UiAccount;

fn global(primary: Pubkey) -> Vec<u8> {
    let idl: Value = serde_json::from_str(include_str!(
        "../../../../../../vendor/sol-trade-sdk/idl/pump.json"
    ))
    .unwrap();
    let fields = idl["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ty| ty["name"] == "Global")
        .unwrap()["type"]["fields"]
        .as_array()
        .unwrap();
    let mut data = GLOBAL_ACCOUNT_DISCRIMINATOR.to_vec();
    // Encode from the pinned IDL, independently of the struct used by the decoder.
    for field in fields {
        let name = field["name"].as_str().unwrap();
        match field["type"].as_str() {
            Some("bool") => data.push(0), // `initialized` is an unused field, not liveness.
            Some("u64") => data.extend(95u64.to_le_bytes()),
            Some("pubkey") => data.extend(if name == "fee_recipient" {
                primary.to_bytes()
            } else {
                [90; 32]
            }),
            _ => {
                assert_eq!(field["type"]["array"][0], "pubkey");
                let count = field["type"]["array"][1].as_u64().unwrap() as usize;
                for i in 0..count {
                    data.extend(if name == "buyback_fee_recipients" {
                        global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS[i].to_bytes()
                    } else {
                        [91; 32]
                    });
                }
            }
        }
        if name == "buyback_basis_points" {
            break;
        }
    }
    assert_eq!(data.len(), GLOBAL_FEE_PREFIX_END);
    data
}

fn account(primary: Pubkey) -> UiAccount {
    ui(accounts::PUMPFUN, &global(primary))
}

#[tokio::test]
async fn native_destinations_use_same_bank_global_set_and_creator_pda() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let params = request(SOL_TOKEN_ACCOUNT, side, None, true);
        let bank = funding_with_global(&params, Some(account(global_constants::FEE_RECIPIENT)));
        let destinations = bank.destinations().unwrap();
        assert_eq!(destinations.protocol_fee, global_constants::FEE_RECIPIENT);
        assert_eq!(
            destinations.creator_vault,
            get_creator_vault_pda(&bank.curve.curve.creator).unwrap()
        );
        assert_eq!(
            destinations.buyback,
            global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS
        );
        bank.inspect_native_destinations(&build(&params).await)
            .unwrap();
    }
}

#[tokio::test]
async fn native_destinations_reject_stale_sdk_recipient_and_unverified_creator_hint() {
    let rotated = Pubkey::new_from_array([80; 32]);
    for side in [TradeType::Buy, TradeType::Sell] {
        let params = request(SOL_TOKEN_ACCOUNT, side, None, true);
        let bank = funding_with_global(&params, Some(account(rotated)));
        let original = build(&params).await;
        assert!(bank
            .inspect_native_destinations(&original)
            .unwrap_err()
            .to_string()
            .contains("differs from the bank"));
        let mut matching = original;
        matching.accounts[1] = AccountMeta::new(rotated, false);
        bank.inspect_native_destinations(&matching).unwrap();
        for damage in 0..4 {
            let mut ix = matching.clone();
            match damage {
                0 => ix.accounts[1].is_writable = false,
                1 => {
                    ix.accounts[if bank.side == Side::Buy { 9 } else { 8 }].pubkey =
                        Pubkey::new_from_array([81; 32])
                }
                2 => ix.accounts.last_mut().unwrap().pubkey = Pubkey::new_from_array([82; 32]),
                _ => ix.accounts.last_mut().unwrap().is_writable = false,
            }
            assert!(
                bank.inspect_native_destinations(&ix).is_err(),
                "damage {damage}"
            );
        }
        let mut hinted = request(SOL_TOKEN_ACCOUNT, params.trade_type, None, true);
        let rogue_vault = Pubkey::new_from_array([81; 32]);
        if let sol_trade_sdk::trading::core::params::DexParamEnum::PumpFun(protocol) =
            &mut hinted.protocol_params
        {
            protocol.creator_vault = rogue_vault;
        }
        let hinted_ix = build(&hinted).await;
        assert_eq!(
            hinted_ix.accounts[if bank.side == Side::Buy { 9 } else { 8 }].pubkey,
            rogue_vault
        );
        let original_bank =
            funding_with_global(&params, Some(account(global_constants::FEE_RECIPIENT)));
        assert!(original_bank
            .inspect_native_destinations(&hinted_ix)
            .is_err());

        // Same Global-selected destination at both fee and program positions would promote
        // the program account's compiled writable privilege without changing its own meta.
        let aliased_bank = funding_with_global(&params, Some(account(accounts::FEE_PROGRAM)));
        let mut aliased = build(&params).await;
        aliased.accounts[1] = AccountMeta::new(accounts::FEE_PROGRAM, false);
        let program_meta = if bank.side == Side::Buy { 15 } else { 13 };
        assert!(!aliased.accounts[program_meta].is_writable);
        let message =
            solana_sdk::message::Message::new(&[aliased.clone()], Some(&params.payer.pubkey()));
        let index = message
            .account_keys
            .iter()
            .position(|key| *key == accounts::FEE_PROGRAM)
            .unwrap();
        assert!(index >= usize::from(message.header.num_required_signatures));
        assert!(
            index
                < message.account_keys.len()
                    - usize::from(message.header.num_readonly_unsigned_accounts)
        );
        assert!(aliased_bank
            .inspect_native_destinations(&aliased)
            .unwrap_err()
            .to_string()
            .contains("aliases"));
    }
}

#[test]
fn native_destinations_refuse_missing_or_incomplete_fee_and_sharing_evidence() {
    let params = request(SOL_TOKEN_ACCOUNT, TradeType::Buy, None, true);
    let incomplete = funding_with_global(&params, None);
    assert!(incomplete
        .destinations()
        .unwrap_err()
        .to_string()
        .contains("truncated"));
    let mut bank = funding_with_global(&params, Some(account(global_constants::FEE_RECIPIENT)));
    for status in [0, 1, 255] {
        bank.curve.sharing = SharingPrefix::Present {
            bump: 254,
            version: 1,
            status,
        };
        assert!(bank.destinations().is_err());
    }
    bank.curve.sharing = SharingPrefix::Absent;
    bank.curve.curve.mayhem = true;
    assert!(bank.destinations().is_err());
    bank.curve.curve.mayhem = false;
    bank.curve.global_data[145] = 2; // Invalid Borsh enable_migrate bool; never reinterpret it as false.
    assert!(bank
        .destinations()
        .unwrap_err()
        .to_string()
        .contains("prefix is invalid"));
    bank.curve.global_data = global(global_constants::FEE_RECIPIENT);
    bank.curve.global_data[997..1005].copy_from_slice(&10_001u64.to_le_bytes());
    assert!(bank
        .destinations()
        .unwrap_err()
        .to_string()
        .contains("fee field"));
    bank.curve.global_data = global(global_constants::FEE_RECIPIENT);
    bank.curve.global_data[741..773].fill(0);
    assert!(bank
        .destinations()
        .unwrap_err()
        .to_string()
        .contains("unset"));
}
