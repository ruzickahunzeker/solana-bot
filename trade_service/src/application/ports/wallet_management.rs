use std::future::Future;

use crate::{
    application::wallet_management::{CreateWalletInput, LoadedWallets},
    domain::wallet::ManagedWallet,
};

pub(crate) trait WalletRepositoryPort: Send + Sync {
    type Material: Send;
    fn list(&self) -> impl Future<Output = anyhow::Result<Vec<ManagedWallet>>> + Send;
    fn create(
        &self,
        input: &CreateWalletInput,
    ) -> impl Future<Output = anyhow::Result<(ManagedWallet, Self::Material)>> + Send;
    fn import(
        &self,
        input: &CreateWalletInput,
        secret: &str,
    ) -> impl Future<Output = anyhow::Result<(ManagedWallet, Self::Material, bool)>> + Send;
    fn rename(
        &self,
        id: &str,
        label: &str,
    ) -> impl Future<Output = anyhow::Result<ManagedWallet>> + Send;
    fn disable(&self, id: &str) -> impl Future<Output = anyhow::Result<ManagedWallet>> + Send;
    fn export(&self, id: &str) -> impl Future<Output = anyhow::Result<String>> + Send;
}

/// Material is opaque to the application, not a signer or executable transaction.
pub(crate) trait WalletRuntimePort<M: Send>: Send + Sync {
    type Mutation<'a>: Send
    where
        Self: 'a;
    fn begin_mutation(&self) -> impl Future<Output = Self::Mutation<'_>> + Send;
    fn loaded(&self) -> LoadedWallets;
    fn install(
        &self,
        mutation: &mut Self::Mutation<'_>,
        id: &str,
        material: M,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn remove(&self, mutation: &mut Self::Mutation<'_>, id: &str) -> anyhow::Result<()>;
}
