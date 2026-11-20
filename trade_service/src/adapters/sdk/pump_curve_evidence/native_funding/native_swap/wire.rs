//! Shared locked native encoding only, without account truth, age or execution permission.
use crate::domain::trade::Side;
use anyhow::{Context, Result};
use sol_trade_sdk::instruction::utils::pumpfun::{
    accounts, get_bonding_curve_v2_pda, get_user_volume_accumulator_pda,
    BUY_EXACT_SOL_IN_DISCRIMINATOR, SELL_DISCRIMINATOR,
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::num::NonZeroU64;

pub(in crate::adapters::sdk::pump_curve_evidence::native_funding) struct NativeRoles<'a> {
    pub keys: &'a [Pubkey; 8],
    pub protocol_fee: Pubkey,
    pub creator_vault: Pubkey,
    pub buyback: Pubkey,
    pub side: Side,
    pub cashback: bool,
}

pub(in crate::adapters::sdk::pump_curve_evidence::native_funding) fn encode(
    roles: NativeRoles<'_>,
    input: NonZeroU64,
    minimum: NonZeroU64,
) -> Result<Instruction> {
    let keys = roles.keys;
    let buy = roles.side == Side::Buy;
    let curve_v2 = get_bonding_curve_v2_pda(&keys[1]).context("bank-only native Pump curve PDA")?;
    let mut metas = vec![
        AccountMeta::new_readonly(keys[2], false),
        AccountMeta::new(roles.protocol_fee, false),
        AccountMeta::new_readonly(keys[1], false),
        AccountMeta::new(keys[0], false),
        AccountMeta::new(keys[4], false),
        AccountMeta::new(keys[5], false),
        AccountMeta::new(keys[6], true),
        AccountMeta::new_readonly(solana_system_interface::program::ID, false),
    ];
    if buy {
        metas.push(AccountMeta::new_readonly(spl_token_interface::ID, false));
        metas.push(AccountMeta::new(roles.creator_vault, false));
    } else {
        metas.push(AccountMeta::new(roles.creator_vault, false));
        metas.push(AccountMeta::new_readonly(spl_token_interface::ID, false));
    }
    metas.extend([accounts::EVENT_AUTHORITY_META, accounts::PUMPFUN_META]);
    if buy {
        metas.push(accounts::GLOBAL_VOLUME_ACCUMULATOR_META);
        let volume = get_user_volume_accumulator_pda(&keys[6])
            .context("bank-only native Pump user volume PDA")?;
        metas.push(AccountMeta::new(volume, false));
    }
    metas.extend([accounts::FEE_CONFIG_META, accounts::FEE_PROGRAM_META]);
    if !buy && roles.cashback {
        let volume = get_user_volume_accumulator_pda(&keys[6])
            .context("bank-only native Pump user volume PDA")?;
        metas.push(AccountMeta::new(volume, false));
    }
    metas.extend([
        AccountMeta::new_readonly(curve_v2, false),
        AccountMeta::new(roles.buyback, false),
    ]);
    let mut data = if buy {
        BUY_EXACT_SOL_IN_DISCRIMINATOR
    } else {
        SELL_DISCRIMINATOR
    }
    .to_vec();
    data.extend_from_slice(&input.get().to_le_bytes());
    data.extend_from_slice(&minimum.get().to_le_bytes());
    if buy {
        data.push(u8::from(roles.cashback));
    }
    Ok(Instruction::new_with_bytes(accounts::PUMPFUN, &data, metas))
}
