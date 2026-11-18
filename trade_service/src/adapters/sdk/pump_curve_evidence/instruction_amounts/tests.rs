use super::*;
use sol_trade_sdk::{
    common::{bonding_curve::BondingCurveAccount, GasFeeStrategy},
    constants::{SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    instruction::{
        pumpfun::PumpFunInstructionBuilder,
        utils::pumpfun::{self, accounts, global_constants},
    },
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, PumpFunParams, SwapParams},
        traits::InstructionBuilder,
    },
};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Keypair};
use std::sync::Arc;

fn nz(amount: u64) -> NonZeroU64 {
    NonZeroU64::new(amount).unwrap()
}

pub(in crate::adapters::sdk::pump_curve_evidence) fn request(
    settlement: Pubkey,
    side: TradeType,
    fixed: Option<u64>,
    cashback: bool,
) -> SwapParams {
    let mint = Pubkey::new_from_array([101; 32]);
    let creator = Pubkey::new_from_array([102; 32]);
    let native = settlement == SOL_TOKEN_ACCOUNT || settlement == WSOL_TOKEN_ACCOUNT;
    let curve = BondingCurveAccount {
        account: pumpfun::get_bonding_curve_pda(&mint).unwrap(),
        virtual_token_reserves: global_constants::INITIAL_VIRTUAL_TOKEN_RESERVES,
        virtual_sol_reserves: global_constants::INITIAL_VIRTUAL_SOL_RESERVES,
        real_token_reserves: global_constants::INITIAL_REAL_TOKEN_RESERVES,
        creator,
        is_cashback_coin: cashback,
        quote_mint: if native {
            WSOL_TOKEN_ACCOUNT
        } else {
            settlement
        },
        ..Default::default()
    };
    let buy = matches!(side, TradeType::Buy);
    SwapParams {
        rpc: None,
        // Same deterministic, unfunded test identity as the established DAMM oracle.
        // This invokes only InstructionBuilder, never SDK execution or signing.
        payer: Arc::new(Keypair::new_from_array([11; 32])),
        trade_type: side,
        input_mint: if buy { settlement } else { mint },
        input_token_program: None,
        output_mint: if buy { mint } else { settlement },
        output_token_program: None,
        input_amount: Some(1_000_000),
        slippage_basis_points: Some(1500),
        address_lookup_table_accounts: vec![],
        recent_blockhash: None,
        wait_tx_confirmed: false,
        protocol_params: DexParamEnum::PumpFun(PumpFunParams {
            bonding_curve: Arc::new(curve),
            associated_bonding_curve: Pubkey::default(),
            observed_trade_creator: None,
            creator_vault: pumpfun::get_creator_vault_pda(&creator).unwrap(),
            fee_sharing_creator_vault_if_active: None,
            token_program: spl_token_interface::ID,
            close_token_account_when_sell: Some(false),
            fee_recipient: global_constants::FEE_RECIPIENT,
            quote_mint: if native {
                WSOL_TOKEN_ACCOUNT
            } else {
                settlement
            },
        }),
        open_seed_optimize: false,
        swqos_clients: Arc::new(vec![]),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: false,
        create_input_mint_ata: false,
        close_input_mint_ata: false,
        create_output_mint_ata: false,
        close_output_mint_ata: false,
        fixed_output_amount: fixed,
        gas_fee_strategy: GasFeeStrategy::new(),
        simulate: false,
        log_enabled: false,
        wait_for_all_submits: false,
        use_dedicated_sender_threads: false,
        sender_thread_cores: None,
        max_sender_concurrency: 1,
        effective_core_ids: Arc::new(vec![]),
        check_min_tip: false,
        transaction_version: Default::default(),
        grpc_recv_us: None,
        use_exact_sol_amount: Some(true),
    }
}

fn only_swap(instructions: Vec<Instruction>) -> Instruction {
    assert_eq!(instructions.len(), 1);
    let ix = instructions.into_iter().next().unwrap();
    assert_eq!(ix.program_id, accounts::PUMPFUN);
    ix
}

#[tokio::test]
async fn actual_sdk_buy_distinguishes_exact_input_minimum_from_fixed_output() {
    for settlement in [SOL_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
        for cashback in [false, true] {
            let layout = if settlement == SOL_TOKEN_ACCOUNT {
                WireLayout::NativeV1
            } else {
                WireLayout::QuoteV2
            };
            for fixed in [None, Some(777)] {
                let params = request(settlement, TradeType::Buy, fixed, cashback);
                let ix = only_swap(
                    PumpFunInstructionBuilder
                        .build_buy_instructions(&params)
                        .await
                        .unwrap(),
                );
                let decoded = PumpWireAmounts::decode(&ix.data).unwrap();
                assert_eq!(decoded.layout, layout);
                assert_eq!(
                    decoded.track_volume,
                    (layout == WireLayout::NativeV1).then_some(cashback)
                );
                if fixed.is_some() {
                    assert_eq!(
                        decoded.amounts,
                        TradeAmounts::BuyExactOutput {
                            output: nz(777),
                            maximum_spend: nz(1_000_000)
                        }
                    );
                    assert!(decoded
                        .require_exact_input_buy(nz(1_000_000), nz(777))
                        .is_err());
                } else {
                    let TradeAmounts::BuyExactInput { spend, minimum } = decoded.amounts else {
                        panic!("SDK chose a different buy type")
                    };
                    assert_eq!(spend, nz(1_000_000));
                    decoded.require_exact_input_buy(spend, minimum).unwrap();
                    assert!(decoded
                        .require_exact_input_buy(spend, nz(minimum.get() + 1))
                        .is_err());
                    assert!(decoded
                        .require_exact_input_buy(nz(spend.get() + 1), minimum)
                        .is_err());
                }
            }
        }
    }
}

#[tokio::test]
async fn actual_sdk_sell_preserves_token_input_and_quote_minimum() {
    for settlement in [SOL_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
        for minimum in [1, 777, u64::MAX] {
            let params = request(settlement, TradeType::Sell, Some(minimum), true);
            let ix = only_swap(
                PumpFunInstructionBuilder
                    .build_sell_instructions(&params)
                    .await
                    .unwrap(),
            );
            let decoded = PumpWireAmounts::decode(&ix.data).unwrap();
            assert_eq!(
                decoded.layout,
                if settlement == SOL_TOKEN_ACCOUNT {
                    WireLayout::NativeV1
                } else {
                    WireLayout::QuoteV2
                }
            );
            assert_eq!(decoded.track_volume, None);
            assert_eq!(
                decoded.amounts,
                TradeAmounts::SellExactInput {
                    input: nz(1_000_000),
                    minimum: nz(minimum)
                }
            );
            assert!(decoded
                .require_exact_input_buy(nz(1_000_000), nz(minimum))
                .is_err());
        }
    }
}

#[tokio::test]
async fn actual_sdk_wire_rejects_unknown_truncated_zero_and_appended_fill_data() {
    for settlement in [SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
        let params = request(settlement, TradeType::Buy, None, false);
        let ix = only_swap(
            PumpFunInstructionBuilder
                .build_buy_instructions(&params)
                .await
                .unwrap(),
        );
        for change in 0..6 {
            let mut data = ix.data.clone();
            match change {
                0 => {
                    data.pop();
                }
                1 => data.push(0), // A future/IDL partial_fill byte is not inferred.
                2 => data[0] ^= 1,
                3 => data[8..16].fill(0),
                4 => data[16..24].fill(0),
                _ => data.truncate(7),
            }
            assert!(PumpWireAmounts::decode(&data).is_err(), "mutation {change}");
        }
        if settlement == SOL_TOKEN_ACCOUNT {
            let mut invalid = ix.data;
            invalid[24] = 2;
            assert!(PumpWireAmounts::decode(&invalid).is_err());
        }
    }
}
