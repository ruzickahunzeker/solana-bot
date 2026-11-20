//! Public-account-only, locked native V1 instruction. No signer or live permission.
use super::{probe::fresh, NativeFundingSnapshot, Side, TradeAmounts, WireLayout};
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
use solana_sdk::{hash::Hash, instruction::Instruction, pubkey::Pubkey};
use std::num::NonZeroU64;

pub(super) struct BankNativeSwap {
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
    let instruction = wire::encode(
        wire::NativeRoles {
            keys: &funding.keys,
            protocol_fee: destinations.protocol_fee,
            creator_vault: destinations.creator_vault,
            buyback,
            side: funding.side,
            cashback: funding.curve.curve.cashback,
        },
        input,
        minimum,
    )?;
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
    pub(super) fn prepare_native_swap(
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

    pub(super) fn into_quote_probe(
        self,
        blockhash: Hash,
    ) -> Result<super::probe::NativeQuoteProbe> {
        self.instruction()?;
        // The existing quote probe still owns discovery floor1 and the unchanged original age.
        self.funding
            .prepare_quote_probe(self.instruction, self.input, blockhash)
    }
}

mod tests;
pub(super) mod wire;
