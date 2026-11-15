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
        wsol_conversion::{
            prepare_unsigned_wsol_transaction, SignedWsolTransaction, UnsignedWsolTransaction,
        },
        SdkTradeClient,
    },
    wallet_repository::WalletRepository,
    wallets::WalletRegistry,
};

pub(crate) struct SdkWsolRuntime {
    wallets: Arc<WalletRegistry>,
    repository: Arc<WalletRepository>,
}
impl SdkWsolRuntime {
    pub(crate) fn new(wallets: Arc<WalletRegistry>, repository: Arc<WalletRepository>) -> Self {
        Self {
            wallets,
            repository,
        }
    }
}

pub(crate) struct WsolWallet {
    client: Arc<SdkTradeClient>,
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
        self.wallets.get(id).map(|client| WsolWallet { client })
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
        let unsigned = prepare_unsigned_wsol_transaction(self.client.wallet(), &rpc, quote).await?;
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
        let runtime = SdkWsolRuntime::new(state.wallets.clone(), h.repo.clone());
        assert!(Arc::ptr_eq(&runtime.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&runtime.repository, &h.repo));
        let metadata = runtime.metadata("source").await.unwrap();
        assert_eq!(metadata.wallet_id, "source");
        assert_eq!(metadata.pubkey, Address::from_bytes([1; 32]).to_string());
        assert!(metadata.enabled);
        assert!(runtime.resolve("source").is_none());
        assert!(runtime.metadata("missing").await.is_err());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
