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
}

impl SdkPoolReadBackend {
    pub fn new(wallets: Arc<WalletRegistry>) -> Self {
        Self { wallets }
    }
}

impl PoolReadBackend for SdkPoolReadBackend {
    async fn resolve(&self, mint: Address) -> Result<PoolRouteHint> {
        self.wallets
            .default()
            .resolve_pool(&Pubkey::new_from_array(mint.to_bytes()))
            .await
    }

    async fn warmup(&self, hint: &PoolRouteHint) -> Result<()> {
        let mint = Pubkey::new_from_array(hint.mint.to_bytes());
        let quote_mint = Pubkey::new_from_array(hint.quote_mint.to_bytes());
        let pool_id = hint.pool_id.map(|id| Pubkey::new_from_array(id.to_bytes()));
        self.wallets
            .default()
            .warmup_params(hint.pool, &mint, &quote_mint, pool_id.as_ref())
            .await
    }

    async fn sol_hop(&self, quote_mint: Address) -> Result<Address> {
        let quote = Pubkey::new_from_array(quote_mint.to_bytes());
        Ok(Address::from_bytes(
            self.wallets
                .default()
                .resolve_stonkfun_sol_hop(&quote)
                .await?
                .to_bytes(),
        ))
    }
}
