use super::quotes::{new_quote_id, now_ms, QuoteStore};
use crate::{
    api::dto::sol_buy::{BoundSolBuyQuote, SolBuyQuoteRequest, SolBuyQuoteResponse},
    application::ports::quote_cache::{QuoteBinding, QuoteCache},
};

// Compatibility mapping only; cache policy and storage are project-owned.
impl QuoteStore {
    pub fn insert(
        &self,
        request: &SolBuyQuoteRequest,
        wallet_pubkey: String,
        quote: SolBuyQuoteResponse,
    ) -> anyhow::Result<BoundSolBuyQuote> {
        let now = now_ms();
        self.bind_at(
            QuoteBinding {
                wallet_id: &request.wallet_id,
                wallet_pubkey,
                slippage_bps: request.slippage_bps,
            },
            quote.into(),
            now,
            new_quote_id,
        )
        .map(Into::into)
    }

    pub fn get(
        &self,
        quote_id: &str,
        wallet_id: &str,
        wallet_pubkey: &str,
    ) -> anyhow::Result<BoundSolBuyQuote> {
        self.read(quote_id, wallet_id, wallet_pubkey)
            .map(Into::into)
    }
}
