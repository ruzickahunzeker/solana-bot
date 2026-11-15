use crate::{
    application::ports::{
        quote_cache::{QuoteBinding, QuoteCache},
        sol_buy_quotes::{SolBuyQuoteInput, SolBuyQuoteSource},
        wallet_identity::LoadedWalletDirectory,
    },
    domain::quote::BoundQuote,
};
use std::{future::Future, pin::Pin, sync::Arc};

#[derive(Debug)]
pub(crate) enum QuoteQueryError {
    InvalidRequest(anyhow::Error),
    WalletMissing,
    Backend(anyhow::Error),
    CacheWrite(anyhow::Error),
    CacheRead(anyhow::Error),
}

pub(crate) type QuoteQueryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<BoundQuote, QuoteQueryError>> + Send + 'a>>;

pub(crate) trait SolBuyQuoteQueries: Send + Sync {
    fn create<'a>(&'a self, input: SolBuyQuoteInput<'a>) -> QuoteQueryFuture<'a>;
    fn get(&self, quote_id: &str, wallet_id: &str) -> Result<BoundQuote, QuoteQueryError>;
}

pub(crate) struct SolBuyQuoteService<S> {
    source: S,
    identities: Arc<dyn LoadedWalletDirectory>,
    cache: Arc<dyn QuoteCache>,
}

impl<S> SolBuyQuoteService<S> {
    pub(crate) fn new(
        source: S,
        identities: Arc<dyn LoadedWalletDirectory>,
        cache: Arc<dyn QuoteCache>,
    ) -> Self {
        Self {
            source,
            identities,
            cache,
        }
    }
}

impl<S: SolBuyQuoteSource> SolBuyQuoteQueries for SolBuyQuoteService<S> {
    fn create<'a>(&'a self, input: SolBuyQuoteInput<'a>) -> QuoteQueryFuture<'a> {
        Box::pin(async move {
            let request = self.source.map_request(input);
            self.source
                .validate(&request)
                .map_err(QuoteQueryError::InvalidRequest)?;
            let wallet = self
                .identities
                .lookup(input.wallet_id)
                .ok_or(QuoteQueryError::WalletMissing)?;
            let quote = self
                .source
                .fetch(&request)
                .await
                .map_err(QuoteQueryError::Backend)?;
            self.cache
                .bind(
                    QuoteBinding {
                        wallet_id: input.wallet_id,
                        wallet_pubkey: wallet.pubkey().to_string(),
                        slippage_bps: input.slippage_bps,
                    },
                    quote,
                )
                .map_err(QuoteQueryError::CacheWrite)
        })
    }

    fn get(&self, quote_id: &str, wallet_id: &str) -> Result<BoundQuote, QuoteQueryError> {
        let wallet = self
            .identities
            .lookup(wallet_id)
            .ok_or(QuoteQueryError::WalletMissing)?;
        self.cache
            .read(quote_id, wallet_id, &wallet.pubkey().to_string())
            .map_err(QuoteQueryError::CacheRead)
    }
}

#[cfg(test)]
mod tests;
