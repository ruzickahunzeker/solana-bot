use super::*;
use base64::Engine;
use serde_json::{json, Value};
use solana_program_pack::Pack;
use spl_token_interface::state::{Account, AccountState, Mint};

pub(crate) fn a(n: u8) -> Address {
    Address::from_bytes([n; 32])
}
pub(crate) fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
pub(crate) fn ui(program: Pubkey, data: &[u8]) -> Value {
    json!({"owner":program.to_string(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"],
        "lamports":10_000_000,"executable":false,"rentEpoch":0,"space":data.len()})
}
pub(crate) fn mint_bytes(decimals: u8) -> Vec<u8> {
    let mut data = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            decimals,
            is_initialized: true,
            supply: u64::MAX,
            ..Default::default()
        },
        &mut data,
    )
    .unwrap();
    data
}
pub(crate) fn token_bytes(units: u64) -> Vec<u8> {
    let mut data = vec![0; Account::LEN];
    Account::pack(
        Account {
            mint: key(9),
            owner: key(1),
            amount: units,
            state: AccountState::Initialized,
            ..Default::default()
        },
        &mut data,
    )
    .unwrap();
    data
}
pub(crate) fn raw_accounts(token2022: bool, units: u64) -> Value {
    let program = if token2022 {
        spl_token_2022_interface::ID
    } else {
        spl_token_interface::ID
    };
    let token = ui(program, &token_bytes(units));
    json!([
        ui(program, &mint_bytes(6)),
        if token2022 {
            Value::Null
        } else {
            token.clone()
        },
        if token2022 { token } else { Value::Null }
    ])
}
fn check(values: Value) -> Result<RawInputBalance> {
    inspect(
        a(1),
        a(9),
        &serde_json::from_value::<Vec<Option<UiAccount>>>(values).unwrap(),
    )
}

#[test]
fn preparation_assets_preserve_classic_and_token2022_raw_units_and_canonical_keys() {
    for token2022 in [false, true] {
        for units in [0, 1, 9007199254740993, u64::MAX] {
            let actual = check(raw_accounts(token2022, units)).unwrap();
            assert_eq!(actual.units, units);
            assert_eq!(actual.decimals, 6);
            let program = if token2022 {
                spl_token_2022_interface::ID
            } else {
                spl_token_interface::ID
            };
            assert_eq!(actual.program.to_bytes(), program.to_bytes());
            assert_eq!(
                input_keys(a(1), a(9))[if token2022 { 2 } else { 1 }],
                sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &key(1),
                    &key(9),
                    &program
                )
            );
        }
    }
}

#[test]
fn preparation_assets_missing_input_is_zero_but_missing_mint_or_wrong_program_account_is_invalid() {
    for token2022 in [false, true] {
        let mut values = raw_accounts(token2022, 1000);
        values[if token2022 { 2 } else { 1 }] = Value::Null;
        assert_eq!(check(values.clone()).unwrap().units, 0);
        values[0] = Value::Null;
        assert!(check(values).is_err());
        let mut values = raw_accounts(token2022, 1000);
        values[if token2022 { 1 } else { 2 }] = ui(spl_token_interface::ID, &token_bytes(1000));
        assert!(check(values).is_err());
    }
}

#[test]
fn preparation_assets_unused_other_program_ata_prefunding_does_not_change_input_balance() {
    for token2022 in [false, true] {
        for damage in 0..4 {
            let mut values = raw_accounts(token2022, 1000);
            let mut prefund = ui(Pubkey::default(), &[]);
            match damage {
                0 => {}
                1 => prefund["executable"] = json!(true),
                2 => prefund["space"] = json!(1),
                _ => prefund = ui(Pubkey::default(), &[1]),
            }
            values[if token2022 { 1 } else { 2 }] = prefund;
            let result = check(values);
            assert_eq!(result.is_ok(), damage == 0);
            if let Ok(balance) = result {
                assert_eq!(balance.units, 1000);
            }
        }
    }
}

#[test]
fn preparation_assets_reject_bad_envelopes_sizes_encoding_or_positional_roles() {
    for pointer in ["/0/owner", "/1/owner", "/0/data/0", "/1/data/0"] {
        let mut values = raw_accounts(false, 1000);
        *values.pointer_mut(pointer).unwrap() = json!("invalid");
        assert!(check(values).is_err());
    }
    for pointer in ["/0/executable", "/1/executable"] {
        let mut values = raw_accounts(false, 1000);
        *values.pointer_mut(pointer).unwrap() = json!(true);
        assert!(check(values).is_err());
    }
    for pointer in ["/0/lamports", "/1/lamports", "/0/space", "/1/space"] {
        let mut values = raw_accounts(false, 1000);
        *values.pointer_mut(pointer).unwrap() = json!(0);
        assert!(check(values).is_err());
    }
    let mut values = raw_accounts(false, 1000);
    values.as_array_mut().unwrap().swap(0, 1);
    assert!(check(values).is_err());
    for count in [0, 2, 4] {
        assert!(check(Value::Array(vec![Value::Null; count])).is_err());
    }
    for data in [vec![0; 2], vec![0; MAX_ACCOUNT_BYTES + 1]] {
        let mut values = raw_accounts(false, 1000);
        values[0] = ui(spl_token_interface::ID, &data);
        assert!(check(values).is_err());
    }
}

#[test]
fn preparation_assets_reject_uninitialized_mint_and_wrong_wallet_mint_frozen_or_authorities() {
    for token2022 in [false, true] {
        let program = if token2022 {
            spl_token_2022_interface::ID
        } else {
            spl_token_interface::ID
        };
        let mut values = raw_accounts(token2022, 1000);
        let mut mint = Mint::unpack(&mint_bytes(6)).unwrap();
        mint.is_initialized = false;
        let mut data = mint_bytes(6);
        Mint::pack(mint, &mut data).unwrap();
        values[0] = ui(program, &data);
        assert!(check(values).is_err());
        for damage in 0..6 {
            let mut values = raw_accounts(token2022, 1000);
            let mut token = Account::unpack(&token_bytes(1000)).unwrap();
            match damage {
                0 => token.mint = key(99),
                1 => token.owner = key(99),
                2 => token.state = AccountState::Frozen,
                3 => token.state = AccountState::Uninitialized,
                4 => token.delegate = Some(key(99)).into(),
                _ => token.close_authority = Some(key(99)).into(),
            }
            let mut data = token_bytes(1000);
            Account::pack(token, &mut data).unwrap();
            values[if token2022 { 2 } else { 1 }] = ui(program, &data);
            assert!(check(values).is_err());
        }
    }
}

pub(crate) fn extended_mint(active_hook: bool, older_bps: u16, newer_bps: u16) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::TransferFee, transfer_hook::TransferHook, BaseStateWithExtensionsMut,
            StateWithExtensionsMut,
        },
        state::Mint,
    };
    let types = [
        ExtensionType::TransferFeeConfig,
        ExtensionType::TransferHook,
    ];
    let mut data = vec![0; ExtensionType::try_calculate_account_len::<Mint>(&types).unwrap()];
    let mut mint = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    let fee = mint.init_extension::<TransferFeeConfig>(false).unwrap();
    fee.older_transfer_fee = TransferFee {
        epoch: 10u64.into(),
        maximum_fee: 5u64.into(),
        transfer_fee_basis_points: older_bps.into(),
    };
    fee.newer_transfer_fee = TransferFee {
        epoch: 20u64.into(),
        maximum_fee: 9u64.into(),
        transfer_fee_basis_points: newer_bps.into(),
    };
    let hook = mint.init_extension::<TransferHook>(false).unwrap();
    if active_hook {
        hook.program_id = Some(key(99)).try_into().unwrap();
    }
    mint.base = Mint {
        decimals: 6,
        is_initialized: true,
        supply: 1_000_000,
        ..Default::default()
    };
    mint.pack_base();
    mint.init_account_type().unwrap();
    data
}
pub(crate) fn extended_token(units: u64, transferring: bool) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{BaseStateWithExtensionsMut, StateWithExtensionsMut},
        state::{Account, AccountState},
    };
    let types = [
        ExtensionType::TransferFeeAmount,
        ExtensionType::TransferHookAccount,
    ];
    let mut data = vec![0; ExtensionType::try_calculate_account_len::<Account>(&types).unwrap()];
    let mut token = StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut data).unwrap();
    token
        .init_extension::<TransferFeeAmount>(false)
        .unwrap()
        .withheld_amount = 17u64.into();
    token
        .init_extension::<TransferHookAccount>(false)
        .unwrap()
        .transferring = transferring.into();
    token.base = Account {
        mint: key(9),
        owner: key(1),
        amount: units,
        state: AccountState::Initialized,
        ..Default::default()
    };
    token.pack_base();
    token.init_account_type().unwrap();
    data
}

#[test]
fn preparation_assets_transfer_fee_units_exclude_withheld_and_both_schedules_and_hooks_are_checked()
{
    for damage in 0..5 {
        let mut values = raw_accounts(true, 1000);
        values[0] = ui(
            spl_token_2022_interface::ID,
            &extended_mint(
                damage == 1,
                if damage == 2 { 10001 } else { 100 },
                if damage == 3 { 10001 } else { 200 },
            ),
        );
        values[2] = ui(
            spl_token_2022_interface::ID,
            &extended_token(1000, damage == 4),
        );
        let result = check(values);
        assert_eq!(result.is_ok(), damage == 0);
        if let Ok(balance) = result {
            assert_eq!(balance.units, 1000);
        }
    }
    let mut values = raw_accounts(true, 1000);
    values[0] = ui(
        spl_token_2022_interface::ID,
        &extended_mint(false, 100, 200),
    );
    assert!(
        check(values).is_err(),
        "required account extensions must be present"
    );
}

#[test]
fn preparation_assets_reject_bad_tlv_duplicate_or_unsupported_extensions() {
    let mint = extended_mint(false, 100, 200);
    let token = extended_token(1000, false);
    for (index, data) in [(0, mint), (2, token)] {
        for damage in 0..4 {
            let mut broken = data.clone();
            match damage {
                0 => {
                    broken.truncate(broken.len() - 1);
                }
                1 => {
                    broken[166..168].copy_from_slice(&u16::MAX.to_le_bytes());
                }
                2 => {
                    broken[168..170].copy_from_slice(&u16::MAX.to_le_bytes());
                }
                _ => {
                    let len = u16::from_le_bytes([broken[168], broken[169]]) as usize;
                    let first = broken[166..170 + len].to_vec();
                    broken.extend_from_slice(&first);
                }
            }
            let mut values = raw_accounts(true, 1000);
            values[0] = ui(
                spl_token_2022_interface::ID,
                &extended_mint(false, 100, 200),
            );
            values[2] = ui(spl_token_2022_interface::ID, &extended_token(1000, false));
            values[index] = ui(spl_token_2022_interface::ID, &broken);
            assert!(check(values).is_err(), "index={index} damage={damage}");
        }
    }
}

#[test]
fn preparation_assets_reject_native_or_residual_delegate_and_supply_inconsistency() {
    for token2022 in [false, true] {
        let program = if token2022 {
            spl_token_2022_interface::ID
        } else {
            spl_token_interface::ID
        };
        for damage in 0..3 {
            let mut values = raw_accounts(token2022, 1000);
            let mut data = token_bytes(1000);
            let mut state = Account::unpack(&data).unwrap();
            match damage {
                0 => state.is_native = Some(100u64).into(),
                1 => state.delegated_amount = 1,
                _ => {
                    let mut mint = Mint::unpack(&mint_bytes(6)).unwrap();
                    mint.supply = 999;
                    let mut data = mint_bytes(6);
                    Mint::pack(mint, &mut data).unwrap();
                    values[0] = ui(program, &data);
                }
            }
            Account::pack(state, &mut data).unwrap();
            values[if token2022 { 2 } else { 1 }] = ui(program, &data);
            assert!(check(values).is_err());
        }
    }
}

#[test]
fn preparation_assets_wsol_raw_units_require_classic_program_decimals_and_native_backing() {
    let mint = WSOL_MINT.parse::<Pubkey>().unwrap();
    for damage in 0..6 {
        let mut m = Mint::unpack(&mint_bytes(9)).unwrap();
        m.supply = 0;
        if damage == 1 {
            m.decimals = 6;
        }
        let mut data = mint_bytes(9);
        Mint::pack(m, &mut data).unwrap();
        let mut token = Account::unpack(&token_bytes(1000)).unwrap();
        token.mint = mint;
        token.is_native = Some(if damage == 5 { u64::MAX } else { 100 }).into();
        if damage == 2 {
            token.is_native = None.into();
        }
        let mut td = token_bytes(1000);
        Account::pack(token, &mut td).unwrap();
        let token2022 = damage == 3;
        let program = if token2022 {
            spl_token_2022_interface::ID
        } else {
            spl_token_interface::ID
        };
        let mut ta = ui(program, &td);
        ta["lamports"] = json!(if damage == 4 { 1099 } else { 1100 });
        let values = if token2022 {
            json!([ui(program, &data), null, ta])
        } else {
            json!([ui(program, &data), ta, null])
        };
        let values = serde_json::from_value::<Vec<Option<UiAccount>>>(values).unwrap();
        assert_eq!(
            inspect(a(1), Address::from_bytes(mint.to_bytes()), &values).is_ok(),
            damage == 0
        );
    }
}

#[test]
fn preparation_assets_reject_known_unaudited_nontransferable_extension() {
    use spl_token_2022_interface::{
        extension::{
            non_transferable::NonTransferable, BaseStateWithExtensionsMut, StateWithExtensionsMut,
        },
        state::Mint,
    };
    let mut data =
        vec![
            0;
            ExtensionType::try_calculate_account_len::<Mint>(&[ExtensionType::NonTransferable])
                .unwrap()
        ];
    let mut mint = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    mint.init_extension::<NonTransferable>(false).unwrap();
    mint.base = Mint {
        decimals: 6,
        is_initialized: true,
        supply: 10000,
        ..Default::default()
    };
    mint.pack_base();
    mint.init_account_type().unwrap();
    let mut values = raw_accounts(true, 1000);
    values[0] = ui(spl_token_2022_interface::ID, &data);
    assert!(check(values).is_err());
}
