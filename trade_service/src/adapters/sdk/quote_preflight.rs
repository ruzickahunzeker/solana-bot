use crate::{
    aggregator_quote,
    application::ports::quote_preflight::{
        QuotePreflightBackend, QuotePreflightFuture, QuotePreflightWallet,
    },
    domain::{quote::BoundQuote, trade::Address},
    sdk_client::SdkTradeClient,
    wallets::WalletRegistry,
};
use std::sync::Arc;

pub(crate) struct SdkQuotePreflightBackend {
    wallets: Arc<WalletRegistry>,
}
impl SdkQuotePreflightBackend {
    pub(crate) fn new(wallets: Arc<WalletRegistry>) -> Self {
        Self { wallets }
    }
}
struct ResolvedPreflightWallet {
    client: Arc<SdkTradeClient>,
}
impl QuotePreflightBackend for SdkQuotePreflightBackend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn QuotePreflightWallet>> {
        self.wallets.get(wallet_id).map(|client| {
            Box::new(ResolvedPreflightWallet { client }) as Box<dyn QuotePreflightWallet>
        })
    }
}
impl QuotePreflightWallet for ResolvedPreflightWallet {
    fn pubkey(&self) -> Address {
        Address::from_bytes(self.client.wallet().to_bytes())
    }
    fn inspect<'a>(&'a self, bound: &'a BoundQuote) -> QuotePreflightFuture<'a> {
        Box::pin(async move {
            let (_, report) = aggregator_quote::prepare_transaction(bound, &self.client).await?;
            Ok(report)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quote_preflight_backend_keeps_existing_registry_without_initializing_missing_wallets()
    {
        let state = crate::http_contract_tests::state().await;
        let backend = SdkQuotePreflightBackend::new(state.wallets.clone());
        assert!(Arc::ptr_eq(&backend.wallets, &state.wallets));
        assert!(backend.resolve("fixture-wallet").is_none());
        assert!(backend.resolve("").is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
