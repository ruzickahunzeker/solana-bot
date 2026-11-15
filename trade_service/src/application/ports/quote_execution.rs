use crate::domain::{
    quote::{BoundQuote, QuotePreflight},
    submission::SignedSubmission,
};
use anyhow::Result;
use std::future::Future;

pub(crate) trait QuoteSubmissionBackend: Sync {
    type Prepared: Send;
    fn ensure_live_allowed(&self) -> Result<()>;
    fn prepare(
        &self,
        bound: &BoundQuote,
    ) -> impl Future<Output = Result<(Self::Prepared, QuotePreflight)>> + Send;
    fn sign(&self, prepared: Self::Prepared) -> Result<SignedSubmission>;
    fn broadcast(&self, signed: &SignedSubmission) -> impl Future<Output = Result<String>> + Send;
}

pub(crate) trait OrdinaryQuoteBackend: QuoteSubmissionBackend {
    fn prepared_message(&self, prepared: &Self::Prepared) -> anyhow::Result<Vec<u8>>;
    fn signed_message(
        &self,
        signed: &crate::domain::submission::SignedSubmission,
    ) -> anyhow::Result<Vec<u8>>;
}
