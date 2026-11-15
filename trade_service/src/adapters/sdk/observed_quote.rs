use crate::{
    adapters::{
        rpc::ordinary_observation::SingleShotStatusReader,
        sdk::quote_submission::{PreparedQuoteTransaction, SdkQuoteSubmission},
    },
    application::ports::{
        quote_execution::{OrdinaryQuoteBackend, QuoteSubmissionBackend},
        reconciliation::SignatureStatusReader,
    },
    domain::{
        quote::{BoundQuote, QuotePreflight},
        reconciliation::ChainObservation,
        submission::SignedSubmission,
    },
    sdk_client::SdkTradeClient,
};

/// Only status reads change; all prepared/sign/send operations retain the original SDK adapter.
pub(crate) struct ObservedQuote<'a> {
    submission: SdkQuoteSubmission<'a>,
    reader: &'a SingleShotStatusReader,
}
impl<'a> ObservedQuote<'a> {
    pub(crate) fn new(client: &'a SdkTradeClient, reader: &'a SingleShotStatusReader) -> Self {
        Self {
            submission: SdkQuoteSubmission(client),
            reader,
        }
    }
}
impl QuoteSubmissionBackend for ObservedQuote<'_> {
    type Prepared = PreparedQuoteTransaction;
    fn ensure_live_allowed(&self) -> anyhow::Result<()> {
        self.submission.ensure_live_allowed()
    }
    async fn prepare(
        &self,
        bound: &BoundQuote,
    ) -> anyhow::Result<(Self::Prepared, QuotePreflight)> {
        self.submission.prepare(bound).await
    }
    fn sign(&self, prepared: Self::Prepared) -> anyhow::Result<SignedSubmission> {
        self.submission.sign(prepared)
    }
    async fn broadcast(&self, signed: &SignedSubmission) -> anyhow::Result<String> {
        self.submission.broadcast(signed).await
    }
}
impl OrdinaryQuoteBackend for ObservedQuote<'_> {
    fn prepared_message(&self, prepared: &Self::Prepared) -> anyhow::Result<Vec<u8>> {
        self.submission.prepared_message(prepared)
    }
    fn signed_message(&self, signed: &SignedSubmission) -> anyhow::Result<Vec<u8>> {
        self.submission.signed_message(signed)
    }
}
impl SignatureStatusReader for ObservedQuote<'_> {
    async fn lookup_signature(&self, signature: &str) -> anyhow::Result<Option<ChainObservation>> {
        self.reader.lookup_foreground(signature).await
    }
}
