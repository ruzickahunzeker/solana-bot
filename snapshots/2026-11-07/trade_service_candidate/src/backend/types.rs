//! SDK-neutral request and result types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteHint {
    pub protocol_id: Option<String>,
    pub program_id: Option<String>,
    pub pool_id: Option<String>,
    pub mint: String,
    pub quote_mint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViaSolRouteRequest {
    pub target: RouteHint,
    pub sol_hop_pool_id: String,
}

/// A server-issued, short-lived SDK route for SOL -> quote -> target buys.
/// Both legs must be admitted together; neither leg is proof by itself that
/// the combined route can be built or executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedViaSolRoute {
    pub route_id: String,
    pub target: ResolvedRoute,
    pub sol_hop: ResolvedRoute,
    pub expires_at_ms: u64,
}

/// Read-only two-leg evidence for a SOL-funded non-WSOL buy. This is not an
/// executable route or an SDK decision; no quote or minimum output is attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedSolBridge {
    pub observation_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub request_fingerprint: String,
    pub target: ResolvedRoute,
    pub sol_hop: ResolvedRoute,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRoute {
    pub route_id: String,
    pub protocol_id: String,
    pub program_id: Option<String>,
    pub pool_id: Option<String>,
    pub mint: String,
    pub input_mint: Option<String>,
    pub output_mint: Option<String>,
    pub quote_mint: Option<String>,
    pub token_program: String,
    #[serde(default)]
    pub quote_token_program: Option<String>,
    pub validated_slot: Option<u64>,
    pub expires_at_ms: u64,
    pub supports_buy: bool,
    pub supports_sell: bool,
    pub supports_sol_route: bool,
    pub supports_token2022: bool,
    pub execution_backend: String,
    /// Set only after the backend has loaded and checked the pool and mint
    /// accounts from RPC. Callers must not manufacture this flag.
    #[serde(default)]
    pub chain_verified: bool,
    #[serde(default)]
    pub pool_owner_verified: bool,
    #[serde(default)]
    pub mint_owner_verified: bool,
    #[serde(default)]
    pub quote_mint_owner_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRequest {
    pub route: ResolvedRoute,
    pub wallet_id: String,
    pub input_amount_units: u64,
    pub side: String,
    #[serde(default)]
    pub slippage_bps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteResult {
    pub quote_id: String,
    pub route_id: String,
    pub wallet_id: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_units: u64,
    pub side: String,
    pub expected_output_units: u64,
    pub min_output_units: u64,
    pub slippage_bps: u64,
    pub fee_units: u64,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradePlan {
    pub route: ResolvedRoute,
    pub quote: QuoteResult,
    pub side: String,
    pub slippage_bps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub ok: bool,
    pub units_consumed: Option<u64>,
    pub logs: Vec<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub route_id: Option<String>,
    #[serde(default)]
    pub expected_output_units: Option<u64>,
    #[serde(default)]
    pub min_output_units: Option<u64>,
    #[serde(default)]
    pub fee_units: Option<u64>,
    #[serde(default)]
    pub simulation_slot: Option<u64>,
    /// Always null for simulation. Internal SDK transaction fingerprints are
    /// deliberately not exposed as broadcast signatures.
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionResult {
    pub request_id: String,
    pub status: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub protocols: Vec<String>,
    pub supports_buy: bool,
    pub supports_sell: bool,
    pub supports_simulate: bool,
    pub supports_live: bool,
}
