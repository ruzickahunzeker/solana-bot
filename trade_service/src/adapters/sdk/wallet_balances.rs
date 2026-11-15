use std::sync::Arc;

use crate::{
    adapters::rpc::wallet_reads::RpcWalletReader,
    application::ports::wallet_balances::WalletBalanceRuntime, wallets::WalletRegistry,
};

pub(crate) struct SdkWalletBalanceRuntime {
    wallets: Arc<WalletRegistry>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}

impl SdkWalletBalanceRuntime {
    pub(crate) fn new(
        wallets: Arc<WalletRegistry>,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> Self {
        Self { wallets, read_rpc }
    }
}

impl WalletBalanceRuntime for SdkWalletBalanceRuntime {
    type Reader = RpcWalletReader;
    fn reader(&self, id: &str) -> Option<Self::Reader> {
        self.wallets.read_access(id, self.read_rpc.clone())
    }
    fn default_reader(&self) -> Self::Reader {
        self.wallets.default_read_access(self.read_rpc.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn balance_runtime_retains_original_registry_without_initializing_clients_or_fallbacks() {
        let state = crate::http_contract_tests::state().await;
        let read_rpc =
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
        let runtime = SdkWalletBalanceRuntime::new(state.wallets.clone(), read_rpc.clone());
        assert!(Arc::ptr_eq(&runtime.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&runtime.read_rpc, &read_rpc));
        for id in ["fixture-wallet", "missing", " fixture-wallet ", ""] {
            assert!(runtime.reader(id).is_none());
        }
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
