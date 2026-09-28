use crate::common::SolanaRpcClient;
use anyhow::{anyhow, bail, Context};
use dynamic_bonding_curve::state::{PoolConfig, VirtualPool};
use solana_account_decoder::UiAccountEncoding;
use solana_sdk::{account::Account, pubkey::Pubkey};

pub mod accounts {
    use solana_sdk::{instruction::AccountMeta, pubkey, pubkey::Pubkey};

    pub const PROGRAM: Pubkey = pubkey!("dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN");
    pub const POOL_AUTHORITY: Pubkey = pubkey!("FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM");
    pub const SYSVAR_INSTRUCTIONS: Pubkey = pubkey!("Sysvar1nstructions1111111111111111111111111");

    pub const PROGRAM_META: AccountMeta =
        AccountMeta { pubkey: PROGRAM, is_signer: false, is_writable: false };
    pub const POOL_AUTHORITY_META: AccountMeta =
        AccountMeta { pubkey: POOL_AUTHORITY, is_signer: false, is_writable: false };
    pub const SYSVAR_INSTRUCTIONS_META: AccountMeta =
        AccountMeta { pubkey: SYSVAR_INSTRUCTIONS, is_signer: false, is_writable: false };
}

pub const VIRTUAL_POOL_DISCRIMINATOR: [u8; 8] = [213, 224, 5, 209, 98, 69, 119, 92];
pub const POOL_CONFIG_DISCRIMINATOR: [u8; 8] = [26, 108, 14, 123, 116, 230, 129, 43];
pub const SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];
pub const SWAP_MODE_EXACT_IN: u8 = 0;
pub const VIRTUAL_POOL_ACCOUNT_SIZE: u64 = 8 + 416;
pub const BASE_MINT_OFFSET: usize = 136;

pub fn decode_virtual_pool(account: &Account) -> anyhow::Result<VirtualPool> {
    decode_account(account, accounts::PROGRAM, VIRTUAL_POOL_DISCRIMINATOR, "VirtualPool")
}

pub fn decode_pool_config(account: &Account) -> anyhow::Result<PoolConfig> {
    decode_account(account, accounts::PROGRAM, POOL_CONFIG_DISCRIMINATOR, "PoolConfig")
}

fn decode_account<T: bytemuck::Pod + Copy>(
    account: &Account,
    owner: Pubkey,
    discriminator: [u8; 8],
    name: &str,
) -> anyhow::Result<T> {
    if account.owner != owner {
        bail!("{name} account is not owned by Meteora DBC");
    }
    let actual = account.data.get(..8).context("DBC account is shorter than its discriminator")?;
    if actual != discriminator {
        bail!("invalid Meteora DBC {name} discriminator");
    }
    let size = std::mem::size_of::<T>();
    let bytes = account
        .data
        .get(8..8 + size)
        .with_context(|| format!("Meteora DBC {name} account is shorter than {size} bytes"))?;
    Ok(bytemuck::pod_read_unaligned(bytes))
}

pub async fn fetch_pool(
    rpc: &SolanaRpcClient,
    pool_address: &Pubkey,
) -> anyhow::Result<VirtualPool> {
    decode_virtual_pool(&rpc.get_account(pool_address).await?)
}

pub async fn fetch_config(
    rpc: &SolanaRpcClient,
    config_address: &Pubkey,
) -> anyhow::Result<PoolConfig> {
    decode_pool_config(&rpc.get_account(config_address).await?)
}

pub async fn find_pool_by_base_mint(
    rpc: &SolanaRpcClient,
    base_mint: &Pubkey,
) -> anyhow::Result<(Pubkey, VirtualPool)> {
    let config = solana_rpc_client_api::config::RpcProgramAccountsConfig {
        filters: Some(vec![
            solana_rpc_client_api::filter::RpcFilterType::DataSize(VIRTUAL_POOL_ACCOUNT_SIZE),
            solana_rpc_client_api::filter::RpcFilterType::Memcmp(
                solana_client::rpc_filter::Memcmp::new_base58_encoded(
                    0,
                    &VIRTUAL_POOL_DISCRIMINATOR,
                ),
            ),
            solana_rpc_client_api::filter::RpcFilterType::Memcmp(
                solana_client::rpc_filter::Memcmp::new_base58_encoded(
                    BASE_MINT_OFFSET,
                    base_mint.as_ref(),
                ),
            ),
        ]),
        account_config: solana_rpc_client_api::config::RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            data_slice: None,
            commitment: None,
            min_context_slot: None,
        },
        with_context: None,
        sort_results: None,
    };
    #[allow(deprecated)]
    let accounts = rpc.get_program_ui_accounts_with_config(&accounts::PROGRAM, config).await?;
    let mut decoded = accounts
        .into_iter()
        .filter_map(|(address, ui_account)| {
            ui_account
                .to_account()
                .and_then(|account| decode_virtual_pool(&account).ok())
                .map(|pool| (address, pool))
        })
        .collect::<Vec<_>>();
    decoded.retain(|(_, pool)| pool.pool_state.base_mint == *base_mint);
    decoded.sort_by(|a, b| {
        a.1.pool_state
            .is_migrated
            .cmp(&b.1.pool_state.is_migrated)
            .then_with(|| b.1.pool_state.quote_reserve.cmp(&a.1.pool_state.quote_reserve))
    });
    decoded
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No Meteora DBC pool found for mint {base_mint}"))
}

#[inline]
pub fn get_event_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[b"__event_authority"], &accounts::PROGRAM).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbc_offsets_match_current_virtual_pool_layout() {
        assert_eq!(std::mem::size_of::<VirtualPool>(), 416);
        let pool = VirtualPool::default();
        let start = &pool as *const _ as usize;
        let base_mint = &pool.pool_state.base_mint as *const _ as usize;
        assert_eq!(8 + base_mint - start, BASE_MINT_OFFSET);
    }

    #[test]
    fn event_authority_matches_official_idl() {
        assert_eq!(
            get_event_authority_pda(),
            solana_sdk::pubkey!("8Ks12pbrD6PXxfty1hVQiE9sc289zgU1zHkvXhrSdriF")
        );
    }
}
