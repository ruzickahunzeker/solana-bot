use super::{super::*, read_rpc::*};
use sol_trade_sdk::{
    common::bonding_curve::BondingCurveAccount,
    instruction::utils::pumpfun::{self, accounts, global_constants},
};

pub(super) fn fixture(complete: bool, sharing: bool) -> (ReadRpc, Pubkey, Pubkey, Pubkey) {
    let mint = Pubkey::new_from_array([101; 32]);
    let creator = Pubkey::new_from_array([102; 32]);
    let fee_recipient = Pubkey::new_from_array([103; 32]);
    let quote = Pubkey::from_str(WSOL_MINT).unwrap();
    let mut body = bincode::serialize(&(
        1_000_001u64,
        2_000_003u64,
        3_000u64,
        4_000u64,
        9_000_000u64,
        complete,
        creator,
        false,
        true,
        quote,
    ))
    .unwrap();
    assert_eq!(body.len(), 107);
    body.extend(
        bincode::serialize(&(37u64, true, true, 41u64, 43u64, 2u8, 47u64, 53u64, 59u64)).unwrap(),
    );
    assert_eq!(body.len(), 158);
    let decoded = BondingCurveAccount::decode_body(&body).unwrap();
    assert_eq!(decoded.creator, creator);
    assert_eq!(decoded.creator_fee_bps, 37);
    assert_eq!(decoded.post_complete_quote_in, 59);
    let mut data = vec![23, 183, 248, 55, 96, 216, 172, 96]; // Current Pump IDL BondingCurve discriminator.
    data.extend(body);
    let curve = pumpfun::get_bonding_curve_pda(&mint).unwrap();
    let mut fixture = ReadRpc::default();
    fixture.account(curve, account(accounts::PUMPFUN, data));
    fixture.account(
        mint,
        account(spl_token_2022_interface::ID, super::cpmm::mint(6)),
    );
    let mut global = pumpfun::GLOBAL_ACCOUNT_DISCRIMINATOR.to_vec();
    global.extend(
        bincode::serialize(&(true, Pubkey::new_from_array([104; 32]), fee_recipient)).unwrap(),
    );
    assert_eq!(global.len(), 73);
    fixture.account(
        global_constants::GLOBAL_ACCOUNT,
        account(accounts::PUMPFUN, global),
    );
    let config = pumpfun::get_fee_sharing_config_pda(&mint).unwrap();
    if sharing {
        let mut data = pumpfun::SHARING_CONFIG_ACCOUNT_DISCRIMINATOR.to_vec();
        data.extend([1, 0, 1]); // version, bump, Active status in current SDK reader.
        data.extend(mint.as_ref());
        assert_eq!(data.len(), 43);
        fixture.account(config, account(accounts::FEE_PROGRAM, data));
    } else {
        fixture.absent(config);
    }
    (fixture, mint, creator, fee_recipient)
}

#[tokio::test]
async fn pump_real_loader_retains_curve_extension_global_fee_and_creator_vault_selection() {
    for sharing in [false, true] {
        let (fixture, mint, creator, fee_recipient) = fixture(false, sharing);
        let rpc = fixture.client();
        let quote = Pubkey::from_str(WSOL_MINT).unwrap();
        warmup_on_rpc(&rpc, Pool::Pump, &mint, &quote, None)
            .await
            .unwrap();
        let params = dex_params(&rpc, Pool::Pump, &mint, &quote, None, None, None)
            .await
            .unwrap();
        let DexParamEnum::PumpFun(params) = params else {
            panic!("wrong Pump variant")
        };
        let curve = &params.bonding_curve;
        assert_eq!(
            curve.account,
            pumpfun::get_bonding_curve_pda(&mint).unwrap()
        );
        assert_eq!(
            (
                curve.virtual_token_reserves,
                curve.virtual_sol_reserves,
                curve.real_token_reserves,
                curve.real_sol_reserves
            ),
            (1_000_001, 2_000_003, 3_000, 4_000)
        );
        assert_eq!(curve.token_total_supply, 9_000_000);
        assert_eq!(curve.creator, creator);
        assert!(!curve.complete);
        assert!(!curve.is_mayhem_mode);
        assert!(curve.is_cashback_coin);
        assert_eq!(curve.quote_mint, quote);
        assert_eq!(curve.creator_fee_bps, 37);
        assert!(curve.can_edit_creator_fee);
        assert!(curve.is_holder_reward);
        assert_eq!(
            (
                curve.creator_fee,
                curve.protocol_fees,
                curve.depth,
                curve.initial_virtual_quote_reserves,
                curve.post_complete_base_out,
                curve.post_complete_quote_in
            ),
            (41, 43, 2, 47, 53, 59)
        );
        let authority = if sharing {
            pumpfun::get_fee_sharing_config_pda(&mint).unwrap()
        } else {
            creator
        };
        let vault = pumpfun::get_creator_vault_pda(&authority).unwrap();
        assert_eq!(params.creator_vault, vault);
        assert_eq!(
            params.fee_sharing_creator_vault_if_active,
            sharing.then_some(vault)
        );
        assert_eq!(params.token_program, spl_token_2022_interface::ID);
        assert_eq!(params.associated_bonding_curve, sol_trade_sdk::common::spl_associated_token_account::get_associated_token_address_with_program_id(&curve.account, &mint, &spl_token_2022_interface::ID));
        assert_eq!(params.fee_recipient, fee_recipient);
        assert_eq!(
            params.quote_mint,
            sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT
        );
        assert!(params.observed_trade_creator.is_none());
        assert!(params.close_token_account_when_sell.is_none());
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 8);
    }
}

#[tokio::test]
async fn pump_complete_curve_is_rejected_only_by_existing_warmup_policy() {
    let (fixture, mint, _, _) = fixture(true, false);
    let rpc = fixture.client();
    let quote = Pubkey::from_str(WSOL_MINT).unwrap();
    assert_eq!(
        warmup_on_rpc(&rpc, Pool::Pump, &mint, &quote, None)
            .await
            .unwrap_err()
            .to_string(),
        "pump bonding curve is complete"
    );
    let DexParamEnum::PumpFun(params) =
        dex_params(&rpc, Pool::Pump, &mint, &quote, None, None, None)
            .await
            .unwrap()
    else {
        panic!("wrong Pump variant")
    };
    assert!(params.bonding_curve.complete);
    fixture.assert_read_only();
    assert_eq!(fixture.calls().len(), 8);
}

#[tokio::test]
async fn pump_invalid_global_account_is_not_replaced_with_static_fee_recipient() {
    for damage in 0..3 {
        let (mut fixture, mint, _, _) = fixture(false, false);
        let mut data = pumpfun::GLOBAL_ACCOUNT_DISCRIMINATOR.to_vec();
        data.resize(73, 0);
        let owner = if damage == 0 {
            Pubkey::default()
        } else {
            accounts::PUMPFUN
        };
        if damage == 1 {
            data[0] ^= 1
        } else if damage == 2 {
            data.pop();
        }
        fixture.account(global_constants::GLOBAL_ACCOUNT, account(owner, data));
        let error = dex_params(
            &fixture.client(),
            Pool::Pump,
            &mint,
            &Pubkey::from_str(WSOL_MINT).unwrap(),
            None,
            None,
            None,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert_eq!(
            error,
            match damage {
                0 => format!(
                    "PumpFun Global account has unexpected owner {}",
                    Pubkey::default()
                ),
                1 => "Invalid PumpFun Global account discriminator".into(),
                _ =>
                    "PumpFun Global account is too short: expected at least 73 bytes, got 72".into(),
            }
        );
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 4);
    }
}
