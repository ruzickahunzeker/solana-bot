use super::*;
use sol_trade_sdk::{
    constants::swqos::*,
    trading::{
        common::extend_compute_budget_instructions, core::transaction_pool::compile_message,
    },
};
use solana_sdk::{
    message::{v1, Message, VersionedMessage},
    signature::Signature,
    transaction::VersionedTransaction,
};

/// Owned immutable nominal transaction. It contains no signer or sending capability.
pub(crate) struct DammSellCandidate {
    pub(super) profile: Arc<DammSellMessageProfile>,
    transaction: VersionedTransaction,
    instructions: Vec<Instruction>,
    metadata: crate::domain::sdk_candidate::SdkCandidateMetadata,
}

impl DammSellCandidate {
    pub(super) fn build(
        profile: Arc<DammSellMessageProfile>,
        destination: Option<Pubkey>,
    ) -> Result<Self> {
        profile.recheck()?;
        ensure!(
            profile.context.version == TradeTransactionVersion::V0
                && !profile.context.seed_optimized
                && profile.intent.recent_blockhash != solana_sdk::hash::Hash::default(),
            "DAMM candidate mode or blockhash is invalid"
        );
        let instructions = full_instructions(
            profile.context.wallet,
            profile.context.provider,
            profile.intent.simulate,
            profile.intent.tip_sol,
            destination,
            &profile.protocol,
        )?;
        let transaction = compile_candidate(
            profile.context.wallet,
            &instructions,
            profile.intent.recent_blockhash,
        )?;
        let message = transaction.message.serialize();
        let metadata = super::metadata::capture(&profile, destination, &message)?;
        super::metadata::validate_candidate(&metadata, &message)?;
        Ok(Self {
            profile,
            transaction,
            instructions,
            metadata,
        })
    }

    pub(crate) fn metadata(&self) -> &crate::domain::sdk_candidate::SdkCandidateMetadata {
        &self.metadata
    }

    pub(super) fn validate_full(&self, actual: &[Instruction]) -> Result<()> {
        ensure!(
            actual == self.instructions,
            "DAMM final instructions or fee budget differ from the frozen profile"
        );
        super::metadata::validate_candidate(self.metadata(), &self.message_bytes())
    }

    pub(crate) fn message_bytes(&self) -> Vec<u8> {
        self.transaction.message.serialize()
    }

    pub(super) fn transaction(&self) -> &VersionedTransaction {
        &self.transaction
    }
}

pub(super) fn full_instructions(
    wallet: Pubkey,
    provider: SwqosType,
    simulate: bool,
    tip_sol: f64,
    destination: Option<Pubkey>,
    protocol: &[Instruction],
) -> Result<Vec<Instruction>> {
    let mut instructions = Vec::with_capacity(protocol.len() + 3);
    let wants_tip = !simulate && provider != SwqosType::Default && tip_sol > 0.0;
    if wants_tip {
        let destination = destination.context("DAMM final relay tip is missing")?;
        ensure!(
            tip_accounts(provider).contains(&destination),
            "DAMM final relay tip does not belong to the selected provider"
        );
        let lamports = (tip_sol * 1_000_000_000.0).min(u64::MAX as f64).round() as u64;
        instructions.push(solana_system_interface::instruction::transfer(
            &wallet,
            &destination,
            lamports,
        ));
    } else {
        ensure!(
            destination.is_none(),
            "DAMM candidate has an unexpected relay tip"
        );
    }
    extend_compute_budget_instructions(&mut instructions, 1_000, 300_000);
    instructions.extend_from_slice(protocol);
    Ok(instructions)
}

pub(super) fn compile_candidate(
    wallet: Pubkey,
    instructions: &[Instruction],
    hash: solana_sdk::hash::Hash,
) -> Result<VersionedTransaction> {
    let message = compile_message(
        &wallet,
        instructions,
        &[],
        hash,
        TradeTransactionVersion::V0,
        v1::TransactionConfig::empty(),
    )?;
    ensure!(
        message.header().num_required_signatures == 1
            && message.static_account_keys().first() == Some(&wallet),
        "DAMM final message requires an unexpected signer or payer"
    );
    validate_compiled_message(wallet, instructions, hash, &message)?;
    let transaction = VersionedTransaction {
        signatures: vec![Signature::default()],
        message,
    };
    ensure!(
        bincode::serialized_size(&transaction)? <= 1232,
        "DAMM final transaction exceeds the V0 wire limit"
    );
    Ok(transaction)
}

fn validate_compiled_message(
    wallet: Pubkey,
    instructions: &[Instruction],
    hash: solana_sdk::hash::Hash,
    actual: &VersionedMessage,
) -> Result<()> {
    // This V0-without-ALT profile is Legacy on the pinned SDK. A wire-format
    // change needs an explicit profile migration, not an automatic fallback.
    let expected = VersionedMessage::Legacy(Message::new_with_blockhash(
        instructions,
        Some(&wallet),
        &hash,
    ));
    ensure!(
        *actual == expected,
        "DAMM compiled message differs from the canonical frozen instructions"
    );
    Ok(())
}

pub(super) fn tip_accounts(provider: SwqosType) -> &'static [Pubkey] {
    match provider {
        SwqosType::Jito => JITO_TIP_ACCOUNTS,
        SwqosType::NextBlock => NEXTBLOCK_TIP_ACCOUNTS,
        SwqosType::ZeroSlot => ZEROSLOT_TIP_ACCOUNTS,
        SwqosType::Temporal => NOZOMI_TIP_ACCOUNTS,
        SwqosType::Bloxroute => BLOX_TIP_ACCOUNTS,
        SwqosType::Node1 => NODE1_TIP_ACCOUNTS,
        SwqosType::FlashBlock => FLASHBLOCK_TIP_ACCOUNTS,
        SwqosType::BlockRazor => BLOCKRAZOR_TIP_ACCOUNTS,
        SwqosType::Astralane => ASTRALANE_TIP_ACCOUNTS,
        SwqosType::Stellium => STELLIUM_TIP_ACCOUNTS,
        SwqosType::Lightspeed => LIGHTSPEED_TIP_ACCOUNTS,
        SwqosType::Soyas => SOYAS_TIP_ACCOUNTS,
        SwqosType::Speedlanding => SPEEDLANDING_TIP_ACCOUNTS,
        SwqosType::Helius => HELIUS_TIP_ACCOUNTS,
        SwqosType::Solami => SOLAMI_TIP_ACCOUNTS,
        SwqosType::LunarLander => LUNARLANDER_TIP_ACCOUNTS,
        SwqosType::Glaive => GLAIVE_TIP_ACCOUNTS,
        SwqosType::Default => &[],
    }
}

#[cfg(test)]
mod tests;
