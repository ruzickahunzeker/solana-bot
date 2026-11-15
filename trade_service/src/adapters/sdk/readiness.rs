use crate::{
    adapters::rpc::read_runtime::SharedReadRuntime,
    application::ports::readiness::ReadinessWalletObserver,
    domain::readiness::WalletReadinessObservation, wallets::WalletRegistry,
};
use std::sync::Arc;

pub(crate) struct SdkReadinessWalletObserver {
    wallets: Arc<WalletRegistry>,
    read_rpc: Arc<SharedReadRuntime>,
}

impl SdkReadinessWalletObserver {
    pub(crate) fn new(wallets: Arc<WalletRegistry>, read_rpc: Arc<SharedReadRuntime>) -> Self {
        Self { wallets, read_rpc }
    }
}

impl ReadinessWalletObserver for SdkReadinessWalletObserver {
    fn observe_wallets(
        &self,
    ) -> impl std::future::Future<Output = WalletReadinessObservation> + Send {
        let probe = self.wallets.default().readiness_probe();
        async move {
            let rpc_ok = probe.ping(&self.read_rpc).await;
            let default_wallet_id = self.wallets.default_wallet_id().to_string();
            let wallet_count = self.wallets.wallet_count();
            let wallets = self.wallets.public_infos();
            WalletReadinessObservation {
                default_wallet_id,
                wallet_count,
                wallets,
                wallet_metadata: probe.into_metadata(),
                rpc_ok,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_settings_sdk_observer_retains_only_original_registry_and_runtime() {
        let state = crate::http_contract_tests::state().await;
        let runtime = Arc::new(SharedReadRuntime::new(&[]).unwrap());
        let wallet_weak = Arc::downgrade(&state.wallets);
        let runtime_weak = Arc::downgrade(&runtime);
        let config_weak = Arc::downgrade(&state.config);
        let observer = SdkReadinessWalletObserver::new(state.wallets.clone(), runtime.clone());
        assert!(Arc::ptr_eq(&observer.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&observer.read_rpc, &runtime));
        drop((state, runtime));
        assert!(config_weak.upgrade().is_none());
        assert!(wallet_weak.upgrade().is_some());
        assert!(runtime_weak.upgrade().is_some());
        drop(observer);
        assert!(wallet_weak.upgrade().is_none());
        assert!(runtime_weak.upgrade().is_none());
    }
}
