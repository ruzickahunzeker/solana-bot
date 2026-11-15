use anyhow::{bail, Result};
use sol_trade_sdk::{
    common::fast_fn::{
        create_associated_token_account_idempotent_fast,
        get_associated_token_address_with_program_id_fast,
    },
    constants::{SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    instruction::utils::meteora_damm_v2::{
        SWAP_MODE_EXACT_IN, SWAP_MODE_EXACT_OUT, SWAP_MODE_PARTIAL_FILL,
    },
    trading::core::params::MeteoraDammV2Params,
};
use solana_sdk::{hash::Hash, pubkey::Pubkey, transaction::Transaction};

/// Restricted quote-only profile, not an authorized or signed ordinary trade.
pub(crate) struct PreparedDammProbe {
    transaction: Transaction,
    accounts: ProbeAccounts,
}

pub(super) struct ProbeAccounts {
    pub wallet: Pubkey,
    pub pool: MeteoraDammV2Params,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub input_ata: Pubkey,
    pub output_ata: Pubkey,
    pub amount: u64,
}

impl PreparedDammProbe {
    pub(super) fn transaction(&self) -> &Transaction {
        &self.transaction
    }
    pub(crate) fn output_ata(&self) -> Pubkey {
        self.accounts.output_ata
    }
    pub(super) fn accounts(&self) -> &ProbeAccounts {
        &self.accounts
    }
}

fn normalize(mint: Pubkey) -> Pubkey {
    if mint == SOL_TOKEN_ACCOUNT {
        WSOL_TOKEN_ACCOUNT
    } else {
        mint
    }
}

/// No RPC, payer, fee strategy, sender or middleware capability enters this builder.
pub(crate) fn prepare_damm_probe(
    wallet: Pubkey,
    pool: &MeteoraDammV2Params,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    blockhash: Hash,
    seed_optimized: bool,
) -> Result<PreparedDammProbe> {
    if amount == 0 {
        bail!("Token amount is not set");
    }
    if ![pool.token_a_mint, pool.token_b_mint]
        .iter()
        .any(|mint| *mint == WSOL_TOKEN_ACCOUNT || *mint == USDC_TOKEN_ACCOUNT)
    {
        bail!("Pool must contain WSOL or USDC");
    }
    let input_mint = normalize(input_mint);
    let output_mint = normalize(output_mint);
    let a_in = input_mint == pool.token_a_mint;
    if !((a_in && output_mint == pool.token_b_mint)
        || (input_mint == pool.token_b_mint && output_mint == pool.token_a_mint))
    {
        bail!("Input/output mints must match opposite DAMM v2 pool sides");
    }
    // Keep the existing probe's fixed_output_amount=1, including exact-out interpretation.
    let (amount_0, amount_1) = match pool.swap_mode {
        SWAP_MODE_EXACT_OUT => (1, amount),
        SWAP_MODE_EXACT_IN | SWAP_MODE_PARTIAL_FILL => (amount, 1),
        mode => bail!("Unsupported MeteoraDammV2 swap_mode {}", mode),
    };
    if seed_optimized {
        bail!("DAMM v2 unsigned quote probe does not support seed-optimized accounts");
    }
    let input_program = if a_in {
        pool.token_a_program
    } else {
        pool.token_b_program
    };
    let output_program = if a_in {
        pool.token_b_program
    } else {
        pool.token_a_program
    };
    let input_ata =
        get_associated_token_address_with_program_id_fast(&wallet, &input_mint, &input_program);
    let output_ata =
        get_associated_token_address_with_program_id_fast(&wallet, &output_mint, &output_program);
    let mut instructions = create_associated_token_account_idempotent_fast(
        &wallet,
        &wallet,
        &output_mint,
        &output_program,
    );
    instructions.push(crate::adapters::sdk::damm_instructions::swap_instruction(
        wallet, pool, input_ata, output_ata, amount_0, amount_1,
    ));
    let transaction = super::unsigned_probe(wallet, &instructions, blockhash)?;
    Ok(PreparedDammProbe {
        transaction,
        accounts: ProbeAccounts {
            wallet,
            pool: pool.clone(),
            input_mint,
            output_mint,
            input_ata,
            output_ata,
            amount,
        },
    })
}
