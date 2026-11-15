use crate::{
    application::ports::{quote_cache::QuoteCache, quote_preflight::QuotePreflightBackend},
    domain::quote::QuotePreflight,
};
use std::{future::Future, pin::Pin, sync::Arc};

#[derive(Debug)]
pub(crate) enum QuotePreflightError {
    WalletMissing,
    Cache(anyhow::Error),
    Backend(anyhow::Error),
}
pub(crate) type PreflightQueryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<QuotePreflight, QuotePreflightError>> + Send + 'a>>;

pub(crate) trait QuotePreflights: Send + Sync {
    fn inspect<'a>(&'a self, quote_id: &'a str, wallet_id: &'a str) -> PreflightQueryFuture<'a>;
}

pub(crate) struct QuotePreflightService {
    backend: Arc<dyn QuotePreflightBackend>,
    cache: Arc<dyn QuoteCache>,
}
impl QuotePreflightService {
    pub(crate) fn new(backend: Arc<dyn QuotePreflightBackend>, cache: Arc<dyn QuoteCache>) -> Self {
        Self { backend, cache }
    }
}
impl QuotePreflights for QuotePreflightService {
    fn inspect<'a>(&'a self, quote_id: &'a str, wallet_id: &'a str) -> PreflightQueryFuture<'a> {
        Box::pin(async move {
            let wallet = self
                .backend
                .resolve(wallet_id)
                .ok_or(QuotePreflightError::WalletMissing)?;
            let bound = self
                .cache
                .read(quote_id, wallet_id, &wallet.pubkey().to_string())
                .map_err(QuotePreflightError::Cache)?;
            let report = wallet
                .inspect(&bound)
                .await
                .map_err(QuotePreflightError::Backend)?;
            self.cache
                .read(quote_id, wallet_id, &wallet.pubkey().to_string())
                .map_err(QuotePreflightError::Cache)?;
            Ok(report)
        })
    }
}

#[cfg(test)]
mod tests;
