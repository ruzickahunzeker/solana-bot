//! Frozen read-only native Pump quote probe. Not a formal trade candidate or signing permit.
use super::{NativeFundingSnapshot, Side, TradeAmounts};
use crate::domain::preparation_balance::MAX_BALANCE_AGE;
use anyhow::{ensure, Context, Result};
use solana_sdk::{
    hash::Hash, instruction::Instruction, message::Message, transaction::Transaction,
};
use std::{num::NonZeroU64, time::Instant};

pub(super) struct NativeQuoteProbe {
    funding: NativeFundingSnapshot,
    transaction: Transaction,
    instruction: Instruction,
    input: NonZeroU64,
}

pub(super) fn fresh(started: Instant) -> Result<()> {
    ensure!(
        Instant::now()
            .checked_duration_since(started)
            .is_some_and(|age| age <= MAX_BALANCE_AGE),
        "native Pump quote account evidence is expired or from the future"
    );
    Ok(())
}

impl NativeFundingSnapshot {
    pub(super) fn prepare_quote_probe(
        self,
        mut instruction: Instruction,
        expected_input: NonZeroU64,
        blockhash: Hash,
    ) -> Result<NativeQuoteProbe> {
        fresh(self.started)?;
        ensure!(
            blockhash != Hash::default(),
            "native Pump quote blockhash is missing"
        );
        self.inspect_native_destinations(&instruction)?;
        let amounts = self.inspect_native_roles(&instruction)?.amounts;
        let input = match amounts {
            TradeAmounts::BuyExactInput { spend, .. } => spend,
            TradeAmounts::SellExactInput { input, .. } => input,
            _ => anyhow::bail!("native Pump quote requires exact input"),
        };
        ensure!(
            input == expected_input,
            "native Pump quote input differs from the frozen intent"
        );
        let holding = self
            .user_base_units
            .context("native Pump quote profile requires an existing user ATA")?;
        match self.side {
            Side::Buy => ensure!(
                self.native_lamports >= input.get(),
                "native Pump quote cannot fund input before fees"
            ),
            Side::Sell => ensure!(
                holding >= input.get(),
                "native Pump quote token input exceeds snapshot balance"
            ),
        }
        // This floor belongs only to a zero-signature discovery probe. It is not a live min-out.
        instruction.data[16..24].copy_from_slice(&1u64.to_le_bytes());
        self.inspect_native_destinations(&instruction)?;
        let wallet = self.keys[6];
        let message =
            Message::new_with_blockhash(&[instruction.clone()], Some(&wallet), &blockhash);
        ensure!(
            message.header.num_required_signatures == 1
                && message.account_keys.first() == Some(&wallet),
            "native Pump quote requires exactly its bank wallet signer"
        );
        let transaction = Transaction::new_unsigned(message);
        ensure!(
            bincode::serialized_size(&transaction)? <= 1232,
            "native Pump quote exceeds packet size"
        );
        fresh(self.started)?;
        Ok(NativeQuoteProbe {
            funding: self,
            transaction,
            instruction,
            input,
        })
    }
}

impl NativeQuoteProbe {
    pub(super) fn packet(&self) -> Result<&Transaction> {
        fresh(self.funding.started)?;
        Ok(&self.transaction)
    }
}

mod observation;
pub(super) mod tests;
