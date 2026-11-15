use anyhow::{bail, Result};
use dashmap::DashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::{
    application::ports::quote_cache::{QuoteBinding, QuoteCache},
    domain::quote::{BoundQuote, QuoteEstimate},
};

const QUOTE_TTL_MS: u64 = 15_000;

#[derive(Default)]
pub struct QuoteStore {
    quotes: DashMap<String, BoundQuote>,
}

impl QuoteCache for QuoteStore {
    fn bind(&self, binding: QuoteBinding<'_>, quote: QuoteEstimate) -> Result<BoundQuote> {
        let now = now_ms();
        self.bind_at(binding, quote, now, new_quote_id)
    }

    fn read(&self, quote_id: &str, wallet_id: &str, wallet_pubkey: &str) -> Result<BoundQuote> {
        self.read_at(quote_id, wallet_id, wallet_pubkey, now_ms)
    }
}

impl QuoteStore {
    pub(super) fn bind_at(
        &self,
        binding: QuoteBinding<'_>,
        quote: QuoteEstimate,
        now: u64,
        new_id: impl FnOnce() -> String,
    ) -> Result<BoundQuote> {
        self.quotes
            .retain(|_, existing| existing.expires_at_ms > now);
        if self.quotes.len() >= 512 {
            bail!("quote capacity reached; retry after active quotes expire");
        }
        let bound = BoundQuote {
            quote_id: new_id(),
            wallet_id: binding.wallet_id.into(),
            wallet_pubkey: binding.wallet_pubkey,
            quote_slippage_bps: binding.slippage_bps,
            expires_at_ms: now + QUOTE_TTL_MS,
            quote,
        };
        self.quotes.insert(bound.quote_id.clone(), bound.clone());
        Ok(bound)
    }

    fn read_at(
        &self,
        quote_id: &str,
        wallet_id: &str,
        wallet_pubkey: &str,
        now: impl FnOnce() -> u64,
    ) -> Result<BoundQuote> {
        let Some(bound) = self.quotes.get(quote_id) else {
            bail!("quote not found or service restarted");
        };
        if bound.expires_at_ms <= now() {
            drop(bound);
            self.quotes.remove(quote_id);
            bail!("quote expired");
        }
        if bound.wallet_id != wallet_id || bound.wallet_pubkey != wallet_pubkey {
            bail!("quote wallet mismatch");
        }
        Ok(bound.value().clone())
    }
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn new_quote_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod contract_tests;
