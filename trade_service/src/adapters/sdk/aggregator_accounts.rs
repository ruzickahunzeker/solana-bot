use crate::{
    adapters::rpc::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime},
    domain::trade::Address,
};
use anyhow::{bail, Result};
use sol_trade_sdk::common::{
    fast_fn::get_associated_token_address_with_program_id_fast, spl_token, spl_token_2022,
};
use solana_address_lookup_table_interface::{program as lookup_program, state::AddressLookupTable};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;

pub(crate) async fn target_ata<T: ReadHttpTransport>(
    runtime: &SharedReadRuntime<T>,
    url: &str,
    commitment: CommitmentConfig,
    wallet: Pubkey,
    mint: Pubkey,
) -> Result<(Pubkey, Pubkey)> {
    let account = runtime
        .aggregator_account(url, Address::from_bytes(mint.to_bytes()), commitment)
        .await?;
    if account.owner != spl_token::ID && account.owner != spl_token_2022::ID {
        bail!("aggregator target mint is not owned by a supported token program");
    }
    let ata = get_associated_token_address_with_program_id_fast(&wallet, &mint, &account.owner);
    Ok((ata, account.owner))
}

pub(crate) async fn lookup_table<T: ReadHttpTransport>(
    runtime: &SharedReadRuntime<T>,
    url: &str,
    commitment: CommitmentConfig,
    address: Pubkey,
) -> Result<Vec<Pubkey>> {
    let account = runtime
        .aggregator_account(url, Address::from_bytes(address.to_bytes()), commitment)
        .await?;
    if account.owner != lookup_program::id() {
        bail!("aggregator lookup table has invalid owner");
    }
    let table = AddressLookupTable::deserialize(&account.data)
        .map_err(|_| anyhow::anyhow!("invalid aggregator lookup table data"))?;
    if table.meta.deactivation_slot != u64::MAX {
        bail!("aggregator lookup table is deactivating or deactivated");
    }
    Ok(table.addresses.into_owned())
}

#[cfg(test)]
mod tests;
