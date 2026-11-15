use crate::{
    application::{
        ports::quote_reconciliation::{QuoteReconciliationRuntime, QuoteReconciliationWallet},
        quote_submission::{FailureKind, SubmissionError},
        submission_reconciler::{self, ReconciliationResult},
    },
    domain::submission::SubmissionKey,
};
use std::{future::Future, pin::Pin, time::Duration};

const STATUS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) type QuoteReconcileFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ReconciliationResult, SubmissionError>> + Send + 'a>>;

pub(crate) trait QuoteReconciliations: Send + Sync {
    fn reconcile<'a>(
        &'a self,
        quote_id: &'a str,
        wallet_id: &'a str,
        idempotency_key: &'a str,
    ) -> QuoteReconcileFuture<'a>;
}

pub(crate) struct QuoteReconciliationService<R> {
    runtime: R,
}

impl<R: QuoteReconciliationRuntime> QuoteReconciliationService<R> {
    pub(crate) fn new(runtime: R) -> Self {
        Self { runtime }
    }
}

impl<R: QuoteReconciliationRuntime> QuoteReconciliations for QuoteReconciliationService<R> {
    fn reconcile<'a>(
        &'a self,
        quote_id: &'a str,
        wallet_id: &'a str,
        idempotency_key: &'a str,
    ) -> QuoteReconcileFuture<'a> {
        Box::pin(async move {
            let key = SubmissionKey::new(wallet_id, idempotency_key).map_err(|error| {
                SubmissionError::new(FailureKind::InvalidRequest, error.to_string())
            })?;
            let wallet = self.runtime.resolve(wallet_id).ok_or_else(|| {
                SubmissionError::new(FailureKind::InvalidRequest, "wallet_id not found")
            })?;
            let journal = self.runtime.journal().ok_or_else(|| {
                SubmissionError::new(
                    FailureKind::Unavailable,
                    "quote submission status requires durable trade persistence",
                )
            })?;
            let reader = wallet.reader();
            submission_reconciler::reconcile(
                &journal,
                &reader,
                &key,
                &wallet.pubkey().to_string(),
                quote_id,
                STATUS_LOOKUP_TIMEOUT,
            )
            .await
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
