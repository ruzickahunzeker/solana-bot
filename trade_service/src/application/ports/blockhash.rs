use anyhow::Result;
use std::future::Future;

use crate::domain::blockhash::Blockhash;

/// Only a fresh RPC observation, without signer, transaction or raw RPC access.
pub(crate) trait LatestBlockhashReader: Sync {
    fn latest_blockhash(&self) -> impl Future<Output = Result<Blockhash>> + Send;
}

/// Reads the existing cache only; it cannot refresh, contact RPC or extend validity.
pub(crate) trait CachedBlockhashRead: Send + Sync {
    fn fresh_hash(&self, max_age_ms: u64) -> Option<Blockhash>;
}
