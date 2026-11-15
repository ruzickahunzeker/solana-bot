use crate::domain::quote::QuoteEstimate;
use std::future::Future;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SolBuyQuoteInput<'a> {
    pub wallet_id: &'a str,
    pub mint: &'a str,
    pub amount_lamports: u64,
    pub slippage_bps: u16,
}

/// Quote acquisition only: no wallet, signer, RPC submission or cache access.
pub(crate) trait SolBuyQuoteSource: Send + Sync {
    type Request: Send + Sync;

    fn map_request(&self, input: SolBuyQuoteInput<'_>) -> Self::Request;
    fn validate(&self, request: &Self::Request) -> anyhow::Result<()>;
    fn fetch(
        &self,
        request: &Self::Request,
    ) -> impl Future<Output = anyhow::Result<QuoteEstimate>> + Send;
}
