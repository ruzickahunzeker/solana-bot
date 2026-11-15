//! PumpSwap account/fee evidence only, not integer spend or signing authorization.
use super::{
    pumpapi_accounts::{MessageAccount, ResolvedPumpApiAccounts},
    pumpapi_assets::{check_mint, check_vault, decode_data, read, MAX_ACCOUNT_BYTES},
};
use crate::{
    application::trade_command::TradeCommand,
    domain::trade::{Address, Side, USD1_MINT, USDC_MINT},
};
use anyhow::{bail, Context};
use bincode::Options;
use sol_trade_sdk::instruction::utils::{
    pumpswap::{self, accounts, PumpSwapFeeBasisPoints},
    pumpswap_types::{effective_quote_reserves, pool_decode, POOL_DISCRIMINATOR},
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{clock::Clock, pubkey::Pubkey, transaction::VersionedTransaction};
use std::{collections::HashSet, time::Duration};

const LABEL: &str = "PumpAPI PumpSwap";

pub(super) async fn check_pumpswap(
    rpc: &RpcClient,
    wallet: Address,
    command: &TradeCommand,
    transaction: &VersionedTransaction,
    view: &ResolvedPumpApiAccounts,
) -> anyhow::Result<u64> {
    view.ensure_bound(wallet, transaction)?;
    if command.aggregator_sol
        || command.route.sol_hop_pool.is_some()
        || command.route.sol_hop_pool_id.is_some()
    {
        bail!("PumpAPI PumpSwap account check only supports a direct quote route")
    }
    let mut swaps = Vec::new();
    for ix in transaction.message.instructions() {
        if view.account(ix.program_id_index)?.address.to_bytes() == accounts::AMM_PROGRAM.to_bytes()
        {
            swaps.push(ix);
        }
    }
    if swaps.len() != 1 {
        bail!("PumpAPI PumpSwap check requires exactly one direct swap")
    }
    let ix = swaps[0];
    let buy = (ix.data.starts_with(&pumpswap::BUY_DISCRIMINATOR)
        || ix
            .data
            .starts_with(&pumpswap::BUY_EXACT_QUOTE_IN_DISCRIMINATOR))
        && ix.data.len() == 25
        && ix.data[24] <= 1;
    let sell = ix.data.starts_with(&pumpswap::SELL_DISCRIMINATOR) && ix.data.len() == 24;
    if (!buy && !sell) || ix.accounts.len() < 23 || ix.accounts.len() > 29 {
        bail!("PumpAPI PumpSwap instruction has unsupported layout")
    }
    let roles = ix
        .accounts
        .iter()
        .map(|i| view.account(*i))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let pool = pubkey(roles[0].address);
    if pool == Pubkey::default()
        || command
            .route
            .pool_id
            .is_some_and(|hint| hint != roles[0].address)
    {
        bail!("PumpAPI PumpSwap pool does not match local hint")
    }
    role(roles[0], pool, false, true)?;
    role(roles[1], pubkey(wallet), true, true)?;
    role(roles[2], accounts::GLOBAL_ACCOUNT, false, false)?;
    role(
        view.account(ix.program_id_index)?,
        accounts::AMM_PROGRAM,
        false,
        false,
    )?;
    tokio::time::timeout(
        Duration::from_secs(5),
        inspect(rpc, wallet, command, &roles, pool, buy, view.lookup_slot()),
    )
    .await
    .context("PumpAPI PumpSwap account check timed out")?
}

async fn inspect(
    rpc: &RpcClient,
    wallet: Address,
    command: &TradeCommand,
    roles: &[&MessageAccount],
    pool_id: Pubkey,
    buy: bool,
    min_slot: Option<u64>,
) -> anyhow::Result<u64> {
    // Message addresses are candidates until the same bank's PoolState confirms every reference.
    let keys = [
        pool_id,
        pubkey(roles[3].address),
        pubkey(roles[4].address),
        pubkey(roles[7].address),
        pubkey(roles[8].address),
        accounts::GLOBAL_ACCOUNT,
        accounts::FEE_CONFIG,
        solana_sdk::sysvar::clock::id(),
    ];
    if keys.iter().collect::<HashSet<_>>().len() != keys.len() {
        bail!("PumpAPI PumpSwap candidate account roles alias each other")
    }
    let (slot, snapshot) = read(rpc, &keys, min_slot, LABEL).await?;
    let bytes = decode_data(
        &snapshot[0],
        accounts::AMM_PROGRAM,
        MAX_ACCOUNT_BYTES,
        LABEL,
    )?;
    if bytes.get(..8) != Some(POOL_DISCRIMINATOR.as_slice()) {
        bail!("PumpAPI PumpSwap pool discriminator is invalid")
    }
    let pool = pool_decode(&bytes[8..]).context("PumpAPI PumpSwap PoolState layout is invalid")?;
    if pool.base_mint != keys[1]
        || pool.quote_mint != keys[2]
        || pool.pool_base_token_account != keys[3]
        || pool.pool_quote_token_account != keys[4]
        || pool.base_mint == pool.quote_mint
    {
        bail!("PumpAPI PumpSwap pool references do not match message candidates")
    }
    let mint = pubkey(command.route.mint);
    let quote = pubkey(command.quote_mint());
    if mint == quote
        || !((mint == pool.base_mint && quote == pool.quote_mint)
            || (mint == pool.quote_mint && quote == pool.base_mint))
    {
        bail!("PumpAPI PumpSwap pool does not match both distinct local mints")
    }
    let (input, output) = if command.amount.side() == Side::Buy {
        (quote, mint)
    } else {
        (mint, quote)
    };
    if buy != (input == pool.quote_mint && output == pool.base_mint) {
        bail!("PumpAPI PumpSwap opcode does not match local trade direction")
    }
    let clock_bytes = decode_data(&snapshot[7], solana_sdk::sysvar::id(), 40, LABEL)?;
    let clock: Clock = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(&clock_bytes)
        .context("PumpAPI PumpSwap Clock layout is invalid")?;
    if clock.slot != slot || clock.unix_timestamp < 0 {
        bail!("PumpAPI PumpSwap Clock does not match bank context")
    }
    let base_program: Pubkey = snapshot[1]
        .owner
        .parse()
        .context("PumpAPI PumpSwap base mint owner is invalid")?;
    let quote_program: Pubkey = snapshot[2]
        .owner
        .parse()
        .context("PumpAPI PumpSwap quote mint owner is invalid")?;
    let base = check_mint(&snapshot[1], base_program, None, clock.epoch, LABEL)?;
    let quote_info = check_mint(&snapshot[2], quote_program, None, clock.epoch, LABEL)?;
    let base_reserve = check_vault(
        &snapshot[3],
        pool.base_mint,
        base_program,
        &base.required,
        pool_id,
        LABEL,
    )?;
    let quote_reserve = check_vault(
        &snapshot[4],
        pool.quote_mint,
        quote_program,
        &quote_info.required,
        pool_id,
        LABEL,
    )?;
    let effective = effective_quote_reserves(quote_reserve, pool.virtual_quote_reserves)
        .filter(|n| *n > 0)
        .context("PumpAPI PumpSwap effective quote reserves are invalid")?;
    if base_reserve == 0 || quote_reserve == 0 {
        bail!("PumpAPI PumpSwap pool has no usable physical reserves")
    }
    if command.amount.side() == Side::Buy {
        let decimals = if matches!(quote.to_string().as_str(), USDC_MINT | USD1_MINT) {
            6
        } else {
            9
        };
        let actual = if input == pool.base_mint {
            base.decimals
        } else {
            quote_info.decimals
        };
        if actual != decimals {
            bail!("PumpAPI PumpSwap quote decimals do not match legacy request precision")
        }
    }
    let global_bytes = decode_data(
        &snapshot[5],
        accounts::AMM_PROGRAM,
        MAX_ACCOUNT_BYTES,
        LABEL,
    )?;
    let global = pumpswap::decode_global_config(&global_bytes)
        .context("PumpAPI PumpSwap GlobalConfig layout is invalid")?;
    // The SDK decoder skips disable_flags; unknown nonzero policy is not silently treated as enabled.
    if global_bytes.get(8 + 32 + 8 + 8).copied() != Some(0) {
        bail!("PumpAPI PumpSwap nonzero disable flags require audited policy")
    }
    let fee_bytes = decode_data(
        &snapshot[6],
        accounts::FEE_PROGRAM,
        MAX_ACCOUNT_BYTES,
        LABEL,
    )?;
    let config = pumpswap::decode_fee_config(&fee_bytes)
        .context("PumpAPI PumpSwap FeeConfig layout is invalid")?;
    check_fees(&PumpSwapFeeBasisPoints::new(
        global.lp_fee_basis_points,
        global.protocol_fee_basis_points,
        global.coin_creator_fee_basis_points,
    ))?;
    check_fees(&config.flat_fees)?;
    check_fees(&config.exotic_flat_fees)?;
    for tiers in [&config.fee_tiers, &config.stable_fee_tiers] {
        for tier in tiers {
            check_fees(&tier.fees)?;
        }
        if tiers
            .windows(2)
            .any(|w| w[0].market_cap_lamports_threshold >= w[1].market_cap_lamports_threshold)
        {
            bail!("PumpAPI PumpSwap fee tiers are not strictly ordered")
        }
    }
    if pool.creator_fee_bps > 10_000 {
        bail!("PumpAPI PumpSwap creator fee override is invalid")
    }
    let selected = pumpswap::try_compute_fee_basis_points_for_pool(
        Some(&config),
        Some(&global),
        &pool,
        base.supply,
        base_reserve,
        effective,
    )?;
    check_fees(&selected)?;
    for (index, key, writable) in [
        (3, pool.base_mint, false),
        (4, pool.quote_mint, false),
        (7, pool.pool_base_token_account, true),
        (8, pool.pool_quote_token_account, true),
        (11, base_program, false),
        (12, quote_program, false),
        (13, solana_system_interface::program::ID, false),
        (14, accounts::ASSOCIATED_TOKEN_PROGRAM, false),
        (15, accounts::EVENT_AUTHORITY, false),
        (16, accounts::AMM_PROGRAM, false),
    ] {
        role(roles[index], key, false, writable)?;
    }
    for index in [5, 6] {
        if roles[index].signer
            || !roles[index].writable
            || keys
                .iter()
                .any(|key| key.to_bytes() == roles[index].address.to_bytes())
        {
            bail!("PumpAPI PumpSwap user token account roles are invalid")
        }
    }
    if roles[5].address == roles[6].address {
        bail!("PumpAPI PumpSwap user token accounts alias each other")
    }
    let recipient = pubkey(roles[9].address);
    let allowed = if pool.is_mayhem_mode {
        recipient == global.reserved_fee_recipient
            || global.reserved_fee_recipients.contains(&recipient)
    } else {
        global.protocol_fee_recipients.contains(&recipient)
    };
    if recipient == Pubkey::default() || !allowed {
        bail!("PumpAPI PumpSwap protocol fee recipient is not authorized by GlobalConfig")
    }
    role(roles[9], recipient, false, false)?;
    role(
        roles[10],
        ata(recipient, pool.quote_mint, quote_program),
        false,
        true,
    )?;
    let creator = Pubkey::find_program_address(
        &[b"creator_vault", pool.coin_creator.as_ref()],
        &accounts::AMM_PROGRAM,
    )
    .0;
    role(
        roles[17],
        ata(creator, pool.quote_mint, quote_program),
        false,
        true,
    )?;
    role(roles[18], creator, false, false)?;
    let user = pubkey(wallet);
    let volume = pumpswap::get_user_volume_accumulator_pda(&user)
        .context("PumpAPI PumpSwap user volume PDA is unavailable")?;
    let mut cursor = 19;
    if buy {
        take(
            roles,
            &mut cursor,
            accounts::GLOBAL_VOLUME_ACCUMULATOR,
            false,
        )?;
        take(roles, &mut cursor, volume, true)?;
    }
    take(roles, &mut cursor, accounts::FEE_CONFIG, false)?;
    take(roles, &mut cursor, accounts::FEE_PROGRAM, false)?;
    if pool.is_cashback_coin {
        take(
            roles,
            &mut cursor,
            ata(volume, pool.quote_mint, quote_program),
            true,
        )?;
        if !buy {
            take(roles, &mut cursor, volume, true)?;
        }
    }
    if pool.coin_creator != Pubkey::default() {
        take(
            roles,
            &mut cursor,
            pumpswap::get_pool_v2_pda(&pool.base_mint)
                .context("PumpAPI PumpSwap pool-v2 PDA is unavailable")?,
            false,
        )?;
    }
    let extra = pubkey(
        roles
            .get(cursor)
            .context("PumpAPI PumpSwap missing buyback recipient")?
            .address,
    );
    if extra == Pubkey::default() || !global.buyback_fee_recipients.contains(&extra) {
        bail!("PumpAPI PumpSwap buyback fee recipient is not authorized by GlobalConfig")
    }
    take(roles, &mut cursor, extra, false)?;
    take(
        roles,
        &mut cursor,
        ata(extra, pool.quote_mint, quote_program),
        true,
    )?;
    if cursor != roles.len() {
        bail!("PumpAPI PumpSwap extra remaining accounts are not audited")
    }
    Ok(slot)
}

fn check_fees(f: &PumpSwapFeeBasisPoints) -> anyhow::Result<()> {
    if f.lp_fee_basis_points
        .checked_add(f.protocol_fee_basis_points)
        .and_then(|n| n.checked_add(f.coin_creator_fee_basis_points))
        .filter(|n| *n < 10_000)
        .is_none()
    {
        bail!("PumpAPI PumpSwap fee basis points are invalid")
    }
    Ok(())
}
fn pubkey(address: Address) -> Pubkey {
    Pubkey::new_from_array(address.to_bytes())
}
fn ata(owner: Pubkey, mint: Pubkey, program: Pubkey) -> Pubkey {
    spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
        &owner, &mint, &program,
    )
}
fn role(account: &MessageAccount, key: Pubkey, signer: bool, writable: bool) -> anyhow::Result<()> {
    if account.address.to_bytes() != key.to_bytes()
        || account.signer != signer
        || account.writable != writable
    {
        bail!("PumpAPI PumpSwap message account does not match expected role")
    }
    Ok(())
}
fn take(
    roles: &[&MessageAccount],
    cursor: &mut usize,
    key: Pubkey,
    writable: bool,
) -> anyhow::Result<()> {
    role(
        roles
            .get(*cursor)
            .context("PumpAPI PumpSwap missing remaining account")?,
        key,
        false,
        writable,
    )?;
    *cursor += 1;
    Ok(())
}

#[cfg(test)]
mod tests;
