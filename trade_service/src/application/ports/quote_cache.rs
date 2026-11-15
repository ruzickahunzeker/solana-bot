use crate::domain::quote::{BoundQuote, QuoteEstimate};

/// Cache binding metadata, not proof of wallet ownership or an executable quote.
pub(crate) struct QuoteBinding<'a> {
    pub wallet_id: &'a str,
    pub wallet_pubkey: String,
    pub slippage_bps: u16,
}

pub(crate) trait QuoteCache: Send + Sync {
    fn bind(&self, binding: QuoteBinding<'_>, quote: QuoteEstimate) -> anyhow::Result<BoundQuote>;
    fn read(
        &self,
        quote_id: &str,
        wallet_id: &str,
        wallet_pubkey: &str,
    ) -> anyhow::Result<BoundQuote>;
}
