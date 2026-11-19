use anyhow::{bail, Context};
use dashmap::DashMap;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

use crate::config::{MevConfig, WalletConfig};
use crate::domain::execution_policy::BroadcastPolicy;
use crate::models::WalletInfo;
use crate::sdk_client::SdkTradeClient;
use tracing::warn;

pub struct WalletRegistry {
    default_wallet_id: String,
    wallets: DashMap<String, Arc<SdkTradeClient>>,
    management_mutation: tokio::sync::Mutex<()>,
    rpc_urls: Vec<String>,
    mev: MevConfig,
    commitment: CommitmentConfig,
    enable_sdk_log: bool,
    broadcast_policy: BroadcastPolicy,
}

pub struct WalletManagementMutation<'a> {
    registry: &'a WalletRegistry,
    _guard: tokio::sync::MutexGuard<'a, ()>,
}

impl WalletManagementMutation<'_> {
    fn validate(&self, registry: &WalletRegistry) -> anyhow::Result<()> {
        if !std::ptr::eq(self.registry, registry) {
            bail!("wallet mutation belongs to another registry");
        }
        Ok(())
    }
}

impl WalletRegistry {
    // No signer or SDK client is constructed. Only for HTTP rejection/read-only tests.
    #[cfg(test)]
    pub(crate) fn empty_for_http_tests(default_wallet_id: String, mev: MevConfig) -> Self {
        Self {
            default_wallet_id,
            wallets: DashMap::new(),
            management_mutation: tokio::sync::Mutex::new(()),
            rpc_urls: Vec::new(),
            mev,
            commitment: CommitmentConfig::processed(),
            enable_sdk_log: false,
            broadcast_policy: BroadcastPolicy::Enabled,
        }
    }

    pub(crate) async fn new(
        wallet_configs: &[WalletConfig],
        default_wallet_id: String,
        rpc_urls: Vec<String>,
        mev: MevConfig,
        commitment: CommitmentConfig,
        enable_sdk_log: bool,
        broadcast_policy: BroadcastPolicy,
    ) -> anyhow::Result<Self> {
        let registry = Self {
            default_wallet_id,
            wallets: DashMap::new(),
            management_mutation: tokio::sync::Mutex::new(()),
            rpc_urls,
            mev,
            commitment,
            enable_sdk_log,
            broadcast_policy,
        };

        for wallet in wallet_configs {
            let sdk = match registry.build_client(wallet).await {
                Ok(sdk) => sdk,
                Err(error) if wallet.wallet_id == registry.default_wallet_id => {
                    return Err(error);
                }
                Err(error) => {
                    warn!(
                        wallet_id = %wallet.wallet_id,
                        error = %error,
                        "skip invalid non-default wallet"
                    );
                    continue;
                }
            };
            registry
                .wallets
                .insert(wallet.wallet_id.clone(), Arc::new(sdk));
        }

        if !registry.wallets.contains_key(&registry.default_wallet_id) {
            bail!(
                "DEFAULT_WALLET_ID {} not found in wallet config",
                registry.default_wallet_id
            );
        }

        Ok(registry)
    }

    pub fn default_wallet_id(&self) -> &str {
        &self.default_wallet_id
    }

    pub fn resolve_wallet_id(&self, wallet_id: Option<&str>) -> String {
        wallet_id.unwrap_or(&self.default_wallet_id).to_string()
    }

    pub fn get(&self, wallet_id: &str) -> Option<Arc<SdkTradeClient>> {
        self.wallets.get(wallet_id).map(|entry| entry.clone())
    }

    pub(crate) fn read_access(
        &self,
        wallet_id: &str,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> Option<crate::adapters::rpc::wallet_reads::RpcWalletReader> {
        self.get(wallet_id)
            .map(|client| client.read_access(read_rpc))
    }

    pub(crate) fn default_read_access(
        &self,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> crate::adapters::rpc::wallet_reads::RpcWalletReader {
        self.read_access(&self.default_wallet_id, read_rpc)
            .expect("default wallet exists")
    }

    pub fn default(&self) -> Arc<SdkTradeClient> {
        self.get(&self.default_wallet_id)
            .expect("default wallet exists")
    }

    pub fn infos(&self) -> Vec<WalletInfo> {
        self.public_infos()
            .into_iter()
            .map(|wallet| WalletInfo {
                wallet_id: wallet.wallet_id,
                pubkey: wallet.pubkey.to_string(),
            })
            .collect()
    }

    pub(crate) fn public_infos(&self) -> Vec<crate::domain::wallet::WalletDescriptor> {
        let mut wallets = self
            .wallets
            .iter()
            .map(|entry| crate::domain::wallet::WalletDescriptor {
                wallet_id: entry.key().clone(),
                pubkey: crate::domain::trade::Address::from_bytes(
                    entry.value().wallet().to_bytes(),
                ),
            })
            .collect::<Vec<_>>();
        wallets.sort_by(|a, b| a.wallet_id.cmp(&b.wallet_id));
        wallets
    }

    pub fn wallet_count(&self) -> usize {
        self.wallets.len()
    }

    // Hold across repository mutation and runtime installation/removal, not just add().
    pub(crate) async fn begin_management_mutation(&self) -> WalletManagementMutation<'_> {
        WalletManagementMutation {
            registry: self,
            _guard: self.management_mutation.lock().await,
        }
    }

    pub async fn add(
        &self,
        mutation: &mut WalletManagementMutation<'_>,
        wallet: &WalletConfig,
    ) -> anyhow::Result<WalletInfo> {
        mutation.validate(self)?;
        if let Some(existing) = self.get(&wallet.wallet_id) {
            return Ok(WalletInfo {
                wallet_id: wallet.wallet_id.clone(),
                pubkey: existing.wallet().to_string(),
            });
        }
        let sdk = self.build_client(wallet).await?;
        let info = WalletInfo {
            wallet_id: wallet.wallet_id.clone(),
            pubkey: sdk.wallet().to_string(),
        };
        self.wallets.insert(wallet.wallet_id.clone(), Arc::new(sdk));
        Ok(info)
    }

    pub fn remove(
        &self,
        mutation: &mut WalletManagementMutation<'_>,
        wallet_id: &str,
    ) -> anyhow::Result<()> {
        mutation.validate(self)?;
        if wallet_id == self.default_wallet_id {
            bail!("default wallet cannot be removed");
        }
        self.wallets
            .remove(wallet_id)
            .with_context(|| format!("wallet {wallet_id} is not loaded"))?;
        Ok(())
    }

    pub fn default_pubkey(&self) -> Pubkey {
        self.default().wallet()
    }

    async fn build_client(&self, wallet: &WalletConfig) -> anyhow::Result<SdkTradeClient> {
        SdkTradeClient::new(
            self.rpc_urls.clone(),
            wallet.private_key_base58.clone(),
            self.mev.clone(),
            self.commitment.clone(),
            self.enable_sdk_log,
            self.broadcast_policy,
        )
        .await
        .with_context(|| format!("initialize wallet {}", wallet.wallet_id))
    }
}
