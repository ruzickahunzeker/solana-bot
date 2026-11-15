use anyhow::bail;
use solana_sdk::pubkey::Pubkey;

pub(crate) fn ensure_pool_contains_mint(
    pool: &str,
    mint: &Pubkey,
    pool_mints: &[Pubkey],
) -> anyhow::Result<()> {
    if pool_mints.iter().any(|pool_mint| pool_mint == mint) {
        return Ok(());
    }
    bail!(
        "{pool} pool does not contain requested mint {mint}; pool mints: {}",
        pool_mints
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub(crate) fn ensure_pool_contains_quote(
    pool: &str,
    quote_mint: &Pubkey,
    pool_mints: &[Pubkey],
) -> anyhow::Result<()> {
    if pool_mints.iter().any(|pool_mint| pool_mint == quote_mint) {
        return Ok(());
    }
    bail!(
        "{pool} pool does not contain requested quote_mint {quote_mint}; pool mints: {}",
        pool_mints
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}
