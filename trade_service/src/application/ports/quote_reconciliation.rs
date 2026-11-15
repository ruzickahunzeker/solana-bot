use crate::{
    application::ports::reconciliation::{ReconciliationJournal, SignatureStatusReader},
    domain::trade::Address,
};

pub(crate) trait QuoteReconciliationWallet: Send + Sync {
    type Reader: SignatureStatusReader + Send;

    fn reader(&self) -> Self::Reader;
    fn pubkey(&self) -> Address;
}

/// Existing handles only; reconciliation cannot initialize storage or authorize submission.
pub(crate) trait QuoteReconciliationRuntime: Send + Sync {
    type Wallet: QuoteReconciliationWallet;
    type Journal: ReconciliationJournal + Send;

    fn resolve(&self, wallet_id: &str) -> Option<Self::Wallet>;
    fn journal(&self) -> Option<Self::Journal>;
}
