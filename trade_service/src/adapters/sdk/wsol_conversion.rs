use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;

use crate::{
    application::{
        ports::wsol_conversion::{
            WsolArtifact, WsolExecutionWallet, WsolRuntime, WsolWalletMetadata,
        },
        wsol_submission_types::WsolQuoteSnapshot,
    },
    domain::trade::Address,
    sdk_client::{
        wsol_conversion::{preparation, SignedWsolTransaction, UnsignedWsolTransaction},
        SdkTradeClient,
    },
    wallet_repository::WalletRepository,
    wallets::WalletRegistry,
};

pub(crate) struct SdkWsolRuntime {
    wallets: Arc<WalletRegistry>,
    repository: Arc<WalletRepository>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}
impl SdkWsolRuntime {
    pub(crate) fn new(
        wallets: Arc<WalletRegistry>,
        repository: Arc<WalletRepository>,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> Self {
        Self {
            wallets,
            repository,
            read_rpc,
        }
    }
}

pub(crate) struct WsolWallet {
    client: Arc<SdkTradeClient>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}
pub(crate) struct PreparedWsol {
    unsigned: UnsignedWsolTransaction,
    rpc: Arc<RpcClient>,
}
pub(crate) struct SignedWsol {
    artifact: SignedWsolTransaction,
    rpc: Arc<RpcClient>,
}
impl WsolArtifact for SignedWsol {
    fn signature(&self) -> &str {
        self.artifact.signature()
    }
    fn signed_bytes(&self) -> &[u8] {
        self.artifact.signed_bytes()
    }
}
impl WsolRuntime for SdkWsolRuntime {
    type Wallet = WsolWallet;
    async fn metadata(&self, id: &str) -> Result<WsolWalletMetadata> {
        crate::adapters::persistence::wsol_wallet_metadata::load(&self.repository, id).await
    }
    fn resolve(&self, id: &str) -> Option<Self::Wallet> {
        self.wallets.get(id).map(|client| WsolWallet {
            client,
            read_rpc: self.read_rpc.clone(),
        })
    }
}
impl WsolExecutionWallet for WsolWallet {
    type Prepared = PreparedWsol;
    type Signed = SignedWsol;
    fn pubkey(&self) -> Address {
        Address::from_bytes(self.client.wallet().to_bytes())
    }
    async fn prepare(&self, quote: &WsolQuoteSnapshot) -> Result<Self::Prepared> {
        let rpc = self
            .client
            .rpcs()
            .into_iter()
            .next()
            .context("wallet has no RPC")?;
        let unsigned = preparation::prepare(
            self.client.wallet(),
            &self.read_rpc,
            &rpc.url(),
            rpc.commitment(),
            quote,
        )
        .await?;
        Ok(PreparedWsol { unsigned, rpc })
    }
    async fn simulate_unsigned(
        &self,
        prepared: &Self::Prepared,
        quote: &WsolQuoteSnapshot,
    ) -> Result<Option<u64>> {
        Ok(prepared
            .unsigned
            .simulate(&prepared.rpc, quote)
            .await?
            .units_consumed)
    }
    fn sign(&self, prepared: Self::Prepared, quote: &WsolQuoteSnapshot) -> Result<Self::Signed> {
        Ok(SignedWsol {
            artifact: self.client.sign_prepared_wsol(prepared.unsigned, quote)?,
            rpc: prepared.rpc,
        })
    }
    async fn simulate_signed(&self, signed: &Self::Signed) -> Result<bool> {
        Ok(signed.artifact.simulate(&signed.rpc).await?.err.is_none())
    }
    async fn broadcast(&self, signed: &Self::Signed) -> Result<String> {
        Ok(self
            .client
            .send_signed_wsol(&signed.artifact, &signed.rpc)
            .await?
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wsol_runtime_reuses_original_registry_and_metadata_without_loading_wallets() {
        let h = crate::application::sol_transfers::test_support::harness().await;
        let state = crate::http_contract_tests::state().await;
        let read_rpc =
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
        let runtime = SdkWsolRuntime::new(state.wallets.clone(), h.repo.clone(), read_rpc.clone());
        assert!(Arc::ptr_eq(&runtime.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&runtime.repository, &h.repo));
        assert!(Arc::ptr_eq(&runtime.read_rpc, &read_rpc));
        let metadata = runtime.metadata("source").await.unwrap();
        assert_eq!(metadata.wallet_id, "source");
        assert_eq!(metadata.pubkey, Address::from_bytes([1; 32]).to_string());
        assert!(metadata.enabled);
        assert!(runtime.resolve("source").is_none());
        assert!(runtime.metadata("missing").await.is_err());
        assert_eq!(state.wallets.wallet_count(), 0);
    }

    #[tokio::test]
    async fn wsol_preparation_ownership_reuses_original_runtime_and_releases_with_wallet_registry()
    {
        let state = crate::http_contract_tests::state().await;
        let read_rpc =
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
        let read_weak = Arc::downgrade(&read_rpc);
        let wallets = Arc::downgrade(&state.wallets);
        let repository = Arc::downgrade(&state.wallet_repository);
        let config = Arc::downgrade(&state.config);
        let runtime = SdkWsolRuntime::new(
            state.wallets.clone(),
            state.wallet_repository.clone(),
            read_rpc.clone(),
        );
        assert!(Arc::ptr_eq(&runtime.read_rpc, &read_rpc));
        drop((state, read_rpc));
        assert!(config.upgrade().is_none());
        assert!(read_weak.upgrade().is_some());
        assert!(wallets.upgrade().is_some());
        assert!(repository.upgrade().is_some());
        drop(runtime);
        assert!(read_weak.upgrade().is_none());
        assert!(wallets.upgrade().is_none());
        assert!(repository.upgrade().is_none());
    }
}
