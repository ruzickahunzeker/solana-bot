//! Complete unsigned message template only. Not final simulation or signing authority.
use super::NativeSwapConstraints;
use crate::adapters::sdk::damm_instructions::{compile_candidate, full_instructions, tip_lamports};
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::swqos::SwqosType;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Signature,
    transaction::VersionedTransaction,
};

// Existing ordinary envelope: 300_000 CU at 1_000 micro-lamports/CU.
const PRIORITY_LAMPORTS: u64 = 300;

#[derive(Debug, PartialEq, Eq)]
struct EnvelopeDebit {
    maximum_debit: u64,
    remaining_without_output: u64,
}

struct NativeEnvelopeTemplate {
    source: NativeSwapConstraints,
    provider: SwqosType,
    tip_sol: f64,
    destination: Option<Pubkey>,
    native: EnvelopeDebit,
    instructions: Vec<Instruction>,
    transaction: VersionedTransaction,
}

fn fee_debit(
    source: &NativeSwapConstraints,
    provider: SwqosType,
    tip_sol: f64,
) -> Result<EnvelopeDebit> {
    ensure!(
        tip_sol.is_finite() && tip_sol >= 0.0 && tip_sol * 1_000_000_000.0 < u64::MAX as f64,
        "native Pump envelope tip is invalid or out of range"
    );
    let tip = if provider == SwqosType::Default {
        0
    } else {
        tip_lamports(tip_sol)
    };
    let maximum_debit = source
        .native
        .maximum_debit
        .checked_add(PRIORITY_LAMPORTS)
        .and_then(|amount| amount.checked_add(tip))
        .context("native Pump envelope native debit overflows")?;
    let remaining_without_output = source
        .native
        .available_before
        .checked_sub(maximum_debit)
        .context("native Pump envelope cannot prefund fees and tip")?;
    Ok(EnvelopeDebit {
        maximum_debit,
        remaining_without_output,
    })
}

impl NativeSwapConstraints {
    fn prepare_envelope(
        self,
        provider: SwqosType,
        tip_sol: f64,
        destination: Option<Pubkey>,
    ) -> Result<NativeEnvelopeTemplate> {
        self.packet()?;
        let native = fee_debit(&self, provider, tip_sol)?;
        let probe = &self.source.observation.probe;
        if let Some(destination) = destination {
            ensure!(
                !probe
                    .transaction
                    .message
                    .account_keys
                    .contains(&destination),
                "native Pump envelope tip aliases a frozen swap account"
            );
        }
        let mut swap = probe.instruction.clone();
        swap.data[16..24].copy_from_slice(&self.minimum.get().to_le_bytes());
        // Live-shaped fees in a zero-signature template; never a live SDK invocation.
        let instructions = full_instructions(
            probe.funding.keys[6],
            provider,
            false,
            tip_sol,
            destination,
            &[swap],
        )?;
        let transaction = compile_candidate(
            probe.funding.keys[6],
            &instructions,
            probe.transaction.message.recent_blockhash,
        )?;
        self.packet()?;
        Ok(NativeEnvelopeTemplate {
            source: self,
            provider,
            tip_sol,
            destination,
            native,
            instructions,
            transaction,
        })
    }
}

impl NativeEnvelopeTemplate {
    fn packet(&self) -> Result<&VersionedTransaction> {
        self.source.packet()?;
        ensure!(
            self.native == fee_debit(&self.source, self.provider, self.tip_sol)?,
            "native Pump envelope budget changed after preparation"
        );
        let probe = &self.source.source.observation.probe;
        if let Some(destination) = self.destination {
            ensure!(
                !probe
                    .transaction
                    .message
                    .account_keys
                    .contains(&destination),
                "native Pump envelope tip aliases a frozen swap account"
            );
        }
        let mut swap = probe.instruction.clone();
        swap.data[16..24].copy_from_slice(&self.source.minimum.get().to_le_bytes());
        let instructions = full_instructions(
            probe.funding.keys[6],
            self.provider,
            false,
            self.tip_sol,
            self.destination,
            &[swap],
        )?;
        let expected = compile_candidate(
            probe.funding.keys[6],
            &instructions,
            probe.transaction.message.recent_blockhash,
        )?;
        ensure!(
            self.instructions == instructions
                && self.transaction.signatures == [Signature::default()]
                && self.transaction.message == expected.message,
            "native Pump envelope differs from the frozen complete message"
        );
        self.source.packet()?;
        Ok(&self.transaction)
    }
}

mod tests;
