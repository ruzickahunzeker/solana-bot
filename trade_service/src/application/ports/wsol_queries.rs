use std::future::Future;

use crate::{
    application::{
        ports::wsol_conversion::WsolWalletMetadata,
        wsol_queries::{WsolQuoteEstimate, WsolQuoteInput},
        wsol_recovery::WsolRecoveryInput,
        wsol_submission_types::{WsolQuoteSnapshot, WsolSubmissionRecord},
    },
    domain::{
        trade::Address,
        wsol_progress::{WsolObservedOutcome, WsolSignatureObservation},
    },
};

pub(crate) trait WsolQuoteReader: Send + Sync {
    fn estimate(
        &self,
        owner: Address,
        input: &WsolQuoteInput,
    ) -> impl Future<Output = anyhow::Result<WsolQuoteEstimate>> + Send;
}
pub(crate) trait WsolStatusReader: Send + Sync {
    fn lookup(
        &self,
    ) -> impl Future<Output = anyhow::Result<Option<WsolSignatureObservation>>> + Send;
}

/// Read-only runtime: no signer, message, reservation or broadcast capability.
pub(crate) trait WsolQuerySource: Send + Sync {
    type QuoteReader: WsolQuoteReader;
    type StatusReader: WsolStatusReader;
    fn metadata(
        &self,
        wallet_id: &str,
    ) -> impl Future<Output = anyhow::Result<WsolWalletMetadata>> + Send;
    fn quote_reader(&self, wallet_id: &str) -> Option<Self::QuoteReader>;
    fn status_reader(&self, wallet_id: &str, signature: &str)
        -> anyhow::Result<Self::StatusReader>;
}

pub(crate) trait WsolQueryJournal: Send + Sync {
    fn recovery(
        &self,
        input: &WsolRecoveryInput,
    ) -> impl Future<Output = anyhow::Result<Option<WsolSubmissionRecord>>> + Send;
    fn save_quote(
        &self,
        quote: &WsolQuoteSnapshot,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn submission(
        &self,
        id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WsolSubmissionRecord>>> + Send;
    fn record_observation(
        &self,
        expected: &WsolSubmissionRecord,
        status: WsolObservedOutcome,
        error: Option<&str>,
        now: u64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}
