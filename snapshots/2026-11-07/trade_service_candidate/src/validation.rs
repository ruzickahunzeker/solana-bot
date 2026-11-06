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

pub(crate) fn has_bridge_sdk_decision_id(request: &TradeRequest) -> bool {
    request
        .sdk_decision_id
        .as_deref()
        .is_some_and(|id| id.starts_with("sdk-decision:bridge:"))
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
    if request.pool == Pool::MeteoraDbc {
        bail!("meteora-dbc requires an available DBC backend");
    }
    if config.simulate_only && !request.simulate.unwrap_or(config.enable_simulate) {
        bail!("this Trade Service instance only accepts simulated trades");
    }
    if request.idempotency_key.trim().is_empty() {
        bail!("idempotency_key is required");
    }
    validate_amount_fields(request)?;
    let mint = parse_pubkey("mint", &request.mint)?;
    let pool_id = parse_optional_pubkey("pool_id", request.pool_id.as_deref())?;
    let quote_mint = normalize_quote_mint(request.quote_mint.as_deref())?;
    let settlement_asset = request.settlement_asset.unwrap_or(SettlementAsset::Quote);
    let execution_method = request.execution_method.unwrap_or(ExecutionMethod::Sdk);
    let aggregator_sol =
        select_aggregator_sol_request(request, &quote_mint, settlement_asset, execution_method)?;
    if !aggregator_sol {
        validate_quote_mint_for_pool(request.pool, &quote_mint)?;
    }
    let slippage_bps = normalize_slippage_bps(request, config)?;
    if slippage_bps > 10_000 {
        bail!("slippage must be between 0% and 100%");
    }
    let tip_sol = request.tip_sol.unwrap_or(config.default_tip_sol);
    if !tip_sol.is_finite() || tip_sol < 0.0 {
        bail!("tip_sol must be greater than or equal to 0");
    }
    let (use_mev, mev_provider) = normalize_mev_override(request)?;
    let sol_hop_pool_id =
        parse_optional_pubkey("sol_hop_pool_id", request.sol_hop_pool_id.as_deref())?;
    validate_via_sol(
        request,
        config,
        &quote_mint,
        settlement_asset,
        sol_hop_pool_id,
    )?;
    validate_sdk_route_id(request, settlement_asset, execution_method)?;
    validate_sdk_decision_id(request, execution_method)?;
    validate_sdk_preview_id(request, config, settlement_asset, execution_method)?;

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

/// A well-formed SOL-funded buy of a non-WSOL quoted pool for which the
/// caller has not supplied a verified bridge. The decision endpoint may
/// report this as unresolved, but this never authorizes a trade or fallback.
pub fn unresolved_sol_bridge_intent(request: &TradeRequest, config: &Config) -> bool {
    if request.side != Side::Buy
        || request.execution_method.unwrap_or(ExecutionMethod::Sdk) != ExecutionMethod::Sdk
        || request.settlement_asset.unwrap_or(SettlementAsset::Quote) != SettlementAsset::Quote
        || request.sol_hop_pool.is_some()
        || request.sol_hop_pool_id.is_some()
        || request.sdk_route_id.is_some()
        || request.sdk_decision_id.is_some()
        || request.sdk_preview_id.is_some()
        || validate_amount_fields(request).is_err()
    {
        return false;
    }
    let amount_valid = match (request.amount_sol, request.amount_lamports) {
        (Some(amount), None) => token_to_units("amount_sol", amount, 9).is_ok_and(|v| v > 0),
        (None, Some(amount)) => amount > 0,
        _ => false,
    };
    if !amount_valid {
        return false;
    }
    let quote_mint = match normalize_quote_mint(request.quote_mint.as_deref()) {
        Ok(value) if !is_wsol_quote(&value) => value,
        _ => return false,
    };
    if validate_quote_mint_for_pool(request.pool, &quote_mint).is_err() {
        return false;
    }
    let mut probe = request.clone();
    probe.amount_sol = None;
    probe.amount_lamports = None;
    probe.amount_quote_units = Some(1);
    validate_trade(&probe, config).is_ok()
}

/// A SOL-funded SDK buy with no caller-supplied quote mint. The decision
/// endpoint must read the selected target pool before it can choose a direct
/// WSOL route or inspect a non-WSOL bridge. This probe never authorizes a
/// fallback if discovery is unavailable.
pub fn sol_buy_requires_quote_discovery(request: &TradeRequest, config: &Config) -> bool {
    request.quote_mint.is_none()
        && request.side == Side::Buy
        && request.execution_method.unwrap_or(ExecutionMethod::Sdk) == ExecutionMethod::Sdk
        && request.settlement_asset.unwrap_or(SettlementAsset::Quote) == SettlementAsset::Quote
        && request.sol_hop_pool.is_none()
        && request.sol_hop_pool_id.is_none()
        && request.sdk_route_id.is_none()
        && request.sdk_decision_id.is_none()
        && request.sdk_preview_id.is_none()
        && matches!(
            (request.amount_sol, request.amount_lamports),
            (Some(_), None) | (None, Some(_))
        )
        && validate_trade(request, config).is_ok()
}

fn validate_sdk_route_id(
    request: &TradeRequest,
    settlement_asset: SettlementAsset,
    execution_method: ExecutionMethod,
) -> anyhow::Result<()> {
    if let Some(route_id) = request.sdk_route_id.as_deref() {
        if route_id.trim().is_empty() || route_id.len() > 128 {
            bail!("sdk_route_id is empty or too long");
        }
        if settlement_asset != SettlementAsset::Sol || execution_method != ExecutionMethod::Sdk {
            bail!("sdk_route_id only applies to an SDK via-SOL trade");
        }
    }
    Ok(())
}

fn validate_sdk_decision_id(
    request: &TradeRequest,
    execution_method: ExecutionMethod,
) -> anyhow::Result<()> {
    if let Some(decision_id) = request.sdk_decision_id.as_deref() {
        if decision_id.trim().is_empty() || decision_id.len() > 128 {
            bail!("sdk_decision_id is empty or too long");
        }
        if execution_method != ExecutionMethod::Sdk {
            bail!("sdk_decision_id only applies to SDK trades");
        }
    }
    Ok(())
}

fn validate_sdk_preview_id(
    request: &TradeRequest,
    config: &Config,
    settlement_asset: SettlementAsset,
    execution_method: ExecutionMethod,
) -> anyhow::Result<()> {
    let preview_id = request.sdk_preview_id.as_deref();
    let bridge_decision = has_bridge_sdk_decision_id(request);
    if preview_id.is_none() && !bridge_decision {
        return Ok(());
    }
    if preview_id.is_some_and(|id| !id.starts_with("observe:sdk-preview:") || id.len() > 128) {
        bail!("sdk_preview_id is invalid");
    }
    if !config.simulate_only
        || request.simulate != Some(true)
        || execution_method != ExecutionMethod::Sdk
        || request.side != Side::Buy
        || !matches!(
            request.pool,
            Pool::PumpAmm | Pool::RaydiumAmmV4 | Pool::RaydiumCpmm
        )
        || settlement_asset != SettlementAsset::Quote
        || (preview_id.is_some() && request.sdk_decision_id.is_some())
        || request.sdk_route_id.is_some()
        || request.sol_hop_pool.is_some()
        || request.sol_hop_pool_id.is_some()
        || request.tip_sol.is_some()
        || request.use_mev.is_some()
        || request.mev_provider.is_some()
        || !matches!(
            (request.amount_sol, request.amount_lamports),
            (Some(_), None) | (None, Some(_))
        )
    {
        bail!("SDK bridge credential requires a simulate-only supported SOL buy");
    }
    Ok(())
}

fn select_aggregator_sol_request(
    request: &TradeRequest,
    quote_mint: &Pubkey,
    settlement_asset: SettlementAsset,
    execution_method: ExecutionMethod,
) -> anyhow::Result<bool> {
    if execution_method == ExecutionMethod::Aggregator {
        if request.side != Side::Buy
            || request.amount_sol.is_none()
            || request.amount_quote.is_some()
            || request.amount_usdc.is_some()
            || request.amount_quote_units.is_some()
            || is_wsol_quote(quote_mint)
            || settlement_asset != SettlementAsset::Quote
            || request.sol_hop_pool.is_some()
            || request.sol_hop_pool_id.is_some()
            || request.tip_sol.is_some()
            || request.use_mev.is_some()
            || request.mev_provider.is_some()
        {
            bail!("execution_method=aggregator requires a SOL exact-in buy without an SDK hop or ignored fee settings");
        }
        return Ok(true);
    }
    if request.amount_sol.is_some()
        && request.sdk_preview_id.is_none()
        && !has_bridge_sdk_decision_id(request)
        && !is_wsol_quote(quote_mint)
        && settlement_asset != SettlementAsset::Sol
    {
        bail!("non-WSOL amount_sol requires a verified SDK SOL bridge decision; resolve the route before trading");
    }
    Ok(false)
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
    if request.sdk_preview_id.is_some() || has_bridge_sdk_decision_id(request) {
        return match (request.amount_sol, request.amount_lamports) {
            (Some(amount), None) => token_to_units("amount_sol", amount, 9),
            (None, Some(amount)) => Ok(amount),
            _ => bail!("SDK preview requires an explicit SOL amount"),
        };
    }
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
    if request.pool == Pool::RaydiumCpmm {
        if !cpmm_two_hop_execution_mode_allowed(
            config.simulate_only,
            config.sdk_two_hop_live_enabled,
            request.simulate,
        ) || (request.side == Side::Buy && !cpmm_two_hop_buy_has_explicit_input(request))
            || request.sol_hop_pool != Some(Pool::RaydiumCpmm)
            || sol_hop_pool_id.is_none()
            || request.pool_id.is_none()
            || is_wsol_quote(quote_mint)
            || request.execution_method != Some(ExecutionMethod::Sdk)
            || request.sdk_preview_id.is_some()
            || request.tip_sol.is_some()
            || request.use_mev.is_some()
            || request.mev_provider.is_some()
        {
            bail!("CPMM via-SOL trade requires an explicitly enabled SDK route with two explicit pools");
        }
        return Ok(());
    }
    if !matches!(request.pool, Pool::StonkFun | Pool::StonkFunSwap) {
        bail!("settlement_asset=sol is only supported for stonkfun pools");
    }
    require_sdk_via_sol_execution(request.execution_method)?;
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

fn cpmm_two_hop_execution_mode_allowed(
    simulate_only: bool,
    sdk_two_hop_live_enabled: bool,
    request_simulate: Option<bool>,
) -> bool {
    match request_simulate {
        Some(true) => simulate_only,
        Some(false) => !simulate_only && sdk_two_hop_live_enabled,
        None => false,
    }
}

fn cpmm_two_hop_buy_has_explicit_input(request: &TradeRequest) -> bool {
    matches!(
        (request.amount_sol, request.amount_lamports),
        (Some(_), None) | (None, Some(_))
    )
}

fn require_sdk_via_sol_execution(method: Option<ExecutionMethod>) -> anyhow::Result<()> {
    if method.unwrap_or(ExecutionMethod::Sdk) != ExecutionMethod::Sdk {
        bail!("settlement_asset=sol requires execution_method=sdk");
    }
    Ok(())
}

pub(crate) fn normalize_slippage_bps(
    request: &TradeRequest,
    config: &Config,
) -> anyhow::Result<u64> {
    if let Some(percent) = request.slippage_percent {
        return percent_to_bps("slippage_percent", percent);
    }
    if let Some(bps) = request.slippage_bps {
        return Ok(bps);
    }
    percent_to_bps("DEFAULT_SLIPPAGE_PERCENT", config.default_slippage_percent)
}

pub(crate) fn token_to_units(name: &str, value: f64, decimals: u8) -> anyhow::Result<u64> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpmm_two_hop_live_requires_an_explicit_non_staging_gate() {
        assert!(cpmm_two_hop_execution_mode_allowed(true, false, Some(true)));
        assert!(!cpmm_two_hop_execution_mode_allowed(
            true,
            true,
            Some(false)
        ));
        assert!(!cpmm_two_hop_execution_mode_allowed(
            false,
            false,
            Some(false)
        ));
        assert!(cpmm_two_hop_execution_mode_allowed(
            false,
            true,
            Some(false)
        ));
        assert!(!cpmm_two_hop_execution_mode_allowed(false, true, None));
    }

    #[test]
    fn cpmm_two_hop_buy_requires_explicit_sol_input() {
        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "side": "buy", "pool": "raydium-cpmm",
            "mint": Pubkey::new_unique().to_string(),
            "idempotency_key": "cpmm-buy-input"
        }))
        .unwrap();
        assert!(!cpmm_two_hop_buy_has_explicit_input(&request));
        request.amount_lamports = Some(1_000_000);
        assert!(cpmm_two_hop_buy_has_explicit_input(&request));
        request.amount_sol = Some(0.001);
        assert!(!cpmm_two_hop_buy_has_explicit_input(&request));
    }

    #[test]
    fn sdk_via_sol_hop_and_explicit_aggregator_are_distinct() {
        let custom_quote = Pubkey::new_unique();
        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "side": "buy",
            "pool": "stonkfun",
            "mint": Pubkey::new_unique().to_string(),
            "quote_mint": custom_quote.to_string(),
            "amount_sol": 0.01,
            "settlement_asset": "sol",
            "sol_hop_pool": "raydium-cpmm",
            "sol_hop_pool_id": Pubkey::new_unique().to_string(),
            "execution_method": "sdk",
            "idempotency_key": "sdk-via-sol"
        }))
        .unwrap();
        assert_eq!(
            select_aggregator_sol_request(
                &request,
                &custom_quote,
                SettlementAsset::Sol,
                ExecutionMethod::Sdk,
            )
            .unwrap(),
            false
        );

        assert!(select_aggregator_sol_request(
            &request,
            &custom_quote,
            SettlementAsset::Sol,
            ExecutionMethod::Aggregator,
        )
        .is_err());

        request.settlement_asset = None;
        request.sol_hop_pool = None;
        request.sol_hop_pool_id = None;
        assert!(select_aggregator_sol_request(
            &request,
            &custom_quote,
            SettlementAsset::Quote,
            ExecutionMethod::Sdk,
        )
        .is_err());
        assert!(select_aggregator_sol_request(
            &request,
            &custom_quote,
            SettlementAsset::Quote,
            ExecutionMethod::Aggregator,
        )
        .unwrap());
        assert!(select_aggregator_sol_request(
            &request,
            &Pubkey::from_str(WSOL_MINT).unwrap(),
            SettlementAsset::Quote,
            ExecutionMethod::Aggregator,
        )
        .is_err());
        request.tip_sol = Some(0.001);
        assert!(select_aggregator_sol_request(
            &request,
            &custom_quote,
            SettlementAsset::Quote,
            ExecutionMethod::Aggregator,
        )
        .is_err());
    }

    #[test]
    fn via_sol_does_not_accept_pumpapi_execution() {
        assert!(require_sdk_via_sol_execution(None).is_ok());
        assert!(require_sdk_via_sol_execution(Some(ExecutionMethod::Sdk)).is_ok());
        assert!(require_sdk_via_sol_execution(Some(ExecutionMethod::Pumpapi)).is_err());
    }

    #[test]
    fn sdk_route_id_cannot_be_reused_on_another_execution_path() {
        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "side": "buy",
            "pool": "stonkfun",
            "mint": Pubkey::new_unique().to_string(),
            "idempotency_key": "route-id-test"
        }))
        .unwrap();
        request.sdk_route_id = Some("sdk11:via-sol:route".into());
        assert!(
            validate_sdk_route_id(&request, SettlementAsset::Sol, ExecutionMethod::Sdk).is_ok()
        );
        assert!(
            validate_sdk_route_id(&request, SettlementAsset::Quote, ExecutionMethod::Sdk).is_err()
        );
        assert!(
            validate_sdk_route_id(&request, SettlementAsset::Sol, ExecutionMethod::Aggregator)
                .is_err()
        );
        request.sdk_route_id = Some(" ".into());
        assert!(
            validate_sdk_route_id(&request, SettlementAsset::Sol, ExecutionMethod::Sdk).is_err()
        );
    }

    #[test]
    fn sdk_decision_id_cannot_select_another_execution_method() {
        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "side": "buy",
            "pool": "pump",
            "mint": Pubkey::new_unique().to_string(),
            "idempotency_key": "decision-id-test"
        }))
        .unwrap();
        request.sdk_decision_id = Some("sdk-decision:valid".into());
        assert!(validate_sdk_decision_id(&request, ExecutionMethod::Sdk).is_ok());
        assert!(validate_sdk_decision_id(&request, ExecutionMethod::Aggregator).is_err());
        assert!(validate_sdk_decision_id(&request, ExecutionMethod::Pumpapi).is_err());
        request.sdk_decision_id = Some(" ".into());
        assert!(validate_sdk_decision_id(&request, ExecutionMethod::Sdk).is_err());
    }
}
