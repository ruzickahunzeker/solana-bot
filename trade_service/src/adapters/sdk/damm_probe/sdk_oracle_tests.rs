use super::*;
use sha2::{Digest, Sha256};
use sol_trade_sdk::{
    common::GasFeeStrategy,
    instruction::meteora_damm_v2::MeteoraDammV2InstructionBuilder,
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, MeteoraDammV2Params, SwapParams},
        traits::InstructionBuilder,
    },
};
use solana_sdk::signature::{Keypair, Signer};
use std::sync::Arc;

pub(super) struct OracleCase {
    pub wallet: Pubkey,
    pub damm: MeteoraDammV2Params,
    pub mint: Pubkey,
    pub quote: Pubkey,
    pub amount: u64,
    pub blockhash: Hash,
    pub instructions: Vec<Instruction>,
    request: SwapParams,
}

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

/// Test-only deterministic, unfunded identity. The SDK builder reads its pubkey; no signing.
pub(super) async fn oracle_cases() -> Vec<OracleCase> {
    let payer = Arc::new(Keypair::new_from_array([11; 32]));
    let mut cases = vec![];
    for mode in [0, 1, 2] {
        for reversed in [false, true] {
            for quote in [
                sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT,
                sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT,
            ] {
                for token_a_program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                    for token_b_program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                        for referral in [None, Some(key(9))] {
                            for sysvar in [false, true] {
                                for amount in [1, 1_000_000, u64::MAX] {
                                    let (a, b) = if reversed {
                                        (quote, key(1))
                                    } else {
                                        (key(1), quote)
                                    };
                                    let mut damm = MeteoraDammV2Params::new(
                                        key(2),
                                        key(3),
                                        key(4),
                                        a,
                                        b,
                                        token_a_program,
                                        token_b_program,
                                    )
                                    .with_swap_mode(mode)
                                    .with_rate_limiter_sysvar(sysvar);
                                    damm.referral_token_account = referral;
                                    let blockhash = Hash::new_from_array([10; 32]);
                                    let request = SwapParams {
                                        rpc: None,
                                        payer: payer.clone(),
                                        trade_type: TradeType::Sell,
                                        input_mint: key(1),
                                        input_token_program: None,
                                        output_mint: quote,
                                        output_token_program: Some(if reversed {
                                            token_a_program
                                        } else {
                                            token_b_program
                                        }),
                                        input_amount: Some(amount),
                                        slippage_basis_points: None,
                                        address_lookup_table_accounts: vec![],
                                        recent_blockhash: Some(blockhash),
                                        wait_tx_confirmed: false,
                                        protocol_params: DexParamEnum::MeteoraDammV2(damm.clone()),
                                        open_seed_optimize: false,
                                        swqos_clients: Arc::new(vec![]),
                                        middleware_manager: None,
                                        durable_nonce: None,
                                        with_tip: false,
                                        create_input_mint_ata: false,
                                        close_input_mint_ata: false,
                                        create_output_mint_ata: true,
                                        close_output_mint_ata: false,
                                        fixed_output_amount: Some(1),
                                        gas_fee_strategy: GasFeeStrategy::new(),
                                        simulate: true,
                                        log_enabled: false,
                                        wait_for_all_submits: false,
                                        use_dedicated_sender_threads: false,
                                        sender_thread_cores: None,
                                        max_sender_concurrency: 1,
                                        effective_core_ids: Arc::new(vec![]),
                                        check_min_tip: false,
                                        transaction_version: Default::default(),
                                        grpc_recv_us: None,
                                        use_exact_sol_amount: None,
                                    };
                                    let instructions = MeteoraDammV2InstructionBuilder
                                        .build_sell_instructions(&request)
                                        .await
                                        .unwrap();
                                    cases.push(OracleCase {
                                        wallet: payer.pubkey(),
                                        damm,
                                        mint: key(1),
                                        quote,
                                        amount,
                                        blockhash,
                                        instructions,
                                        request,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    cases
}

#[tokio::test]
async fn damm_sell_message_final_profile_matches_actual_sdk_instruction_builder() {
    use crate::{
        adapters::sdk::{
            damm_sell_message::{DammSellContext, DammSellMessageProfile},
            ordinary_lanes::SdkLaneReport,
            ordinary_params::ordinary_sell_params,
            trade_command::ExecuteTrade,
        },
        domain::{
            preparation_balance::{PreparationBalanceSnapshot, PreparationInput},
            trade::{Address, Pool},
        },
    };
    use sol_trade_sdk::{
        common::{
            fast_fn::get_associated_token_address_with_program_id_fast, TradeTransactionVersion,
        },
        swqos::SwqosType,
    };

    let mut count = 0;
    for case in oracle_cases().await {
        let a_in = case.mint == case.damm.token_a_mint;
        let (input_program, output_program) = if a_in {
            (case.damm.token_a_program, case.damm.token_b_program)
        } else {
            (case.damm.token_b_program, case.damm.token_a_program)
        };
        if case.damm.swap_mode != 0
            || case.amount == u64::MAX
            || output_program != spl_token_interface::ID
        {
            continue;
        }
        for percent in [50, 100] {
            for minimum in [1, 777, u64::MAX] {
                let balance = case.amount * (100 / percent);
                let intent = ExecuteTrade {
                    pool: Pool::MeteoraDammV2,
                    mint: case.mint,
                    quote_mint: case.quote,
                    pool_id: Some(case.damm.pool),
                    sol_hop_pool: None,
                    sol_hop_pool_id: None,
                    amount_lamports: None,
                    sell_percent: Some(percent),
                    slippage_bps: 1500,
                    tip_sol: 0.001,
                    recent_blockhash: case.blockhash,
                    wait_tx_confirmed: false,
                    mev_submit_verify_ms: 250,
                    simulate: false,
                    use_mev: Some(true),
                    mev_provider: None,
                };
                let request = ordinary_sell_params(
                    &intent,
                    DexParamEnum::MeteoraDammV2(case.damm.clone()),
                    Some(minimum),
                    percent,
                    SwqosType::Temporal,
                )
                .unwrap();
                let holding = Arc::new(
                    PreparationBalanceSnapshot::from_account(
                        Address::from_bytes(case.wallet.to_bytes()),
                        Address::from_bytes(case.mint.to_bytes()),
                        balance,
                        PreparationInput {
                            account: Address::from_bytes(
                                get_associated_token_address_with_program_id_fast(
                                    &case.wallet,
                                    &case.mint,
                                    &input_program,
                                )
                                .to_bytes(),
                            ),
                            program: Address::from_bytes(input_program.to_bytes()),
                            decimals: 6,
                        },
                        123,
                        std::time::Instant::now(),
                    )
                    .unwrap(),
                );
                let quote = super::DammOutputEvidence::fixture_for_profile(
                    case.wallet,
                    &intent,
                    &request,
                    balance,
                    percent,
                    &holding,
                );
                let profile = DammSellMessageProfile::new(
                    DammSellContext {
                        wallet: case.wallet,
                        provider: SwqosType::Temporal,
                        version: TradeTransactionVersion::V0,
                        seed_optimized: false,
                    },
                    &intent,
                    &request,
                    balance,
                    percent,
                    holding,
                    quote,
                )
                .unwrap();
                // Reuse the existing unfunded oracle payer only for SDK instruction assembly.
                let mut sdk = case.request.clone();
                sdk.fixed_output_amount = Some(minimum);
                sdk.close_input_mint_ata = percent == 100;
                let instructions = MeteoraDammV2InstructionBuilder
                    .build_sell_instructions(&sdk)
                    .await
                    .unwrap();
                let returned = profile
                    .invoke(|manager| async move {
                        let checked = manager
                            .apply_middlewares_process_protocol_instructions(
                                instructions.clone(),
                                "MeteoraDammV2",
                                false,
                            )
                            .unwrap();
                        assert_eq!(checked, instructions);
                        let mut full = vec![solana_system_interface::instruction::transfer(
                            &case.wallet,
                            &sol_trade_sdk::constants::swqos::NOZOMI_TIP_ACCOUNTS[0],
                            1_000_000,
                        )];
                        sol_trade_sdk::trading::common::extend_compute_budget_instructions(
                            &mut full, 1000, 300_000,
                        );
                        full.extend(instructions);
                        assert_eq!(
                            manager
                                .apply_middlewares_process_full_instructions(
                                    full.clone(),
                                    "MeteoraDammV2",
                                    false
                                )
                                .unwrap(),
                            full
                        );
                        Ok(SdkLaneReport {
                            success: false,
                            signatures: vec![],
                            error: Some("instruction-only oracle; no signing or submission".into()),
                        })
                    })
                    .await
                    .unwrap_or_else(|failure| panic!("{}", failure.message));
                assert!(!returned.success);
                count += 1;
            }
        }
    }
    assert_eq!(count, 384);
}

#[tokio::test]
async fn existing_sdk_damm_probe_message_fingerprint() {
    let cases = oracle_cases().await;
    assert_eq!(cases.len(), 576);
    let messages = cases
        .iter()
        .map(|case| {
            bincode::serialize(&Message::new_with_blockhash(
                &case.instructions,
                Some(&case.wallet),
                &case.blockhash,
            ))
            .unwrap()
        })
        .collect::<Vec<_>>();
    let hash = format!(
        "{:x}",
        Sha256::digest(bincode::serialize(&messages).unwrap())
    );
    assert_eq!(
        hash,
        "0e70ba8d25dce8faaea08a4a556fc974ae39bd0904c3162665c39651e7d226d7"
    );
}

#[tokio::test]
async fn public_only_probe_plan_matches_all_frozen_sdk_messages_without_signing() {
    for case in oracle_cases().await {
        let prepared = prepare_damm_probe(
            case.wallet,
            &case.damm,
            case.mint,
            case.quote,
            case.amount,
            case.blockhash,
            false,
        )
        .unwrap();
        let expected = Transaction::new_unsigned(Message::new_with_blockhash(
            &case.instructions,
            Some(&case.wallet),
            &case.blockhash,
        ));
        assert_eq!(prepared.transaction(), &expected);
        assert_eq!(
            prepared.transaction().signatures,
            [solana_sdk::signature::Signature::default()]
        );
        let token_program = if case.quote == case.damm.token_a_mint {
            case.damm.token_a_program
        } else {
            case.damm.token_b_program
        };
        assert_eq!(
            prepared.output_ata(),
            sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &case.wallet,
                &case.quote,
                &token_program
            )
        );
    }
}
