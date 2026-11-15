use crate::{
    application::{ports::wsol_conversion::WsolWalletMetadata, wsol_failure::WsolFailure},
    wallet_repository::WalletRepository,
};

pub(crate) async fn load(
    repository: &WalletRepository,
    id: &str,
) -> anyhow::Result<WsolWalletMetadata> {
    let wallet = repository
        .find(id)
        .await
        .map_err(|error| match error.downcast_ref::<sqlx::Error>() {
            Some(
                sqlx::Error::ColumnDecode { .. }
                | sqlx::Error::Decode(_)
                | sqlx::Error::ColumnNotFound(_),
            ) => WsolFailure::integrity("wallet metadata could not be decoded").into(),
            _ => error,
        })?
        .ok_or_else(|| WsolFailure::rejected("wallet not found"))?;
    Ok(WsolWalletMetadata {
        wallet_id: wallet.wallet_id,
        pubkey: wallet.pubkey,
        owner_type: wallet.owner_type,
        owner_id: wallet.owner_id,
        enabled: wallet.enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{application::wsol_failure::WsolFailureKind, domain::trade::Address};

    #[tokio::test]
    async fn wsol_quote_simulate_metadata_matches_legacy_projection_and_types_missing_rows() {
        let h = crate::application::sol_transfers::test_support::harness().await;
        let old = h.repo.get("source").await.unwrap();
        let found = h.repo.find("source").await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&found).unwrap(),
            serde_json::to_value(&old).unwrap()
        );
        let metadata = load(&h.repo, "source").await.unwrap();
        assert_eq!(metadata.wallet_id, old.wallet_id);
        assert_eq!(metadata.pubkey, old.pubkey);
        assert_eq!(metadata.owner_type, old.owner_type);
        assert_eq!(metadata.owner_id, old.owner_id);
        assert_eq!(metadata.enabled, old.enabled);
        assert!(h.repo.find(" source ").await.unwrap().is_none());
        let error = load(&h.repo, "missing").await.err().unwrap();
        assert_eq!(error.to_string(), "wallet not found");
        assert_eq!(
            error.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Rejected
        );
    }

    #[tokio::test]
    async fn wsol_quote_simulate_metadata_corruption_and_closed_database_are_never_not_found_or_panics(
    ) {
        for column in [
            "owner_type",
            "owner_id",
            "label",
            "pubkey",
            "is_default",
            "enabled",
        ] {
            let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
            sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES('fixture','telegram','42','fixture',?,'deliberately-invalid-ciphertext',1)")
                .bind(Address::from_bytes([1;32]).to_string()).execute(&pool).await.unwrap();
            assert!(load(&repository, "fixture").await.is_ok());
            sqlx::query(&format!("UPDATE wallets SET {column}=X'FF'"))
                .execute(&pool)
                .await
                .unwrap();
            let error = load(&repository, "fixture").await.err().unwrap();
            assert_eq!(
                error.downcast_ref::<WsolFailure>().unwrap().kind,
                WsolFailureKind::Integrity,
                "{column}"
            );
            assert_eq!(error.to_string(), "wallet metadata could not be decoded");
        }
        let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
        pool.close().await;
        let error = load(&repository, "missing").await.err().unwrap();
        assert!(error.downcast_ref::<sqlx::Error>().is_some());
        assert!(error.downcast_ref::<WsolFailure>().is_none());
    }
}
