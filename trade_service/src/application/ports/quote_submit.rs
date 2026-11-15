use crate::{
    application::ports::{
        quote_execution::QuoteSubmissionBackend, submission_journal::SubmissionJournal,
    },
    domain::trade::Address,
};

pub(crate) trait QuoteSubmissionWallet: QuoteSubmissionBackend + Send {
    fn pubkey(&self) -> Address;
}

/// Resolves existing handles only; journal acquisition must not initialize or recover storage.
pub(crate) trait QuoteSubmissionRuntime: Send + Sync {
    type Wallet: QuoteSubmissionWallet;
    type Journal: SubmissionJournal + Send;

    fn resolve(&self, wallet_id: &str) -> Option<Self::Wallet>;
    fn journal(&self) -> Option<Self::Journal>;
}
