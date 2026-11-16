use super::{DammSellCandidate, DammSellContext};
use crate::application::{
    ordinary_sdk_submission::SdkSigningPermit, trade_admission::AdmittedTrade,
};
use crate::domain::{sdk_prepared_record::SdkPreparedRecord, submission::SignedSubmission};
use anyhow::{ensure, Result};
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::VersionedTransaction;
use std::sync::Arc;

/// Original live candidate and signed bytes stay together; neither field is mutable outside SDK.
pub(crate) struct DammSignedCandidate {
    candidate: DammSellCandidate,
    signed: SignedSubmission,
    request_id: String,
    request_hash: String,
}

impl DammSellCandidate {
    pub(crate) fn authorize_live(
        &self,
        execution: &AdmittedTrade,
        context: DammSellContext,
    ) -> Result<()> {
        ensure!(
            !self.profile.intent.simulate && self.profile.context == context,
            "DAMM original live context is missing or changed"
        );
        let intent = &self.profile.intent;
        use crate::domain::trade::MevProvider;
        use sol_trade_sdk::swqos::SwqosType;
        let provider_allowed = match (intent.use_mev, intent.mev_provider) {
            (Some(false), Some(requested))
                if !matches!(requested, MevProvider::Auto | MevProvider::Default) =>
            {
                false
            }
            (Some(true), Some(MevProvider::Default)) => false,
            (Some(false), _) | (_, Some(MevProvider::Default)) => {
                context.provider == SwqosType::Default
            }
            (_, Some(requested)) if requested != MevProvider::Auto => {
                crate::sdk_client::provider_swqos_type(requested) == Some(context.provider)
            }
            (Some(true), _) => context.provider != SwqosType::Default,
            _ => true,
        };
        ensure!(
            provider_allowed,
            "DAMM selected provider differs from the original MEV policy"
        );
        let message = self.message_bytes();
        crate::application::sdk_preparation::validate_origin(execution, self.metadata(), &message)?;
        self.profile.recheck()?;
        ensure!(
            super::metadata::capture(
                &self.profile,
                self.metadata()
                    .sell()?
                    .details()
                    .tip_destination
                    .map(solana_sdk::pubkey::Pubkey::new_from_array),
                &message
            )? == *self.metadata(),
            "DAMM live candidate metadata differs from its original evidence"
        );
        self.validate_full(&self.instructions)?;
        super::validate_candidate(self.metadata(), &message)
    }

    pub(crate) fn sign_original(
        self,
        permit: SdkSigningPermit<'_>,
        context: DammSellContext,
        acquire_signer: impl FnOnce() -> Result<Arc<Keypair>>,
    ) -> Result<DammSignedCandidate> {
        permit.validate(self.metadata(), &self.message_bytes())?;
        self.authorize_live(permit.execution(), context)?;
        let request_id = permit.execution().reservation().request_id().to_owned();
        let request_hash = permit.execution().reservation().request_hash().to_owned();
        // No await or SDK worker may run between this evidence check and key acquisition.
        let signer = acquire_signer()?;
        ensure!(
            signer.pubkey() == context.wallet,
            "DAMM signer differs from the original wallet"
        );
        self.profile.recheck()?;
        let transaction =
            VersionedTransaction::try_new(self.transaction().message.clone(), &[signer.as_ref()])?;
        let signature = transaction.signatures[0].to_string();
        let signed = (|| -> Result<SignedSubmission> {
            let signed = SignedSubmission {
                signature: signature.clone(),
                bytes: bincode::serialize(&transaction)?,
            };
            validate_signed_candidate(self.metadata(), &self.message_bytes(), &signed)?;
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
        Ok(DammSignedCandidate {
            candidate: self,
            signed,
            request_id,
            request_hash,
        })
    }
}

impl DammSignedCandidate {
    pub(crate) fn submission(&self) -> &SignedSubmission {
        &self.signed
    }

    #[cfg(test)]
    pub(crate) fn expire_holding_for_test(&mut self) {
        self.candidate.expire_holding_for_test();
    }

    pub(crate) fn recheck(
        &self,
        execution: &AdmittedTrade,
        context: DammSellContext,
    ) -> Result<()> {
        ensure!(
            self.request_id == execution.reservation().request_id()
                && self.request_hash == execution.reservation().request_hash(),
            "DAMM signed artifact belongs to a different original order"
        );
        self.candidate.authorize_live(execution, context)?;
        validate_signed_candidate(
            self.candidate.metadata(),
            &self.candidate.message_bytes(),
            &self.signed,
        )
    }

    pub(crate) fn check_submission(
        &self,
        execution: &AdmittedTrade,
        context: DammSellContext,
        signed: &SignedSubmission,
    ) -> Result<()> {
        self.recheck(execution, context)?;
        ensure!(
            self.signed.signature == signed.signature && self.signed.bytes == signed.bytes,
            "DAMM broadcast packet differs from the original signed artifact"
        );
        Ok(())
    }
}

/// Evidence validation only: never signs, refreshes a quote, or authorizes execution.
pub(crate) fn validate_signed_candidate(
    metadata: &SdkPreparedRecord,
    message: &[u8],
    signed: &SignedSubmission,
) -> Result<()> {
    metadata.sell()?;
    super::super::prepared_candidate::validate_signed_candidate(metadata, message, signed)
}
