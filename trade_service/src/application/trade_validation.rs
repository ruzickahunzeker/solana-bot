use anyhow::{bail, Context};

use crate::{
    application::{
        ports::address_parser::TradeAddressParser,
        trade_input::{TradeInput, WarmupInput},
        trade_settings::TradeValidationSettings,
        validated_trade::{ValidatedTrade, ValidatedWarmup},
    },
    domain::trade::{
        Address, ExecutionMethod, MevProvider, Pool, SettlementAsset, Side, SOL_MINT, USD1_MINT,
        USDC_MINT, WSOL_MINT,
    },
    protocol::registry::{self, PoolIdPolicy, QuotePolicy},
};

mod sdk_recovery;

/// Local input policy only; parsed addresses are not verified routes or signing authority.
pub(crate) struct TradeValidator<'a> {
    parser: &'a dyn TradeAddressParser,
}

impl<'a> TradeValidator<'a> {
    pub(crate) fn new(parser: &'a dyn TradeAddressParser) -> Self {
        Self { parser }
    }

    pub(crate) fn validate_warmup(
        &self,
        request: &WarmupInput<'_>,
    ) -> anyhow::Result<ValidatedWarmup> {
        let mint = self.parse_pubkey("mint", request.mint)?;
        let pool_id = self.parse_optional_pubkey("pool_id", request.pool_id)?;
        let quote_mint = self.normalize_quote_mint(request.quote_mint)?;
        self.validate_quote_mint_for_pool(request.pool, &quote_mint)?;
        self.validate_pool_id_rule(request.pool, pool_id, ExecutionMethod::Sdk)?;
        Ok(ValidatedWarmup {
            mint,
            quote_mint,
            pool_id,
        })
    }

    // Resolve/hop inputs historically parse raw syntax without required checks or Quote aliases.
    pub(crate) fn parse_query_address(&self, field: &str, value: &str) -> anyhow::Result<Address> {
        self.parser
            .parse(value)
            .with_context(|| format!("{field} must be a valid pubkey"))
    }

    pub(crate) fn validate(
        &self,
        request: &TradeInput<'_>,
        settings: &TradeValidationSettings,
    ) -> anyhow::Result<ValidatedTrade> {
        if request.idempotency_key.trim().is_empty() {
            bail!("idempotency_key is required");
        }
        self.validate_amount_fields(request)?;
        let mint = self.parse_pubkey("mint", request.mint)?;
        let pool_id = self.parse_optional_pubkey("pool_id", request.pool_id)?;
        let quote_mint = self.normalize_quote_mint(request.quote_mint)?;
        let aggregator_sol = self.uses_sol_aggregator(request, &quote_mint);
        if !aggregator_sol {
            self.validate_quote_mint_for_pool(request.pool, &quote_mint)?;
        }
        let execution_method = request.execution_method.unwrap_or(ExecutionMethod::Sdk);
        let slippage_bps = self.normalize_slippage_bps(request, settings)?;
        if slippage_bps > 10_000 {
            bail!("slippage must be between 0% and 100%");
        }
        let tip_sol = request.tip_sol.unwrap_or(settings.default_tip_sol);
        if !tip_sol.is_finite() || tip_sol < 0.0 {
            bail!("tip_sol must be greater than or equal to 0");
        }
        let (use_mev, mev_provider) = self.normalize_mev_override(request)?;
        let settlement_asset = request.settlement_asset.unwrap_or(SettlementAsset::Quote);
        let sol_hop_pool_id =
            self.parse_optional_pubkey("sol_hop_pool_id", request.sol_hop_pool_id)?;
        self.validate_via_sol(
            request,
            settings,
            &quote_mint,
            settlement_asset,
            sol_hop_pool_id,
        )?;

        let (amount_lamports, sell_percent) = match request.side {
            Side::Buy => {
                let amount =
                    self.normalize_amount_units(request, settings, &quote_mint, settlement_asset)?;
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

        self.validate_execution_method(request, execution_method, use_mev, mev_provider)?;
        self.validate_pool_id_rule(request.pool, pool_id, execution_method)?;

        Ok(ValidatedTrade {
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

    // Replay checks must not reapply today's defaults, feature flags or protocol policy.
    pub(crate) fn replay_uses_sol_aggregator(
        &self,
        request: &TradeInput<'_>,
    ) -> anyhow::Result<bool> {
        self.validate_amount_fields(request)?;
        let quote_mint = self.normalize_quote_mint(request.quote_mint)?;
        Ok(self.uses_sol_aggregator(request, &quote_mint))
    }

    // A read-only V1 interpretation, not today's admission/defaults or execution permission.
    pub(crate) fn replay_sol_observation_amount(
        &self,
        request: &TradeInput<'_>,
    ) -> anyhow::Result<(u64, Option<u64>)> {
        if !self.replay_uses_sol_aggregator(request)? || request.simulate == Some(true) {
            bail!("stored request is not a live ordinary SOL aggregate");
        }
        let amount = self.token_to_units(
            "amount_sol",
            request.amount_sol.context("stored SOL amount missing")?,
            9,
        )?;
        if amount == 0 {
            bail!("stored SOL amount is zero");
        }
        let slippage = if let Some(percent) = request.slippage_percent {
            Some(self.percent_to_bps("slippage_percent", percent)?)
        } else {
            request.slippage_bps
        };
        Ok((amount, slippage))
    }

    fn uses_sol_aggregator(&self, request: &TradeInput<'_>, quote_mint: &Address) -> bool {
        request.side == Side::Buy
            && request.amount_sol.is_some()
            && request.amount_quote.is_none()
            && request.amount_usdc.is_none()
            && request.amount_quote_units.is_none()
            && !self.is_wsol_quote(quote_mint)
    }

    fn validate_amount_fields(&self, request: &TradeInput<'_>) -> anyhow::Result<()> {
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
        &self,
        request: &TradeInput<'_>,
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
        &self,
        request: &TradeInput<'_>,
    ) -> anyhow::Result<(Option<bool>, Option<MevProvider>)> {
        match (request.use_mev, request.mev_provider) {
            (Some(false), Some(provider)) if self.is_mev_provider(provider) => {
                bail!("mev_provider requires use_mev=true or use_mev omitted")
            }
            (Some(true), Some(MevProvider::Default)) => {
                bail!("mev_provider=default conflicts with use_mev=true")
            }
            (None, Some(MevProvider::Default)) => Ok((Some(false), Some(MevProvider::Default))),
            (None, Some(provider)) if self.is_mev_provider(provider) => {
                Ok((Some(true), Some(provider)))
            }
            _ => Ok((request.use_mev, request.mev_provider)),
        }
    }

    fn is_mev_provider(&self, provider: MevProvider) -> bool {
        !matches!(provider, MevProvider::Auto | MevProvider::Default)
    }

    fn validate_pool_id_rule(
        &self,
        pool: Pool,
        pool_id: Option<Address>,
        execution_method: ExecutionMethod,
    ) -> anyhow::Result<()> {
        if execution_method == ExecutionMethod::Pumpapi {
            return Ok(());
        }
        match registry::for_pool(pool).pool_id_policy {
            PoolIdPolicy::Forbidden => {
                if pool_id.is_some() {
                    bail!("pool_id is not used for this pool");
                }
                Ok(())
            }
            PoolIdPolicy::Optional => Ok(()),
            PoolIdPolicy::Required => {
                if pool_id.is_none() {
                    bail!("pool_id is required for {:?}", pool);
                }
                Ok(())
            }
        }
    }

    fn normalize_amount_units(
        &self,
        request: &TradeInput<'_>,
        settings: &TradeValidationSettings,
        quote_mint: &Address,
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
                return self.token_to_units("amount_sol", amount_sol, 9);
            }
            if let Some(amount_lamports) = request.amount_lamports {
                return Ok(amount_lamports);
            }
            return self.token_to_units("DEFAULT_BUY_SOL", settings.default_buy_sol, 9);
        }
        if let Some(units) = request.amount_quote_units {
            return Ok(units);
        }

        if self.is_usdc_quote(quote_mint) {
            if let Some(amount_usdc) = request.amount_usdc {
                return self.token_to_units("amount_usdc", amount_usdc, 6);
            }
            if let Some(amount_quote) = request.amount_quote {
                return self.token_to_units("amount_quote", amount_quote, 6);
            }
            if let Some(amount_sol) = request.amount_sol {
                return self.token_to_units("amount_sol", amount_sol, 9);
            }
            if request.amount_lamports.is_some() {
                bail!("USDC quote requires amount_quote, amount_usdc, or amount_quote_units");
            }
            bail!("USDC quote requires amount_quote or amount_usdc");
        }

        if self.is_usd1_quote(quote_mint) {
            if request.amount_usdc.is_some() {
                bail!("USD1 quote uses amount_quote or amount_quote_units, not amount_usdc");
            }
            if let Some(amount_quote) = request.amount_quote {
                return self.token_to_units("amount_quote", amount_quote, 6);
            }
            if let Some(amount_sol) = request.amount_sol {
                return self.token_to_units("amount_sol", amount_sol, 9);
            }
            if request.amount_lamports.is_some() {
                bail!("USD1 quote requires amount_quote or amount_quote_units");
            }
            bail!("USD1 quote requires amount_quote");
        }

        if !self.is_wsol_quote(quote_mint) {
            if let Some(amount_sol) = request.amount_sol {
                return self.token_to_units("amount_sol", amount_sol, 9);
            }
            bail!("custom quote token requires amount_quote_units");
        }

        if request.amount_usdc.is_some() {
            bail!("amount_usdc is only valid when quote_mint is USDC");
        }
        if let Some(amount_quote) = request.amount_quote {
            return self.token_to_units("amount_quote", amount_quote, 9);
        }
        if let Some(amount_sol) = request.amount_sol {
            return self.token_to_units("amount_sol", amount_sol, 9);
        }
        if let Some(amount_lamports) = request.amount_lamports {
            return Ok(amount_lamports);
        }
        self.token_to_units("DEFAULT_BUY_SOL", settings.default_buy_sol, 9)
    }

    fn validate_via_sol(
        &self,
        request: &TradeInput<'_>,
        settings: &TradeValidationSettings,
        quote_mint: &Address,
        settlement_asset: SettlementAsset,
        sol_hop_pool_id: Option<Address>,
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
        if self.is_wsol_quote(quote_mint) {
            bail!("settlement_asset=sol is unnecessary when quote_mint is WSOL");
        }
        if !settings.via_sol_enabled {
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
                .unwrap_or(settings.default_buy_sol);
            if requested_sol > settings.via_sol_max_buy_sol {
                bail!("stonkfun Via-SOL buy exceeds STONKFUN_VIA_SOL_MAX_BUY_SOL");
            }
        }
        Ok(())
    }

    fn normalize_slippage_bps(
        &self,
        request: &TradeInput<'_>,
        settings: &TradeValidationSettings,
    ) -> anyhow::Result<u64> {
        if let Some(percent) = request.slippage_percent {
            return self.percent_to_bps("slippage_percent", percent);
        }
        if let Some(bps) = request.slippage_bps {
            return Ok(bps);
        }
        self.percent_to_bps(
            "DEFAULT_SLIPPAGE_PERCENT",
            settings.default_slippage_percent,
        )
    }

    fn token_to_units(&self, name: &str, value: f64, decimals: u8) -> anyhow::Result<u64> {
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

    fn percent_to_bps(&self, name: &str, value: f64) -> anyhow::Result<u64> {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            bail!("{name} must be between 0 and 100");
        }
        Ok((value * 100.0).round() as u64)
    }

    fn parse_optional_pubkey(
        &self,
        name: &str,
        value: Option<&str>,
    ) -> anyhow::Result<Option<Address>> {
        value
            .map(|value| self.parse_pubkey(name, value))
            .transpose()
    }

    fn parse_pubkey(&self, name: &str, value: &str) -> anyhow::Result<Address> {
        if value.trim().is_empty() {
            bail!("{name} is required");
        }
        self.parser
            .parse(value)
            .with_context(|| format!("{name} must be a valid pubkey"))
    }

    fn normalize_quote_mint(&self, value: Option<&str>) -> anyhow::Result<Address> {
        let value = value.unwrap_or(WSOL_MINT);
        let pubkey = self
            .parser
            .parse(value)
            .context("quote_mint must be a valid pubkey")?;
        if pubkey == self.parse_known_pubkey(SOL_MINT) {
            return Ok(self.parse_known_pubkey(WSOL_MINT));
        }
        Ok(pubkey)
    }

    fn validate_quote_mint_for_pool(&self, pool: Pool, quote_mint: &Address) -> anyhow::Result<()> {
        match registry::for_pool(pool)
            .quote_policy
            .context("protocol Quote policy is unavailable")?
        {
            QuotePolicy::WsolOrUsd1 => {
                if !self.is_wsol_quote(quote_mint) && !self.is_usd1_quote(quote_mint) {
                    bail!("bonk quote_mint must be WSOL or USD1");
                }
            }
            QuotePolicy::Any => {}
            QuotePolicy::WsolOrUsdc => {
                if !self.is_wsol_quote(quote_mint) && !self.is_usdc_quote(quote_mint) {
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

    fn is_wsol_quote(&self, quote_mint: &Address) -> bool {
        *quote_mint == self.parse_known_pubkey(WSOL_MINT)
    }

    fn is_usdc_quote(&self, quote_mint: &Address) -> bool {
        *quote_mint == self.parse_known_pubkey(USDC_MINT)
    }

    fn is_usd1_quote(&self, quote_mint: &Address) -> bool {
        *quote_mint == self.parse_known_pubkey(USD1_MINT)
    }

    fn parse_known_pubkey(&self, value: &str) -> Address {
        self.parser
            .parse(value)
            .expect("known mint pubkey must be valid")
    }
}

#[cfg(test)]
mod tests;
