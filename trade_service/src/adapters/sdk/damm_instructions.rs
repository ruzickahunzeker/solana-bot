use sol_trade_sdk::{
    instruction::utils::meteora_damm_v2::{accounts, get_event_authority_pda, SWAP2_DISCRIMINATOR},
    trading::core::params::MeteoraDammV2Params,
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

/// ABI projection only; callers must bind accounts, amounts and mode before signing.
pub(super) fn swap_instruction(
    wallet: Pubkey,
    pool: &MeteoraDammV2Params,
    input: Pubkey,
    output: Pubkey,
    amount_0: u64,
    amount_1: u64,
) -> Instruction {
    let mut metas = vec![
        accounts::AUTHORITY_META,
        AccountMeta::new(pool.pool, false),
        AccountMeta::new(input, false),
        AccountMeta::new(output, false),
        AccountMeta::new(pool.token_a_vault, false),
        AccountMeta::new(pool.token_b_vault, false),
        AccountMeta::new_readonly(pool.token_a_mint, false),
        AccountMeta::new_readonly(pool.token_b_mint, false),
        AccountMeta::new_readonly(wallet, true),
        AccountMeta::new_readonly(pool.token_a_program, false),
        AccountMeta::new_readonly(pool.token_b_program, false),
        pool.referral_token_account
            .map_or(accounts::METEORA_DAMM_V2_META, |referral| {
                AccountMeta::new(referral, false)
            }),
        AccountMeta::new_readonly(get_event_authority_pda(), false),
        accounts::METEORA_DAMM_V2_META,
    ];
    if pool.include_rate_limiter_sysvar {
        metas.push(accounts::SYSVAR_INSTRUCTIONS_META);
    }
    let mut data = [0; 25];
    data[..8].copy_from_slice(SWAP2_DISCRIMINATOR);
    data[8..16].copy_from_slice(&amount_0.to_le_bytes());
    data[16..24].copy_from_slice(&amount_1.to_le_bytes());
    data[24] = pool.swap_mode;
    Instruction::new_with_bytes(accounts::METEORA_DAMM_V2, &data, metas)
}
