use anyhow::Result;
use std::future::Future;

use crate::domain::{trade::Address, wallet::QuoteBalance};

/// No signer, transaction, broadcast method or raw RPC handle is exposed.
pub(crate) trait WalletReadPort: Sync {
    fn quote_balance(&self, mint: Address) -> impl Future<Output = Result<QuoteBalance>> + Send;
    fn wallet_balances(&self, wallets: &[Address])
        -> impl Future<Output = Result<Vec<u64>>> + Send;
}
