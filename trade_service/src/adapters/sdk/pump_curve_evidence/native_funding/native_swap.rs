//! Public-account-only, locked native V1 instruction. No signer or live permission.
use super::{probe::fresh, NativeFundingSnapshot, Side, TradeAmounts, WireLayout};
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::{
    constants::WSOL_TOKEN_ACCOUNT,
    instruction::utils::pumpfun::{
        accounts, get_bonding_curve_v2_pda, get_user_volume_accumulator_pda,
        BUY_EXACT_SOL_IN_DISCRIMINATOR, SELL_DISCRIMINATOR,
    },
};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::num::NonZeroU64;

struct BankNativeSwap {
    funding: NativeFundingSnapshot,
    input: NonZeroU64,
    minimum: NonZeroU64,
    buyback: Pubkey,
    instruction: Instruction,
}

fn build(
    funding: &NativeFundingSnapshot,
    input: NonZeroU64,
    minimum: NonZeroU64,
    buyback: Pubkey,
) -> Result<Instruction> {
    fresh(funding.started)?;
    ensure!(
        funding.curve.quote == WSOL_TOKEN_ACCOUNT
            && funding.curve.base_mint_layout.program == spl_token_interface::ID
            && !funding.curve.curve.complete,
        "bank-only native Pump profile requires native SOL and classic SPL"
    );
    let holding = funding
        .user_base_units
        .context("bank-only native Pump profile requires an existing user ATA")?;
    let buy = funding.side == Side::Buy;
    ensure!(
        if buy {
            funding.native_lamports >= input.get()
        } else {
            holding >= input.get()
        },
        "bank-only native Pump input exceeds the frozen funding snapshot"
    );
    let destinations = funding.destinations()?;
    ensure!(
        destinations.buyback.contains(&buyback),
        "bank-only native Pump buyback is outside the bank set"
    );
    let keys = &funding.keys;
    let curve_v2 = get_bonding_curve_v2_pda(&keys[1]).context("bank-only native Pump curve PDA")?;
    let mut metas = vec![
        AccountMeta::new_readonly(keys[2], false),
        AccountMeta::new(destinations.protocol_fee, false),
        AccountMeta::new_readonly(keys[1], false),
        AccountMeta::new(keys[0], false),
        AccountMeta::new(keys[4], false),
        AccountMeta::new(keys[5], false),
        AccountMeta::new(keys[6], true),
        AccountMeta::new_readonly(solana_system_interface::program::ID, false),
    ];
    if buy {
        metas.push(AccountMeta::new_readonly(spl_token_interface::ID, false));
        metas.push(AccountMeta::new(destinations.creator_vault, false));
    } else {
        metas.push(AccountMeta::new(destinations.creator_vault, false));
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
    if !buy && funding.curve.curve.cashback {
        let volume = get_user_volume_accumulator_pda(&keys[6])
            .context("bank-only native Pump user volume PDA")?;
        metas.push(AccountMeta::new(volume, false));
    }
    metas.extend([
        AccountMeta::new_readonly(curve_v2, false),
        AccountMeta::new(buyback, false),
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
        data.push(u8::from(funding.curve.curve.cashback));
    }
    let instruction = Instruction::new_with_bytes(accounts::PUMPFUN, &data, metas);
    funding.inspect_native_destinations(&instruction)?;
    let amounts = funding.inspect_native_roles(&instruction)?;
    ensure!(
        amounts.layout == WireLayout::NativeV1
            && amounts.amounts
                == if buy {
                    TradeAmounts::BuyExactInput {
                        spend: input,
                        minimum,
                    }
                } else {
                    TradeAmounts::SellExactInput { input, minimum }
                },
        "bank-only native Pump encoding differs from the frozen integer intent"
    );
    fresh(funding.started)?;
    Ok(instruction)
}

impl NativeFundingSnapshot {
    fn prepare_native_swap(
        self,
        input: NonZeroU64,
        minimum: NonZeroU64,
        buyback: Pubkey,
    ) -> Result<BankNativeSwap> {
        let instruction = build(&self, input, minimum, buyback)?;
        Ok(BankNativeSwap {
            funding: self,
            input,
            minimum,
            buyback,
            instruction,
        })
    }
}

impl BankNativeSwap {
    fn instruction(&self) -> Result<&Instruction> {
        ensure!(
            self.instruction == build(&self.funding, self.input, self.minimum, self.buyback)?,
            "bank-only native Pump instruction changed after preparation"
        );
        Ok(&self.instruction)
    }

    fn into_quote_probe(self, blockhash: Hash) -> Result<super::probe::NativeQuoteProbe> {
        self.instruction()?;
        // The existing quote probe still owns discovery floor1 and the unchanged original age.
        self.funding
            .prepare_quote_probe(self.instruction, self.input, blockhash)
    }
}

mod tests;
