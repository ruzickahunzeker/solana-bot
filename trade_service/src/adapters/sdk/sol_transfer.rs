use std::sync::Arc;

use crate::{
    adapters::persistence::sol_transfer_journal::SqliteSolTransferJournal,
    application::ports::sol_transfer_execution::{
        SolTransferArtifact, SolTransferExecutionWallet, SolTransferRuntime, TransferWalletMetadata,
    },
    domain::{
        sol_transfer::{SolTransferAmount, SolTransferSnapshot},
        submission::SignedSubmission,
        trade::Address,
    },
    sdk_client::{
        sol_transfer::{PreparedSolTransfer, SignedSolTransfer},
        SdkTradeClient,
    },
    wallet_repository::WalletRepository,
    wallets::WalletRegistry,
};

pub(crate) struct SdkSolTransferRuntime {
    wallets: Arc<WalletRegistry>,
    repository: Arc<WalletRepository>,
}
impl SdkSolTransferRuntime {
    pub(crate) fn new(wallets: Arc<WalletRegistry>, repository: Arc<WalletRepository>) -> Self {
        Self {
            wallets,
            repository,
        }
    }
}
pub(crate) struct NativeTransferWallet {
    client: Arc<SdkTradeClient>,
}
impl SolTransferRuntime for SdkSolTransferRuntime {
    type Wallet = NativeTransferWallet;
    type Journal = SqliteSolTransferJournal;
    async fn metadata(&self, id: &str) -> anyhow::Result<TransferWalletMetadata> {
        let wallet = self.repository.get(id).await?;
        Ok(TransferWalletMetadata {
            owner_type: wallet.owner_type,
            owner_id: wallet.owner_id,
            pubkey: wallet.pubkey,
            enabled: wallet.enabled,
        })
    }
    fn journal(&self) -> Option<Self::Journal> {
        self.repository.sol_transfer_journal()
    }
    fn resolve(&self, id: &str) -> Option<Self::Wallet> {
        self.wallets
            .get(id)
            .map(|client| NativeTransferWallet { client })
    }
}
impl SolTransferArtifact for SignedSolTransfer {
    fn submission(&self) -> &SignedSubmission {
        SignedSolTransfer::submission(self)
    }
    fn snapshot(&self) -> &SolTransferSnapshot {
        SignedSolTransfer::snapshot(self)
    }
}
impl SolTransferExecutionWallet for NativeTransferWallet {
    type Prepared = PreparedSolTransfer;
    type Signed = SignedSolTransfer;
    fn pubkey(&self) -> Address {
        Address::from_bytes(self.client.wallet().to_bytes())
    }
    async fn prepare(
        &self,
        destination: Address,
        amount: SolTransferAmount,
    ) -> anyhow::Result<Self::Prepared> {
        let (fixed, percent) = amount.legacy_params();
        self.client
            .prepare_sol_transfer(
                &solana_sdk::pubkey::Pubkey::new_from_array(destination.to_bytes()),
                fixed,
                percent,
            )
            .await
    }
    fn prepared_snapshot<'a>(&self, prepared: &'a Self::Prepared) -> &'a SolTransferSnapshot {
        prepared.snapshot()
    }
    fn sign(&self, prepared: Self::Prepared) -> anyhow::Result<Self::Signed> {
        self.client.sign_sol_transfer(prepared)
    }
    async fn broadcast(&self, signed: &Self::Signed) -> anyhow::Result<String> {
        Ok(self
            .client
            .send_signed_sol_transfer(signed)
            .await?
            .to_string())
    }
    async fn confirm(&self, signed: &Self::Signed) -> bool {
        let Ok(signature) = signed.submission().signature.parse() else {
            return false;
        };
        self.client.confirm_sol_transfer(signed, &signature).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_runtime_reuses_registry_and_repository_without_sdk_initialization() {
        let state = crate::http_contract_tests::state().await;
        let runtime =
            SdkSolTransferRuntime::new(state.wallets.clone(), state.wallet_repository.clone());
        assert!(Arc::ptr_eq(&runtime.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&runtime.repository, &state.wallet_repository));
        assert!(runtime.resolve("missing").is_none());
        assert!(runtime.journal().is_none());
        assert!(runtime.metadata("missing").await.is_err());
        assert_eq!(state.wallets.wallet_count(), 0);
    }

    #[tokio::test]
    async fn native_runtime_metadata_and_journal_do_not_load_or_decrypt_public_fixture_wallets() {
        let h = crate::application::sol_transfers::test_support::harness().await;
        let state = crate::http_contract_tests::state().await;
        let runtime = SdkSolTransferRuntime::new(state.wallets.clone(), h.repo.clone());
        let metadata = runtime.metadata("source").await.unwrap();
        assert_eq!(metadata.pubkey, Address::from_bytes([1; 32]).to_string());
        assert_eq!(metadata.owner_id, "42");
        assert!(metadata.enabled);
        assert!(runtime.resolve("source").is_none());
        assert!(runtime.journal().is_some());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
