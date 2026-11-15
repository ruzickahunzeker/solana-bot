use std::sync::Arc;

use crate::{
    application::{
        ports::wallet_management::{WalletRepositoryPort, WalletRuntimePort},
        wallet_management::{CreateWalletInput, LoadedWallets},
    },
    config::WalletConfig,
    domain::wallet::ManagedWallet,
    wallet_repository::WalletRepository,
    wallets::WalletRegistry,
};

pub(crate) struct WalletMaterial(String);

impl std::fmt::Debug for WalletMaterial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WalletMaterial(<redacted>)")
    }
}

#[derive(Clone)]
pub(crate) struct SdkWalletManagement {
    repository: Arc<WalletRepository>,
    wallets: Arc<WalletRegistry>,
}

impl SdkWalletManagement {
    pub(crate) fn new(repository: Arc<WalletRepository>, wallets: Arc<WalletRegistry>) -> Self {
        Self {
            repository,
            wallets,
        }
    }
}

impl WalletRepositoryPort for SdkWalletManagement {
    type Material = WalletMaterial;
    async fn list(&self) -> anyhow::Result<Vec<ManagedWallet>> {
        self.repository.list().await
    }
    async fn create(
        &self,
        input: &CreateWalletInput,
    ) -> anyhow::Result<(ManagedWallet, Self::Material)> {
        self.repository
            .create(&input.owner_type, &input.owner_id, &input.label)
            .await
            .map(|(wallet, key)| (wallet, WalletMaterial(key)))
    }
    async fn import(
        &self,
        input: &CreateWalletInput,
        secret: &str,
    ) -> anyhow::Result<(ManagedWallet, Self::Material, bool)> {
        self.repository
            .import(&input.owner_type, &input.owner_id, &input.label, secret)
            .await
            .map(|(wallet, created)| (wallet, WalletMaterial(secret.to_string()), created))
    }
    async fn rename(&self, id: &str, label: &str) -> anyhow::Result<ManagedWallet> {
        self.repository.rename(id, label).await
    }
    async fn disable(&self, id: &str) -> anyhow::Result<ManagedWallet> {
        self.repository.disable(id).await
    }
    async fn export(&self, id: &str) -> anyhow::Result<String> {
        self.repository.export(id).await
    }
}

impl WalletRuntimePort<WalletMaterial> for SdkWalletManagement {
    fn loaded(&self) -> LoadedWallets {
        LoadedWallets {
            default_wallet_id: self.wallets.default_wallet_id().to_string(),
            wallets: self.wallets.infos(),
        }
    }
    async fn install(&self, id: &str, material: WalletMaterial) -> anyhow::Result<()> {
        let config = WalletConfig {
            wallet_id: id.into(),
            private_key_base58: material.0,
        };
        self.wallets.add(&config).await?;
        Ok(())
    }
    fn remove(&self, id: &str) -> anyhow::Result<()> {
        self.wallets.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::trade::Address;

    #[tokio::test]
    async fn management_adapter_keeps_original_resources_and_redacts_material_debug_without_clients(
    ) {
        let state = crate::http_contract_tests::state().await;
        let backend =
            SdkWalletManagement::new(state.wallet_repository.clone(), state.wallets.clone());
        let cloned = backend.clone();
        assert!(Arc::ptr_eq(&cloned.repository, &state.wallet_repository));
        assert!(Arc::ptr_eq(&cloned.wallets, &state.wallets));
        let loaded = WalletRuntimePort::loaded(&backend);
        assert_eq!(loaded.default_wallet_id, "fixture-wallet");
        assert!(loaded.wallets.is_empty());
        assert_eq!(
            format!("{:?}", WalletMaterial("not-a-key-test-only".into())),
            "WalletMaterial(<redacted>)"
        );
        assert_eq!(state.wallets.wallet_count(), 0);
    }

    #[tokio::test]
    async fn metadata_methods_and_negative_secret_paths_keep_existing_repository_behavior_without_keys(
    ) {
        let state = crate::http_contract_tests::state().await;
        let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
        let backend = SdkWalletManagement::new(Arc::new(repository), state.wallets.clone());
        sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES('public-fixture','telegram','42','old',?,'not-ciphertext-test-only',1)")
            .bind(Address::from_bytes([1;32]).to_string()).execute(&pool).await.unwrap();
        let listed = WalletRepositoryPort::list(&backend).await.unwrap();
        assert_eq!(
            serde_json::to_value(&listed[0]).unwrap(),
            serde_json::json!({
                "wallet_id":"public-fixture","owner_type":"telegram","owner_id":"42","label":"old",
                "pubkey":Address::from_bytes([1;32]).to_string(),"is_default":false,"enabled":true
            })
        );
        assert_eq!(
            WalletRepositoryPort::rename(&backend, "public-fixture", "new")
                .await
                .unwrap()
                .label,
            "new"
        );
        assert_eq!(
            WalletRepositoryPort::export(&backend, "public-fixture")
                .await
                .unwrap_err()
                .to_string(),
            "wallet database passphrase is unavailable"
        );
        let input = CreateWalletInput {
            owner_type: "telegram".into(),
            owner_id: "42".into(),
            label: "ignored".into(),
            private_key_base58: None,
        };
        assert_eq!(
            WalletRepositoryPort::import(&backend, &input, "invalid-key-not-secret")
                .await
                .unwrap_err()
                .to_string(),
            "private key must be a valid base58 Solana keypair"
        );
        assert!(
            !WalletRepositoryPort::disable(&backend, "public-fixture")
                .await
                .unwrap()
                .enabled
        );
        assert_eq!(state.wallets.wallet_count(), 0);
        pool.close().await;
    }
}
