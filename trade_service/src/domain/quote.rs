//! SDK-independent quote values. Serde shapes are frozen journal-v1 snapshots, not HTTP ownership.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QuoteRouteLeg {
    pub label: String,
    pub input_mint: String,
    pub output_mint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QuoteEstimate {
    pub input_mint: String,
    pub output_mint: String,
    pub amount_lamports: u64,
    pub expected_output_units: u64,
    pub minimum_output_units: u64,
    pub price_impact_percent: String,
    pub route: Vec<QuoteRouteLeg>,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QuotePreflight {
    pub quote_id: String,
    pub wallet_id: String,
    pub quote_still_valid: bool,
    pub unsigned_transaction_built: bool,
    pub aggregator_simulation_ok: bool,
    pub local_simulation_ok: bool,
    pub instruction_count: usize,
    pub address_lookup_table_count: usize,
    pub resolved_account_count: usize,
    pub instruction_audit_passed: bool,
    pub executable: bool,
    pub reason: String,
    pub prioritization_fee_lamports: u64,
    pub compute_unit_limit: u64,
    pub calculated_priority_fee_lamports: u64,
    pub explicit_system_spend_lamports: u64,
    pub writable_account_count: usize,
    pub target_ata: String,
    pub top_level_programs: Vec<String>,
    pub simulated_fee_lamports: Option<u64>,
    pub simulated_units_consumed: Option<u64>,
    pub simulated_payer_debit_lamports: Option<u64>,
    pub simulated_inner_instruction_groups: Option<usize>,
    pub simulated_target_token_delta_units: Option<u64>,
    pub simulated_target_token_post_units: Option<u64>,
    pub baseline_slot: u64,
    pub simulation_slot: u64,
    pub observed_same_slot_payer_debit_lamports: Option<u64>,
    pub observed_same_slot_target_token_delta_units: Option<u64>,
    pub simulated_inner_instruction_count: Option<usize>,
    pub simulated_inner_programs: Option<Vec<String>>,
    pub simulated_inner_payer_system_spend_lamports: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BoundQuote {
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub quote_slippage_bps: u16,
    pub expires_at_ms: u64,
    #[serde(flatten)]
    pub quote: QuoteEstimate,
}
