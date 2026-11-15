use crate::{
    application::ports::wallet_identity::{LoadedWalletDirectory, LoadedWalletIdentity},
    domain::trade::Address,
    sdk_client::SdkTradeClient,
    wallets::WalletRegistry,
};
use std::sync::Arc;

pub(crate) struct SdkWalletIdentityDirectory {
    wallets: Arc<WalletRegistry>,
}

impl SdkWalletIdentityDirectory {
    pub(crate) fn new(wallets: Arc<WalletRegistry>) -> Self {
        Self { wallets }
    }
}

// Keep the original client alive for the acceptance scope without exposing it.
struct IdentityLease(Arc<SdkTradeClient>);

impl LoadedWalletIdentity for IdentityLease {
    fn pubkey(&self) -> Address {
        Address::from_bytes(self.0.wallet().to_bytes())
    }
}

impl LoadedWalletDirectory for SdkWalletIdentityDirectory {
    fn resolve_id(&self, requested: Option<&str>) -> String {
        self.wallets.resolve_wallet_id(requested)
    }
    fn lookup(&self, wallet_id: &str) -> Option<Box<dyn LoadedWalletIdentity>> {
        self.wallets
            .get(wallet_id)
            .map(|wallet| Box::new(IdentityLease(wallet)) as Box<dyn LoadedWalletIdentity>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identity_directory_uses_loaded_registry_and_preserves_literal_default_resolution() {
        let state = crate::http_contract_tests::state().await;
        let directory = SdkWalletIdentityDirectory::new(state.wallets.clone());
        assert_eq!(directory.resolve_id(None), "fixture-wallet");
        for literal in ["other", "", " fixture-wallet "] {
            assert_eq!(directory.resolve_id(Some(literal)), literal);
        }
        assert!(directory.lookup("fixture-wallet").is_none());
        assert!(directory.lookup("other").is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
