use anyhow::{bail, Context};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::config::Config;
use crate::models::{
    ExecutionMethod, MevProvider, Pool, SettlementAsset, Side, TradeRequest, WarmupRequest,
    SOL_MINT, USD1_MINT, USDC_MINT, WSOL_MINT,
};

pub struct NormalizedTrade {
    pub mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_id: Option<Pubkey>,
    pub settlement_asset: SettlementAsset,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<Pubkey>,
    pub amount_lamports: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub execution_method: ExecutionMethod,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
    pub simulate: Option<bool>,
    pub aggregator_sol: bool,
}

pub struct NormalizedWarmup {
    pub mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_id: Option<Pubkey>,
}

pub fn validate_warmup(request: &WarmupRequest) -> anyhow::Result<NormalizedWarmup> {
    let mint = parse_pubkey("mint", &request.mint)?;
    let pool_id = parse_optional_pubkey("pool_id", request.pool_id.as_deref())?;
    let quote_mint = normalize_quote_mint(request.quote_mint.as_deref())?;
    validate_quote_mint_for_pool(request.pool, &quote_mint)?;
    validate_pool_id_rule(request.pool, pool_id, ExecutionMethod::Sdk)?;
    Ok(NormalizedWarmup {
        mint,
        quote_mint,
        pool_id,
    })
}

pub fn validate_trade(request: &TradeRequest, config: &Config) -> anyhow::Result<NormalizedTrade> {
    if request.idempotency_key.trim().is_empty() {
        bail!("idempotency_key is required");
    }
    validate_amount_fields(request)?;
    let mint = parse_pubkey("mint", &request.mint)?;
    let pool_id = parse_optional_pubkey("pool_id", request.pool_id.as_deref())?;
    let quote_mint = normalize_quote_mint(request.quote_mint.as_deref())?;
    let aggregator_sol = request.side == Side::Buy
        && request.amount_sol.is_some()
        && request.amount_quote.is_none()
        && request.amount_usdc.is_none()
        && request.amount_quote_units.is_none()
        && !is_wsol_quote(&quote_mint);
    if !aggregator_sol {
        validate_quote_mint_for_pool(request.pool, &quote_mint)?;
    }
    let execution_method = request.execution_method.unwrap_or(ExecutionMethod::Sdk);
    let slippage_bps = normalize_slippage_bps(request, config)?;
    if slippage_bps > 10_000 {
        bail!("slippage must be between 0% and 100%");
    }
    let tip_sol = request.tip_sol.unwrap_or(config.default_tip_sol);
    if !tip_sol.is_finite() || tip_sol < 0.0 {
        bail!("tip_sol must be greater than or equal to 0");
    }
    let (use_mev, mev_provider) = normalize_mev_override(request)?;
    let settlement_asset = request.settlement_asset.unwrap_or(SettlementAsset::Quote);
    let sol_hop_pool_id =
        parse_optional_pubkey("sol_hop_pool_id", request.sol_hop_pool_id.as_deref())?;
    validate_via_sol(
        request,
        config,
        &quote_mint,
        settlement_asset,
        sol_hop_pool_id,
    )?;

    let (amount_lamports, sell_percent) = match request.side {
        Side::Buy => {
            let amount = normalize_amount_units(request, config, &quote_mint, settlement_asset)?;
            if amount == 0 {
                bail!("buy amount must be greater than 0");
            }
            (Some(amount), None)
        }
        Side::Sell => {
            let percent = request
                .sell_percent
                .context("sell_percent is required for sell")?;
            if !(1..=100).contains(&percent) {
                bail!("sell_percent must be between 1 and 100");
            }
            (None, Some(percent))
        }
    };

    validate_execution_method(request, execution_method, use_mev, mev_provider)?;
    validate_pool_id_rule(request.pool, pool_id, execution_method)?;

    Ok(NormalizedTrade {
        mint,
        quote_mint,
        pool_id,
        settlement_asset,
        sol_hop_pool: request.sol_hop_pool,
        sol_hop_pool_id,
        amount_lamports,
        sell_percent,
        slippage_bps,
        tip_sol,
        execution_method,
        use_mev,
        mev_provider,
        simulate: request.simulate,
        aggregator_sol,
    })
}

fn validate_amount_fields(request: &TradeRequest) -> anyhow::Result<()> {
    let count = [
        request.amount_sol.is_some(),
        request.amount_lamports.is_some(),
        request.amount_quote.is_some(),
        request.amount_usdc.is_some(),
        request.amount_quote_units.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    match request.side {
        Side::Buy if count > 1 => bail!("buy request must provide at most one amount field"),
        Side::Sell if count > 0 => bail!("sell request must not provide buy amount fields"),
        Side::Buy if request.sell_percent.is_some() => {
            bail!("buy request must not provide sell_percent")
        }
        _ => Ok(()),
    }
}

fn validate_execution_method(
    request: &TradeRequest,
    execution_method: ExecutionMethod,
    use_mev: Option<bool>,
    mev_provider: Option<MevProvider>,
) -> anyhow::Result<()> {
    if execution_method != ExecutionMethod::Pumpapi {
        return Ok(());
    }

    if use_mev == Some(true) {
        bail!("execution_method=pumpapi does not support use_mev=true in the first version");
    }
    if mev_provider.is_some_and(|provider| !matches!(provider, MevProvider::Default)) {
        bail!(
            "execution_method=pumpapi only supports mev_provider=default/rpc in the first version"
        );
    }
    if matches!(request.side, Side::Sell) && request.sell_percent != Some(100) {
        bail!("execution_method=pumpapi only supports sell_percent=100 in the first version");
    }
    Ok(())
}

fn normalize_mev_override(
    request: &TradeRequest,
) -> anyhow::Result<(Option<bool>, Option<MevProvider>)> {
    match (request.use_mev, request.mev_provider) {
        (Some(false), Some(provider)) if is_mev_provider(provider) => {
            bail!("mev_provider requires use_mev=true or use_mev omitted")
        }
        (Some(true), Some(MevProvider::Default)) => {
            bail!("mev_provider=default conflicts with use_mev=true")
        }
        (None, Some(MevProvider::Default)) => Ok((Some(false), Some(MevProvider::Default))),
        (None, Some(provider)) if is_mev_provider(provider) => Ok((Some(true), Some(provider))),
        _ => Ok((request.use_mev, request.mev_provider)),
    }
}

fn is_mev_provider(provider: MevProvider) -> bool {
    !matches!(provider, MevProvider::Auto | MevProvider::Default)
}

fn validate_pool_id_rule(
    pool: Pool,
    pool_id: Option<Pubkey>,
    execution_method: ExecutionMethod,
) -> anyhow::Result<()> {
    if execution_method == ExecutionMethod::Pumpapi {
        return Ok(());
    }
    match pool {
        Pool::Pump | Pool::Bonk => {
            if pool_id.is_some() {
                bail!("pool_id is not used for this pool");
            }
            Ok(())
        }
        Pool::PumpAmm => Ok(()),
        Pool::StonkFun
        | Pool::StonkFunSwap
        | Pool::MeteoraDammV2
        | Pool::MeteoraDbc
        | Pool::RaydiumAmmV4
        | Pool::RaydiumCpmm => {
            if pool_id.is_none() {
                bail!("pool_id is required for {:?}", pool);
            }
            Ok(())
        }
    }
}

fn normalize_amount_units(
    request: &TradeRequest,
    config: &Config,
    quote_mint: &Pubkey,
    settlement_asset: SettlementAsset,
) -> anyhow::Result<u64> {
    if settlement_asset == SettlementAsset::Sol {
        if request.amount_quote.is_some()
            || request.amount_usdc.is_some()
            || request.amount_quote_units.is_some()
        {
            bail!("settlement_asset=sol accepts only amount_sol or amount_lamports");
        }
        if let Some(amount_sol) = request.amount_sol {
            return token_to_units("amount_sol", amount_sol, 9);
        }
        if let Some(amount_lamports) = request.amount_lamports {
            return Ok(amount_lamports);
        }
        return token_to_units("DEFAULT_BUY_SOL", config.default_buy_sol, 9);
    }
    if let Some(units) = request.amount_quote_units {
        return Ok(units);
    }

    if is_usdc_quote(quote_mint) {
        if let Some(amount_usdc) = request.amount_usdc {
            return token_to_units("amount_usdc", amount_usdc, 6);
        }
        if let Some(amount_quote) = request.amount_quote {
            return token_to_units("amount_quote", amount_quote, 6);
        }
        if let Some(amount_sol) = request.amount_sol {
            return token_to_units("amount_sol", amount_sol, 9);
        }
        if request.amount_lamports.is_some() {
            bail!("USDC quote requires amount_quote, amount_usdc, or amount_quote_units");
        }
        bail!("USDC quote requires amount_quote or amount_usdc");
    }

    if is_usd1_quote(quote_mint) {
        if request.amount_usdc.is_some() {
            bail!("USD1 quote uses amount_quote or amount_quote_units, not amount_usdc");
        }
        if let Some(amount_quote) = request.amount_quote {
            return token_to_units("amount_quote", amount_quote, 6);
        }
        if let Some(amount_sol) = request.amount_sol {
            return token_to_units("amount_sol", amount_sol, 9);
        }
        if request.amount_lamports.is_some() {
            bail!("USD1 quote requires amount_quote or amount_quote_units");
        }
        bail!("USD1 quote requires amount_quote");
    }

    if !is_wsol_quote(quote_mint) {
        if let Some(amount_sol) = request.amount_sol {
            return token_to_units("amount_sol", amount_sol, 9);
        }
        bail!("custom quote token requires amount_quote_units");
    }

    if request.amount_usdc.is_some() {
        bail!("amount_usdc is only valid when quote_mint is USDC");
    }
    if let Some(amount_quote) = request.amount_quote {
        return token_to_units("amount_quote", amount_quote, 9);
    }
    if let Some(amount_sol) = request.amount_sol {
        return token_to_units("amount_sol", amount_sol, 9);
    }
    if let Some(amount_lamports) = request.amount_lamports {
        return Ok(amount_lamports);
    }
    token_to_units("DEFAULT_BUY_SOL", config.default_buy_sol, 9)
}

fn validate_via_sol(
    request: &TradeRequest,
    config: &Config,
    quote_mint: &Pubkey,
    settlement_asset: SettlementAsset,
    sol_hop_pool_id: Option<Pubkey>,
) -> anyhow::Result<()> {
    if settlement_asset != SettlementAsset::Sol {
        if request.sol_hop_pool.is_some() || sol_hop_pool_id.is_some() {
            bail!("sol_hop_pool and sol_hop_pool_id require settlement_asset=sol");
        }
        return Ok(());
    }
    if !matches!(request.pool, Pool::StonkFun | Pool::StonkFunSwap) {
        bail!("settlement_asset=sol is only supported for stonkfun pools");
    }
    if is_wsol_quote(quote_mint) {
        bail!("settlement_asset=sol is unnecessary when quote_mint is WSOL");
    }
    if !config.stonkfun_via_sol.enabled {
        bail!("stonkfun Via-SOL is disabled by STONKFUN_VIA_SOL_ENABLED");
    }
    if request.sol_hop_pool != Some(Pool::RaydiumCpmm) {
        bail!("stonkfun Via-SOL currently requires sol_hop_pool=raydium-cpmm");
    }
    if sol_hop_pool_id.is_none() {
        bail!("sol_hop_pool_id is required for stonkfun Via-SOL");
    }
    if matches!(request.side, Side::Buy) {
        let requested_sol = request
            .amount_sol
            .or_else(|| {
                request
                    .amount_lamports
                    .map(|value| value as f64 / 1_000_000_000.0)
            })
            .unwrap_or(config.default_buy_sol);
        if requested_sol > config.stonkfun_via_sol.max_buy_sol {
            bail!("stonkfun Via-SOL buy exceeds STONKFUN_VIA_SOL_MAX_BUY_SOL");
        }
    }
    Ok(())
}

fn normalize_slippage_bps(request: &TradeRequest, config: &Config) -> anyhow::Result<u64> {
    if let Some(percent) = request.slippage_percent {
        return percent_to_bps("slippage_percent", percent);
    }
    if let Some(bps) = request.slippage_bps {
        return Ok(bps);
    }
    percent_to_bps("DEFAULT_SLIPPAGE_PERCENT", config.default_slippage_percent)
}

fn token_to_units(name: &str, value: f64, decimals: u8) -> anyhow::Result<u64> {
    if !value.is_finite() || value < 0.0 {
        bail!("{name} must be greater than or equal to 0");
    }
    let scale = 10_u64.pow(decimals as u32) as f64;
    let units = value * scale;
    if units > u64::MAX as f64 {
        bail!("{name} is too large");
    }
    Ok(units.round() as u64)
}

fn percent_to_bps(name: &str, value: f64) -> anyhow::Result<u64> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        bail!("{name} must be between 0 and 100");
    }
    Ok((value * 100.0).round() as u64)
}

fn parse_optional_pubkey(name: &str, value: Option<&str>) -> anyhow::Result<Option<Pubkey>> {
    value.map(|value| parse_pubkey(name, value)).transpose()
}

fn parse_pubkey(name: &str, value: &str) -> anyhow::Result<Pubkey> {
    if value.trim().is_empty() {
        bail!("{name} is required");
    }
    Pubkey::from_str(value).with_context(|| format!("{name} must be a valid pubkey"))
}

fn normalize_quote_mint(value: Option<&str>) -> anyhow::Result<Pubkey> {
    let value = value.unwrap_or(WSOL_MINT);
    let pubkey = Pubkey::from_str(value).context("quote_mint must be a valid pubkey")?;
    if pubkey == parse_known_pubkey(SOL_MINT) {
        return Ok(parse_known_pubkey(WSOL_MINT));
    }
    Ok(pubkey)
}

fn validate_quote_mint_for_pool(pool: Pool, quote_mint: &Pubkey) -> anyhow::Result<()> {
    match pool {
        Pool::Bonk => {
            if !is_wsol_quote(quote_mint) && !is_usd1_quote(quote_mint) {
                bail!("bonk quote_mint must be WSOL or USD1");
            }
        }
        Pool::StonkFun | Pool::StonkFunSwap | Pool::RaydiumCpmm => {}
        Pool::PumpAmm => {}
        Pool::Pump | Pool::MeteoraDammV2 | Pool::MeteoraDbc | Pool::RaydiumAmmV4 => {
            if !is_wsol_quote(quote_mint) && !is_usdc_quote(quote_mint) {
                bail!(
                    "{:?} quote_mint must be WSOL or USDC; received {}",
                    pool,
                    quote_mint
                );
            }
        }
    }
    Ok(())
}

fn is_wsol_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(WSOL_MINT)
}

fn is_usdc_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(USDC_MINT)
}

fn is_usd1_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(USD1_MINT)
}

fn parse_known_pubkey(value: &str) -> Pubkey {
    Pubkey::from_str(value).expect("known mint pubkey must be valid")
}
