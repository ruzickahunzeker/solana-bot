//! SDK-neutral route and quote lifecycle validation.
//!
//! A route is an authorization boundary: resolving a mint is not enough to
//! permit execution. Callers must validate the protocol, pool, mints, token
//! program and freshness before a quote or simulation is accepted.

use crate::backend::types::{
    ObservedSolBridge, QuoteResult, ResolvedRoute, TradePlan, VerifiedViaSolRoute,
};
use crate::models::{ExecutionMethod, SettlementAsset, Side, TradeRequest};
use crate::validation::NormalizedTrade;
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_QUOTE_TTL_MS: u64 = 30_000;

/// Short lived server side route registry. A route returned to a caller is
/// only admissible later when it exactly matches the verified snapshot stored
/// here. This prevents callers from manufacturing `chain_verified` flags or
/// changing pool/mint metadata in a JSON request.
#[derive(Clone, Default)]
pub struct RouteStore {
    routes: Arc<RwLock<HashMap<String, ResolvedRoute>>>,
    via_sol_routes: Arc<RwLock<HashMap<String, VerifiedViaSolRoute>>>,
    observed_sol_bridges: Arc<RwLock<HashMap<String, ObservedSolBridge>>>,
    observed_sdk_previews: Arc<RwLock<HashMap<String, ObservedSdkPreviewSnapshot>>>,
    sdk_decisions: Arc<RwLock<HashMap<String, SdkTradeDecisionSnapshot>>>,
}

/// Short lived proof of a successful unsigned SDK simulation. It is bound to
/// the observation and request, and cannot be used as an execution decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSdkPreviewSnapshot {
    pub preview_id: String,
    pub observation_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub request_fingerprint: String,
    pub input_lamports: u64,
    pub bridge_min_output_units: u64,
    pub target_min_output_units: u64,
    pub target_token_delta_units: u64,
    pub transaction_bytes: usize,
    pub transaction_version: &'static str,
    pub validated_slot: u64,
    pub simulation_slot: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SdkTradeDecisionSnapshot {
    pub decision_id: String,
    pub route_id: String,
    pub via_sol: bool,
    /// Successful unsigned SDK bridge preview for a simulate-only decision.
    /// Ordinary single-pool and Via-SOL decisions leave this empty.
    pub bridge_preview_id: Option<String>,
    /// Bound reverse CPMM route proof. It never authorizes signing or sending.
    pub cpmm_sell_evidence: Option<SdkCpmmSellEvidence>,
    /// Bound forward CPMM route proof. A fresh simulation is still required
    /// before the message can be signed or sent.
    pub cpmm_buy_evidence: Option<SdkCpmmBuyEvidence>,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub idempotency_key: String,
    pub request_fingerprint: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SdkCpmmSellEvidence {
    pub input_token_units: u64,
    pub minimum_wsol_units: u64,
    pub validated_slot: u64,
    pub simulation_slot: u64,
}

#[derive(Debug, Clone)]
pub struct SdkCpmmBuyEvidence {
    pub input_lamports: u64,
    pub minimum_target_units: u64,
    pub validated_slot: u64,
    pub simulation_slot: u64,
}

impl RouteStore {
    pub async fn insert(&self, route: ResolvedRoute, now_ms: u64) -> Result<()> {
        validate_verified_route(&route, now_ms)?;
        let mut routes = self.routes.write().await;
        routes.retain(|_, existing| existing.expires_at_ms > now_ms);
        routes.insert(route.route_id.clone(), route);
        Ok(())
    }

    pub async fn admit(&self, route: &ResolvedRoute, now_ms: u64) -> Result<()> {
        validate_verified_route(route, now_ms)?;
        let stored = self
            .routes
            .read()
            .await
            .get(&route.route_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("route_id is not present in server route store"))?;
        if stored != *route {
            bail!("route snapshot does not match server route store");
        }
        Ok(())
    }

    pub async fn get(&self, route_id: &str, now_ms: u64) -> Result<ResolvedRoute> {
        let route = self
            .routes
            .read()
            .await
            .get(route_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("route_id is not present in server route store"))?;
        validate_verified_route(&route, now_ms)?;
        Ok(route)
    }

    pub async fn purge_expired(&self, now_ms: u64) {
        self.routes
            .write()
            .await
            .retain(|_, route| route.expires_at_ms > now_ms);
        self.via_sol_routes
            .write()
            .await
            .retain(|_, route| route.expires_at_ms > now_ms);
        self.observed_sol_bridges
            .write()
            .await
            .retain(|_, route| route.expires_at_ms > now_ms);
        self.observed_sdk_previews
            .write()
            .await
            .retain(|_, preview| preview.expires_at_ms > now_ms);
        self.sdk_decisions
            .write()
            .await
            .retain(|_, decision| decision.expires_at_ms > now_ms);
    }

    pub async fn insert_via_sol(&self, route: VerifiedViaSolRoute, now_ms: u64) -> Result<()> {
        validate_verified_via_sol_route(&route, now_ms)?;
        let mut routes = self.via_sol_routes.write().await;
        routes.retain(|_, existing| existing.expires_at_ms > now_ms);
        routes.insert(route.route_id.clone(), route);
        Ok(())
    }

    pub async fn admit_via_sol(&self, route: &VerifiedViaSolRoute, now_ms: u64) -> Result<()> {
        validate_verified_via_sol_route(route, now_ms)?;
        let stored = self
            .via_sol_routes
            .read()
            .await
            .get(&route.route_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("via-SOL route_id is not present in server route store")
            })?;
        if stored != *route {
            bail!("via-SOL route snapshot does not match server route store");
        }
        Ok(())
    }

    pub async fn get_via_sol(&self, route_id: &str, now_ms: u64) -> Result<VerifiedViaSolRoute> {
        let route = self
            .via_sol_routes
            .read()
            .await
            .get(route_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("via-SOL route_id is not present in server route store")
            })?;
        validate_verified_via_sol_route(&route, now_ms)?;
        Ok(route)
    }

    pub async fn insert_observed_sol_bridge(
        &self,
        observation: ObservedSolBridge,
        now_ms: u64,
    ) -> Result<()> {
        validate_observed_sol_bridge(&observation, now_ms)?;
        let mut observations = self.observed_sol_bridges.write().await;
        observations.retain(|_, existing| existing.expires_at_ms > now_ms);
        if observations.contains_key(&observation.observation_id) {
            bail!("SOL bridge observation_id already exists");
        }
        observations.insert(observation.observation_id.clone(), observation);
        Ok(())
    }

    pub async fn get_observed_sol_bridge(
        &self,
        observation_id: &str,
        now_ms: u64,
    ) -> Result<ObservedSolBridge> {
        let observation = self
            .observed_sol_bridges
            .read()
            .await
            .get(observation_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("SOL bridge observation_id is not present"))?;
        validate_observed_sol_bridge(&observation, now_ms)?;
        Ok(observation)
    }

    pub async fn admit_observed_sol_bridge(
        &self,
        observation: &ObservedSolBridge,
        now_ms: u64,
    ) -> Result<()> {
        let stored = self
            .get_observed_sol_bridge(&observation.observation_id, now_ms)
            .await?;
        if stored != *observation {
            bail!("SOL bridge observation does not match server snapshot");
        }
        Ok(())
    }

    pub async fn insert_observed_sdk_preview(
        &self,
        preview: ObservedSdkPreviewSnapshot,
        now_ms: u64,
    ) -> Result<()> {
        let observation = self
            .get_observed_sol_bridge(&preview.observation_id, now_ms)
            .await?;
        if !preview.preview_id.starts_with("observe:sdk-preview:")
            || preview.preview_id.len() > 128
            || preview.wallet_id != observation.wallet_id
            || preview.wallet_pubkey != observation.wallet_pubkey
            || preview.request_fingerprint != observation.request_fingerprint
            || preview.input_lamports == 0
            || preview.bridge_min_output_units == 0
            || preview.target_min_output_units == 0
            || preview.target_token_delta_units < preview.target_min_output_units
            || !matches!(preview.transaction_version, "legacy" | "v1")
            || preview.transaction_bytes == 0
            || preview.transaction_bytes
                > if preview.transaction_version == "v1" {
                    solana_sdk::message::v1::MAX_TRANSACTION_SIZE
                } else {
                    1_232
                }
            || preview.simulation_slot < preview.validated_slot
            || preview.expires_at_ms <= now_ms
            || preview.expires_at_ms > observation.expires_at_ms
            || preview.expires_at_ms > now_ms.saturating_add(5_000)
        {
            bail!("SDK preview binding is invalid");
        }
        let mut previews = self.observed_sdk_previews.write().await;
        previews.retain(|_, existing| existing.expires_at_ms > now_ms);
        if previews.contains_key(&preview.preview_id) {
            bail!("SDK preview_id already exists");
        }
        previews.insert(preview.preview_id.clone(), preview);
        Ok(())
    }

    pub async fn get_observed_sdk_preview(
        &self,
        preview_id: &str,
        now_ms: u64,
    ) -> Result<ObservedSdkPreviewSnapshot> {
        let preview = self
            .observed_sdk_previews
            .read()
            .await
            .get(preview_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("SDK preview_id is not present"))?;
        if preview.expires_at_ms <= now_ms {
            bail!("SDK preview has expired");
        }
        self.get_observed_sol_bridge(&preview.observation_id, now_ms)
            .await?;
        Ok(preview)
    }

    pub async fn insert_sdk_decision(
        &self,
        decision: SdkTradeDecisionSnapshot,
        now_ms: u64,
    ) -> Result<()> {
        if decision.decision_id.trim().is_empty()
            || decision.wallet_id.trim().is_empty()
            || decision.wallet_pubkey.trim().is_empty()
            || decision.idempotency_key.trim().is_empty()
            || decision.request_fingerprint.len() != 64
            || decision.expires_at_ms <= now_ms
        {
            bail!("SDK decision binding is invalid");
        }
        if decision.cpmm_sell_evidence.is_some() && decision.cpmm_buy_evidence.is_some() {
            bail!("CPMM decision cannot bind both directions");
        }
        if let Some(evidence) = decision.cpmm_buy_evidence.as_ref() {
            if !decision.via_sol
                || decision.bridge_preview_id.is_some()
                || evidence.input_lamports == 0
                || evidence.minimum_target_units == 0
                || evidence.simulation_slot < evidence.validated_slot
                || decision.expires_at_ms > now_ms.saturating_add(5_000)
            {
                bail!("CPMM buy decision evidence is invalid");
            }
            let route = self.get_via_sol(&decision.route_id, now_ms).await?;
            if route.target.protocol_id != "raydium-cpmm" {
                bail!("CPMM buy decision target pool changed");
            }
        }
        if let Some(evidence) = decision.cpmm_sell_evidence.as_ref() {
            if !decision.via_sol
                || decision.bridge_preview_id.is_some()
                || decision.idempotency_key.trim().is_empty()
                || evidence.input_token_units == 0
                || evidence.minimum_wsol_units == 0
                || evidence.simulation_slot < evidence.validated_slot
                || decision.expires_at_ms > now_ms.saturating_add(5_000)
            {
                bail!("CPMM sell decision evidence is invalid");
            }
            let route = self.get_via_sol(&decision.route_id, now_ms).await?;
            if route.target.protocol_id != "raydium-cpmm" {
                bail!("CPMM sell decision target pool changed");
            }
        }
        let route_expiry = if let Some(preview_id) = decision.bridge_preview_id.as_deref() {
            if decision.via_sol || !decision.decision_id.starts_with("sdk-decision:bridge:") {
                bail!("SDK bridge decision kind is invalid");
            }
            let preview = self.get_observed_sdk_preview(preview_id, now_ms).await?;
            if decision.route_id != preview.observation_id
                || decision.wallet_id != preview.wallet_id
                || decision.wallet_pubkey != preview.wallet_pubkey
                || decision.request_fingerprint != preview.request_fingerprint
            {
                bail!("SDK bridge decision does not match preview evidence");
            }
            preview.expires_at_ms
        } else if decision.via_sol {
            if decision.decision_id.starts_with("sdk-decision:bridge:") {
                bail!("SDK bridge decision has no preview evidence");
            }
            self.get_via_sol(&decision.route_id, now_ms)
                .await?
                .expires_at_ms
        } else {
            if decision.decision_id.starts_with("sdk-decision:bridge:") {
                bail!("SDK bridge decision has no preview evidence");
            }
            self.get(&decision.route_id, now_ms).await?.expires_at_ms
        };
        if decision.expires_at_ms > route_expiry {
            bail!("SDK decision expires after its verified route");
        }
        let mut decisions = self.sdk_decisions.write().await;
        decisions.retain(|_, existing| existing.expires_at_ms > now_ms);
        if decisions.contains_key(&decision.decision_id) {
            bail!("SDK decision_id already exists");
        }
        decisions.insert(decision.decision_id.clone(), decision);
        Ok(())
    }

    pub async fn get_sdk_decision(
        &self,
        decision_id: &str,
        now_ms: u64,
    ) -> Result<SdkTradeDecisionSnapshot> {
        let decision = self
            .sdk_decisions
            .read()
            .await
            .get(decision_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("SDK decision_id is not present in server store"))?;
        if decision.expires_at_ms <= now_ms {
            bail!("SDK decision is expired");
        }
        if let Some(preview_id) = decision.bridge_preview_id.as_deref() {
            let preview = self.get_observed_sdk_preview(preview_id, now_ms).await?;
            if preview.observation_id != decision.route_id
                || preview.expires_at_ms < decision.expires_at_ms
            {
                bail!("SDK bridge decision evidence has changed");
            }
        } else if decision.via_sol {
            self.get_via_sol(&decision.route_id, now_ms).await?;
        } else {
            self.get(&decision.route_id, now_ms).await?;
        }
        Ok(decision)
    }
}

pub async fn validate_sdk_decision_trade(
    store: &RouteStore,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
    wallet_pubkey: &str,
    now_ms: u64,
) -> Result<Option<solana_sdk::pubkey::Pubkey>> {
    let decision_id = request
        .sdk_decision_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("sdk_decision_id is required"))?;
    let decision = store.get_sdk_decision(decision_id, now_ms).await?;
    let cpmm_sell = request.pool == crate::models::Pool::RaydiumCpmm
        && request.side == Side::Sell
        && normalized.settlement_asset == SettlementAsset::Sol;
    if cpmm_sell != decision.cpmm_sell_evidence.is_some() {
        bail!("SDK CPMM sell decision kind does not match trade");
    }
    let cpmm_buy = request.pool == crate::models::Pool::RaydiumCpmm
        && request.side == Side::Buy
        && normalized.settlement_asset == SettlementAsset::Sol;
    if cpmm_buy != decision.cpmm_buy_evidence.is_some() {
        bail!("SDK CPMM buy decision kind does not match trade");
    }
    if let Some(preview_id) = decision.bridge_preview_id.as_deref() {
        if !crate::validation::has_bridge_sdk_decision_id(request)
            || !decision.decision_id.starts_with("sdk-decision:bridge:")
            || decision.wallet_id != wallet_id
            || decision.wallet_pubkey != wallet_pubkey
            || decision.idempotency_key != request.idempotency_key
        {
            bail!("SDK bridge decision does not match wallet or trade request");
        }
        let mut preview_request = request.clone();
        preview_request.sdk_decision_id = None;
        preview_request.sdk_preview_id = Some(preview_id.to_string());
        let (preview, observation) = validate_sdk_preview_trade(
            store,
            &preview_request,
            normalized,
            wallet_id,
            wallet_pubkey,
            now_ms,
        )
        .await?;
        if preview.request_fingerprint != decision.request_fingerprint
            || observation.observation_id != decision.route_id
        {
            bail!("SDK bridge decision preview binding changed");
        }
        return Ok(None);
    }
    let fingerprint = crate::persistence::request_hash(wallet_id, request, normalized)?;
    if decision.wallet_id != wallet_id
        || decision.wallet_pubkey != wallet_pubkey
        || decision.idempotency_key != request.idempotency_key
        || decision.request_fingerprint != fingerprint
    {
        bail!("SDK decision does not match wallet or trade request");
    }
    if decision.via_sol {
        let route = store.get_via_sol(&decision.route_id, now_ms).await?;
        validate_via_sol_trade_match(&route, request, normalized, now_ms)?;
        Ok(None)
    } else {
        let route = store.get(&decision.route_id, now_ms).await?;
        validate_single_sdk_trade_match(&route, request, normalized, now_ms)?;
        route
            .pool_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(Into::into)
    }
}

pub fn validate_single_sdk_trade_match(
    route: &ResolvedRoute,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    now_ms: u64,
) -> Result<()> {
    validate_verified_route(route, now_ms)?;
    if normalized.execution_method != ExecutionMethod::Sdk
        || normalized.settlement_asset != SettlementAsset::Quote
        || normalized.sol_hop_pool.is_some()
        || normalized.sol_hop_pool_id.is_some()
        || route.execution_backend != "sdk-11"
    {
        bail!("single-pool SDK decision requires a direct SDK trade");
    }
    let pool_id = normalized.pool_id.map(|value| value.to_string());
    let pool_matches = match pool_id.as_deref() {
        Some(pool_id) => route.pool_id.as_deref() == Some(pool_id),
        None if request.pool == crate::models::Pool::PumpAmm => route.pool_id.is_some(),
        None if matches!(
            request.pool,
            crate::models::Pool::Pump | crate::models::Pool::Bonk
        ) =>
        {
            route.pool_id.is_none()
        }
        None => false,
    };
    let mint = normalized.mint.to_string();
    let quote_mint = normalized.quote_mint.to_string();
    if route.protocol_id != crate::backend::registry::protocol_id(request.pool)
        || !pool_matches
        || route.mint != mint
        || route.input_mint.as_deref() != Some(quote_mint.as_str())
        || route.output_mint.as_deref() != Some(mint.as_str())
        || route.quote_mint.as_deref() != Some(quote_mint.as_str())
    {
        bail!("single-pool SDK decision does not match trade route");
    }
    match request.side {
        Side::Buy if !route.supports_buy => bail!("single-pool SDK route does not support buy"),
        Side::Sell if !route.supports_sell => bail!("single-pool SDK route does not support sell"),
        _ => Ok(()),
    }
}

/// Validate two independently verified legs before considering any SOL-funded
/// non-WSOL quote route. This does not claim that a transaction builder exists.
pub fn validate_wsol_quote_cpmm_bridge(
    target: &ResolvedRoute,
    sol_hop: &ResolvedRoute,
    now_ms: u64,
) -> Result<u64> {
    use crate::models::WSOL_MINT;

    validate_verified_route(target, now_ms)?;
    validate_verified_route(sol_hop, now_ms)?;
    if sol_hop.protocol_id != "raydium-cpmm"
        || target.execution_backend != "sdk-11"
        || sol_hop.execution_backend != "sdk-11"
    {
        bail!("WSOL/Quote bridge has unsupported SDK legs");
    }
    if !target.supports_buy || !sol_hop.supports_buy {
        bail!("WSOL/Quote bridge does not support buy");
    }
    // Pump.fun bonding curves have a deterministic mint-derived PDA instead
    // of a caller-supplied pool ID. The RPC adapter already verified that PDA
    // and its owner; retain the program identity check at this boundary.
    let verified_pump_curve = target.protocol_id == "pump"
        && target.pool_id.is_none()
        && target
            .program_id
            .as_deref()
            .and_then(|value| value.parse::<solana_sdk::pubkey::Pubkey>().ok())
            == Some(sol_trade_sdk::instruction::utils::pumpfun::accounts::PUMPFUN);
    if sol_hop.pool_id.is_none() || (target.pool_id.is_none() && !verified_pump_curve) {
        bail!("WSOL/Quote bridge requires a verified target identity and CPMM pool ID");
    }
    let target_quote = target
        .quote_mint
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("WSOL/Quote target quote mint is missing"))?;
    if target_quote == WSOL_MINT
        || target.input_mint.as_deref() != Some(target_quote)
        || target.output_mint.as_deref() != Some(target.mint.as_str())
        || sol_hop.mint != target_quote
        || sol_hop.output_mint.as_deref() != Some(target_quote)
        || sol_hop.input_mint.as_deref() != Some(WSOL_MINT)
        || sol_hop.quote_mint.as_deref() != Some(WSOL_MINT)
        || target.quote_token_program.as_deref() != Some(sol_hop.token_program.as_str())
        || sol_hop.quote_token_program.as_deref() != Some("spl-token")
    {
        bail!("WSOL/Quote bridge mint continuity is invalid");
    }
    if target.pool_id == sol_hop.pool_id {
        bail!("WSOL/Quote bridge uses the same pool twice");
    }
    let target_slot = target.validated_slot.unwrap_or_default();
    let hop_slot = sol_hop.validated_slot.unwrap_or_default();
    if target_slot.abs_diff(hop_slot) > 16 {
        bail!("WSOL/Quote bridge legs were verified too far apart");
    }
    Ok(target.expires_at_ms.min(sol_hop.expires_at_ms))
}

pub fn sol_bridge_request_fingerprint(
    request: &TradeRequest,
    wallet_pubkey: &str,
) -> Result<String> {
    let bytes = serde_json::to_vec(&(wallet_pubkey, request))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// A preview ID is a lookup credential, not part of the original bridge
/// intent. All other request fields, including the idempotency key, remain
/// bound to the unsigned simulation.
pub fn sdk_preview_request_fingerprint(
    request: &TradeRequest,
    wallet_pubkey: &str,
) -> Result<String> {
    let mut original = request.clone();
    original.sdk_preview_id = None;
    sol_bridge_request_fingerprint(&original, wallet_pubkey)
}

pub async fn validate_sdk_preview_trade(
    store: &RouteStore,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    wallet_id: &str,
    wallet_pubkey: &str,
    now_ms: u64,
) -> Result<(ObservedSdkPreviewSnapshot, ObservedSolBridge)> {
    let preview_id = request
        .sdk_preview_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("sdk_preview_id is required"))?;
    let preview = store.get_observed_sdk_preview(preview_id, now_ms).await?;
    let observation = store
        .get_observed_sol_bridge(&preview.observation_id, now_ms)
        .await?;
    // A quote mint omitted from the original SOL buy remains omitted in the
    // request fingerprint. The chain-verified target quote in the observation
    // supplies the intermediate mint for this simulate-only credential.
    let quote_matches = if request.quote_mint.is_none() {
        normalized.quote_mint.to_string() == crate::models::WSOL_MINT
    } else {
        observation.target.quote_mint.as_deref() == Some(normalized.quote_mint.to_string().as_str())
    };
    if preview.wallet_id != wallet_id
        || preview.wallet_pubkey != wallet_pubkey
        || preview.request_fingerprint != sdk_preview_request_fingerprint(request, wallet_pubkey)?
        || observation.request_fingerprint != preview.request_fingerprint
        || observation.target.protocol_id != crate::backend::registry::protocol_id(request.pool)
        || normalized.execution_method != ExecutionMethod::Sdk
        || normalized.settlement_asset != SettlementAsset::Quote
        || request.side != Side::Buy
        || !matches!(
            request.pool,
            crate::models::Pool::PumpAmm
                | crate::models::Pool::RaydiumAmmV4
                | crate::models::Pool::RaydiumCpmm
        )
        || observation.target.pool_id != request.pool_id
        || observation.target.mint != normalized.mint.to_string()
        || !quote_matches
        || normalized.amount_lamports != Some(preview.input_lamports)
    {
        bail!("SDK preview does not match wallet or trade route");
    }
    Ok((preview, observation))
}

pub fn validate_observed_sol_bridge(observation: &ObservedSolBridge, now_ms: u64) -> Result<()> {
    if !observation
        .observation_id
        .starts_with("observe:sol-bridge:")
        || observation.observation_id.len() > 128
        || observation.wallet_id.trim().is_empty()
        || observation
            .wallet_pubkey
            .parse::<solana_sdk::pubkey::Pubkey>()
            .is_err()
        || observation.request_fingerprint.len() != 64
        || !observation
            .request_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("SOL bridge observation binding is invalid");
    }
    let expires_at_ms =
        validate_wsol_quote_cpmm_bridge(&observation.target, &observation.sol_hop, now_ms)?;
    if observation.expires_at_ms != expires_at_ms || expires_at_ms <= now_ms {
        bail!("SOL bridge observation expiry is invalid");
    }
    Ok(())
}

/// A slow unsigned preview may outlive its original five-second observation.
/// A new chain observation can extend the credential only when it still names
/// the same two pools, mints, programs, wallet and original request. Slots and
/// expiry are intentionally refreshed; no other route field may change.
pub fn validate_refreshed_sol_bridge(
    original: &ObservedSolBridge,
    refreshed: &ObservedSolBridge,
    now_ms: u64,
) -> Result<()> {
    validate_observed_sol_bridge(refreshed, now_ms)?;
    let same_route = |old: &ResolvedRoute, new: &ResolvedRoute| {
        let mut old = old.clone();
        let mut new = new.clone();
        let old_slot = old.validated_slot.take().unwrap_or_default();
        let new_slot = new.validated_slot.take().unwrap_or_default();
        old.route_id.clear();
        new.route_id.clear();
        old.expires_at_ms = 0;
        new.expires_at_ms = 0;
        new_slot >= old_slot && old == new
    };
    if original.wallet_id != refreshed.wallet_id
        || original.wallet_pubkey != refreshed.wallet_pubkey
        || original.request_fingerprint != refreshed.request_fingerprint
        || !same_route(&original.target, &refreshed.target)
        || !same_route(&original.sol_hop, &refreshed.sol_hop)
    {
        bail!("refreshed SOL bridge changed the bound route identity");
    }
    Ok(())
}

pub fn validate_verified_via_sol_route(route: &VerifiedViaSolRoute, now_ms: u64) -> Result<()> {
    if route.route_id.trim().is_empty() {
        bail!("via-SOL route_id is empty");
    }
    if !matches!(
        route.target.protocol_id.as_str(),
        "stonkfun" | "stonkfun-swap" | "raydium-cpmm"
    ) {
        bail!("via-SOL route has unsupported target protocol");
    }
    let expires_at_ms = validate_wsol_quote_cpmm_bridge(&route.target, &route.sol_hop, now_ms)?;
    if route.expires_at_ms != expires_at_ms || route.expires_at_ms <= now_ms {
        bail!("via-SOL route expiry is invalid");
    }
    Ok(())
}

/// Bind a server-issued two-leg snapshot to the normalized trade fields. A
/// client route ID is only a lookup key; pool, mint and side still come from
/// the verified server snapshot and must match the request exactly.
pub fn validate_via_sol_trade_match(
    route: &VerifiedViaSolRoute,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
    now_ms: u64,
) -> Result<()> {
    validate_verified_via_sol_route(route, now_ms)?;
    if normalized.execution_method != ExecutionMethod::Sdk
        || normalized.settlement_asset != SettlementAsset::Sol
        || normalized.sol_hop_pool != Some(crate::models::Pool::RaydiumCpmm)
    {
        bail!("via-SOL snapshot requires an SDK SOL-settled trade");
    }
    if request
        .sdk_route_id
        .as_deref()
        .is_some_and(|id| id != route.route_id)
    {
        bail!("via-SOL route_id does not match request");
    }
    let target_pool_id = normalized.pool_id.map(|id| id.to_string());
    let hop_pool_id = normalized.sol_hop_pool_id.map(|id| id.to_string());
    let quote_mint = normalized.quote_mint.to_string();
    if route.target.protocol_id != crate::backend::registry::protocol_id(request.pool)
        || route.target.pool_id != target_pool_id
        || route.sol_hop.pool_id != hop_pool_id
        || route.target.mint != normalized.mint.to_string()
        || route.target.quote_mint.as_deref() != Some(quote_mint.as_str())
    {
        bail!("via-SOL snapshot does not match trade pool or mints");
    }
    match request.side {
        Side::Buy if !route.target.supports_buy || !route.sol_hop.supports_buy => {
            bail!("via-SOL snapshot does not support buy")
        }
        Side::Sell if !route.target.supports_sell || !route.sol_hop.supports_sell => {
            bail!("via-SOL snapshot does not support sell")
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_route(route: &ResolvedRoute, now_ms: u64) -> Result<()> {
    if route.route_id.trim().is_empty() {
        bail!("route_id is empty");
    }
    if route.protocol_id.trim().is_empty() || route.execution_backend.trim().is_empty() {
        bail!("route protocol and execution backend are required");
    }
    if route.mint.parse::<solana_sdk::pubkey::Pubkey>().is_err() {
        bail!("route mint is invalid");
    }
    for (label, value) in [
        ("input_mint", route.input_mint.as_deref()),
        ("output_mint", route.output_mint.as_deref()),
        ("quote_mint", route.quote_mint.as_deref()),
    ] {
        if let Some(value) = value {
            if value.parse::<solana_sdk::pubkey::Pubkey>().is_err() {
                bail!("route {label} is invalid");
            }
        }
    }
    if let Some(output_mint) = route.output_mint.as_deref() {
        if output_mint != route.mint {
            bail!("route output mint does not match route mint");
        }
    }
    if let Some(pool_id) = route.pool_id.as_deref() {
        if pool_id.parse::<solana_sdk::pubkey::Pubkey>().is_err() {
            bail!("route pool_id is invalid");
        }
    }
    if let Some(program_id) = route.program_id.as_deref() {
        if program_id.parse::<solana_sdk::pubkey::Pubkey>().is_err() {
            bail!("route program_id is invalid");
        }
    }
    if route.token_program != "spl-token" && route.token_program != "token-2022" {
        bail!("unsupported token program");
    }
    if let Some(program) = route.quote_token_program.as_deref() {
        if program != "spl-token" && program != "token-2022" {
            bail!("unsupported quote token program");
        }
    }
    if route.expires_at_ms <= now_ms {
        bail!("route is expired");
    }
    if let Some(slot) = route.validated_slot {
        if slot == 0 {
            bail!("validated_slot must be positive");
        }
    }
    if !route.supports_buy && !route.supports_sell {
        bail!("route has no supported side");
    }
    Ok(())
}

/// Admission check for routes returned by an RPC-backed adapter. Keeping this
/// separate from `validate_route` lets old persisted route fixtures remain
/// readable while ensuring new quote/simulation requests cannot skip account
/// ownership and program checks.
pub fn validate_verified_route(route: &ResolvedRoute, now_ms: u64) -> Result<()> {
    validate_route(route, now_ms)?;
    if !route.chain_verified {
        bail!("route has not been verified against chain state");
    }
    if !route.pool_owner_verified {
        bail!("route pool owner is not verified");
    }
    if !route.mint_owner_verified {
        bail!("route mint owner is not verified");
    }
    if !route.quote_mint_owner_verified {
        bail!("route quote mint owner is not verified");
    }
    if route.program_id.is_none() {
        bail!("route program_id is required after chain verification");
    }
    if route.validated_slot.is_none() {
        bail!("route validated_slot is required after chain verification");
    }
    if route.quote_token_program.is_none() {
        bail!("route quote token program is required after chain verification");
    }
    Ok(())
}

pub fn validate_route_hint(
    route: &ResolvedRoute,
    expected_protocol: Option<&str>,
    expected_pool: Option<&str>,
    expected_program: Option<&str>,
    expected_quote_mint: Option<&str>,
    now_ms: u64,
) -> Result<()> {
    validate_route(route, now_ms)?;
    if let Some(protocol) = expected_protocol {
        if route.protocol_id != protocol {
            bail!("route protocol mismatch");
        }
    }
    if let Some(pool) = expected_pool {
        if route.pool_id.as_deref() != Some(pool) {
            bail!("route pool mismatch");
        }
    }
    if let Some(program) = expected_program {
        if route.program_id.as_deref() != Some(program) {
            bail!("route program mismatch");
        }
    }
    if let Some(quote) = expected_quote_mint {
        if route.quote_mint.as_deref() != Some(quote) {
            bail!("route quote mint mismatch");
        }
    }
    Ok(())
}

pub fn validate_quote(
    quote: &QuoteResult,
    route: &ResolvedRoute,
    wallet_id: &str,
    now_ms: u64,
) -> Result<()> {
    validate_route(route, now_ms)?;
    if quote.route_id != route.route_id {
        bail!("quote route mismatch");
    }
    if quote.wallet_id != wallet_id {
        bail!("quote wallet mismatch");
    }
    if quote.input_amount_units == 0 || quote.expected_output_units == 0 {
        bail!("quote amounts must be positive");
    }
    if quote.quote_id.trim().is_empty() {
        bail!("quote_id is empty");
    }
    if quote.created_at_ms >= quote.expires_at_ms {
        bail!("quote expiry must be after creation");
    }
    let expected_input = match quote.side.as_str() {
        "buy" => route.quote_mint.as_deref(),
        "sell" => route.output_mint.as_deref().or(Some(route.mint.as_str())),
        _ => None,
    };
    if expected_input != Some(quote.input_mint.as_str()) {
        bail!("quote input mint does not match route side");
    }
    let expected_output = match quote.side.as_str() {
        "buy" => route.output_mint.as_deref().or(Some(route.mint.as_str())),
        "sell" => route.quote_mint.as_deref(),
        _ => None,
    };
    if expected_output != Some(quote.output_mint.as_str()) {
        bail!("quote output mint does not match route side");
    }
    if quote.side != "buy" && quote.side != "sell" {
        bail!("quote side must be buy or sell");
    }
    if quote.min_output_units > quote.expected_output_units {
        bail!("quote minimum output exceeds expected output");
    }
    if quote.slippage_bps > 10_000 {
        bail!("quote slippage_bps exceeds 10000");
    }
    if quote.expires_at_ms <= now_ms {
        bail!("quote is expired");
    }
    if quote.expires_at_ms > route.expires_at_ms {
        bail!("quote expires after its verified route");
    }
    if quote.expires_at_ms > quote.created_at_ms.saturating_add(MAX_QUOTE_TTL_MS) {
        bail!("quote expiry exceeds maximum lifetime");
    }
    Ok(())
}

pub fn validate_trade_plan(plan: &TradePlan, wallet_id: &str, now_ms: u64) -> Result<()> {
    validate_quote(&plan.quote, &plan.route, wallet_id, now_ms)?;
    if plan.side != "buy" && plan.side != "sell" {
        bail!("side must be buy or sell");
    }
    if plan.quote.side != plan.side {
        bail!("quote side mismatch");
    }
    if plan.side == "buy" && !plan.route.supports_buy {
        bail!("route does not support buy");
    }
    if plan.side == "sell" && !plan.route.supports_sell {
        bail!("route does not support sell");
    }
    if plan.slippage_bps > 10_000 {
        bail!("slippage_bps exceeds 10000");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    fn route() -> ResolvedRoute {
        ResolvedRoute {
            route_id: "r1".into(),
            protocol_id: "pump".into(),
            program_id: None,
            pool_id: None,
            mint: "So11111111111111111111111111111111111111112".into(),
            input_mint: None,
            output_mint: Some("So11111111111111111111111111111111111111112".into()),
            quote_mint: Some("So11111111111111111111111111111111111111112".into()),
            token_program: "spl-token".into(),
            quote_token_program: Some("spl-token".into()),
            validated_slot: Some(1),
            expires_at_ms: 2_000,
            supports_buy: true,
            supports_sell: true,
            supports_sol_route: false,
            supports_token2022: false,
            execution_backend: "sdk-11".into(),
            chain_verified: false,
            pool_owner_verified: false,
            mint_owner_verified: false,
            quote_mint_owner_verified: false,
        }
    }
    fn quote() -> QuoteResult {
        QuoteResult {
            quote_id: "q1".into(),
            route_id: "r1".into(),
            wallet_id: "w1".into(),
            input_mint: "So11111111111111111111111111111111111111112".into(),
            output_mint: "So11111111111111111111111111111111111111112".into(),
            input_amount_units: 10,
            side: "buy".into(),
            expected_output_units: 9,
            min_output_units: 8,
            slippage_bps: 1_000,
            fee_units: 1,
            created_at_ms: 1_000,
            expires_at_ms: 2_000,
        }
    }
    #[test]
    fn rejects_stale_route() {
        assert!(validate_route(&route(), 2_000).is_err());
    }
    #[test]
    fn rejects_quote_wallet_mismatch() {
        assert!(validate_quote(&quote(), &route(), "w2", 1_100).is_err());
    }
    #[test]
    fn accepts_valid_plan() {
        let plan = TradePlan {
            route: route(),
            quote: quote(),
            side: "buy".into(),
            slippage_bps: 150,
        };
        assert!(validate_trade_plan(&plan, "w1", 1_100).is_ok());
    }

    #[test]
    fn verified_route_requires_chain_metadata() {
        let mut route = route();
        assert!(validate_verified_route(&route, 1_100).is_err());
        route.program_id = Some("11111111111111111111111111111111".into());
        route.chain_verified = true;
        route.pool_owner_verified = true;
        route.mint_owner_verified = true;
        route.quote_mint_owner_verified = true;
        assert!(validate_verified_route(&route, 1_100).is_ok());
    }

    #[test]
    fn rejects_route_with_mismatched_output_mint() {
        let mut route = route();
        route.output_mint = Some("11111111111111111111111111111111".into());
        assert!(validate_route(&route, 1_100).is_err());
    }

    #[test]
    fn rejects_quote_with_wrong_side_mints() {
        let mut quote = quote();
        quote.input_mint = "11111111111111111111111111111111".into();
        assert!(validate_quote(&quote, &route(), "w1", 1_100).is_err());
    }

    #[test]
    fn rejects_quote_with_invalid_lifetime_or_slippage() {
        let mut expired_quote = quote();
        expired_quote.created_at_ms = expired_quote.expires_at_ms;
        assert!(validate_quote(&expired_quote, &route(), "w1", 1_100).is_err());
        let mut excessive_slippage_quote = quote();
        excessive_slippage_quote.slippage_bps = 10_001;
        assert!(validate_quote(&excessive_slippage_quote, &route(), "w1", 1_100).is_err());
        let mut shorter_route = route();
        shorter_route.expires_at_ms = 1_900;
        assert!(validate_quote(&quote(), &shorter_route, "w1", 1_100).is_err());
    }

    #[tokio::test]
    async fn route_store_rejects_client_modified_snapshot() {
        let mut verified = route();
        verified.program_id = Some("11111111111111111111111111111111".into());
        verified.chain_verified = true;
        verified.pool_owner_verified = true;
        verified.mint_owner_verified = true;
        verified.quote_mint_owner_verified = true;
        let store = RouteStore::default();
        store.insert(verified.clone(), 1_100).await.unwrap();

        let mut modified = verified;
        modified.quote_mint = Some("11111111111111111111111111111111".into());
        assert!(store.admit(&modified, 1_100).await.is_err());
    }

    fn verified_via_sol_route() -> VerifiedViaSolRoute {
        let target_mint = Pubkey::new_unique().to_string();
        let target_quote = Pubkey::new_unique().to_string();
        let mut target = route();
        target.route_id = "target-route".into();
        target.protocol_id = "stonkfun".into();
        target.program_id = Some(Pubkey::new_unique().to_string());
        target.pool_id = Some(Pubkey::new_unique().to_string());
        target.mint = target_mint.clone();
        target.output_mint = Some(target_mint);
        target.input_mint = Some(target_quote.clone());
        target.quote_mint = Some(target_quote.clone());
        target.validated_slot = Some(100);
        target.chain_verified = true;
        target.pool_owner_verified = true;
        target.mint_owner_verified = true;
        target.quote_mint_owner_verified = true;

        let mut sol_hop = target.clone();
        sol_hop.route_id = "hop-route".into();
        sol_hop.protocol_id = "raydium-cpmm".into();
        sol_hop.pool_id = Some(Pubkey::new_unique().to_string());
        sol_hop.mint = target_quote.clone();
        sol_hop.output_mint = Some(target_quote);
        sol_hop.input_mint = Some(crate::models::WSOL_MINT.into());
        sol_hop.quote_mint = Some(crate::models::WSOL_MINT.into());
        sol_hop.validated_slot = Some(102);
        sol_hop.expires_at_ms = 1_900;
        VerifiedViaSolRoute {
            route_id: "via-sol-route".into(),
            target,
            sol_hop,
            expires_at_ms: 1_900,
        }
    }

    #[test]
    fn generic_wsol_quote_bridge_binds_both_pools_mints_programs_and_slots() {
        let mut route = verified_via_sol_route();
        route.target.protocol_id = "pump-amm".into();
        assert_eq!(
            validate_wsol_quote_cpmm_bridge(&route.target, &route.sol_hop, 1_100).unwrap(),
            1_900
        );

        let mut wrong_program = route.clone();
        wrong_program.sol_hop.token_program = "token-2022".into();
        assert!(validate_wsol_quote_cpmm_bridge(
            &wrong_program.target,
            &wrong_program.sol_hop,
            1_100
        )
        .is_err());

        let mut wrong_pool = route.clone();
        wrong_pool.sol_hop.pool_id = wrong_pool.target.pool_id.clone();
        assert!(
            validate_wsol_quote_cpmm_bridge(&wrong_pool.target, &wrong_pool.sol_hop, 1_100)
                .is_err()
        );

        let mut stale_hop = route;
        stale_hop.sol_hop.validated_slot = Some(140);
        assert!(
            validate_wsol_quote_cpmm_bridge(&stale_hop.target, &stale_hop.sol_hop, 1_100).is_err()
        );
    }

    #[test]
    fn expired_bridge_can_refresh_only_the_same_verified_route_identity() {
        let route = verified_via_sol_route();
        let original = ObservedSolBridge {
            observation_id: "observe:sol-bridge:old".into(),
            wallet_id: "wallet".into(),
            wallet_pubkey: Pubkey::new_unique().to_string(),
            request_fingerprint: "a".repeat(64),
            target: route.target,
            sol_hop: route.sol_hop,
            expires_at_ms: 1_900,
        };
        let mut refreshed = original.clone();
        refreshed.observation_id = "observe:sol-bridge:new".into();
        refreshed.target.route_id = "target-new".into();
        refreshed.sol_hop.route_id = "hop-new".into();
        refreshed.target.validated_slot = Some(105);
        refreshed.sol_hop.validated_slot = Some(107);
        refreshed.target.expires_at_ms = 2_900;
        refreshed.sol_hop.expires_at_ms = 2_900;
        refreshed.expires_at_ms = 2_900;
        assert!(validate_refreshed_sol_bridge(&original, &refreshed, 2_100).is_ok());

        let mut changed = refreshed.clone();
        changed.sol_hop.pool_id = Some(Pubkey::new_unique().to_string());
        assert!(validate_refreshed_sol_bridge(&original, &changed, 2_100).is_err());
        let mut regressed = refreshed.clone();
        regressed.target.validated_slot = Some(99);
        assert!(validate_refreshed_sol_bridge(&original, &regressed, 2_100).is_err());
        let mut changed_wallet = refreshed;
        changed_wallet.wallet_pubkey = Pubkey::new_unique().to_string();
        assert!(validate_refreshed_sol_bridge(&original, &changed_wallet, 2_100).is_err());
    }

    #[test]
    fn pump_curve_bridge_uses_verified_program_identity_without_pool_id() {
        let mut route = verified_via_sol_route();
        route.target.protocol_id = "pump".into();
        route.target.pool_id = None;
        route.target.program_id =
            Some(sol_trade_sdk::instruction::utils::pumpfun::accounts::PUMPFUN.to_string());
        assert_eq!(
            validate_wsol_quote_cpmm_bridge(&route.target, &route.sol_hop, 1_100).unwrap(),
            1_900
        );

        route.target.program_id = Some(Pubkey::new_unique().to_string());
        assert!(validate_wsol_quote_cpmm_bridge(&route.target, &route.sol_hop, 1_100).is_err());
        route.target.program_id =
            Some(sol_trade_sdk::instruction::utils::pumpfun::accounts::PUMPFUN.to_string());
        route.sol_hop.pool_id = None;
        assert!(validate_wsol_quote_cpmm_bridge(&route.target, &route.sol_hop, 1_100).is_err());
    }

    #[tokio::test]
    async fn observed_bridge_is_short_lived_bound_and_never_an_sdk_decision() {
        let mut route = verified_via_sol_route();
        route.target.protocol_id = "pump-amm".into();
        let observation = ObservedSolBridge {
            observation_id: "observe:sol-bridge:test".into(),
            wallet_id: "wallet-1".into(),
            wallet_pubkey: Pubkey::new_unique().to_string(),
            request_fingerprint: "a".repeat(64),
            target: route.target,
            sol_hop: route.sol_hop,
            expires_at_ms: route.expires_at_ms,
        };
        let store = RouteStore::default();
        store
            .insert_observed_sol_bridge(observation.clone(), 1_100)
            .await
            .unwrap();
        store
            .admit_observed_sol_bridge(&observation, 1_100)
            .await
            .unwrap();
        assert_eq!(
            store
                .get_observed_sol_bridge(&observation.observation_id, 1_100)
                .await
                .unwrap(),
            observation
        );
        assert!(store
            .get_sdk_decision(&observation.observation_id, 1_100)
            .await
            .is_err());

        let mut tampered = observation.clone();
        tampered.wallet_pubkey = Pubkey::new_unique().to_string();
        assert!(store
            .admit_observed_sol_bridge(&tampered, 1_100)
            .await
            .is_err());
        tampered = observation.clone();
        tampered.sol_hop.pool_id = Some(Pubkey::new_unique().to_string());
        assert!(store
            .admit_observed_sol_bridge(&tampered, 1_100)
            .await
            .is_err());
        assert!(store
            .get_observed_sol_bridge(&observation.observation_id, 1_900)
            .await
            .is_err());
        store.purge_expired(1_900).await;
        assert!(store
            .get_observed_sol_bridge(&observation.observation_id, 1_100)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn successful_sdk_preview_is_bound_short_lived_and_cannot_authorize_trade() {
        let mut route = verified_via_sol_route();
        route.target.protocol_id = "pump-amm".into();
        let observation = ObservedSolBridge {
            observation_id: "observe:sol-bridge:preview-test".into(),
            wallet_id: "wallet-1".into(),
            wallet_pubkey: Pubkey::new_unique().to_string(),
            request_fingerprint: "b".repeat(64),
            target: route.target,
            sol_hop: route.sol_hop,
            expires_at_ms: route.expires_at_ms,
        };
        let store = RouteStore::default();
        store
            .insert_observed_sol_bridge(observation.clone(), 1_100)
            .await
            .unwrap();
        let preview = ObservedSdkPreviewSnapshot {
            preview_id: "observe:sdk-preview:test".into(),
            observation_id: observation.observation_id.clone(),
            wallet_id: observation.wallet_id.clone(),
            wallet_pubkey: observation.wallet_pubkey.clone(),
            request_fingerprint: observation.request_fingerprint.clone(),
            input_lamports: 1_000_000,
            bridge_min_output_units: 400,
            target_min_output_units: 10_000,
            target_token_delta_units: 10_100,
            transaction_bytes: 1_387,
            transaction_version: "v1",
            validated_slot: 100,
            simulation_slot: 101,
            expires_at_ms: 1_500,
        };
        store
            .insert_observed_sdk_preview(preview.clone(), 1_100)
            .await
            .unwrap();
        assert_eq!(
            store
                .get_observed_sdk_preview(&preview.preview_id, 1_499)
                .await
                .unwrap(),
            preview
        );
        assert!(store
            .get_sdk_decision(&preview.preview_id, 1_100)
            .await
            .is_err());
        assert!(store
            .get_observed_sdk_preview(&preview.preview_id, 1_500)
            .await
            .is_err());
        assert!(RouteStore::default()
            .get_observed_sdk_preview(&preview.preview_id, 1_100)
            .await
            .is_err());

        let mut bad = preview.clone();
        bad.preview_id = "observe:sdk-preview:wrong-wallet".into();
        bad.wallet_pubkey = Pubkey::new_unique().to_string();
        assert!(store.insert_observed_sdk_preview(bad, 1_100).await.is_err());
        let mut bad = preview.clone();
        bad.preview_id = "observe:sdk-preview:short-output".into();
        bad.target_token_delta_units = 9_999;
        assert!(store.insert_observed_sdk_preview(bad, 1_100).await.is_err());
        let mut bad = preview;
        bad.preview_id = "observe:sdk-preview:too-long".into();
        bad.expires_at_ms = 1_901;
        assert!(store.insert_observed_sdk_preview(bad, 1_100).await.is_err());
    }

    #[tokio::test]
    async fn omitted_quote_preview_uses_observed_target_and_keeps_raw_request_bound() {
        let mut route = verified_via_sol_route();
        route.target.protocol_id = "pump-amm".into();
        let wallet_pubkey = Pubkey::new_unique().to_string();
        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id": "wallet-1",
            "side": "buy",
            "pool": "pump-amm",
            "mint": route.target.mint,
            "pool_id": route.target.pool_id,
            "amount_lamports": 1_000_000,
            "slippage_bps": 100,
            "simulate": true,
            "sdk_preview_id": "observe:sdk-preview:omitted-quote",
            "idempotency_key": "omitted-quote-buy"
        }))
        .unwrap();
        let mut original = request.clone();
        original.sdk_preview_id = None;
        let fingerprint = sol_bridge_request_fingerprint(&original, &wallet_pubkey).unwrap();
        let observation = ObservedSolBridge {
            observation_id: "observe:sol-bridge:omitted-quote".into(),
            wallet_id: "wallet-1".into(),
            wallet_pubkey: wallet_pubkey.clone(),
            request_fingerprint: fingerprint.clone(),
            target: route.target,
            sol_hop: route.sol_hop,
            expires_at_ms: route.expires_at_ms,
        };
        let store = RouteStore::default();
        store
            .insert_observed_sol_bridge(observation.clone(), 1_100)
            .await
            .unwrap();
        store
            .insert_observed_sdk_preview(
                ObservedSdkPreviewSnapshot {
                    preview_id: request.sdk_preview_id.clone().unwrap(),
                    observation_id: observation.observation_id.clone(),
                    wallet_id: observation.wallet_id.clone(),
                    wallet_pubkey: wallet_pubkey.clone(),
                    request_fingerprint: fingerprint,
                    input_lamports: 1_000_000,
                    bridge_min_output_units: 400,
                    target_min_output_units: 10_000,
                    target_token_delta_units: 10_100,
                    transaction_bytes: 1_387,
                    transaction_version: "v1",
                    validated_slot: 100,
                    simulation_slot: 101,
                    expires_at_ms: 1_500,
                },
                1_100,
            )
            .await
            .unwrap();
        let normalized = NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: crate::models::WSOL_MINT.parse().unwrap(),
            pool_id: request.pool_id.as_deref().unwrap().parse().ok(),
            settlement_asset: SettlementAsset::Quote,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: Some(1_000_000),
            sell_percent: None,
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(true),
            aggregator_sol: false,
        };
        assert!(validate_sdk_preview_trade(
            &store,
            &request,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100,
        )
        .await
        .is_ok());
        let mut decision_trade = request.clone();
        decision_trade.sdk_preview_id = None;
        decision_trade.sdk_decision_id = Some("sdk-decision:bridge:test".into());
        let decision = SdkTradeDecisionSnapshot {
            decision_id: decision_trade.sdk_decision_id.clone().unwrap(),
            route_id: observation.observation_id.clone(),
            via_sol: false,
            bridge_preview_id: request.sdk_preview_id.clone(),
            cpmm_sell_evidence: None,
            cpmm_buy_evidence: None,
            wallet_id: observation.wallet_id.clone(),
            wallet_pubkey: wallet_pubkey.clone(),
            idempotency_key: request.idempotency_key.clone(),
            request_fingerprint: observation.request_fingerprint.clone(),
            expires_at_ms: 1_500,
        };
        store
            .insert_sdk_decision(decision.clone(), 1_100)
            .await
            .unwrap();
        assert!(validate_sdk_decision_trade(
            &store,
            &decision_trade,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100,
        )
        .await
        .is_ok());
        let mut changed_trade = decision_trade.clone();
        changed_trade.amount_lamports = Some(1_000_001);
        assert!(validate_sdk_decision_trade(
            &store,
            &changed_trade,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100,
        )
        .await
        .is_err());
        assert!(validate_sdk_decision_trade(
            &store,
            &decision_trade,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_500,
        )
        .await
        .is_err());
        let mut fake_decision = decision;
        fake_decision.decision_id = "sdk-decision:bridge:missing-preview".into();
        fake_decision.bridge_preview_id = Some("observe:sdk-preview:missing".into());
        assert!(store
            .insert_sdk_decision(fake_decision, 1_100)
            .await
            .is_err());
        request.quote_mint = observation.target.quote_mint.clone();
        assert!(validate_sdk_preview_trade(
            &store,
            &request,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100,
        )
        .await
        .is_err());
        request.quote_mint = None;
        request.amount_lamports = Some(1_000_001);
        assert!(validate_sdk_preview_trade(
            &store,
            &request,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn amm_v4_bridge_decision_cannot_be_reused_as_pumpswap() {
        let mut route = verified_via_sol_route();
        route.target.protocol_id = "raydium-amm-v4".into();
        let wallet_pubkey = Pubkey::new_unique().to_string();
        let request: TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id": "wallet-1", "side": "buy", "pool": "raydium-amm-v4",
            "mint": route.target.mint, "pool_id": route.target.pool_id,
            "quote_mint": route.target.quote_mint,
            "amount_lamports": 1_000_000, "slippage_bps": 100,
            "simulate": true, "sdk_preview_id": "observe:sdk-preview:amm-v4",
            "idempotency_key": "amm-v4-buy"
        }))
        .unwrap();
        let mut original = request.clone();
        original.sdk_preview_id = None;
        let fingerprint = sol_bridge_request_fingerprint(&original, &wallet_pubkey).unwrap();
        let observation = ObservedSolBridge {
            observation_id: "observe:sol-bridge:amm-v4".into(),
            wallet_id: "wallet-1".into(),
            wallet_pubkey: wallet_pubkey.clone(),
            request_fingerprint: fingerprint.clone(),
            target: route.target,
            sol_hop: route.sol_hop,
            expires_at_ms: route.expires_at_ms,
        };
        let store = RouteStore::default();
        store
            .insert_observed_sol_bridge(observation.clone(), 1_100)
            .await
            .unwrap();
        store
            .insert_observed_sdk_preview(
                ObservedSdkPreviewSnapshot {
                    preview_id: request.sdk_preview_id.clone().unwrap(),
                    observation_id: observation.observation_id.clone(),
                    wallet_id: observation.wallet_id.clone(),
                    wallet_pubkey: wallet_pubkey.clone(),
                    request_fingerprint: fingerprint,
                    input_lamports: 1_000_000,
                    bridge_min_output_units: 400,
                    target_min_output_units: 10_000,
                    target_token_delta_units: 10_100,
                    transaction_bytes: 926,
                    transaction_version: "legacy",
                    validated_slot: 100,
                    simulation_slot: 101,
                    expires_at_ms: 1_500,
                },
                1_100,
            )
            .await
            .unwrap();
        let normalized = NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: request.quote_mint.as_deref().unwrap().parse().unwrap(),
            pool_id: request.pool_id.as_deref().unwrap().parse().ok(),
            settlement_asset: SettlementAsset::Quote,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: Some(1_000_000),
            sell_percent: None,
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(true),
            aggregator_sol: false,
        };
        assert!(validate_sdk_preview_trade(
            &store,
            &request,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100
        )
        .await
        .is_ok());
        let mut wrong_pool = request.clone();
        wrong_pool.pool = crate::models::Pool::PumpAmm;
        assert!(validate_sdk_preview_trade(
            &store,
            &wrong_pool,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100
        )
        .await
        .is_err());
        let mut wrong_amount = request.clone();
        wrong_amount.amount_lamports = Some(1_000_001);
        assert!(validate_sdk_preview_trade(
            &store,
            &wrong_amount,
            &normalized,
            "wallet-1",
            &wallet_pubkey,
            1_100
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn via_sol_store_binds_both_legs_and_rejects_stale_or_tampered_snapshots() {
        let route = verified_via_sol_route();
        let store = RouteStore::default();
        store.insert_via_sol(route.clone(), 1_100).await.unwrap();
        store.admit_via_sol(&route, 1_100).await.unwrap();
        assert_eq!(
            store.get_via_sol(&route.route_id, 1_100).await.unwrap(),
            route
        );

        let mut changed_hop = route.clone();
        changed_hop.sol_hop.pool_id = Some(Pubkey::new_unique().to_string());
        assert!(store.admit_via_sol(&changed_hop, 1_100).await.is_err());

        let mut disconnected = route.clone();
        disconnected.sol_hop.mint = Pubkey::new_unique().to_string();
        disconnected.sol_hop.output_mint = Some(disconnected.sol_hop.mint.clone());
        assert!(store.insert_via_sol(disconnected, 1_100).await.is_err());

        let mut distant_slot = route.clone();
        distant_slot.sol_hop.validated_slot = Some(140);
        assert!(store.insert_via_sol(distant_slot, 1_100).await.is_err());

        let mut changed_token_program = route.clone();
        changed_token_program.sol_hop.token_program = "token-2022".into();
        assert!(store
            .insert_via_sol(changed_token_program, 1_100)
            .await
            .is_err());
        assert!(store.admit_via_sol(&route, 1_900).await.is_err());
        assert!(store.get_via_sol(&route.route_id, 1_900).await.is_err());
    }

    #[test]
    fn via_sol_trade_match_binds_target_hop_side_and_snapshot_id() {
        let route = verified_via_sol_route();
        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "side": "buy",
            "pool": "stonkfun",
            "mint": route.target.mint,
            "pool_id": route.target.pool_id,
            "quote_mint": route.target.quote_mint,
            "settlement_asset": "sol",
            "sol_hop_pool": "raydium-cpmm",
            "sol_hop_pool_id": route.sol_hop.pool_id,
            "sdk_route_id": route.route_id,
            "amount_lamports": 1_000_000,
            "execution_method": "sdk",
            "idempotency_key": "via-sol-test"
        }))
        .unwrap();
        let mut normalized = NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: request.quote_mint.as_deref().unwrap().parse().unwrap(),
            pool_id: request
                .pool_id
                .as_deref()
                .map(str::parse)
                .transpose()
                .unwrap(),
            settlement_asset: SettlementAsset::Sol,
            sol_hop_pool: Some(crate::models::Pool::RaydiumCpmm),
            sol_hop_pool_id: request
                .sol_hop_pool_id
                .as_deref()
                .map(str::parse)
                .transpose()
                .unwrap(),
            amount_lamports: Some(1_000_000),
            sell_percent: None,
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(true),
            aggregator_sol: false,
        };
        assert!(validate_via_sol_trade_match(&route, &request, &normalized, 1_100).is_ok());

        request.sdk_route_id = Some("other-route".into());
        assert!(validate_via_sol_trade_match(&route, &request, &normalized, 1_100).is_err());
        request.sdk_route_id = Some(route.route_id.clone());

        normalized.sol_hop_pool_id = Some(Pubkey::new_unique());
        assert!(validate_via_sol_trade_match(&route, &request, &normalized, 1_100).is_err());
        normalized.sol_hop_pool_id = request
            .sol_hop_pool_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .unwrap();

        request.side = Side::Sell;
        let mut sell_disabled = route.clone();
        sell_disabled.target.supports_sell = false;
        assert!(
            validate_via_sol_trade_match(&sell_disabled, &request, &normalized, 1_100).is_err()
        );
        assert!(validate_via_sol_trade_match(&route, &request, &normalized, 1_900).is_err());
    }

    #[tokio::test]
    async fn sdk_decision_binds_wallet_request_idempotency_and_route_until_expiry() {
        let mint = Pubkey::new_unique().to_string();
        let quote_mint = crate::models::WSOL_MINT.to_string();
        let mut verified = route();
        verified.program_id = Some(Pubkey::new_unique().to_string());
        verified.mint = mint.clone();
        verified.input_mint = Some(quote_mint.clone());
        verified.output_mint = Some(mint.clone());
        verified.quote_mint = Some(quote_mint.clone());
        verified.chain_verified = true;
        verified.pool_owner_verified = true;
        verified.mint_owner_verified = true;
        verified.quote_mint_owner_verified = true;
        let store = RouteStore::default();
        store.insert(verified.clone(), 1_100).await.unwrap();

        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id": "w1",
            "side": "buy",
            "pool": "pump",
            "mint": mint,
            "quote_mint": quote_mint,
            "amount_lamports": 1_000_000,
            "slippage_bps": 100,
            "execution_method": "sdk",
            "simulate": true,
            "idempotency_key": "decision-order-1",
            "sdk_decision_id": "decision-1"
        }))
        .unwrap();
        let mut normalized = NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: request.quote_mint.as_deref().unwrap().parse().unwrap(),
            pool_id: None,
            settlement_asset: SettlementAsset::Quote,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: Some(1_000_000),
            sell_percent: None,
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(true),
            aggregator_sol: false,
        };
        let fingerprint = crate::persistence::request_hash("w1", &request, &normalized).unwrap();
        store
            .insert_sdk_decision(
                SdkTradeDecisionSnapshot {
                    decision_id: "decision-1".into(),
                    route_id: verified.route_id.clone(),
                    via_sol: false,
                    bridge_preview_id: None,
                    cpmm_sell_evidence: None,
                    cpmm_buy_evidence: None,
                    wallet_id: "w1".into(),
                    wallet_pubkey: "pubkey-1".into(),
                    idempotency_key: request.idempotency_key.clone(),
                    request_fingerprint: fingerprint,
                    expires_at_ms: 2_000,
                },
                1_100,
            )
            .await
            .unwrap();
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_ok());
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w2",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-2",
            1_100
        )
        .await
        .is_err());
        request.idempotency_key = "decision-order-2".into();
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
        request.idempotency_key = "decision-order-1".into();
        normalized.amount_lamports = Some(2_000_000);
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
        normalized.amount_lamports = Some(1_000_000);
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            2_000
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn mint_only_pumpswap_decision_pins_the_server_discovered_pool() {
        let mint = Pubkey::new_unique().to_string();
        let pool_id = Pubkey::new_unique();
        let quote_mint = crate::models::WSOL_MINT.to_string();
        let mut verified = route();
        verified.protocol_id = "pump-amm".into();
        verified.program_id = Some(Pubkey::new_unique().to_string());
        verified.pool_id = Some(pool_id.to_string());
        verified.mint = mint.clone();
        verified.input_mint = Some(quote_mint.clone());
        verified.output_mint = Some(mint.clone());
        verified.quote_mint = Some(quote_mint.clone());
        verified.chain_verified = true;
        verified.pool_owner_verified = true;
        verified.mint_owner_verified = true;
        verified.quote_mint_owner_verified = true;
        let store = RouteStore::default();
        store.insert(verified.clone(), 1_100).await.unwrap();
        let request: TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id": "w1", "side": "buy", "pool": "pump-amm", "mint": mint,
            "quote_mint": quote_mint, "amount_lamports": 1_000_000,
            "slippage_bps": 100, "execution_method": "sdk", "simulate": true,
            "idempotency_key": "mint-only-pumpswap", "sdk_decision_id": "decision-pool-1"
        }))
        .unwrap();
        let mut normalized = NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: request.quote_mint.as_deref().unwrap().parse().unwrap(),
            pool_id: None,
            settlement_asset: SettlementAsset::Quote,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: Some(1_000_000),
            sell_percent: None,
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(true),
            aggregator_sol: false,
        };
        let fingerprint = crate::persistence::request_hash("w1", &request, &normalized).unwrap();
        store
            .insert_sdk_decision(
                SdkTradeDecisionSnapshot {
                    decision_id: "decision-pool-1".into(),
                    route_id: verified.route_id,
                    via_sol: false,
                    bridge_preview_id: None,
                    cpmm_sell_evidence: None,
                    cpmm_buy_evidence: None,
                    wallet_id: "w1".into(),
                    wallet_pubkey: "pubkey-1".into(),
                    idempotency_key: request.idempotency_key.clone(),
                    request_fingerprint: fingerprint,
                    expires_at_ms: 2_000,
                },
                1_100,
            )
            .await
            .unwrap();
        assert_eq!(
            validate_sdk_decision_trade(&store, &request, &normalized, "w1", "pubkey-1", 1_100)
                .await
                .unwrap(),
            Some(pool_id)
        );
        normalized.pool_id = Some(Pubkey::new_unique());
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn sdk_decision_keeps_via_sol_hops_as_one_bound_snapshot() {
        let route = verified_via_sol_route();
        let store = RouteStore::default();
        store.insert_via_sol(route.clone(), 1_100).await.unwrap();
        let mut request: TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id": "w1",
            "side": "buy",
            "pool": "stonkfun",
            "mint": route.target.mint,
            "pool_id": route.target.pool_id,
            "quote_mint": route.target.quote_mint,
            "settlement_asset": "sol",
            "sol_hop_pool": "raydium-cpmm",
            "sol_hop_pool_id": route.sol_hop.pool_id,
            "sdk_route_id": route.route_id,
            "sdk_decision_id": "via-decision-1",
            "amount_lamports": 1_000_000,
            "slippage_bps": 100,
            "execution_method": "sdk",
            "simulate": true,
            "idempotency_key": "via-decision-order"
        }))
        .unwrap();
        let mut normalized = NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: request.quote_mint.as_deref().unwrap().parse().unwrap(),
            pool_id: request.pool_id.as_deref().unwrap().parse().ok(),
            settlement_asset: SettlementAsset::Sol,
            sol_hop_pool: Some(crate::models::Pool::RaydiumCpmm),
            sol_hop_pool_id: request.sol_hop_pool_id.as_deref().unwrap().parse().ok(),
            amount_lamports: Some(1_000_000),
            sell_percent: None,
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(true),
            aggregator_sol: false,
        };
        store
            .insert_sdk_decision(
                SdkTradeDecisionSnapshot {
                    decision_id: "via-decision-1".into(),
                    route_id: route.route_id.clone(),
                    via_sol: true,
                    bridge_preview_id: None,
                    cpmm_sell_evidence: None,
                    cpmm_buy_evidence: None,
                    wallet_id: "w1".into(),
                    wallet_pubkey: "pubkey-1".into(),
                    idempotency_key: request.idempotency_key.clone(),
                    request_fingerprint: crate::persistence::request_hash(
                        "w1",
                        &request,
                        &normalized,
                    )
                    .unwrap(),
                    expires_at_ms: route.expires_at_ms,
                },
                1_100,
            )
            .await
            .unwrap();
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_ok());
        normalized.sol_hop_pool_id = Some(Pubkey::new_unique());
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
        normalized.sol_hop_pool_id = request.sol_hop_pool_id.as_deref().unwrap().parse().ok();
        request.sdk_route_id = Some("other-route".into());
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn cpmm_buy_decision_binds_direction_input_and_both_pools() {
        let mut route = verified_via_sol_route();
        route.target.protocol_id = "raydium-cpmm".into();
        let store = RouteStore::default();
        store.insert_via_sol(route.clone(), 1_100).await.unwrap();
        let request: TradeRequest = serde_json::from_value(serde_json::json!({
            "wallet_id": "w1", "side": "buy", "pool": "raydium-cpmm",
            "mint": route.target.mint, "pool_id": route.target.pool_id,
            "quote_mint": route.target.quote_mint,
            "settlement_asset": "sol", "sol_hop_pool": "raydium-cpmm",
            "sol_hop_pool_id": route.sol_hop.pool_id,
            "sdk_decision_id": "sdk-decision:cpmm-buy-test",
            "amount_lamports": 1_000_000, "slippage_bps": 100,
            "execution_method": "sdk", "simulate": false,
            "idempotency_key": "cpmm-buy-decision"
        }))
        .unwrap();
        let mut normalized = NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: request.quote_mint.as_deref().unwrap().parse().unwrap(),
            pool_id: request.pool_id.as_deref().unwrap().parse().ok(),
            settlement_asset: SettlementAsset::Sol,
            sol_hop_pool: Some(crate::models::Pool::RaydiumCpmm),
            sol_hop_pool_id: request.sol_hop_pool_id.as_deref().unwrap().parse().ok(),
            amount_lamports: Some(1_000_000),
            sell_percent: None,
            slippage_bps: 100,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: None,
            mev_provider: None,
            simulate: Some(false),
            aggregator_sol: false,
        };
        let decision = SdkTradeDecisionSnapshot {
            decision_id: request.sdk_decision_id.clone().unwrap(),
            route_id: route.route_id,
            via_sol: true,
            bridge_preview_id: None,
            cpmm_sell_evidence: None,
            cpmm_buy_evidence: Some(SdkCpmmBuyEvidence {
                input_lamports: 1_000_000,
                minimum_target_units: 100,
                validated_slot: 100,
                simulation_slot: 101,
            }),
            wallet_id: "w1".into(),
            wallet_pubkey: "pubkey-1".into(),
            idempotency_key: request.idempotency_key.clone(),
            request_fingerprint: crate::persistence::request_hash("w1", &request, &normalized)
                .unwrap(),
            expires_at_ms: 1_500,
        };
        store
            .insert_sdk_decision(decision.clone(), 1_100)
            .await
            .unwrap();
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_ok());
        normalized.amount_lamports = Some(2_000_000);
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
        normalized.amount_lamports = Some(1_000_000);
        normalized.sol_hop_pool_id = Some(Pubkey::new_unique());
        assert!(validate_sdk_decision_trade(
            &store,
            &request,
            &normalized,
            "w1",
            "pubkey-1",
            1_100
        )
        .await
        .is_err());
        let mut both_directions = decision;
        both_directions.decision_id = "sdk-decision:both-directions".into();
        both_directions.cpmm_sell_evidence = Some(SdkCpmmSellEvidence {
            input_token_units: 1,
            minimum_wsol_units: 1,
            validated_slot: 100,
            simulation_slot: 101,
        });
        assert!(store
            .insert_sdk_decision(both_directions, 1_100)
            .await
            .is_err());
    }
}
