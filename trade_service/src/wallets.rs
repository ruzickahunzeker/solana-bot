use anyhow::{bail, Context};
use dashmap::DashMap;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

use crate::config::{MevConfig, WalletConfig};
use crate::models::WalletInfo;
use crate::sdk_client::SdkTradeClient;
use tracing::warn;

pub struct WalletRegistry {
    default_wallet_id: String,
    wallets: DashMap<String, Arc<SdkTradeClient>>,
    rpc_urls: Vec<String>,
    mev: MevConfig,
    commitment: CommitmentConfig,
    enable_sdk_log: bool,
}

impl WalletRegistry {
    pub async fn new(
        wallet_configs: &[WalletConfig],
        default_wallet_id: String,
        rpc_urls: Vec<String>,
        mev: MevConfig,
        commitment: CommitmentConfig,
        enable_sdk_log: bool,
    ) -> anyhow::Result<Self> {
        let registry = Self {
            default_wallet_id,
            wallets: DashMap::new(),
            rpc_urls,
            mev,
            commitment,
            enable_sdk_log,
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

    pub fn default(&self) -> Arc<SdkTradeClient> {
        self.get(&self.default_wallet_id)
            .expect("default wallet exists")
    }

    pub fn infos(&self) -> Vec<WalletInfo> {
        let mut wallets = self
            .wallets
            .iter()
            .map(|entry| WalletInfo {
                wallet_id: entry.key().clone(),
                pubkey: entry.value().wallet().to_string(),
            })
            .collect::<Vec<_>>();
        wallets.sort_by(|a, b| a.wallet_id.cmp(&b.wallet_id));
        wallets
    }

    pub fn wallet_count(&self) -> usize {
        self.wallets.len()
    }

    pub async fn add(&self, wallet: &WalletConfig) -> anyhow::Result<WalletInfo> {
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

    pub fn remove(&self, wallet_id: &str) -> anyhow::Result<()> {
        if wallet_id == self.default_wallet_id {
            bail!("default wallet cannot be removed");
        }
        self.wallets
            .remove(wallet_id)
            .with_context(|| format!("wallet {wallet_id} is not loaded"))?;
        Ok(())
    }

    pub fn rpcs(&self) -> Vec<Arc<sol_trade_sdk::common::SolanaRpcClient>> {
        self.default().rpcs()
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
        )
        .await
        .with_context(|| format!("initialize wallet {}", wallet.wallet_id))
    }
}
