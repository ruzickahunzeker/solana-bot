use super::*;
use crate::adapters::sdk::pump_curve_evidence::tests::{snapshot, sol, ui, BASE};
use solana_program_pack::Pack;
use spl_token_interface::state::{Account, AccountState};

const WALLET: Pubkey = Pubkey::new_from_array([33; 32]);
const SLOT: u64 = 175;

fn token(mint: Pubkey, owner: Pubkey, amount: u64) -> Account {
    Account {
        mint,
        owner,
        amount,
        state: AccountState::Initialized,
        ..Account::default()
    }
}

fn plain_token(program: Pubkey, state: Account) -> UiAccount {
    let mut bytes = [0; Account::LEN];
    Account::pack(state, &mut bytes).unwrap();
    ui(program, &bytes)
}

fn clock(slot: u64, epoch: u64) -> Option<UiAccount> {
    Some(ui(
        sysvar::id(),
        &bincode::serialize(&Clock {
            slot,
            epoch,
            ..Clock::default()
        })
        .unwrap(),
    ))
}

pub(super) fn fixture(
    side: Side,
    program: Pubkey,
) -> (NativeFundingPlan, [Option<UiAccount>; NATIVE_ACCOUNTS]) {
    let plan = NativeFundingPlan::new(sol(), WALLET, program, side).unwrap();
    let [curve, mint, global, sharing] = snapshot(None, program);
    let mut wallet = ui(solana_system_interface::program::ID, &[]);
    wallet.lamports = 20_000_000;
    let values = [
        curve,
        mint,
        global,
        sharing,
        Some(plain_token(program, token(BASE, plan.keys()[0], 3001))),
        Some(plain_token(program, token(BASE, WALLET, 21))),
        Some(wallet),
        clock(SLOT, 19),
    ];
    (plan, values)
}

pub(super) fn decode(
    plan: &NativeFundingPlan,
    values: &[Option<UiAccount>],
) -> Result<NativeFundingSnapshot> {
    plan.decode(plan.keys(), SLOT, SLOT - 1, values)
}

#[test]
fn native_funding_binds_clock_payer_curve_and_canonical_token_roles() {
    for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
        for side in [Side::Buy, Side::Sell] {
            let (plan, values) = fixture(side, program);
            let derived = |owner: &Pubkey| {
                sol_trade_sdk::common::spl_associated_token_account::get_associated_token_address_with_program_id(owner, &BASE, &program)
            };
            assert_eq!(
                plan.keys()[4..],
                [
                    derived(&plan.keys()[0]),
                    derived(&WALLET),
                    WALLET,
                    sysvar::clock::id()
                ]
            );
            let actual = decode(&plan, &values).unwrap();
            assert_eq!(
                (actual.curve.slot, actual.curve.quote, actual.epoch),
                (SLOT, WSOL_TOKEN_ACCOUNT, 19)
            );
            assert_eq!(
                (
                    actual.native_lamports,
                    actual.curve_base_units,
                    actual.user_base_units
                ),
                (20_000_000, 3001, Some(21))
            );
        }
    }
}

#[test]
fn native_funding_rejects_stale_or_wrong_slots_keys_counts_and_owners() {
    let program = spl_token_interface::ID;
    let (plan, values) = fixture(Side::Sell, program);
    let mut wrong_keys = *plan.keys();
    wrong_keys.swap(4, 5);
    assert!(plan.decode(&wrong_keys, SLOT, SLOT - 1, &values).is_err());
    assert!(plan.decode(plan.keys(), SLOT, SLOT + 1, &values).is_err());
    assert!(plan.decode(plan.keys(), SLOT, 0, &values).is_err());
    assert!(decode(&plan, &values[..7]).is_err());
    let mut wrong = values.clone();
    wrong[7] = clock(SLOT - 1, 19);
    assert!(decode(&plan, &wrong)
        .unwrap_err()
        .to_string()
        .contains("Clock slot"));
    wrong[7] = clock(SLOT, 19);
    wrong[6] = Some(ui(program, &[]));
    assert!(decode(&plan, &wrong)
        .unwrap_err()
        .to_string()
        .contains("wallet"));
    wrong = values;
    wrong[1].as_mut().unwrap().owner = spl_token_2022_interface::ID.to_string();
    assert!(decode(&plan, &wrong)
        .unwrap_err()
        .to_string()
        .contains("mint program"));
    assert!(NativeFundingPlan::new(sol(), plan.keys()[0], program, Side::Buy).is_err());
}

#[test]
fn native_funding_distinguishes_absent_buy_ata_from_bad_or_missing_sell_assets() {
    let program = spl_token_interface::ID;
    let (buy, mut values) = fixture(Side::Buy, program);
    let (sell, _) = fixture(Side::Sell, program);
    values[5] = None;
    assert_eq!(decode(&buy, &values).unwrap().user_base_units, None);
    assert!(decode(&sell, &values)
        .unwrap_err()
        .to_string()
        .contains("sell input ATA"));
    values[5] = Some(plain_token(program, token(BASE, buy.keys()[0], 21)));
    assert!(decode(&buy, &values)
        .unwrap_err()
        .to_string()
        .contains("user base ATA"));
    values[5] = Some(plain_token(program, token(BASE, WALLET, 0)));
    assert_eq!(decode(&sell, &values).unwrap().user_base_units, Some(0));
    values[4] = Some(plain_token(program, token(BASE, buy.keys()[0], 2999)));
    assert!(decode(&sell, &values)
        .unwrap_err()
        .to_string()
        .contains("real reserves"));
}

#[test]
fn native_funding_checks_epoch_fee_and_mandatory_vault_extensions() {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::{TransferFee, TransferFeeAmount, TransferFeeConfig},
            BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
        },
        state::{Account as ExtendedAccount, AccountState, Mint},
    };
    let program = spl_token_2022_interface::ID;
    let (plan, mut values) = fixture(Side::Sell, program);
    let mut mint_data =
        vec![
            0;
            ExtensionType::try_calculate_account_len::<Mint>(&[ExtensionType::TransferFeeConfig])
                .unwrap()
        ];
    let mut mint = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut mint_data).unwrap();
    let fee = mint.init_extension::<TransferFeeConfig>(false).unwrap();
    fee.older_transfer_fee = TransferFee {
        epoch: 10u64.into(),
        maximum_fee: 3u64.into(),
        transfer_fee_basis_points: 10u16.into(),
    };
    fee.newer_transfer_fee = TransferFee {
        epoch: 20u64.into(),
        maximum_fee: 5u64.into(),
        transfer_fee_basis_points: 10_001u16.into(),
    };
    mint.base = Mint {
        decimals: 6,
        is_initialized: true,
        supply: 9_000_000,
        ..Mint::default()
    };
    mint.pack_base();
    mint.init_account_type().unwrap();
    values[1] = Some(ui(program, &mint_data));
    assert!(decode(&plan, &values)
        .unwrap_err()
        .to_string()
        .contains("missing required mint extensions"));
    for (index, owner, amount) in [(4, plan.keys()[0], 3001), (5, WALLET, 21)] {
        let mut data = vec![
            0;
            ExtensionType::try_calculate_account_len::<ExtendedAccount>(&[
                ExtensionType::TransferFeeAmount
            ])
            .unwrap()
        ];
        let mut token =
            StateWithExtensionsMut::<ExtendedAccount>::unpack_uninitialized(&mut data).unwrap();
        token.init_extension::<TransferFeeAmount>(false).unwrap();
        token.base = ExtendedAccount {
            mint: BASE,
            owner,
            amount,
            state: AccountState::Initialized,
            ..ExtendedAccount::default()
        };
        token.pack_base();
        token.init_account_type().unwrap();
        values[index] = Some(ui(program, &data));
    }
    assert_eq!(decode(&plan, &values).unwrap().epoch, 19);
    values[7] = clock(SLOT, 20);
    assert!(decode(&plan, &values)
        .unwrap_err()
        .to_string()
        .contains("transfer fee basis points"));
}

#[tokio::test]
async fn native_funding_reads_all_eight_accounts_once_from_the_selected_bank() {
    use crate::adapters::test_support::read_rpc_server::{LocalServer, Reply};
    use serde_json::json;
    use std::time::Duration;

    let (plan, values) = fixture(Side::Sell, spl_token_interface::ID);
    for (bank, valid) in [(SLOT, true), (SLOT + 1, false), (SLOT - 2, false)] {
        let reply = Reply::FixedAccountSnapshot {
            count: NATIVE_ACCOUNTS,
            result: json!({"context":{"slot":bank},"value":&values}),
            mode: Box::new(Reply::Success),
        };
        let server = LocalServer::new(reply).await;
        let spare = LocalServer::new(Reply::Success).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone(), spare.url.clone()]).unwrap();
        let result = plan
            .read(
                &runtime,
                &server.url,
                SLOT - 1,
                Instant::now() + Duration::from_secs(3),
            )
            .await;
        if valid {
            let snapshot = result.unwrap();
            assert_eq!(
                (
                    snapshot.curve.slot,
                    snapshot.epoch,
                    snapshot.user_base_units
                ),
                (SLOT, 19, Some(21))
            );
        } else {
            let message = result.unwrap_err().to_string();
            assert!(message.contains(if bank > SLOT {
                "Clock slot"
            } else {
                "bank context"
            }));
        }
        let keys = plan.keys().map(|key| key.to_string());
        assert_eq!(
            *server.requests.lock().unwrap(),
            [json!({
                "jsonrpc":"2.0","id":1,"method":"getMultipleAccounts","params":[keys,
                    {"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":SLOT-1}]
            })]
        );
        assert!(spare.requests.lock().unwrap().is_empty());
        assert!(runtime.parameter_slot().is_ok());
        server.finish().await;
        spare.finish().await;
    }
}
