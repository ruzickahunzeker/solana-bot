use super::*;
use crate::adapters::sdk::damm_probe::sdk_oracle_tests::{oracle_cases, OracleCase};
use sol_trade_sdk::{
    instruction::meteora_damm_v2::MeteoraDammV2InstructionBuilder,
    swqos::TradeType,
    trading::core::{params::SwapParams, traits::InstructionBuilder},
};
use solana_sdk::{instruction::Instruction, message::Message, signature::Signature};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn pool(quote: Pubkey) -> MeteoraDammV2Params {
    MeteoraDammV2Params::new(
        key(1),
        key(2),
        key(3),
        key(4),
        quote,
        spl_token_2022_interface::ID,
        spl_token_interface::ID,
    )
    .with_swap_mode(0)
}
fn build(pool: &MeteoraDammV2Params, input: Pubkey) -> Result<PreparedDammBuyProbe> {
    prepare(
        key(5),
        pool,
        input,
        key(4),
        1_000_000,
        Hash::new_from_array([6; 32]),
        false,
    )
}
fn instructions(plan: &PreparedDammBuyProbe) -> Vec<Instruction> {
    let message = &plan.transaction.message;
    message
        .instructions
        .iter()
        .map(|ix| {
            Instruction::new_with_bytes(
                message.account_keys[usize::from(ix.program_id_index)],
                &ix.data,
                ix.accounts
                    .iter()
                    .map(|i| {
                        let i = usize::from(*i);
                        solana_sdk::instruction::AccountMeta {
                            pubkey: message.account_keys[i],
                            is_signer: message.is_signer(i),
                            is_writable: message.is_maybe_writable_with_reserved_addresses(
                                i,
                                None::<&std::collections::HashSet<Pubkey>>,
                            ),
                        }
                    })
                    .collect(),
            )
        })
        .collect()
}
fn buy_request(case: &OracleCase, input: Pubkey) -> SwapParams {
    let mut request = case.request.clone();
    request.trade_type = TradeType::Buy;
    request.input_mint = input;
    request.output_mint = case.mint;
    request.output_token_program = Some(if case.mint == case.damm.token_a_mint {
        case.damm.token_a_program
    } else {
        case.damm.token_b_program
    });
    request.create_input_mint_ata = true;
    request.close_input_mint_ata = false;
    request.create_output_mint_ata = true;
    request.close_output_mint_ata = false;
    request
}
fn audited(case: &OracleCase) -> bool {
    let quote_program = if case.quote == case.damm.token_a_mint {
        case.damm.token_a_program
    } else {
        case.damm.token_b_program
    };
    case.damm.swap_mode == 0
        && case.damm.referral_token_account.is_none()
        && quote_program == spl_token_interface::ID
}

#[tokio::test]
async fn damm_buy_probe_matches_72_actual_sdk_buy_messages_without_signing() {
    let mut count = 0;
    for case in oracle_cases().await.into_iter().filter(audited) {
        let mut inputs = vec![case.quote];
        if case.quote == WSOL_TOKEN_ACCOUNT {
            inputs.push(SOL_TOKEN_ACCOUNT);
        }
        for input in inputs {
            let prepared = prepare(
                case.wallet,
                &case.damm,
                input,
                case.mint,
                case.amount,
                case.blockhash,
                false,
            )
            .unwrap();
            let expected = MeteoraDammV2InstructionBuilder
                .build_buy_instructions(&buy_request(&case, input))
                .await
                .unwrap();
            let expected = Transaction::new_unsigned(Message::new_with_blockhash(
                &expected,
                Some(&case.wallet),
                &case.blockhash,
            ));
            assert_eq!(prepared.transaction, expected);
            assert_eq!(prepared.transaction.signatures, [Signature::default()]);
            assert_eq!(prepared.accounts.wallet, case.wallet);
            assert_eq!(prepared.accounts.pool.pool, case.damm.pool);
            assert_eq!(prepared.accounts.input_mint, case.quote);
            assert_eq!(prepared.accounts.output_mint, case.mint);
            assert_eq!(prepared.accounts.amount, case.amount);
            assert_eq!(
                prepared.transaction.message.header.num_required_signatures,
                1
            );
            count += 1;
        }
    }
    assert_eq!(count, 72);
}

#[test]
fn damm_buy_probe_native_funding_is_exact_and_does_not_close_existing_wsol() {
    for input in [SOL_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT] {
        let plan = build(&pool(WSOL_TOKEN_ACCOUNT), input).unwrap();
        let ix = instructions(&plan);
        assert_eq!(plan.funding, Funding::NativeSol);
        assert_eq!(ix.len(), 5);
        assert_eq!(
            ix[1],
            solana_system_interface::instruction::transfer(
                &key(5),
                &plan.accounts.input_ata,
                1_000_000
            )
        );
        assert_eq!(ix[2].program_id, spl_token_interface::ID);
        assert_eq!(ix[2].data, [17]);
        assert_eq!(ix[2].accounts[0].pubkey, plan.accounts.input_ata);
        assert_eq!(
            ix[3].program_id,
            spl_associated_token_account_interface::program::id()
        );
        assert!(ix
            .iter()
            .all(|i| { i.program_id != spl_token_interface::ID || i.data != [9] }));
        assert_eq!(ix[4].data[8..16], 1_000_000u64.to_le_bytes());
        assert_eq!(ix[4].data[16..24], 1u64.to_le_bytes());
    }
}

#[test]
fn damm_buy_probe_usdc_does_not_convert_lamports_into_quote_units() {
    let plan = build(&pool(USDC_TOKEN_ACCOUNT), USDC_TOKEN_ACCOUNT).unwrap();
    let ix = instructions(&plan);
    assert_eq!(plan.funding, Funding::ExistingUsdc);
    assert_eq!(ix.len(), 3);
    assert_eq!(
        ix[0].program_id,
        spl_associated_token_account_interface::program::id()
    );
    assert_eq!(ix[0].accounts[1].pubkey, plan.accounts.input_ata);
    assert_eq!(ix[1].accounts[1].pubkey, plan.accounts.output_ata);
    assert!(ix
        .iter()
        .all(|i| i.program_id != solana_system_interface::program::id()));
    assert_eq!(ix[2].data[8..16], 1_000_000u64.to_le_bytes());
    // ATA rent remains native SOL; this contract only concerns the exact swap input.
    assert_eq!(plan.accounts.input_mint, USDC_TOKEN_ACCOUNT);
}

#[test]
fn damm_buy_probe_rejects_unaudited_modes_programs_quotes_and_side_aliases() {
    for mutation in 0..12 {
        let mut source = pool(USDC_TOKEN_ACCOUNT);
        let mut quote = USDC_TOKEN_ACCOUNT;
        let mut mint = key(4);
        let mut amount = 1;
        let mut seed = false;
        match mutation {
            0 => amount = 0,
            1 => seed = true,
            2 => source.swap_mode = 1,
            3 => source.swap_mode = 2,
            4 => source.swap_mode = 255,
            5 => source.referral_token_account = Some(key(99)),
            6 => quote = key(99),
            7 => mint = USDC_TOKEN_ACCOUNT,
            8 => mint = key(99),
            9 => source.token_b_program = spl_token_2022_interface::ID,
            10 => source.token_b_program = key(99),
            _ => source.token_a_program = key(99),
        }
        assert!(
            prepare(key(5), &source, quote, mint, amount, Hash::default(), seed).is_err(),
            "mutation={mutation}"
        );
    }
}

#[test]
fn damm_buy_probe_binds_wallet_blockhash_pool_assets_amount_and_optional_sysvar() {
    let source = pool(USDC_TOKEN_ACCOUNT);
    let original = build(&source, USDC_TOKEN_ACCOUNT).unwrap();
    for mutation in 0..9 {
        let mut source = source.clone();
        let mut wallet = key(5);
        let mut hash = Hash::new_from_array([6; 32]);
        let mut amount = 1_000_000;
        let mut mint = key(4);
        match mutation {
            0 => wallet = key(99),
            1 => hash = Hash::new_from_array([99; 32]),
            2 => amount += 1,
            3 => source.pool = key(99),
            4 => source.token_a_vault = key(99),
            5 => source.token_b_vault = key(99),
            6 => source.token_a_program = spl_token_interface::ID,
            7 => source.include_rate_limiter_sysvar = true,
            _ => {
                source.token_a_mint = key(99);
                mint = key(99);
            }
        }
        let next = prepare(
            wallet,
            &source,
            USDC_TOKEN_ACCOUNT,
            mint,
            amount,
            hash,
            false,
        )
        .unwrap();
        assert_ne!(next.transaction.message, original.transaction.message);
        assert_eq!(next.transaction.signatures, [Signature::default()]);
    }
}

#[test]
fn damm_buy_probe_is_not_the_prefunded_reverse_sell_probe() {
    let source = pool(WSOL_TOKEN_ACCOUNT);
    let buy = build(&source, WSOL_TOKEN_ACCOUNT).unwrap();
    let old = crate::adapters::sdk::damm_probe::prepare_damm_probe(
        key(5),
        &source,
        WSOL_TOKEN_ACCOUNT,
        key(4),
        1_000_000,
        Hash::new_from_array([6; 32]),
        false,
    )
    .unwrap();
    assert_ne!(
        std::any::TypeId::of::<PreparedDammBuyProbe>(),
        std::any::TypeId::of::<super::super::plan::PreparedDammProbe>()
    );
    assert_ne!(buy.transaction.message, old.transaction().message);
    assert_eq!(old.transaction().message.instructions.len(), 2);
    assert_eq!(buy.transaction.message.instructions.len(), 5);
}

#[tokio::test]
async fn damm_buy_sdk_minimum_operand_is_fixed_output_not_automatic_slippage() {
    let cases = oracle_cases().await;
    let case = cases
        .iter()
        .find(|c| audited(c) && c.quote == USDC_TOKEN_ACCOUNT)
        .unwrap();
    for slippage in [0, 1, 1500, 9999] {
        for minimum in [1, 777, u64::MAX] {
            let mut request = buy_request(case, case.quote);
            request.slippage_basis_points = Some(slippage);
            request.fixed_output_amount = Some(minimum);
            let ix = MeteoraDammV2InstructionBuilder
                .build_buy_instructions(&request)
                .await
                .unwrap();
            let swap = ix.last().unwrap();
            assert_eq!(swap.data.len(), 25);
            assert_eq!(swap.data[8..16], case.amount.to_le_bytes());
            assert_eq!(swap.data[16..24], minimum.to_le_bytes());
            assert_eq!(swap.data[24], 0);
        }
    }
}
