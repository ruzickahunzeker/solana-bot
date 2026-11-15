use super::*;
#[cfg(test)]
pub(super) use crate::adapters::sdk::damm_instructions::tip_accounts;
#[cfg(test)]
use crate::adapters::sdk::damm_instructions::validate_compiled_message;
pub(super) use crate::adapters::sdk::damm_instructions::{compile_candidate, full_instructions};
use solana_sdk::transaction::VersionedTransaction;
#[cfg(test)]
use solana_sdk::{
    message::{Message, VersionedMessage},
    signature::Signature,
};

/// Owned immutable nominal transaction. It contains no signer or sending capability.
pub(crate) struct DammSellCandidate {
    pub(super) profile: Arc<DammSellMessageProfile>,
    transaction: VersionedTransaction,
    pub(super) instructions: Vec<Instruction>,
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

    #[cfg(test)]
    pub(crate) fn expire_holding_for_test(&mut self) {
        let profile =
            Arc::get_mut(&mut self.profile).expect("test requires a uniquely owned profile");
        profile.holding = Arc::new(profile.holding.expired_copy_for_test());
    }
}

#[cfg(test)]
mod tests;
