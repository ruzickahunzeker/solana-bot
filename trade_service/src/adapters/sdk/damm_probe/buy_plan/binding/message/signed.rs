use super::*;
use crate::{
    application::{
        ordinary_sdk_submission::SdkSigningPermit, sdk_preparation::validate_origin,
        trade_admission::AdmittedTrade,
    },
    domain::submission::SignedSubmission,
};
use solana_sdk::signature::{Keypair, Signer};

/// Only the original live candidate can produce this artifact. Stored metadata cannot restore it.
pub(crate) struct DammBuySignedCandidate {
    candidate: DammBuyCandidate,
    signed: SignedSubmission,
    request_id: String,
    request_hash: String,
}

impl DammBuyCandidate {
    pub(crate) fn authorize_live(
        &self,
        execution: &AdmittedTrade,
        context: DammBuyContext,
    ) -> Result<()> {
        let quote = &self.profile.quote;
        ensure!(
            !quote.intent.simulate && quote.context == context,
            "DAMM buy original live context is missing or changed"
        );
        crate::adapters::sdk::ordinary_lanes::ensure_requested_provider(
            quote.intent.use_mev,
            quote.intent.mev_provider,
            context.provider,
        )?;
        validate_origin(execution, self.metadata(), &self.message_bytes())?;
        self.validate_full(&self.instructions)
    }

    pub(crate) fn sign_original(
        self,
        permit: SdkSigningPermit<'_>,
        context: DammBuyContext,
        acquire_signer: impl FnOnce() -> Result<Arc<Keypair>>,
    ) -> Result<DammBuySignedCandidate> {
        permit.validate(self.metadata(), &self.message_bytes())?;
        self.authorize_live(permit.execution(), context)?;
        let request_id = permit.execution().reservation().request_id().to_owned();
        let request_hash = permit.execution().reservation().request_hash().to_owned();
        // No await or high-level SDK worker is allowed between authorization and key acquisition.
        let signer = acquire_signer()?;
        ensure!(
            signer.pubkey() == context.wallet,
            "DAMM buy signer differs from the original wallet"
        );
        self.validate_full(&self.instructions)?;
        let transaction =
            VersionedTransaction::try_new(self.transaction.message.clone(), &[signer.as_ref()])?;
        let signature = transaction.signatures[0].to_string();
        let signed = (|| -> Result<SignedSubmission> {
            let signed = SignedSubmission {
                signature: signature.clone(),
                bytes: bincode::serialize(&transaction)?,
            };
            crate::adapters::sdk::prepared_candidate::validate_signed_candidate(
                self.metadata(),
                &self.message_bytes(),
                &signed,
            )?;
            Ok(signed)
        })()
        .map_err(|error| {
            anyhow::Error::new(
                crate::application::trade_execution::UncertainSdkSubmission::new(
                    vec![signature],
                    crate::sanitize::redact_urls(&error.to_string()),
                ),
            )
        })?;
        Ok(DammBuySignedCandidate {
            candidate: self,
            signed,
            request_id,
            request_hash,
        })
    }
}

impl DammBuySignedCandidate {
    #[cfg(test)]
    pub(crate) fn expire_for_test(&mut self) {
        self.candidate.expire_for_test();
    }
    pub(crate) fn submission(&self) -> &SignedSubmission {
        &self.signed
    }

    pub(crate) fn recheck(&self, execution: &AdmittedTrade, context: DammBuyContext) -> Result<()> {
        ensure!(
            self.request_id == execution.reservation().request_id()
                && self.request_hash == execution.reservation().request_hash(),
            "DAMM buy signed artifact belongs to another original order"
        );
        self.candidate.authorize_live(execution, context)?;
        crate::adapters::sdk::prepared_candidate::validate_signed_candidate(
            self.candidate.metadata(),
            &self.candidate.message_bytes(),
            &self.signed,
        )
    }

    pub(crate) fn check_submission(
        &self,
        execution: &AdmittedTrade,
        context: DammBuyContext,
        signed: &SignedSubmission,
    ) -> Result<()> {
        self.recheck(execution, context)?;
        ensure!(
            signed.signature == self.signed.signature && signed.bytes == self.signed.bytes,
            "DAMM buy broadcast packet differs from the original signed artifact"
        );
        Ok(())
    }
}
