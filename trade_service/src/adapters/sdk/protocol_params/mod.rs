//! SDK-specific read/parameter mapping only; no payer, signing or submitting capability.
pub(crate) mod checks;
pub(crate) mod quote_tokens;
#[cfg(test)]
mod tests;

use crate::domain::trade::{Pool, WSOL_MINT};
use anyhow::{anyhow, bail, Context};
use checks::{ensure_pool_contains_mint, ensure_pool_contains_quote};
use quote_tokens::is_usd1_quote;
use sol_trade_sdk::trading::core::params::{
    BonkParams, DexParamEnum, MeteoraDammV2Params, MeteoraDbcParams, PumpFunParams, PumpSwapParams,
    RaydiumAmmV4Params, RaydiumCpmmParams, StonkFunParams, StonkFunViaSolParams,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub(crate) async fn warmup_on_rpc(
    rpc: &RpcClient,
    pool: Pool,
    mint: &Pubkey,
    quote_mint: &Pubkey,
    pool_id: Option<&Pubkey>,
) -> anyhow::Result<()> {
    match pool {
        Pool::Pump => {
            let params = PumpFunParams::from_mint_by_rpc(rpc, mint).await?;
            if params.bonding_curve.complete {
                bail!("pump bonding curve is complete");
            }
            Ok(())
        }
        Pool::PumpAmm => {
            let params = if let Some(pool_id) = pool_id {
                PumpSwapParams::from_pool_address_by_rpc(rpc, pool_id).await?
            } else {
                PumpSwapParams::from_mint_by_rpc(rpc, mint).await?
            };
            ensure_pool_contains_quote(
                "pump-amm",
                quote_mint,
                &[params.base_mint, params.quote_mint],
            )?;
            Ok(())
        }
        Pool::Bonk => BonkParams::from_mint_by_rpc(rpc, mint, is_usd1_quote(quote_mint))
            .await
            .map(|_| ()),
        Pool::StonkFun => {
            let pool_id = pool_id.context("pool_id is required for stonkfun")?;
            load_stonkfun_params(rpc, pool_id, mint, quote_mint).await?;
            Ok(())
        }
        Pool::StonkFunSwap => {
            let pool_id = pool_id.context("pool_id is required for stonkfun-swap")?;
            let params = RaydiumCpmmParams::from_pool_address_by_rpc(rpc, pool_id).await?;
            ensure_pool_contains_mint(
                "stonkfun-swap",
                mint,
                &[params.base_mint, params.quote_mint],
            )?;
            ensure_pool_contains_quote(
                "stonkfun-swap",
                quote_mint,
                &[params.base_mint, params.quote_mint],
            )?;
            Ok(())
        }
        Pool::MeteoraDammV2 => {
            let pool_id = pool_id.context("pool_id is required for meteora-damm-v2")?;
            let params = MeteoraDammV2Params::from_pool_address_by_rpc(rpc, pool_id).await?;
            ensure_pool_contains_quote(
                "meteora-damm-v2",
                quote_mint,
                &[params.token_a_mint, params.token_b_mint],
            )?;
            Ok(())
        }
        Pool::MeteoraDbc => {
            let pool_id = pool_id.context("pool_id is required for meteora-dbc")?;
            let params = MeteoraDbcParams::from_pool_address_by_rpc(rpc, pool_id).await?;
            ensure_pool_contains_mint("meteora-dbc", mint, &[params.base_mint])?;
            ensure_pool_contains_quote("meteora-dbc", quote_mint, &[params.quote_mint])?;
            Ok(())
        }
        Pool::RaydiumAmmV4 => {
            let pool_id = pool_id.context("pool_id is required for raydium-amm-v4")?;
            let params = RaydiumAmmV4Params::from_amm_address_by_rpc(rpc, *pool_id).await?;
            ensure_pool_contains_quote(
                "raydium-amm-v4",
                quote_mint,
                &[params.coin_mint, params.pc_mint],
            )?;
            Ok(())
        }
        Pool::RaydiumCpmm => {
            let pool_id = pool_id.context("pool_id is required for raydium-cpmm")?;
            let params = RaydiumCpmmParams::from_pool_address_by_rpc(rpc, pool_id).await?;
            ensure_pool_contains_quote(
                "raydium-cpmm",
                quote_mint,
                &[params.base_mint, params.quote_mint],
            )?;
            Ok(())
        }
    }
}

pub(crate) async fn dex_params(
    rpc: &RpcClient,
    pool: Pool,
    mint: &Pubkey,
    quote_mint: &Pubkey,
    pool_id: Option<&Pubkey>,
    sol_hop_pool: Option<Pool>,
    sol_hop_pool_id: Option<&Pubkey>,
) -> anyhow::Result<DexParamEnum> {
    Ok(match pool {
        Pool::Pump => DexParamEnum::PumpFun(PumpFunParams::from_mint_by_rpc(rpc, mint).await?),
        Pool::PumpAmm => {
            let params = if let Some(pool_id) = pool_id {
                PumpSwapParams::from_pool_address_by_rpc(rpc, pool_id).await?
            } else {
                PumpSwapParams::from_mint_by_rpc(rpc, mint).await?
            };
            ensure_pool_contains_mint("pump-amm", mint, &[params.base_mint, params.quote_mint])?;
            ensure_pool_contains_quote(
                "pump-amm",
                quote_mint,
                &[params.base_mint, params.quote_mint],
            )?;
            DexParamEnum::PumpSwap(params)
        }
        Pool::Bonk => DexParamEnum::Bonk(
            BonkParams::from_mint_by_rpc(rpc, mint, is_usd1_quote(quote_mint))
                .await
                .map_err(|error| describe_bonk_param_error(mint, quote_mint, error))?,
        ),
        Pool::StonkFun => {
            let pool_id = pool_id.context("pool_id is required for stonkfun")?;
            let params = load_stonkfun_params(rpc, pool_id, mint, quote_mint).await?;
            if let Some(hop_id) = sol_hop_pool_id {
                let hop = load_stonkfun_sol_hop(rpc, sol_hop_pool, hop_id, quote_mint).await?;
                DexParamEnum::StonkFunViaSol(StonkFunViaSolParams::curve_with_cpmm(params, hop))
            } else {
                DexParamEnum::StonkFun(params)
            }
        }
        Pool::StonkFunSwap => {
            let pool_id = pool_id.context("pool_id is required for stonkfun-swap")?;
            let params = RaydiumCpmmParams::from_pool_address_by_rpc(rpc, pool_id).await?;
            ensure_pool_contains_mint(
                "stonkfun-swap",
                mint,
                &[params.base_mint, params.quote_mint],
            )?;
            ensure_pool_contains_quote(
                "stonkfun-swap",
                quote_mint,
                &[params.base_mint, params.quote_mint],
            )?;
            if let Some(hop_id) = sol_hop_pool_id {
                let hop = load_stonkfun_sol_hop(rpc, sol_hop_pool, hop_id, quote_mint).await?;
                DexParamEnum::StonkFunViaSol(StonkFunViaSolParams::graduated_with_cpmm(params, hop))
            } else {
                DexParamEnum::StonkFunSwap(params)
            }
        }
        Pool::MeteoraDammV2 => {
            let pool_id = pool_id.context("pool_id is required for meteora-damm-v2")?;
            let params = MeteoraDammV2Params::from_pool_address_by_rpc(rpc, pool_id)
                .await?
                .with_rate_limiter_sysvar(true);
            ensure_pool_contains_mint(
                "meteora-damm-v2",
                mint,
                &[params.token_a_mint, params.token_b_mint],
            )?;
            ensure_pool_contains_quote(
                "meteora-damm-v2",
                quote_mint,
                &[params.token_a_mint, params.token_b_mint],
            )?;
            DexParamEnum::MeteoraDammV2(params)
        }
        Pool::MeteoraDbc => {
            let pool_id = pool_id.context("pool_id is required for meteora-dbc")?;
            let params = MeteoraDbcParams::from_pool_address_by_rpc(rpc, pool_id).await?;
            ensure_pool_contains_mint("meteora-dbc", mint, &[params.base_mint])?;
            ensure_pool_contains_quote("meteora-dbc", quote_mint, &[params.quote_mint])?;
            DexParamEnum::MeteoraDbc(params)
        }
        Pool::RaydiumAmmV4 => {
            let pool_id = pool_id.context("pool_id is required for raydium-amm-v4")?;
            let params = RaydiumAmmV4Params::from_amm_address_by_rpc(rpc, *pool_id).await?;
            ensure_pool_contains_mint("raydium-amm-v4", mint, &[params.coin_mint, params.pc_mint])?;
            ensure_pool_contains_quote(
                "raydium-amm-v4",
                quote_mint,
                &[params.coin_mint, params.pc_mint],
            )?;
            DexParamEnum::RaydiumAmmV4(params)
        }
        Pool::RaydiumCpmm => {
            let pool_id = pool_id.context("pool_id is required for raydium-cpmm")?;
            let params = RaydiumCpmmParams::from_pool_address_by_rpc(rpc, pool_id).await?;
            ensure_pool_contains_mint(
                "raydium-cpmm",
                mint,
                &[params.base_mint, params.quote_mint],
            )?;
            ensure_pool_contains_quote(
                "raydium-cpmm",
                quote_mint,
                &[params.base_mint, params.quote_mint],
            )?;
            DexParamEnum::RaydiumCpmm(params)
        }
    })
}

async fn load_stonkfun_params(
    rpc: &RpcClient,
    pool_id: &Pubkey,
    mint: &Pubkey,
    quote_mint: &Pubkey,
) -> anyhow::Result<StonkFunParams> {
    use sol_trade_sdk::instruction::utils::bonk::fetch_pool_state;
    let state = fetch_pool_state(rpc, pool_id).await?;
    ensure_pool_contains_mint("stonkfun", mint, &[state.base_mint])?;
    ensure_pool_contains_quote("stonkfun", quote_mint, &[state.quote_mint])?;
    ensure_stonkfun_platform(pool_id, &state.platform_config)?;
    StonkFunParams::from_pool_by_rpc(rpc, pool_id).await
}

fn ensure_stonkfun_platform(pool_id: &Pubkey, platform: &Pubkey) -> anyhow::Result<()> {
    use sol_trade_sdk::instruction::utils::bonk::accounts;
    if *platform != accounts::STONKFUN_STANDARD_PLATFORM_CONFIG
        && *platform != accounts::STONKFUN_REWARD_PLATFORM_CONFIG
    {
        bail!(
            "launchlab pool {} is not a recognized StonkFun platform: {}",
            pool_id,
            platform
        );
    }
    Ok(())
}

async fn load_stonkfun_sol_hop(
    rpc: &RpcClient,
    pool: Option<Pool>,
    pool_id: &Pubkey,
    quote_mint: &Pubkey,
) -> anyhow::Result<RaydiumCpmmParams> {
    if pool != Some(Pool::RaydiumCpmm) {
        bail!("stonkfun Via-SOL only supports a Raydium CPMM SOL hop");
    }
    let params = RaydiumCpmmParams::from_pool_address_by_rpc(rpc, pool_id).await?;
    let mints = [params.base_mint, params.quote_mint];
    ensure_pool_contains_quote("stonkfun Via-SOL hop", quote_mint, &mints)?;
    let wsol = Pubkey::from_str(WSOL_MINT).expect("known WSOL mint must be valid");
    ensure_pool_contains_quote("stonkfun Via-SOL hop", &wsol, &mints)?;
    if quote_mint == &wsol {
        bail!("stonkfun Via-SOL hop requires a non-WSOL quote mint");
    }
    Ok(params)
}

pub(crate) fn describe_bonk_param_error(
    mint: &Pubkey,
    quote_mint: &Pubkey,
    error: anyhow::Error,
) -> anyhow::Error {
    let error_text = error.to_string();
    if error_text.contains("AccountNotFound") {
        let quote = if is_usd1_quote(quote_mint) {
            "USD1"
        } else {
            "WSOL"
        };
        anyhow!("bonk {quote} pool not found for mint {mint}; this mint may not have an active Bonk {quote} launchpad pool, may use another quote route, or may already be migrated. SDK error: {error_text}")
    } else {
        error
    }
}
