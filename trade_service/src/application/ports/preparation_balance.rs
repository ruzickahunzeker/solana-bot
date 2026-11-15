use crate::domain::{preparation_balance::PreparationBalanceSnapshot, trade::Address};
use std::future::Future;

/// Public addresses in, parsed metadata out; no signer, SDK execution or submission capability.
pub(crate) trait PreparationBalanceRead: Send + Sync {
    fn snapshot(
        &self,
        mint: Address,
    ) -> impl Future<Output = anyhow::Result<PreparationBalanceSnapshot>> + Send;
}
