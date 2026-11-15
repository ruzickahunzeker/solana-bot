use anyhow::Result;
use std::future::Future;

use crate::domain::blockhash::Blockhash;

/// Only a fresh RPC observation, without signer, transaction or raw RPC access.
pub(crate) trait LatestBlockhashReader: Sync {
    fn latest_blockhash(&self) -> impl Future<Output = Result<Blockhash>> + Send;
}
