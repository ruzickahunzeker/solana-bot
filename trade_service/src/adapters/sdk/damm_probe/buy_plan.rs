//! Quote-only fixture prototype. Not wired to preparation, signing or submission.
use super::{plan::ProbeAccounts, unsigned_probe};
use anyhow::{ensure, Result};
use sol_trade_sdk::{
    common::fast_fn::{
        create_associated_token_account_idempotent_fast,
        get_associated_token_address_with_program_id_fast,
    },
    constants::{SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    instruction::utils::meteora_damm_v2::SWAP_MODE_EXACT_IN,
    trading::core::params::MeteoraDammV2Params,
};
use solana_sdk::{hash::Hash, pubkey::Pubkey, transaction::Transaction};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Funding {
    NativeSol,
    ExistingUsdc,
}

// Nominally separate from the sell probe: its account and transfer evidence is not yet audited.
struct PreparedDammBuyProbe {
    transaction: Transaction,
    accounts: ProbeAccounts,
    funding: Funding,
}

fn prepare(
    wallet: Pubkey,
    pool: &MeteoraDammV2Params,
    input: Pubkey,
    mint: Pubkey,
    amount: u64,
    blockhash: Hash,
    seed_optimized: bool,
) -> Result<PreparedDammBuyProbe> {
    ensure!(amount > 0, "DAMM buy probe input must be positive");
    ensure!(
        !seed_optimized,
        "DAMM buy probe seed-optimized accounts are not audited"
    );
    ensure!(
        pool.swap_mode == SWAP_MODE_EXACT_IN,
        "DAMM buy probe requires exact-in mode"
    );
    ensure!(
        pool.referral_token_account.is_none(),
        "DAMM buy probe referral accounts are not audited"
    );
    let (input, funding) = match input {
        SOL_TOKEN_ACCOUNT | WSOL_TOKEN_ACCOUNT => (WSOL_TOKEN_ACCOUNT, Funding::NativeSol),
        USDC_TOKEN_ACCOUNT => (USDC_TOKEN_ACCOUNT, Funding::ExistingUsdc),
        _ => anyhow::bail!("DAMM buy probe requires SOL/WSOL or USDC input"),
    };
    let a_in = input == pool.token_a_mint;
    ensure!(
        input != mint
            && ((a_in && mint == pool.token_b_mint)
                || (input == pool.token_b_mint && mint == pool.token_a_mint)),
        "DAMM buy probe assets must match opposite pool sides"
    );
    let (input_program, output_program) = if a_in {
        (pool.token_a_program, pool.token_b_program)
    } else {
        (pool.token_b_program, pool.token_a_program)
    };
    ensure!(
        input_program == spl_token_interface::ID,
        "DAMM buy probe canonical Quote program is invalid"
    );
    ensure!(
        output_program == spl_token_interface::ID || output_program == spl_token_2022_interface::ID,
        "DAMM buy probe output Token program is unsupported"
    );
    let input_ata =
        get_associated_token_address_with_program_id_fast(&wallet, &input, &input_program);
    let output_ata =
        get_associated_token_address_with_program_id_fast(&wallet, &mint, &output_program);
    let mut instructions = match funding {
        Funding::NativeSol => sol_trade_sdk::trading::common::handle_wsol(&wallet, amount)
            .into_iter()
            .collect(),
        Funding::ExistingUsdc => create_associated_token_account_idempotent_fast(
            &wallet,
            &wallet,
            &input,
            &input_program,
        ),
    };
    instructions.extend(create_associated_token_account_idempotent_fast(
        &wallet,
        &wallet,
        &mint,
        &output_program,
    ));
    // One is a simulation-only discovery floor, not the formal trade's slippage minimum.
    instructions.push(crate::adapters::sdk::damm_instructions::swap_instruction(
        wallet, pool, input_ata, output_ata, amount, 1,
    ));
    Ok(PreparedDammBuyProbe {
        transaction: unsigned_probe(wallet, &instructions, blockhash)?,
        accounts: ProbeAccounts {
            wallet,
            pool: pool.clone(),
            input_mint: input,
            output_mint: mint,
            input_ata,
            output_ata,
            amount,
        },
        funding,
    })
}

#[cfg(test)]
mod tests;
