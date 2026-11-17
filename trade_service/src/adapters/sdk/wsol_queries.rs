use anyhow::{Context, Result};
use std::sync::Arc;

use crate::{
    adapters::rpc::wsol_status::WsolSignatureReader,
    application::{
        ports::{
            wsol_conversion::WsolWalletMetadata,
            wsol_queries::{WsolQuerySource, WsolQuoteReader},
        },
        wsol_queries::{WsolQuoteEstimate, WsolQuoteInput},
    },
    domain::trade::Address,
    wallet_repository::WalletRepository,
    wallets::WalletRegistry,
};

pub(crate) struct SdkWsolQuerySource {
    wallets: Arc<WalletRegistry>,
    repository: Arc<WalletRepository>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}
impl SdkWsolQuerySource {
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
pub(crate) struct SdkWsolQuoteReader {
    lane: Option<(String, solana_commitment_config::CommitmentConfig)>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}
impl WsolQuerySource for SdkWsolQuerySource {
    type QuoteReader = SdkWsolQuoteReader;
    type StatusReader = WsolSignatureReader;
    async fn metadata(&self, id: &str) -> Result<WsolWalletMetadata> {
        crate::adapters::persistence::wsol_wallet_metadata::load(&self.repository, id).await
    }
    fn quote_reader(&self, id: &str) -> Option<Self::QuoteReader> {
        self.wallets.get(id).map(|client| SdkWsolQuoteReader {
            lane: client
                .rpcs()
                .into_iter()
                .next()
                .map(|rpc| (rpc.url(), rpc.commitment())),
            read_rpc: self.read_rpc.clone(),
        })
    }
    fn status_reader(&self, id: &str, signature: &str) -> Result<Self::StatusReader> {
        let signature = signature.parse()?;
        let client = self.wallets.get(id).context("wallet is not loaded")?;
        let rpc = client
            .rpcs()
            .into_iter()
            .next()
            .context("trade service has no RPC")?;
        Ok(WsolSignatureReader::new(rpc, signature))
    }
}
impl WsolQuoteReader for SdkWsolQuoteReader {
    async fn estimate(&self, owner: Address, input: &WsolQuoteInput) -> Result<WsolQuoteEstimate> {
        let (url, commitment) = self.lane.as_ref().context("wallet has no RPC")?;
        crate::adapters::sdk::wsol_quote::estimate(
            &self.read_rpc,
            url,
            *commitment,
            solana_sdk::pubkey::Pubkey::new_from_array(owner.to_bytes()),
            input,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wsol_query_source_reuses_metadata_and_validates_signature_before_wallet_resolution() {
        let state = crate::http_contract_tests::state().await;
        let h = crate::application::sol_transfers::test_support::harness().await;
        let read_rpc =
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
        let source =
            SdkWsolQuerySource::new(state.wallets.clone(), h.repo.clone(), read_rpc.clone());
        assert!(Arc::ptr_eq(&source.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&source.repository, &h.repo));
        assert!(Arc::ptr_eq(&source.read_rpc, &read_rpc));
        let metadata = source.metadata("source").await.unwrap();
        assert_eq!(metadata.pubkey, Address::from_bytes([1; 32]).to_string());
        assert!(source.quote_reader("source").is_none());
        assert_eq!(
            source
                .status_reader("source", "bad")
                .err()
                .unwrap()
                .to_string(),
            "bad"
                .parse::<solana_sdk::signature::Signature>()
                .unwrap_err()
                .to_string()
        );
        assert_eq!(
            source
                .status_reader(
                    "source",
                    &solana_sdk::signature::Signature::from([7; 64]).to_string()
                )
                .err()
                .unwrap()
                .to_string(),
            "wallet is not loaded"
        );
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
