//! Provider-neutral aggregator contracts.
//!
//! Provider-specific response formats must stay inside their adapter modules.
//! These types are internal to Trade Service; they are not a permission for a
//! client to select a provider or submit an arbitrary transaction.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletBalanceSnapshot {
    pub balance_lamports: u64,
    pub fetched_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Jupiter,
    Dflow,
}

impl ProviderId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jupiter => "jupiter",
            Self::Dflow => "dflow",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatorCapabilities {
    pub supports_quote: bool,
    pub supports_build: bool,
    pub supports_preflight: bool,
    pub supports_submit: bool,
    pub read_only: bool,
}

impl AggregatorCapabilities {
    pub const fn read_only_quote() -> Self {
        Self {
            supports_quote: true,
            supports_build: false,
            supports_preflight: false,
            supports_submit: false,
            read_only: true,
        }
    }

    pub const fn disabled_read_only() -> Self {
        Self {
            supports_quote: false,
            supports_build: false,
            supports_preflight: false,
            supports_submit: false,
            read_only: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorQuoteRequest {
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_units: u64,
    pub slippage_bps: u16,
    pub balance_snapshot: Option<WalletBalanceSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorRouteLeg {
    pub label: String,
    pub input_mint: String,
    pub output_mint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorQuote {
    pub provider: ProviderId,
    pub provider_quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_units: u64,
    pub expected_output_units: u64,
    pub minimum_output_units: u64,
    pub slippage_bps: u16,
    pub route_fingerprint: String,
    pub request_hash: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub price_impact_percent: Option<String>,
    pub route: Vec<AggregatorRouteLeg>,
    pub balance_snapshot: Option<WalletBalanceSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsignedTransaction {
    /// Opaque internal bytes. Adapters must never expose this through a public
    /// bot API or persist private key material alongside it.
    #[serde(skip_serializing)]
    pub bytes: Vec<u8>,
    pub message_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightReport {
    pub provider: ProviderId,
    pub ok: bool,
    pub instruction_audit_passed: bool,
    pub executable: bool,
    pub reason: String,
}
