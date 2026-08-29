use crate::{
    common::SolanaRpcClient,
    instruction::utils::meteora_dbc::{fetch_config, fetch_pool, find_pool_by_base_mint},
};
use anyhow::{bail, Context};
use dynamic_bonding_curve::state::{PoolConfig, VirtualPool};
use solana_sdk::pubkey::Pubkey;

/// Accounts and current curve state required by Meteora Dynamic Bonding Curve `swap2`.
#[derive(Clone)]
pub struct MeteoraDbcParams {
    pub pool: Pubkey,
    pub config: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
    pub virtual_pool: VirtualPool,
    pub pool_config: PoolConfig,
    pub referral_token_account: Option<Pubkey>,
    pub include_instructions_sysvar: bool,
}

impl MeteoraDbcParams {
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> anyhow::Result<Self> {
        let virtual_pool = fetch_pool(rpc, pool_address).await?;
        let state = &virtual_pool.pool_state;
        if state.is_migrated != 0 {
            bail!("Meteora DBC pool {pool_address} has migrated");
        }
        let pool_config = fetch_config(rpc, &state.config).await?;
        let mint_accounts = rpc
            .get_multiple_accounts(&[state.base_mint, pool_config.quote_mint])
            .await?;
        let base_token_program = mint_accounts
            .first()
            .and_then(Option::as_ref)
            .map(|account| account.owner)
            .context("Meteora DBC base mint account not found")?;
        let quote_token_program = mint_accounts
            .get(1)
            .and_then(Option::as_ref)
            .map(|account| account.owner)
            .context("Meteora DBC quote mint account not found")?;
        Ok(Self {
            pool: *pool_address,
            config: state.config,
            base_vault: state.base_vault,
            quote_vault: state.quote_vault,
            base_mint: state.base_mint,
            quote_mint: pool_config.quote_mint,
            base_token_program,
            quote_token_program,
            virtual_pool,
            pool_config,
            referral_token_account: None,
            // Harmless when inactive and required by rate-limited configurations.
            include_instructions_sysvar: true,
        })
    }

    pub async fn from_base_mint_by_rpc(
        rpc: &SolanaRpcClient,
        base_mint: &Pubkey,
    ) -> anyhow::Result<Self> {
        let (pool, _) = find_pool_by_base_mint(rpc, base_mint).await?;
        Self::from_pool_address_by_rpc(rpc, &pool).await
    }

    pub fn with_referral_token_account(mut self, referral: Pubkey) -> Self {
        self.referral_token_account = Some(referral);
        self
    }
}
