//! Swap-only simulation constraints. Not a complete execution candidate or signing permit.
use super::NativeBankObservation;
use crate::adapters::sdk::pump_curve_evidence::native_funding::probe::fresh;
use crate::adapters::sdk::pump_curve_evidence::native_funding::Side;
use anyhow::{ensure, Context, Result};
use solana_sdk::{message::Message, signature::Signature, transaction::Transaction};
use std::num::NonZeroU64;

#[derive(Debug, PartialEq, Eq)]
struct SwapNativeDebit {
    available_before: u64,
    maximum_debit: u64,
    remaining_without_output: u64,
}

struct NativeSwapConstraints {
    source: NativeBankObservation,
    slippage_bps: u64,
    minimum: NonZeroU64,
    native: SwapNativeDebit,
    transaction: Transaction,
}

fn minimum_output(output: NonZeroU64, slippage_bps: u64) -> Result<NonZeroU64> {
    ensure!(
        slippage_bps < 10_000,
        "native Pump swap slippage is invalid"
    );
    let minimum = u128::from(output.get()) * u128::from(10_000 - slippage_bps) / 10_000;
    u64::try_from(minimum)
        .ok()
        .and_then(NonZeroU64::new)
        .context("native Pump swap minimum rounds to zero")
}

fn native_debit(source: &NativeBankObservation) -> Result<SwapNativeDebit> {
    let probe = &source.observation.probe;
    ensure!(
        source.bank.snapshot.started == probe.funding.started
            && source.bank.snapshot.keys == probe.funding.keys
            && source.bank.snapshot.side == probe.funding.side
            && source.bank.snapshot.curve.quote == probe.funding.curve.quote
            && source.bank.snapshot.curve.slot == source.observation.simulation_slot,
        "native Pump swap post-bank context differs from its observation"
    );
    let fee = source.observation.network_fee.get();
    let buy = probe.funding.side == Side::Buy;
    let maximum_debit = if buy {
        probe
            .input
            .get()
            .checked_add(fee)
            .context("native Pump swap native debit overflows")?
    } else {
        fee
    };
    // Reconstruct only from the already cross-checked same-response post bank and deltas.
    // Never use the earlier RPC wallet balance, sell proceeds, or a future close refund.
    let post = source.bank.snapshot.native_lamports;
    let available_before = if buy {
        post.checked_add(maximum_debit)
            .context("native Pump swap pre balance overflows")?
    } else {
        u64::try_from(
            i128::from(post) + i128::from(fee) - i128::from(source.observation.output.get()),
        )
        .context("native Pump swap pre balance is invalid")?
    };
    let remaining_without_output = available_before
        .checked_sub(maximum_debit)
        .context("native Pump swap cannot prefund its native debit")?;
    Ok(SwapNativeDebit {
        available_before,
        maximum_debit,
        remaining_without_output,
    })
}

impl NativeBankObservation {
    fn swap_constraints(self, slippage_bps: u64) -> Result<NativeSwapConstraints> {
        let probe = &self.observation.probe;
        probe.packet()?;
        let minimum = minimum_output(self.observation.output, slippage_bps)?;
        let native = native_debit(&self)?;
        let mut instruction = probe.instruction.clone();
        instruction.data[16..24].copy_from_slice(&minimum.get().to_le_bytes());
        probe.funding.inspect_native_destinations(&instruction)?;
        let transaction = Transaction::new_unsigned(Message::new_with_blockhash(
            &[instruction],
            Some(&probe.funding.keys[6]),
            &probe.transaction.message.recent_blockhash,
        ));
        ensure!(
            bincode::serialized_size(&transaction)? <= 1232,
            "native Pump swap template exceeds packet size"
        );
        fresh(probe.funding.started)?;
        Ok(NativeSwapConstraints {
            source: self,
            slippage_bps,
            minimum,
            native,
            transaction,
        })
    }
}

impl NativeSwapConstraints {
    fn packet(&self) -> Result<&Transaction> {
        let probe = &self.source.observation.probe;
        probe.packet()?;
        ensure!(
            self.minimum == minimum_output(self.source.observation.output, self.slippage_bps)?,
            "native Pump swap minimum changed after preparation"
        );
        ensure!(
            self.native == native_debit(&self.source)?,
            "native Pump swap native budget changed after preparation"
        );
        let mut expected = probe.instruction.clone();
        expected.data[16..24].copy_from_slice(&self.minimum.get().to_le_bytes());
        ensure!(
            self.transaction.signatures == [Signature::default()]
                && self.transaction.message
                    == Message::new_with_blockhash(
                        &[expected],
                        Some(&probe.funding.keys[6]),
                        &probe.transaction.message.recent_blockhash
                    ),
            "native Pump swap template differs from its frozen instruction"
        );
        fresh(probe.funding.started)?;
        Ok(&self.transaction)
    }
}

mod envelope;
pub(crate) use envelope::NativeBoundIntent;
#[cfg(test)]
mod tests;
