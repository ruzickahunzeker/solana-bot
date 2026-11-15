use crate::domain::{
    quote::{BoundQuote, QuotePreflight},
    trade::Address,
};
use std::{future::Future, pin::Pin};

pub(crate) type QuotePreflightFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<QuotePreflight>> + Send + 'a>>;

/// A retained, resolved wallet with read-only quote inspection, not a signing interface.
pub(crate) trait QuotePreflightWallet: Send + Sync {
    fn pubkey(&self) -> Address;
    fn inspect<'a>(&'a self, bound: &'a BoundQuote) -> QuotePreflightFuture<'a>;
}

pub(crate) trait QuotePreflightBackend: Send + Sync {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn QuotePreflightWallet>>;
}
