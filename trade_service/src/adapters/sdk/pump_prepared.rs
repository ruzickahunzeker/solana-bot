use super::{
    prepared_candidate::{validate_candidate, validate_signed_candidate},
    pump_curve_evidence::PumpNativeBankEvidence,
};
use crate::{
    application::{
        ordinary_sdk_submission::SdkSigningPermit, sdk_preparation::validate_origin,
        trade_admission::AdmittedTrade,
    },
    domain::{
        pump_native_candidate::PumpNativeCandidateRecord, sdk_prepared_record::SdkPreparedRecord,
        submission::SignedSubmission,
    },
};
use anyhow::{ensure, Result};
use sol_trade_sdk::{common::TradeTransactionVersion, swqos::SwqosType};
use solana_sdk::{
    hash::Hash,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PumpPreparedContext {
    pub wallet: Pubkey,
    pub provider: SwqosType,
    pub version: TradeTransactionVersion,
    pub seed_optimized: bool,
    pub blockhash: Hash,
    pub tip_destination: Option<Pubkey>,
}

pub(crate) struct PumpPreparedCandidate<'a> {
    evidence: PumpNativeBankEvidence<'a>,
    metadata: SdkPreparedRecord,
    message: Vec<u8>,
    context: PumpPreparedContext,
}

pub(crate) struct PumpSignedCandidate<'a> {
    candidate: PumpPreparedCandidate<'a>,
    signed: SignedSubmission,
}

impl<'a> PumpPreparedCandidate<'a> {
    pub(crate) fn new(
        evidence: PumpNativeBankEvidence<'a>,
        context: PumpPreparedContext,
        execution: &AdmittedTrade,
    ) -> Result<Self> {
        let message = evidence.packet()?.message.serialize();
        let metadata = SdkPreparedRecord::PumpPrepared(evidence.record()?.into_prepared(&message)?);
        let candidate = Self {
            evidence,
            metadata,
            message,
            context,
        };
        candidate.authorize_live(execution, context)?;
        Ok(candidate)
    }

    pub(crate) fn metadata(&self) -> &SdkPreparedRecord {
        &self.metadata
    }
    pub(crate) fn message(&self) -> &[u8] {
        &self.message
    }

    pub(crate) fn authorize_live(
        &self,
        execution: &AdmittedTrade,
        context: PumpPreparedContext,
    ) -> Result<()> {
        ensure!(
            self.context == context
                && context.version == TradeTransactionVersion::V0
                && !context.seed_optimized,
            "native Pump original context changed or is unsupported"
        );
        self.evidence.ensure_execution(execution)?;
        let SdkPreparedRecord::PumpPrepared(record) = &self.metadata else {
            anyhow::bail!("native Pump prepared tag is missing")
        };
        let d = record.details();
        ensure!(
            d.wallet == context.wallet.to_bytes()
                && d.relay == context.provider.as_str()
                && d.recent_blockhash == context.blockhash.to_bytes()
                && d.tip_destination == context.tip_destination.map(|key| key.to_bytes()),
            "native Pump original wallet, relay, hash or tip changed"
        );
        validate_origin(execution, &self.metadata, &self.message)?;
        let historical = PumpNativeCandidateRecord::new(d.clone(), &self.message)?;
        self.evidence.check_record(&historical, &self.message)?;
        validate_candidate(&self.metadata, &self.message)
    }

    pub(crate) fn sign_original(
        self,
        permit: SdkSigningPermit<'_>,
        context: PumpPreparedContext,
        acquire: impl FnOnce() -> Result<Arc<Keypair>>,
    ) -> Result<PumpSignedCandidate<'a>> {
        permit.validate(&self.metadata, &self.message)?;
        self.authorize_live(permit.execution(), context)?;
        let signer = acquire()?;
        ensure!(
            signer.pubkey() == context.wallet,
            "native Pump signer differs from the original wallet"
        );
        self.authorize_live(permit.execution(), context)?;
        let transaction = VersionedTransaction::try_new(
            self.evidence.packet()?.message.clone(),
            &[signer.as_ref()],
        )?;
        let signature = transaction.signatures[0].to_string();
        let signed = (|| -> Result<SignedSubmission> {
            self.authorize_live(permit.execution(), context)?;
            let signed = SignedSubmission {
                signature: signature.clone(),
                bytes: bincode::serialize(&transaction)?,
            };
            validate_signed_candidate(&self.metadata, &self.message, &signed)?;
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
        Ok(PumpSignedCandidate {
            candidate: self,
            signed,
        })
    }
}

impl PumpSignedCandidate<'_> {
    pub(crate) fn submission(&self) -> &SignedSubmission {
        &self.signed
    }
    pub(crate) fn recheck(
        &self,
        execution: &AdmittedTrade,
        context: PumpPreparedContext,
    ) -> Result<()> {
        self.candidate.authorize_live(execution, context)?;
        validate_signed_candidate(
            self.candidate.metadata(),
            self.candidate.message(),
            &self.signed,
        )
    }
    pub(crate) fn check_submission(
        &self,
        execution: &AdmittedTrade,
        context: PumpPreparedContext,
        signed: &SignedSubmission,
    ) -> Result<()> {
        self.recheck(execution, context)?;
        ensure!(
            self.signed.signature == signed.signature && self.signed.bytes == signed.bytes,
            "native Pump broadcast bytes differ from the original artifact"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
