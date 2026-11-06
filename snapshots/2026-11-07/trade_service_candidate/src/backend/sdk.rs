//! SDK 11 adapter for read-only route discovery and parameter preparation.
//!
//! The adapter owns the SDK-specific client. Higher layers depend on this
//! narrow surface and do not import SDK parameter types.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::{
    BackendCapabilities, QuoteRequest, QuoteResult, ResolvedRoute, RouteHint, SimulationResult,
    SubmissionResult, TradePlan,
};
use super::ExecutionBackend;
use crate::models::{Pool, ResolvePoolResponse};
use crate::sdk_client::SdkTradeClient;
use crate::wallets::WalletRegistry;

#[derive(Clone)]
pub struct SdkBackend {
    wallets: Arc<WalletRegistry>,
}

impl SdkBackend {
    pub fn new(wallets: Arc<WalletRegistry>) -> Self {
        Self { wallets }
    }

    fn default_client(&self) -> Arc<SdkTradeClient> {
        self.wallets.default()
    }

    fn client_for_wallet(&self, wallet_id: &str) -> Result<Arc<SdkTradeClient>> {
        self.wallets
            .get(wallet_id)
            .with_context(|| format!("wallet {wallet_id} is not loaded in SDK backend"))
    }

    pub async fn resolve_pool(&self, mint: &Pubkey) -> Result<ResolvePoolResponse> {
        self.default_client().resolve_pool(mint).await
    }

    pub async fn warmup_params(
        &self,
        pool: Pool,
        mint: &Pubkey,
        quote_mint: &Pubkey,
        pool_id: Option<&Pubkey>,
    ) -> Result<()> {
        self.default_client()
            .warmup_params(pool, mint, quote_mint, pool_id)
            .await
            .with_context(|| format!("SDK 11 parameter warmup failed for {pool:?}"))
    }

    fn route_from_response(response: ResolvePoolResponse) -> ResolvedRoute {
        let protocol_id = crate::backend::registry::protocol_id(response.pool).to_string();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_millis() as u64);
        ResolvedRoute {
            route_id: format!("sdk11:{}:{}", protocol_id, uuid::Uuid::new_v4().simple()),
            protocol_id,
            program_id: None,
            pool_id: response.pool_id,
            mint: response.mint.clone(),
            input_mint: None,
            output_mint: Some(response.mint),
            quote_mint: Some(response.quote_mint),
            token_program: "spl-token".to_string(),
            quote_token_program: Some("spl-token".to_string()),
            validated_slot: None,
            expires_at_ms: now_ms.saturating_add(5_000),
            supports_buy: true,
            supports_sell: true,
            supports_sol_route: false,
            supports_token2022: false,
            execution_backend: "sdk-11".to_string(),
            chain_verified: false,
            pool_owner_verified: false,
            mint_owner_verified: false,
            quote_mint_owner_verified: false,
        }
    }

    fn pool_from_protocol(protocol_id: &str) -> Result<Pool> {
        match protocol_id {
            "pump" => Ok(Pool::Pump),
            "pump-amm" => Ok(Pool::PumpAmm),
            "bonk" => Ok(Pool::Bonk),
            "stonkfun" => Ok(Pool::StonkFun),
            "stonkfun-swap" => Ok(Pool::StonkFunSwap),
            "meteora-damm-v2" => Ok(Pool::MeteoraDammV2),
            "raydium-amm-v4" => Ok(Pool::RaydiumAmmV4),
            "raydium-cpmm" => Ok(Pool::RaydiumCpmm),
            _ => bail!("SDK 11 adapter does not support protocol {protocol_id}"),
        }
    }
}

/// Turn a caller's pool hint into a candidate for RPC verification. This
/// deliberately returns no negative SDK capability conclusion on its own.
fn explicit_response_from_hint(
    hint: &RouteHint,
    mint: &Pubkey,
) -> Result<Option<ResolvePoolResponse>> {
    if hint.protocol_id.as_deref() == Some("pump") && hint.pool_id.is_some() {
        bail!("Pump bonding curve hint must omit pool_id; the SDK derives it from mint");
    }
    if hint.protocol_id.as_deref() == Some("pump-amm") && hint.pool_id.is_none() {
        return Ok(None);
    }
    if hint.protocol_id.as_deref() == Some("bonk") {
        if hint.pool_id.is_some() {
            bail!("bonk route hint must omit pool_id; the SDK derives it from mint and quote");
        }
        let quote_mint = hint
            .quote_mint
            .as_deref()
            .context("bonk route resolution requires quote_mint")?;
        return Ok(Some(ResolvePoolResponse {
            mint: mint.to_string(),
            pool: Pool::Bonk,
            pool_id: None,
            quote_mint: quote_mint.to_string(),
        }));
    }
    let pool = match hint.protocol_id.as_deref() {
        Some("pump-amm") => Pool::PumpAmm,
        Some("raydium-cpmm") => Pool::RaydiumCpmm,
        Some("stonkfun") => Pool::StonkFun,
        Some("stonkfun-swap") => Pool::StonkFunSwap,
        Some("meteora-damm-v2") => Pool::MeteoraDammV2,
        Some("raydium-amm-v4") => Pool::RaydiumAmmV4,
        _ => return Ok(None),
    };
    let pool_id = hint
        .pool_id
        .as_deref()
        .context("explicit SDK route resolution requires pool_id")?;
    let quote_mint = hint
        .quote_mint
        .as_deref()
        .context("explicit SDK route resolution requires quote_mint")?;
    Ok(Some(ResolvePoolResponse {
        mint: mint.to_string(),
        pool,
        pool_id: Some(pool_id.to_string()),
        quote_mint: quote_mint.to_string(),
    }))
}

/// Pump.fun uses a native SOL sentinel in its curve account. Route contracts
/// use WSOL as the canonical quote mint for SOL-funded buys; this alias is
/// valid only for the Pump bonding curve, never for an arbitrary pool.
fn canonical_route_quote_mint<'a>(pool: Pool, quote_mint: &'a str) -> &'a str {
    if pool == Pool::Pump && quote_mint == crate::models::SOL_MINT {
        crate::models::WSOL_MINT
    } else {
        quote_mint
    }
}

/// Reject a mint-only discovery result that is already inconsistent with the
/// caller's selected pool before fetching that result's on-chain accounts.
/// Chain-derived program and token-owner checks still run after this guard.
fn validate_discovered_response_hint(
    hint: &RouteHint,
    response: &ResolvePoolResponse,
) -> Result<()> {
    if hint.mint != response.mint {
        bail!("discovered mint does not match requested mint");
    }
    if hint
        .protocol_id
        .as_deref()
        .is_some_and(|value| value != crate::backend::registry::protocol_id(response.pool))
    {
        bail!("discovered protocol does not match requested protocol");
    }
    if hint
        .pool_id
        .as_deref()
        .is_some_and(|value| response.pool_id.as_deref() != Some(value))
    {
        bail!("discovered pool does not match requested pool");
    }
    if hint.quote_mint.as_deref().is_some_and(|value| {
        canonical_route_quote_mint(response.pool, value)
            != canonical_route_quote_mint(response.pool, &response.quote_mint)
    }) {
        bail!("discovered quote mint does not match requested quote mint");
    }
    Ok(())
}

#[async_trait]
impl ExecutionBackend for SdkBackend {
    fn name(&self) -> &'static str {
        "sdk-11"
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            protocols: vec![
                "pump".to_string(),
                "pump-amm".to_string(),
                "bonk".to_string(),
                "stonkfun".to_string(),
                "stonkfun-swap".to_string(),
                "meteora-damm-v2".to_string(),
                "raydium-amm-v4".to_string(),
                "raydium-cpmm".to_string(),
            ],
            supports_buy: true,
            supports_sell: true,
            supports_simulate: true,
            supports_live: false,
        }
    }

    async fn resolve_pool(&self, hint: RouteHint) -> Result<ResolvedRoute> {
        let mint = Pubkey::from_str(&hint.mint).context("route mint must be a valid pubkey")?;
        let client = self.default_client();
        // Mint-only discovery currently covers Pump/PumpSwap. A caller with an
        // explicit pool hint can ask the SDK to verify that pool directly; failure
        // here must not be interpreted as proof that the SDK has no route.
        let response = if hint.protocol_id.as_deref() == Some("pump-amm")
            && hint.pool_id.is_some()
            && hint.quote_mint.is_none()
        {
            let pool_id = Pubkey::from_str(
                hint.pool_id
                    .as_deref()
                    .context("selected PumpSwap pool_id is missing")?,
            )
            .context("selected PumpSwap pool_id is invalid")?;
            client.resolve_pumpswap_pool_by_id(&mint, &pool_id).await?
        } else if matches!(
            hint.protocol_id.as_deref(),
            Some("raydium-cpmm" | "raydium-amm-v4")
        ) && hint.pool_id.is_some()
            && hint.quote_mint.is_none()
        {
            let pool_id = Pubkey::from_str(
                hint.pool_id
                    .as_deref()
                    .context("selected Raydium pool_id is missing")?,
            )
            .context("selected Raydium pool_id is invalid")?;
            if hint.protocol_id.as_deref() == Some("raydium-cpmm") {
                client.resolve_cpmm_pool_by_id(&mint, &pool_id).await?
            } else {
                client.resolve_amm_v4_pool_by_id(&mint, &pool_id).await?
            }
        } else if let Some(response) = explicit_response_from_hint(&hint, &mint)? {
            response
        } else {
            let requested_pool = match hint.protocol_id.as_deref() {
                Some("pump") => Some(Pool::Pump),
                Some("pump-amm") => Some(Pool::PumpAmm),
                None => None,
                Some(_) => bail!("mint-only discovery requires a supported Pump-family hint"),
            };
            client
                .resolve_pump_family_pool(&mint, requested_pool)
                .await?
        };
        validate_discovered_response_hint(&hint, &response)?;
        let metadata = client.verify_resolved_route(&response).await?;
        let pool = response.pool;
        let mut route = Self::route_from_response(response);
        route.program_id = Some(metadata.program_id.to_string());
        route.pool_id = metadata.pool_id.map(|value| value.to_string());
        let quote_mint =
            canonical_route_quote_mint(pool, &metadata.quote_mint.to_string()).to_string();
        route.input_mint = if pool == Pool::Pump {
            Some(quote_mint.clone())
        } else {
            metadata.input_mint.map(|value| value.to_string())
        };
        route.output_mint = Some(metadata.output_mint.to_string());
        route.quote_mint = Some(quote_mint);
        route.token_program = if metadata.token_program == sol_trade_sdk::common::spl_token_2022::ID
        {
            "token-2022".to_string()
        } else if metadata.token_program == sol_trade_sdk::common::spl_token::ID {
            "spl-token".to_string()
        } else {
            bail!("resolved route uses unsupported token program")
        };
        route.quote_token_program = Some(
            if metadata.quote_token_program == sol_trade_sdk::common::spl_token_2022::ID {
                "token-2022".to_string()
            } else if metadata.quote_token_program == sol_trade_sdk::common::spl_token::ID {
                "spl-token".to_string()
            } else {
                bail!("resolved route uses unsupported quote token program")
            },
        );
        route.supports_token2022 = route.token_program == "token-2022";
        route.supports_sol_route = metadata.quote_mint
            == sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT
            || metadata.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
        route.validated_slot = Some(metadata.validated_slot);
        route.chain_verified = true;
        route.pool_owner_verified = metadata.pool_owner_verified;
        route.mint_owner_verified = metadata.mint_owner_verified;
        route.quote_mint_owner_verified = metadata.quote_mint_owner_verified;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_millis() as u64);
        if let Some(protocol_id) = hint.protocol_id {
            if protocol_id != route.protocol_id {
                bail!("resolved protocol does not match requested protocol");
            }
        }
        if let Some(pool_id) = hint.pool_id {
            if route.pool_id.as_deref() != Some(pool_id.as_str()) {
                bail!("resolved pool does not match requested pool");
            }
        }
        if let Some(program_id) = hint.program_id {
            if route.program_id.as_deref() != Some(program_id.as_str()) {
                bail!("resolved program does not match requested program");
            }
        }
        if let Some(quote_mint) = hint.quote_mint {
            if route.quote_mint.as_deref()
                != Some(canonical_route_quote_mint(pool, quote_mint.as_str()))
            {
                bail!("resolved quote mint does not match requested quote mint");
            }
        }
        crate::route_registry::validate_verified_route(&route, now_ms)
            .context("SDK 11 returned an invalid route")?;
        Ok(route)
    }

    async fn warmup(&self, hint: RouteHint) -> Result<()> {
        let protocol_id = hint
            .protocol_id
            .as_deref()
            .context("warmup protocol_id is required")?;
        let pool = Self::pool_from_protocol(protocol_id)?;
        let mint = Pubkey::from_str(&hint.mint).context("warmup mint must be a valid pubkey")?;
        let quote_mint = Pubkey::from_str(
            hint.quote_mint
                .as_deref()
                .context("warmup quote_mint is required")?,
        )
        .context("warmup quote_mint must be a valid pubkey")?;
        let pool_id = hint
            .pool_id
            .as_deref()
            .map(Pubkey::from_str)
            .transpose()
            .context("warmup pool_id must be a valid pubkey")?;
        self.warmup_params(pool, &mint, &quote_mint, pool_id.as_ref())
            .await
    }

    async fn quote_exact_in(&self, request: QuoteRequest) -> Result<QuoteResult> {
        let side = request.side.as_str();
        if side != "buy" && side != "sell" {
            bail!("quote side must be buy or sell");
        }
        if request.input_amount_units == 0 {
            bail!("quote input amount must be positive");
        }
        if request.slippage_bps > 10_000 {
            bail!("quote slippage_bps exceeds 10000");
        }
        if (side == "buy" && !request.route.supports_buy)
            || (side == "sell" && !request.route.supports_sell)
        {
            bail!("route does not support requested quote side");
        }
        let input_mint = match side {
            "buy" => request
                .route
                .quote_mint
                .as_deref()
                .context("route quote_mint is required for buy")?,
            "sell" => request
                .route
                .output_mint
                .as_deref()
                .unwrap_or(request.route.mint.as_str()),
            _ => unreachable!(),
        };
        let output_mint = match side {
            "buy" => request
                .route
                .output_mint
                .as_deref()
                .unwrap_or(request.route.mint.as_str()),
            "sell" => request
                .route
                .quote_mint
                .as_deref()
                .context("route quote_mint is required for sell")?,
            _ => unreachable!(),
        };
        let pool_id = request
            .route
            .pool_id
            .as_deref()
            .context("quote requires a pool_id")?
            .parse::<Pubkey>()
            .context("quote pool_id is invalid")?;
        let protocol = request.route.protocol_id.as_str();
        let (expected_output_units, fee_units) = match protocol {
            "raydium-cpmm" => {
                let input = Pubkey::from_str(input_mint).context("quote input mint is invalid")?;
                let output =
                    Pubkey::from_str(output_mint).context("quote output mint is invalid")?;
                self.client_for_wallet(&request.wallet_id)?
                    .quote_raydium_cpmm(&pool_id, &input, &output, request.input_amount_units)
                    .await?
            }
            "raydium-amm-v4" => {
                let input = Pubkey::from_str(input_mint).context("quote input mint is invalid")?;
                let output =
                    Pubkey::from_str(output_mint).context("quote output mint is invalid")?;
                self.client_for_wallet(&request.wallet_id)?
                    .quote_raydium_amm_v4(&pool_id, &input, &output, request.input_amount_units)
                    .await?
            }
            "pump-amm" if side == "buy" => {
                let input = Pubkey::from_str(input_mint).context("quote input mint is invalid")?;
                let output =
                    Pubkey::from_str(output_mint).context("quote output mint is invalid")?;
                self.client_for_wallet(&request.wallet_id)?
                    .quote_pumpswap_buy(&pool_id, &input, &output, request.input_amount_units)
                    .await?
            }
            _ => bail!("SDK 11 quote adapter is not implemented for protocol {protocol}"),
        };
        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_millis() as u64);
        if request.route.expires_at_ms <= created_at_ms {
            bail!("SDK route expired while loading the quote snapshot");
        }
        let min_output_units = u64::try_from(
            (expected_output_units as u128)
                .saturating_mul((10_000u64 - request.slippage_bps) as u128)
                / 10_000,
        )
        .context("quote minimum output exceeds u64")?;
        Ok(QuoteResult {
            quote_id: format!("sdk11-quote:{}", uuid::Uuid::new_v4().simple()),
            route_id: request.route.route_id,
            wallet_id: request.wallet_id,
            input_mint: input_mint.to_string(),
            output_mint: output_mint.to_string(),
            input_amount_units: request.input_amount_units,
            side: request.side,
            expected_output_units,
            min_output_units,
            slippage_bps: request.slippage_bps,
            fee_units,
            created_at_ms,
            expires_at_ms: request
                .route
                .expires_at_ms
                .min(created_at_ms.saturating_add(5_000)),
        })
    }

    async fn simulate(&self, plan: TradePlan) -> Result<SimulationResult> {
        if plan.side != "buy" {
            bail!("SDK 11 simulation adapter currently supports buy plans only")
        }
        let protocol = plan.route.protocol_id.clone();
        let pool = crate::backend::registry::pool_for_protocol(&protocol)
            .context("simulation route protocol is not registered")?;
        if pool != crate::models::Pool::RaydiumCpmm {
            bail!("SDK 11 simulation adapter is not implemented for protocol {protocol}")
        }
        let mint = Pubkey::from_str(&plan.route.mint).context("simulation mint is invalid")?;
        let quote_mint = Pubkey::from_str(
            plan.route
                .quote_mint
                .as_deref()
                .context("simulation quote_mint is required")?,
        )
        .context("simulation quote_mint is invalid")?;
        let pool_id = plan
            .route
            .pool_id
            .as_deref()
            .context("simulation pool_id is required")?
            .parse::<Pubkey>()
            .context("simulation pool_id is invalid")?;
        let result = self
            .client_for_wallet(&plan.quote.wallet_id)?
            .simulate_buy(
                pool,
                mint,
                quote_mint,
                Some(pool_id),
                plan.quote.input_amount_units,
                plan.slippage_bps,
            )
            .await?;
        let logs = result.error.into_iter().collect();
        Ok(SimulationResult {
            ok: result.success,
            units_consumed: None,
            logs,
            protocol: Some(protocol),
            source: Some("sdk".to_string()),
            route_id: Some(plan.route.route_id),
            expected_output_units: Some(plan.quote.expected_output_units),
            min_output_units: Some(plan.quote.min_output_units),
            fee_units: Some(plan.quote.fee_units),
            simulation_slot: None,
            signature: None,
        })
    }

    async fn submit(&self, _plan: TradePlan, _idempotency_key: &str) -> Result<SubmissionResult> {
        bail!("SDK 11 adapter submit is disabled until unified simulation passes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sol_trade_sdk::trading::core::params::RaydiumCpmmParams;

    #[test]
    fn raydium_cpmm_fixture_quotes_both_directions_with_fee() {
        let base = Pubkey::new_unique();
        let quote = Pubkey::new_unique();
        let params = RaydiumCpmmParams::from_trade(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            base,
            quote,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            sol_trade_sdk::constants::TOKEN_PROGRAM,
            sol_trade_sdk::constants::TOKEN_PROGRAM,
            Pubkey::new_unique(),
            1_000_000_000,
            2_000_000_000,
        );
        let buy = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &params, true, 1_000_000, 0,
        )
        .unwrap();
        let sell = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &params, false, 1_000_000, 0,
        )
        .unwrap();
        assert!(buy.amount_out > 0 && sell.amount_out > 0);
        assert!(buy.fee > 0 && sell.fee > 0);
        assert!(buy.amount_out < 2_000_000);
        assert!(sell.amount_out < 1_000_000);
    }

    #[test]
    fn explicit_sdk_pool_hints_require_both_pool_and_quote() {
        let mint = Pubkey::new_unique();
        let pool_id = Pubkey::new_unique().to_string();
        let quote_mint = Pubkey::new_unique().to_string();
        let pump_curve_hint = RouteHint {
            protocol_id: Some("pump".into()),
            program_id: None,
            pool_id: Some(pool_id.clone()),
            mint: mint.to_string(),
            quote_mint: Some(quote_mint.clone()),
        };
        assert!(explicit_response_from_hint(&pump_curve_hint, &mint).is_err());
        for (protocol, pool) in [
            ("pump-amm", Pool::PumpAmm),
            ("raydium-cpmm", Pool::RaydiumCpmm),
            ("stonkfun", Pool::StonkFun),
            ("stonkfun-swap", Pool::StonkFunSwap),
            ("meteora-damm-v2", Pool::MeteoraDammV2),
            ("raydium-amm-v4", Pool::RaydiumAmmV4),
        ] {
            let mut hint = RouteHint {
                protocol_id: Some(protocol.to_string()),
                program_id: None,
                pool_id: Some(pool_id.clone()),
                mint: mint.to_string(),
                quote_mint: Some(quote_mint.clone()),
            };
            let response = explicit_response_from_hint(&hint, &mint).unwrap().unwrap();
            assert_eq!(response.pool, pool);
            assert_eq!(response.pool_id.as_deref(), Some(pool_id.as_str()));
            hint.pool_id = None;
            if protocol == "pump-amm" {
                assert!(explicit_response_from_hint(&hint, &mint).unwrap().is_none());
            } else {
                assert!(explicit_response_from_hint(&hint, &mint).is_err());
            }
            hint.pool_id = Some(pool_id.clone());
            hint.quote_mint = None;
            assert!(explicit_response_from_hint(&hint, &mint).is_err());
        }
    }

    #[test]
    fn mint_discovery_rejects_a_different_selected_pool_before_chain_reads() {
        let mint = Pubkey::new_unique().to_string();
        let quote = Pubkey::new_unique().to_string();
        let selected_pool = Pubkey::new_unique().to_string();
        let hint = RouteHint {
            protocol_id: Some("pump-amm".into()),
            program_id: None,
            pool_id: None,
            mint: mint.clone(),
            quote_mint: Some(quote.clone()),
        };
        let mut discovered = ResolvePoolResponse {
            mint: mint.clone(),
            pool: Pool::Pump,
            pool_id: None,
            quote_mint: quote.clone(),
        };
        assert!(validate_discovered_response_hint(&hint, &discovered).is_err());
        discovered.pool = Pool::PumpAmm;
        discovered.pool_id = Some(selected_pool.clone());
        assert!(validate_discovered_response_hint(&hint, &discovered).is_ok());
        discovered.quote_mint = Pubkey::new_unique().to_string();
        assert!(validate_discovered_response_hint(&hint, &discovered).is_err());
        discovered.quote_mint = quote;
        let mut pinned_hint = hint;
        pinned_hint.pool_id = Some(Pubkey::new_unique().to_string());
        assert!(validate_discovered_response_hint(&pinned_hint, &discovered).is_err());
        pinned_hint.pool_id = Some(selected_pool);
        assert!(validate_discovered_response_hint(&pinned_hint, &discovered).is_ok());
    }

    #[test]
    fn pump_native_sol_curve_uses_wsol_route_alias_only_for_pump() {
        let mint = Pubkey::new_unique().to_string();
        let hint = RouteHint {
            protocol_id: Some("pump".into()),
            program_id: None,
            pool_id: None,
            mint: mint.clone(),
            quote_mint: Some(crate::models::WSOL_MINT.into()),
        };
        let response = ResolvePoolResponse {
            mint,
            pool: Pool::Pump,
            pool_id: None,
            quote_mint: crate::models::SOL_MINT.into(),
        };
        assert!(validate_discovered_response_hint(&hint, &response).is_ok());
        assert_eq!(
            canonical_route_quote_mint(Pool::Pump, crate::models::SOL_MINT),
            crate::models::WSOL_MINT
        );
        assert_eq!(
            canonical_route_quote_mint(Pool::PumpAmm, crate::models::SOL_MINT),
            crate::models::SOL_MINT
        );
        let mut wrong_quote = hint;
        wrong_quote.quote_mint = Some(crate::models::USDC_MINT.into());
        assert!(validate_discovered_response_hint(&wrong_quote, &response).is_err());
    }

    #[test]
    fn bonk_hint_uses_sdk_derived_pool_and_requires_quote() {
        let mint = Pubkey::new_unique();
        let mut hint = RouteHint {
            protocol_id: Some("bonk".into()),
            program_id: None,
            pool_id: None,
            mint: mint.to_string(),
            quote_mint: Some(crate::models::WSOL_MINT.into()),
        };
        let response = explicit_response_from_hint(&hint, &mint).unwrap().unwrap();
        assert_eq!(response.pool, Pool::Bonk);
        assert_eq!(response.pool_id, None);
        hint.quote_mint = None;
        assert!(explicit_response_from_hint(&hint, &mint).is_err());
        hint.quote_mint = Some(crate::models::WSOL_MINT.into());
        hint.pool_id = Some(Pubkey::new_unique().to_string());
        assert!(explicit_response_from_hint(&hint, &mint).is_err());
    }
}
