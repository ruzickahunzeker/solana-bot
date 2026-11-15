use crate::application::ports::quote_submit::QuoteSubmissionWallet;
use crate::application::{
    ports::{quote_cache::QuoteCache, quote_submit::QuoteSubmissionRuntime},
    quote_submission::{self, FailureKind, SubmissionError},
    quote_submission_types::{QuoteSubmissionCommand, QuoteSubmissionResult},
};
use std::{future::Future, pin::Pin, sync::Arc};

pub(crate) type QuoteSubmitFuture<'a> =
    Pin<Box<dyn Future<Output = Result<QuoteSubmissionResult, SubmissionError>> + Send + 'a>>;

pub(crate) trait SolBuySubmissions: Send + Sync {
    fn submit<'a>(
        &'a self,
        quote_id: &'a str,
        request: &'a QuoteSubmissionCommand,
    ) -> QuoteSubmitFuture<'a>;
}

pub(crate) struct SolBuySubmissionService<R> {
    runtime: R,
    cache: Arc<dyn QuoteCache>,
}

impl<R: QuoteSubmissionRuntime> SolBuySubmissionService<R> {
    pub(crate) fn new(runtime: R, cache: Arc<dyn QuoteCache>) -> Self {
        Self { runtime, cache }
    }
}

impl<R: QuoteSubmissionRuntime> SolBuySubmissions for SolBuySubmissionService<R> {
    fn submit<'a>(
        &'a self,
        quote_id: &'a str,
        request: &'a QuoteSubmissionCommand,
    ) -> QuoteSubmitFuture<'a> {
        Box::pin(async move {
            let wallet = self.runtime.resolve(&request.wallet_id).ok_or_else(|| {
                SubmissionError::new(FailureKind::InvalidRequest, "wallet_id not found")
            })?;
            let key = quote_submission::validate(request)?;
            let journal = self.runtime.journal();
            if !request.simulate {
                let journal = journal.as_ref().ok_or_else(|| {
                    SubmissionError::new(
                        FailureKind::Unavailable,
                        "live SOL quote submission requires durable trade persistence",
                    )
                })?;
                // A durable replay must remain available after the ephemeral cache is lost.
                if let Some(result) = quote_submission::existing(
                    journal,
                    &key,
                    &wallet.pubkey().to_string(),
                    quote_id,
                )
                .await?
                {
                    return Ok(result);
                }
            }
            let bound = self
                .cache
                .read(quote_id, &request.wallet_id, &wallet.pubkey().to_string())
                .map_err(|error| SubmissionError::new(FailureKind::Expired, error.to_string()))?;
            // Keep the core's own validation and second history lookup; they guard races.
            quote_submission::execute(journal.as_ref(), &wallet, &bound, request).await
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
