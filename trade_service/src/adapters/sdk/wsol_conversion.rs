use anyhow::{Context, Result};
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
            broadcast::WsolBroadcastLane, preparation, simulation, SignedWsolTransaction,
            UnsignedWsolTransaction,
        },
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
    rpc: WsolBroadcastLane,
}
pub(crate) struct SignedWsol {
    artifact: SignedWsolTransaction,
    rpc: WsolBroadcastLane,
}
impl PreparedWsol {
    async fn simulate<T: crate::adapters::rpc::read_http::ReadHttpTransport>(
        &self,
        runtime: &crate::adapters::rpc::read_runtime::SharedReadRuntime<T>,
        quote: &WsolQuoteSnapshot,
    ) -> Result<Option<u64>> {
        Ok(simulation::simulate_unsigned(
            &self.unsigned,
            quote,
            runtime,
            &self.rpc.url(),
            self.rpc.commitment(),
        )
        .await?
        .units_consumed)
    }
}
impl SignedWsol {
    async fn simulate<T: crate::adapters::rpc::read_http::ReadHttpTransport>(
        &self,
        runtime: &crate::adapters::rpc::read_runtime::SharedReadRuntime<T>,
    ) -> Result<bool> {
        Ok(simulation::simulate_signed(
            &self.artifact,
            runtime,
            &self.rpc.url(),
            self.rpc.commitment(),
        )
        .await?
        .err
        .is_none())
    }
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
        Ok(PreparedWsol {
            unsigned,
            rpc: WsolBroadcastLane::from_selected(rpc),
        })
    }
    async fn simulate_unsigned(
        &self,
        prepared: &Self::Prepared,
        quote: &WsolQuoteSnapshot,
    ) -> Result<Option<u64>> {
        prepared.simulate(&self.read_rpc, quote).await
    }
    fn sign(&self, prepared: Self::Prepared, quote: &WsolQuoteSnapshot) -> Result<Self::Signed> {
        Ok(SignedWsol {
            artifact: self.client.sign_prepared_wsol(prepared.unsigned, quote)?,
            rpc: prepared.rpc,
        })
    }
    async fn simulate_signed(&self, signed: &Self::Signed) -> Result<bool> {
        signed.simulate(&self.read_rpc).await
    }
    async fn broadcast(&self, signed: &Self::Signed) -> Result<String> {
        Ok(self
            .client
            .broadcast_signed_wsol(&signed.artifact, &signed.rpc)
            .await?
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wsol_broadcast_adapter_actual_wallet_guard_rejects_before_empty_sdk_wallet_or_rpc_access(
    ) {
        use crate::sdk_client::wsol_conversion::{
            broadcast::tests::disabled_client, simulation::tests as fixture,
        };
        let legacy = fixture::Legacy::new(serde_json::Value::Null);
        let signed = SignedWsol {
            artifact: fixture::signed(true),
            rpc: WsolBroadcastLane::from_selected(Arc::new(
                legacy.client(solana_commitment_config::CommitmentConfig::confirmed()),
            )),
        };
        let before = signed.signed_bytes().to_vec();
        let wallet = WsolWallet {
            client: Arc::new(disabled_client()),
            read_rpc: Arc::new(
                crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap(),
            ),
        };
        assert_eq!(
            wallet.broadcast(&signed).await.unwrap_err().to_string(),
            crate::domain::execution_policy::BROADCAST_DISABLED_ERROR
        );
        assert_eq!(signed.signed_bytes(), before);
        assert!(legacy.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn wsol_broadcast_adapter_original_selected_lane_moves_through_wrappers_without_new_rpc_owner()
    {
        use crate::sdk_client::wsol_conversion::simulation::tests as fixture;
        let legacy = fixture::Legacy::new(serde_json::Value::Null);
        let rpc = Arc::new(legacy.client(solana_commitment_config::CommitmentConfig::finalized()));
        let weak = Arc::downgrade(&rpc);
        let prepared = PreparedWsol {
            unsigned: fixture::unsigned(true),
            rpc: WsolBroadcastLane::from_selected(rpc),
        };
        assert_eq!(weak.strong_count(), 1);
        // Only ownership is tested here; no signer is initialized or called.
        let signed = SignedWsol {
            artifact: fixture::signed(true),
            rpc: prepared.rpc,
        };
        drop(prepared.unsigned);
        assert_eq!(weak.strong_count(), 1);
        assert_eq!(signed.rpc.url(), fixture::URL);
        assert_eq!(
            signed.rpc.commitment(),
            solana_commitment_config::CommitmentConfig::finalized()
        );
        assert!(legacy.calls.lock().unwrap().is_empty());
        drop(signed);
        assert!(weak.upgrade().is_none());
    }

    #[tokio::test]
    async fn wsol_simulation_adapter_both_real_wrapper_paths_use_selected_shared_runtime_not_execution_rpc(
    ) {
        use crate::sdk_client::wsol_conversion::simulation::tests as fixture;
        use solana_commitment_config::CommitmentConfig;
        for error in [
            serde_json::Value::Null,
            serde_json::json!({"InstructionError":[0,{"Custom":42}]}),
        ] {
            let legacy = fixture::Legacy::new(fixture::result(error.clone()));
            let runtime = fixture::runtime(fixture::result(error.clone()), fixture::Mode::Good);
            let unsigned = fixture::unsigned(true);
            let quote = fixture::quote(&unsigned);
            let prepared = PreparedWsol {
                unsigned,
                rpc: WsolBroadcastLane::from_selected(Arc::new(
                    legacy.client(CommitmentConfig::confirmed()),
                )),
            };
            let report = prepared.simulate(&runtime, &quote).await;
            if error.is_null() {
                assert_eq!(report.unwrap(), Some(999));
            } else {
                assert!(report.is_err());
            }
            let signed = SignedWsol {
                artifact: fixture::signed(true),
                rpc: prepared.rpc,
            };
            let before = signed.artifact.signed_bytes().to_vec();
            assert_eq!(signed.simulate(&runtime).await.unwrap(), error.is_null());
            assert_eq!(signed.artifact.signed_bytes(), before.as_slice());
            assert!(legacy.calls.lock().unwrap().is_empty());
            let calls = runtime.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert!(calls.iter().all(|(url, body, _)| url == fixture::URL
                && body["params"][1]["commitment"] == "confirmed"));
        }
    }

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
