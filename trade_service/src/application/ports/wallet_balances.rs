use std::future::Future;

use crate::{application::ports::wallet_reads::WalletReadPort, domain::wallet::WalletMetadata};

/// Public metadata only. Does not expose ciphertext, mutation or runtime clients.
pub(crate) trait WalletMetadataSource: Send + Sync {
    fn wallet(&self, id: &str) -> impl Future<Output = anyhow::Result<WalletMetadata>> + Send;
    fn wallets(&self) -> impl Future<Output = anyhow::Result<Vec<WalletMetadata>>> + Send;
}

/// Readers expose neither a signer nor raw RPC/transaction resources.
pub(crate) trait WalletBalanceRuntime: Send + Sync {
    type Reader: WalletReadPort + Send;
    fn reader(&self, id: &str) -> Option<Self::Reader>;
    fn default_reader(&self) -> Self::Reader;
}
