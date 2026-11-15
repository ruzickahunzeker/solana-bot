//! Partial protocol evidence for direct CPMM only; not a signing or spend permit.
use anyhow::{bail, Context};
use bincode::Options;
use sol_trade_sdk::instruction::utils::{
    raydium_cpmm::{
        accounts, get_observation_state_pda, SWAP_BASE_IN_DISCRIMINATOR,
        SWAP_BASE_OUT_DISCRIMINATOR,
    },
    raydium_cpmm_types::{
        amm_config_decode, pool_state_decode, PoolState, AMM_CONFIG_DISCRIMINATOR, AMM_CONFIG_SIZE,
        POOL_STATE_DISCRIMINATOR, POOL_STATE_SIZE,
    },
};
use solana_account_decoder::UiAccount;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{clock::Clock, pubkey::Pubkey, transaction::VersionedTransaction};
use std::{collections::HashSet, time::Duration};

use super::pumpapi_accounts::{MessageAccount, ResolvedPumpApiAccounts};
use super::pumpapi_assets::{check_mint, check_vault, decode_data, read, MAX_ACCOUNT_BYTES};
use crate::{
    application::trade_command::TradeCommand,
    domain::trade::{Address, Pool, Side, USD1_MINT, USDC_MINT},
};

const LABEL: &str = "PumpAPI CPMM";
const SWAP_DISABLED: u8 = 1 << 2;

pub(crate) async fn check_cpmm_if_applicable(
    rpc: &RpcClient,
    wallet: Address,
    command: &TradeCommand,
    transaction: &VersionedTransaction,
    view: &ResolvedPumpApiAccounts,
) -> anyhow::Result<Option<u64>> {
    if !matches!(command.route.pool, Pool::RaydiumCpmm | Pool::StonkFunSwap) {
        return Ok(None);
    }
    view.ensure_bound(wallet, transaction)?;
    if command.aggregator_sol
        || command.route.sol_hop_pool.is_some()
        || command.route.sol_hop_pool_id.is_some()
    {
        bail!("PumpAPI CPMM account check only supports a direct quote route");
    }
    let pool_id = Pubkey::new_from_array(
        command
            .route
            .pool_id
            .context("PumpAPI CPMM check requires pool_id")?
            .to_bytes(),
    );
    let mut matches = Vec::new();
    for instruction in transaction.message.instructions() {
        if view
            .account(instruction.program_id_index)?
            .address
            .to_bytes()
            == accounts::RAYDIUM_CPMM.to_bytes()
        {
            matches.push(instruction);
        }
    }
    if matches.len() != 1 {
        bail!("PumpAPI CPMM check requires exactly one direct CPMM swap");
    }
    let instruction = matches[0];
    if instruction.accounts.len() != 13
        || instruction.data.len() != 24
        || (!instruction.data.starts_with(SWAP_BASE_IN_DISCRIMINATOR)
            && !instruction.data.starts_with(SWAP_BASE_OUT_DISCRIMINATOR))
    {
        bail!("PumpAPI CPMM instruction has unsupported layout");
    }
    let roles = instruction
        .accounts
        .iter()
        .map(|index| view.account(*index))
        .collect::<anyhow::Result<Vec<_>>>()?;
    require_role(
        roles[0],
        Pubkey::new_from_array(wallet.to_bytes()),
        true,
        true,
    )?;
    require_role(roles[3], pool_id, false, true)?;
    require_role(
        view.account(instruction.program_id_index)?,
        accounts::RAYDIUM_CPMM,
        false,
        false,
    )?;
    let slot = tokio::time::timeout(
        Duration::from_secs(5),
        inspect(rpc, command, pool_id, &roles, view.lookup_slot()),
    )
    .await
    .context("PumpAPI CPMM account check timed out")??;
    Ok(Some(slot))
}

async fn inspect(
    rpc: &RpcClient,
    command: &TradeCommand,
    pool_id: Pubkey,
    roles: &[&MessageAccount],
    minimum_slot: Option<u64>,
) -> anyhow::Result<u64> {
    let (first_slot, first) = read(rpc, &[pool_id], minimum_slot, LABEL).await?;
    let initial = decode_pool(&first[0])?;
    check_pair(command, &initial)?;
    let clock_id = solana_sdk::sysvar::clock::id();
    let keys = [
        pool_id,
        initial.token0_mint,
        initial.token1_mint,
        initial.token0_vault,
        initial.token1_vault,
        initial.amm_config,
        initial.observation_key,
        clock_id,
    ];
    if keys.iter().collect::<HashSet<_>>().len() != keys.len() {
        bail!("PumpAPI CPMM pool account roles alias each other");
    }
    let (slot, snapshot) = read(rpc, &keys, Some(first_slot), LABEL).await?;
    let pool = decode_pool(&snapshot[0])?;
    if !same_identity(&initial, &pool) {
        bail!("PumpAPI CPMM pool identity changed during account read");
    }
    check_pair(command, &pool)?;
    let clock_data = decode_data(&snapshot[7], solana_sdk::sysvar::id(), 40, LABEL)?;
    let clock: Clock = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(&clock_data)
        .context("PumpAPI CPMM Clock data is invalid")?;
    if clock.slot != slot || pool.recent_epoch > clock.epoch {
        bail!("PumpAPI CPMM bank and Clock context are inconsistent");
    }
    let unix =
        u64::try_from(clock.unix_timestamp).context("PumpAPI CPMM Clock timestamp is invalid")?;
    if pool.status & SWAP_DISABLED != 0 || pool.open_time > unix || pool.creator_fee_on > 2 {
        bail!("PumpAPI CPMM pool is not open for swaps");
    }
    let extensions0 = check_mint(
        &snapshot[1],
        pool.token0_program,
        Some(pool.mint0_decimals),
        clock.epoch,
        LABEL,
    )?;
    let extensions1 = check_mint(
        &snapshot[2],
        pool.token1_program,
        Some(pool.mint1_decimals),
        clock.epoch,
        LABEL,
    )?;
    check_config(&snapshot[5])?;
    let observation = get_observation_state_pda(&pool_id)
        .context("PumpAPI CPMM observation PDA is unavailable")?;
    if pool.observation_key != observation
        || decode_data(
            &snapshot[6],
            accounts::RAYDIUM_CPMM,
            MAX_ACCOUNT_BYTES,
            LABEL,
        )?
        .len()
            < 8
    {
        bail!("PumpAPI CPMM observation account is invalid");
    }
    let reserve0 = check_vault(
        &snapshot[3],
        pool.token0_mint,
        pool.token0_program,
        &extensions0.required,
        accounts::AUTHORITY,
        LABEL,
    )?;
    let reserve1 = check_vault(
        &snapshot[4],
        pool.token1_mint,
        pool.token1_program,
        &extensions1.required,
        accounts::AUTHORITY,
        LABEL,
    )?;
    for (reserve, protocol, fund, creator) in [
        (
            reserve0,
            pool.protocol_fees_token0,
            pool.fund_fees_token0,
            pool.creator_fees_token0,
        ),
        (
            reserve1,
            pool.protocol_fees_token1,
            pool.fund_fees_token1,
            pool.creator_fees_token1,
        ),
    ] {
        if protocol
            .checked_add(fund)
            .and_then(|n| n.checked_add(creator))
            .and_then(|n| reserve.checked_sub(n))
            .filter(|n| *n > 0)
            .is_none()
        {
            bail!("PumpAPI CPMM vault has no usable reserve after accrued fees");
        }
    }
    let mint = Pubkey::new_from_array(command.route.mint.to_bytes());
    let quote = Pubkey::new_from_array(command.quote_mint().to_bytes());
    let (input, output) = if command.amount.side() == Side::Buy {
        (quote, mint)
    } else {
        (mint, quote)
    };
    let (input_vault, output_vault, input_program, output_program, input_decimals) =
        if input == pool.token0_mint {
            (
                pool.token0_vault,
                pool.token1_vault,
                pool.token0_program,
                pool.token1_program,
                pool.mint0_decimals,
            )
        } else {
            (
                pool.token1_vault,
                pool.token0_vault,
                pool.token1_program,
                pool.token0_program,
                pool.mint1_decimals,
            )
        };
    if command.amount.side() == Side::Buy {
        let decimals = if matches!(quote.to_string().as_str(), USDC_MINT | USD1_MINT) {
            6
        } else {
            9
        };
        if input_decimals != decimals {
            bail!("PumpAPI CPMM quote decimals do not match legacy request precision");
        }
    }
    require_role(roles[1], accounts::AUTHORITY, false, false)?;
    require_role(roles[2], pool.amm_config, false, false)?;
    require_role(roles[6], input_vault, false, true)?;
    require_role(roles[7], output_vault, false, true)?;
    require_role(roles[8], input_program, false, false)?;
    require_role(roles[9], output_program, false, false)?;
    require_role(roles[10], input, false, false)?;
    require_role(roles[11], output, false, false)?;
    require_role(roles[12], pool.observation_key, false, true)?;
    if roles[4].signer
        || !roles[4].writable
        || roles[5].signer
        || !roles[5].writable
        || roles[4].address == roles[5].address
        || [roles[4].address, roles[5].address]
            .iter()
            .any(|a| keys.iter().any(|key| key.to_bytes() == a.to_bytes()))
    {
        bail!("PumpAPI CPMM user token account roles are invalid");
    }
    Ok(slot)
}

fn require_role(
    role: &MessageAccount,
    key: Pubkey,
    signer: bool,
    writable: bool,
) -> anyhow::Result<()> {
    if role.address.to_bytes() != key.to_bytes()
        || role.signer != signer
        || role.writable != writable
    {
        bail!("PumpAPI CPMM message account does not match expected pool role");
    }
    Ok(())
}

fn decode_pool(account: &UiAccount) -> anyhow::Result<PoolState> {
    let data = decode_data(account, accounts::RAYDIUM_CPMM, MAX_ACCOUNT_BYTES, LABEL)?;
    if data.len() < 8 + POOL_STATE_SIZE || data[..8] != POOL_STATE_DISCRIMINATOR {
        bail!("PumpAPI CPMM pool header is invalid")
    }
    pool_state_decode(&data[8..]).context("PumpAPI CPMM PoolState layout is invalid")
}

fn check_pair(command: &TradeCommand, pool: &PoolState) -> anyhow::Result<()> {
    let mint = Pubkey::new_from_array(command.route.mint.to_bytes());
    let quote = Pubkey::new_from_array(command.quote_mint().to_bytes());
    if mint == quote
        || pool.token0_mint == pool.token1_mint
        || !((mint == pool.token0_mint && quote == pool.token1_mint)
            || (mint == pool.token1_mint && quote == pool.token0_mint))
    {
        bail!("PumpAPI CPMM pool does not match both distinct requested mints");
    }
    Ok(())
}

fn same_identity(a: &PoolState, b: &PoolState) -> bool {
    fn keys(p: &PoolState) -> [Pubkey; 10] {
        [
            p.amm_config,
            p.pool_creator,
            p.token0_vault,
            p.token1_vault,
            p.lp_mint,
            p.token0_mint,
            p.token1_mint,
            p.token0_program,
            p.token1_program,
            p.observation_key,
        ]
    }
    keys(a) == keys(b)
        && [
            a.auth_bump,
            a.lp_mint_decimals,
            a.mint0_decimals,
            a.mint1_decimals,
        ] == [
            b.auth_bump,
            b.lp_mint_decimals,
            b.mint0_decimals,
            b.mint1_decimals,
        ]
}

fn check_config(account: &UiAccount) -> anyhow::Result<()> {
    let data = decode_data(account, accounts::RAYDIUM_CPMM, MAX_ACCOUNT_BYTES, LABEL)?;
    if data.len() < 8 + AMM_CONFIG_SIZE || data[..8] != AMM_CONFIG_DISCRIMINATOR {
        bail!("PumpAPI CPMM fee config header is invalid")
    }
    let config =
        amm_config_decode(&data[8..]).context("PumpAPI CPMM fee config layout is invalid")?;
    if [
        config.trade_fee_rate,
        config.protocol_fee_rate,
        config.fund_fee_rate,
        config.creator_fee_rate,
        config.creator_fee_share_rate,
    ]
    .iter()
    .any(|rate| *rate > 1_000_000)
    {
        bail!("PumpAPI CPMM fee config rate is invalid");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
