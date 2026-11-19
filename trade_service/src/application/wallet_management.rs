use std::{future::Future, pin::Pin};

use crate::{
    application::ports::wallet_management::{WalletRepositoryPort, WalletRuntimePort},
    domain::wallet::{ManagedWallet, WalletInfo},
};

#[cfg(test)]
mod mutation_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub(crate) struct CreateWalletInput {
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub private_key_base58: Option<String>,
}

#[derive(Debug)]
pub(crate) struct LoadedWallets {
    pub default_wallet_id: String,
    pub wallets: Vec<WalletInfo>,
}

#[derive(Debug)]
pub(crate) struct ManagedWalletView {
    pub wallet: ManagedWallet,
    pub created: bool,
}

pub(crate) struct ExportedWallet {
    pub wallet_id: String,
    pub private_key_base58: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WalletFailureKind {
    InvalidRequest,
    Unavailable,
    LoadFailed,
    RemoveFailed,
}

#[derive(Debug)]
pub(crate) struct WalletManagementError {
    pub kind: WalletFailureKind,
    pub message: String,
}

impl WalletManagementError {
    fn new(kind: WalletFailureKind, error: anyhow::Error) -> Self {
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

pub(crate) type WalletManagementFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, WalletManagementError>> + Send + 'a>>;

pub(crate) trait WalletManagement: Send + Sync {
    fn loaded(&self) -> LoadedWallets;
    fn managed(&self) -> WalletManagementFuture<'_, Vec<ManagedWalletView>>;
    fn create(&self, input: CreateWalletInput) -> WalletManagementFuture<'_, ManagedWalletView>;
    fn rename<'a>(
        &'a self,
        id: &'a str,
        label: &'a str,
    ) -> WalletManagementFuture<'a, ManagedWalletView>;
    fn delete<'a>(&'a self, id: &'a str) -> WalletManagementFuture<'a, ManagedWalletView>;
    fn export<'a>(&'a self, id: &'a str) -> WalletManagementFuture<'a, ExportedWallet>;
}

pub(crate) struct WalletManagementService<R, L> {
    repository: R,
    runtime: L,
}

impl<R, L> WalletManagementService<R, L> {
    pub(crate) fn new(repository: R, runtime: L) -> Self {
        Self {
            repository,
            runtime,
        }
    }
}

impl<R, L> WalletManagement for WalletManagementService<R, L>
where
    R: WalletRepositoryPort,
    L: WalletRuntimePort<R::Material>,
{
    fn loaded(&self) -> LoadedWallets {
        self.runtime.loaded()
    }

    fn managed(&self) -> WalletManagementFuture<'_, Vec<ManagedWalletView>> {
        Box::pin(async move {
            self.repository
                .list()
                .await
                .map(|wallets| {
                    wallets
                        .into_iter()
                        .map(|wallet| ManagedWalletView {
                            wallet,
                            created: false,
                        })
                        .collect()
                })
                .map_err(|error| WalletManagementError::new(WalletFailureKind::Unavailable, error))
        })
    }

    fn create(&self, input: CreateWalletInput) -> WalletManagementFuture<'_, ManagedWalletView> {
        Box::pin(async move {
            let mut mutation = self.runtime.begin_mutation().await;
            let result = match input.private_key_base58.as_deref() {
                Some(secret) => self.repository.import(&input, secret).await,
                None => self
                    .repository
                    .create(&input)
                    .await
                    .map(|(wallet, material)| (wallet, material, true)),
            };
            let (wallet, material, created) = result.map_err(|error| {
                WalletManagementError::new(WalletFailureKind::InvalidRequest, error)
            })?;
            if wallet.enabled {
                if let Err(error) = self
                    .runtime
                    .install(&mut mutation, &wallet.wallet_id, material)
                    .await
                {
                    if created {
                        let _ = self.repository.disable(&wallet.wallet_id).await;
                    }
                    return Err(WalletManagementError::new(
                        WalletFailureKind::LoadFailed,
                        error,
                    ));
                }
            }
            Ok(ManagedWalletView { wallet, created })
        })
    }

    fn rename<'a>(
        &'a self,
        id: &'a str,
        label: &'a str,
    ) -> WalletManagementFuture<'a, ManagedWalletView> {
        Box::pin(async move {
            self.repository
                .rename(id, label.trim())
                .await
                .map(|wallet| ManagedWalletView {
                    wallet,
                    created: false,
                })
                .map_err(|error| {
                    WalletManagementError::new(WalletFailureKind::InvalidRequest, error)
                })
        })
    }

    fn delete<'a>(&'a self, id: &'a str) -> WalletManagementFuture<'a, ManagedWalletView> {
        Box::pin(async move {
            let mut mutation = self.runtime.begin_mutation().await;
            let wallet = self.repository.disable(id).await.map_err(|error| {
                WalletManagementError::new(WalletFailureKind::InvalidRequest, error)
            })?;
            self.runtime.remove(&mut mutation, id).map_err(|error| {
                WalletManagementError::new(WalletFailureKind::RemoveFailed, error)
            })?;
            Ok(ManagedWalletView {
                wallet,
                created: false,
            })
        })
    }

    fn export<'a>(&'a self, id: &'a str) -> WalletManagementFuture<'a, ExportedWallet> {
        Box::pin(async move {
            self.repository
                .export(id)
                .await
                .map(|private_key_base58| ExportedWallet {
                    wallet_id: id.into(),
                    private_key_base58,
                })
                .map_err(|error| {
                    WalletManagementError::new(WalletFailureKind::InvalidRequest, error)
                })
        })
    }
}
