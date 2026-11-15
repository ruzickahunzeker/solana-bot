use std::future::Future;

use crate::{application::vanity::VanityCreateCommand, domain::wallet::VanityJob};

/// Job control only; no keys, signer, raw transaction, generation loop or runtime client.
pub(crate) trait VanityWorkerPort: Send + Sync {
    fn create(
        &self,
        command: VanityCreateCommand,
    ) -> impl Future<Output = anyhow::Result<VanityJob>> + Send;
    fn get(&self, id: &str) -> impl Future<Output = anyhow::Result<VanityJob>> + Send;
    fn cancel(
        &self,
        id: &str,
        owner_type: &str,
        owner_id: &str,
    ) -> impl Future<Output = anyhow::Result<VanityJob>> + Send;
}
