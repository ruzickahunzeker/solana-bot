use super::*;
use crate::adapters::sdk::pump_curve_evidence::{
    instruction_amounts::tests::request,
    tests::{snapshot, ui},
};
use sol_trade_sdk::{
    constants::SOL_TOKEN_ACCOUNT,
    instruction::pumpfun::PumpFunInstructionBuilder,
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, SwapParams},
        traits::InstructionBuilder,
    },
};
use solana_program_pack::Pack;
use solana_sdk::signature::Signer;
use spl_token_interface::state::{Account, AccountState};

fn funding(params: &SwapParams) -> NativeFundingSnapshot {
    let buy = matches!(params.trade_type, TradeType::Buy);
    let base = if buy {
        params.output_mint
    } else {
        params.input_mint
    };
    let DexParamEnum::PumpFun(protocol) = &params.protocol_params else {
        panic!("wrong fixture")
    };
    let wallet = params.payer.pubkey();
    let curve = PumpCurveAccounts::new(base, None, WSOL_TOKEN_ACCOUNT).unwrap();
    let plan = NativeFundingPlan::new(
        curve,
        wallet,
        protocol.token_program,
        if buy { Side::Buy } else { Side::Sell },
    )
    .unwrap();
    let [curve, mint, global, sharing] = snapshot(None, protocol.token_program);
    let token = |owner| {
        let mut bytes = [0; Account::LEN];
        Account::pack(
            Account {
                mint: base,
                owner,
                amount: 3_001,
                state: AccountState::Initialized,
                ..Account::default()
            },
            &mut bytes,
        )
        .unwrap();
        Some(ui(protocol.token_program, &bytes))
    };
    let values = [
        curve,
        mint,
        global,
        sharing,
        token(plan.keys()[0]),
        token(wallet),
        Some(ui(solana_system_interface::program::ID, &[])),
        Some(ui(
            sysvar::id(),
            &bincode::serialize(&Clock {
                slot: 175,
                epoch: 19,
                ..Clock::default()
            })
            .unwrap(),
        )),
    ];
    plan.decode(plan.keys(), 175, 174, &values).unwrap()
}

async fn build(params: &SwapParams) -> Instruction {
    let instructions = if matches!(params.trade_type, TradeType::Buy) {
        PumpFunInstructionBuilder
            .build_buy_instructions(params)
            .await
            .unwrap()
    } else {
        PumpFunInstructionBuilder
            .build_sell_instructions(params)
            .await
            .unwrap()
    };
    assert_eq!(instructions.len(), 1);
    instructions.into_iter().next().unwrap()
}

#[tokio::test]
async fn native_roles_bind_actual_sdk_swap_to_the_bank_wallet_curve_and_atas() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let params = request(SOL_TOKEN_ACCOUNT, side, None, true);
        let bank = funding(&params);
        let instruction = build(&params).await;
        bank.inspect_native_roles(&instruction).unwrap();
        for mutation in 0..12 {
            let mut changed = instruction.clone();
            match mutation {
                0 => changed.accounts[6].pubkey = Pubkey::new_from_array([77; 32]),
                1 => changed.accounts[2].is_writable = true,
                2 => changed.accounts[3].is_writable = false,
                3 => changed.accounts.swap(4, 5),
                4 => changed.accounts[1].is_signer = true,
                5 => changed.accounts[1].pubkey = changed.accounts[6].pubkey,
                6 => {
                    changed.accounts.pop();
                }
                7 => changed.program_id = solana_system_interface::program::ID,
                8 => changed.accounts[7].is_writable = true,
                9 => changed.accounts[10].pubkey = Pubkey::new_from_array([78; 32]),
                10 => {
                    let last_curve = changed.accounts.len() - 2;
                    changed.accounts[last_curve].is_writable = true;
                }
                _ => {
                    if matches!(params.trade_type, TradeType::Buy) {
                        changed.data[24] ^= 1;
                    } else {
                        changed.accounts[14].pubkey = Pubkey::new_from_array([79; 32]);
                    }
                }
            }
            assert!(
                bank.inspect_native_roles(&changed).is_err(),
                "mutation {mutation}"
            );
        }
    }
}

#[tokio::test]
async fn native_roles_reject_sdk_suffix_program_override_and_exact_output_buy() {
    let mut params = request(SOL_TOKEN_ACCOUNT, TradeType::Buy, None, true);
    let mint = "E3JvmGcGFDzhu2Cnxyeq5BRvN7HH9JZUsfAUh2v8pump"
        .parse::<Pubkey>()
        .unwrap();
    params.output_mint = mint;
    if let DexParamEnum::PumpFun(protocol) = &mut params.protocol_params {
        let mut curve = (*protocol.bonding_curve).clone();
        curve.account =
            sol_trade_sdk::instruction::utils::pumpfun::get_bonding_curve_pda(&mint).unwrap();
        protocol.bonding_curve = std::sync::Arc::new(curve);
    }
    let bank = funding(&params); // SPL owner in the bank must not be replaced by suffix policy.
    let instruction = build(&params).await;
    assert_eq!(instruction.accounts[8].pubkey, spl_token_2022_interface::ID);
    assert!(bank.inspect_native_roles(&instruction).is_err());

    let fixed = request(SOL_TOKEN_ACCOUNT, TradeType::Buy, Some(777), true);
    let bank = funding(&fixed);
    assert!(bank
        .inspect_native_roles(&build(&fixed).await)
        .unwrap_err()
        .to_string()
        .contains("amount mode"));
}
