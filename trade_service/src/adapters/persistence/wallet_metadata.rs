use std::sync::Arc;

use crate::{
    application::ports::wallet_balances::WalletMetadataSource,
    domain::wallet::WalletMetadata,
    wallet_repository::{ManagedWallet, WalletRepository},
};

pub(crate) struct RepositoryWalletMetadata {
    repository: Arc<WalletRepository>,
}

impl RepositoryWalletMetadata {
    pub(crate) fn new(repository: Arc<WalletRepository>) -> Self {
        Self { repository }
    }
}

fn metadata(wallet: ManagedWallet) -> WalletMetadata {
    WalletMetadata {
        wallet_id: wallet.wallet_id,
        pubkey: wallet.pubkey,
        owner_type: wallet.owner_type,
        owner_id: wallet.owner_id,
        enabled: wallet.enabled,
    }
}

impl WalletMetadataSource for RepositoryWalletMetadata {
    async fn wallet(&self, id: &str) -> anyhow::Result<WalletMetadata> {
        Ok(metadata(self.repository.get(id).await?))
    }
    async fn wallets(&self) -> anyhow::Result<Vec<WalletMetadata>> {
        Ok(self
            .repository
            .list()
            .await?
            .into_iter()
            .map(metadata)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{WalletStorageConfig, WalletStorageMode},
        domain::trade::Address,
    };

    #[tokio::test]
    async fn original_repository_is_reused_and_projects_only_public_metadata_without_decryption() {
        let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
        let repository = Arc::new(repository);
        let source = RepositoryWalletMetadata::new(repository.clone());
        assert!(Arc::ptr_eq(&source.repository, &repository));
        sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES(' source ','telegram','42','public label',?,'not-ciphertext-test-only',1)")
            .bind(Address::from_bytes([1;32]).to_string()).execute(&pool).await.unwrap();
        let expected = WalletMetadata {
            wallet_id: " source ".into(),
            pubkey: Address::from_bytes([1; 32]).to_string(),
            owner_type: "telegram".into(),
            owner_id: "42".into(),
            enabled: true,
        };
        assert_eq!(source.wallet(" source ").await.unwrap(), expected);
        assert_eq!(source.wallets().await.unwrap(), [expected.clone()]);
        let alias: crate::application::ports::wsol_conversion::WsolWalletMetadata = expected;
        assert!(alias.enabled);
        let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallet_audit_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(audit, 0);
        sqlx::query("UPDATE wallets SET enabled=0")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!source.wallet(" source ").await.unwrap().enabled);
        pool.close().await;
    }

    #[tokio::test]
    async fn repository_empty_missing_file_mode_and_closed_pool_errors_keep_their_original_meaning()
    {
        let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
        let source = RepositoryWalletMetadata::new(Arc::new(repository));
        assert!(source.wallets().await.unwrap().is_empty());
        assert_eq!(
            source.wallet("missing").await.unwrap_err().to_string(),
            "wallet not found"
        );
        pool.close().await;
        assert!(source
            .wallet("missing")
            .await
            .unwrap_err()
            .downcast_ref::<sqlx::Error>()
            .is_some());
        assert!(source
            .wallets()
            .await
            .unwrap_err()
            .downcast_ref::<sqlx::Error>()
            .is_some());
        let file = WalletRepository::new(
            &WalletStorageConfig {
                mode: WalletStorageMode::File,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 100,
                passphrase: None,
            },
            &[],
            "fixture-wallet",
        )
        .await
        .unwrap();
        let file = RepositoryWalletMetadata::new(Arc::new(file));
        assert_eq!(
            file.wallets().await.unwrap_err().to_string(),
            "wallet database mode is not enabled"
        );
        assert_eq!(
            file.wallet("missing").await.unwrap_err().to_string(),
            "wallet database mode is not enabled"
        );
    }
}
