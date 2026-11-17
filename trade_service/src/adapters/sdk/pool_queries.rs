use crate::{
    application::ports::pool_queries::PoolReadBackend,
    domain::{route::PoolRouteHint, trade::Address},
    wallets::WalletRegistry,
};
use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

pub(crate) struct SdkPoolReadBackend {
    wallets: Arc<WalletRegistry>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}

impl SdkPoolReadBackend {
    pub fn new(
        wallets: Arc<WalletRegistry>,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> Self {
        Self { wallets, read_rpc }
    }
}

impl PoolReadBackend for SdkPoolReadBackend {
    async fn resolve(&self, mint: Address) -> Result<PoolRouteHint> {
        self.wallets
            .default()
            .resolve_pool(
                &Pubkey::new_from_array(mint.to_bytes()),
                self.read_rpc.clone(),
            )
            .await
    }

    async fn warmup(&self, hint: &PoolRouteHint) -> Result<()> {
        self.wallets
            .default()
            .warmup_params(hint, self.read_rpc.clone())
            .await
    }

    async fn sol_hop(&self, quote_mint: Address) -> Result<Address> {
        let quote = Pubkey::new_from_array(quote_mint.to_bytes());
        Ok(Address::from_bytes(
            self.wallets
                .default()
                .resolve_stonkfun_sol_hop(&quote, self.read_rpc.clone())
                .await?
                .to_bytes(),
        ))
    }
}
