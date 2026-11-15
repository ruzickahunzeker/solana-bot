use crate::{
    application::ports::sol_buy_quotes::SolBuyQuoteInput,
    application::quote_submission_types::{
        QuoteSubmissionCommand, QuoteSubmissionOutcome, QuoteSubmissionResult,
    },
    domain::quote::{BoundQuote, QuoteEstimate, QuotePreflight, QuoteRouteLeg},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct SolBuyQuoteRequest {
    pub wallet_id: String,
    pub mint: String,
    pub amount_lamports: u64,
    pub slippage_bps: u16,
}

impl SolBuyQuoteRequest {
    pub(crate) fn query_input(&self) -> SolBuyQuoteInput<'_> {
        SolBuyQuoteInput {
            wallet_id: &self.wallet_id,
            mint: &self.mint,
            amount_lamports: self.amount_lamports,
            slippage_bps: self.slippage_bps,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SolBuyQuoteResponse {
    pub input_mint: String,
    pub output_mint: String,
    pub amount_lamports: u64,
    pub expected_output_units: u64,
    pub minimum_output_units: u64,
    pub price_impact_percent: String,
    pub route: Vec<QuoteLeg>,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuoteLeg {
    pub label: String,
    pub input_mint: String,
    pub output_mint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolBuyPreflightResponse {
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

#[derive(Debug, Clone, Deserialize)]
pub struct SolBuySubmitRequest {
    pub wallet_id: String,
    pub confirmed: bool,
    pub simulate: bool,
    pub idempotency_key: String,
}

#[derive(Deserialize)]
pub(crate) struct QuoteWalletQuery {
    pub wallet_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SolBuySubmitResponse {
    pub quote_id: String,
    pub wallet_id: String,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub preflight: SolBuyPreflightResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoundSolBuyQuote {
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub quote_slippage_bps: u16,
    pub expires_at_ms: u64,
    #[serde(flatten)]
    pub quote: SolBuyQuoteResponse,
}

impl From<SolBuySubmitRequest> for QuoteSubmissionCommand {
    fn from(value: SolBuySubmitRequest) -> Self {
        Self {
            wallet_id: value.wallet_id,
            confirmed: value.confirmed,
            simulate: value.simulate,
            idempotency_key: value.idempotency_key,
        }
    }
}

impl From<QuoteLeg> for QuoteRouteLeg {
    fn from(value: QuoteLeg) -> Self {
        Self {
            label: value.label,
            input_mint: value.input_mint,
            output_mint: value.output_mint,
        }
    }
}

impl From<QuoteRouteLeg> for QuoteLeg {
    fn from(value: QuoteRouteLeg) -> Self {
        Self {
            label: value.label,
            input_mint: value.input_mint,
            output_mint: value.output_mint,
        }
    }
}

impl From<SolBuyQuoteResponse> for QuoteEstimate {
    fn from(value: SolBuyQuoteResponse) -> Self {
        Self {
            input_mint: value.input_mint,
            output_mint: value.output_mint,
            amount_lamports: value.amount_lamports,
            expected_output_units: value.expected_output_units,
            minimum_output_units: value.minimum_output_units,
            price_impact_percent: value.price_impact_percent,
            route: value.route.into_iter().map(Into::into).collect(),
            executable: value.executable,
        }
    }
}

impl From<QuoteEstimate> for SolBuyQuoteResponse {
    fn from(value: QuoteEstimate) -> Self {
        Self {
            input_mint: value.input_mint,
            output_mint: value.output_mint,
            amount_lamports: value.amount_lamports,
            expected_output_units: value.expected_output_units,
            minimum_output_units: value.minimum_output_units,
            price_impact_percent: value.price_impact_percent,
            route: value.route.into_iter().map(Into::into).collect(),
            executable: value.executable,
        }
    }
}

impl From<SolBuyPreflightResponse> for QuotePreflight {
    fn from(value: SolBuyPreflightResponse) -> Self {
        Self {
            quote_id: value.quote_id,
            wallet_id: value.wallet_id,
            quote_still_valid: value.quote_still_valid,
            unsigned_transaction_built: value.unsigned_transaction_built,
            aggregator_simulation_ok: value.aggregator_simulation_ok,
            local_simulation_ok: value.local_simulation_ok,
            instruction_count: value.instruction_count,
            address_lookup_table_count: value.address_lookup_table_count,
            resolved_account_count: value.resolved_account_count,
            instruction_audit_passed: value.instruction_audit_passed,
            executable: value.executable,
            reason: value.reason,
            prioritization_fee_lamports: value.prioritization_fee_lamports,
            compute_unit_limit: value.compute_unit_limit,
            calculated_priority_fee_lamports: value.calculated_priority_fee_lamports,
            explicit_system_spend_lamports: value.explicit_system_spend_lamports,
            writable_account_count: value.writable_account_count,
            target_ata: value.target_ata,
            top_level_programs: value.top_level_programs,
            simulated_fee_lamports: value.simulated_fee_lamports,
            simulated_units_consumed: value.simulated_units_consumed,
            simulated_payer_debit_lamports: value.simulated_payer_debit_lamports,
            simulated_inner_instruction_groups: value.simulated_inner_instruction_groups,
            simulated_target_token_delta_units: value.simulated_target_token_delta_units,
            simulated_target_token_post_units: value.simulated_target_token_post_units,
            baseline_slot: value.baseline_slot,
            simulation_slot: value.simulation_slot,
            observed_same_slot_payer_debit_lamports: value.observed_same_slot_payer_debit_lamports,
            observed_same_slot_target_token_delta_units: value
                .observed_same_slot_target_token_delta_units,
            simulated_inner_instruction_count: value.simulated_inner_instruction_count,
            simulated_inner_programs: value.simulated_inner_programs,
            simulated_inner_payer_system_spend_lamports: value
                .simulated_inner_payer_system_spend_lamports,
        }
    }
}

impl From<QuotePreflight> for SolBuyPreflightResponse {
    fn from(value: QuotePreflight) -> Self {
        Self {
            quote_id: value.quote_id,
            wallet_id: value.wallet_id,
            quote_still_valid: value.quote_still_valid,
            unsigned_transaction_built: value.unsigned_transaction_built,
            aggregator_simulation_ok: value.aggregator_simulation_ok,
            local_simulation_ok: value.local_simulation_ok,
            instruction_count: value.instruction_count,
            address_lookup_table_count: value.address_lookup_table_count,
            resolved_account_count: value.resolved_account_count,
            instruction_audit_passed: value.instruction_audit_passed,
            executable: value.executable,
            reason: value.reason,
            prioritization_fee_lamports: value.prioritization_fee_lamports,
            compute_unit_limit: value.compute_unit_limit,
            calculated_priority_fee_lamports: value.calculated_priority_fee_lamports,
            explicit_system_spend_lamports: value.explicit_system_spend_lamports,
            writable_account_count: value.writable_account_count,
            target_ata: value.target_ata,
            top_level_programs: value.top_level_programs,
            simulated_fee_lamports: value.simulated_fee_lamports,
            simulated_units_consumed: value.simulated_units_consumed,
            simulated_payer_debit_lamports: value.simulated_payer_debit_lamports,
            simulated_inner_instruction_groups: value.simulated_inner_instruction_groups,
            simulated_target_token_delta_units: value.simulated_target_token_delta_units,
            simulated_target_token_post_units: value.simulated_target_token_post_units,
            baseline_slot: value.baseline_slot,
            simulation_slot: value.simulation_slot,
            observed_same_slot_payer_debit_lamports: value.observed_same_slot_payer_debit_lamports,
            observed_same_slot_target_token_delta_units: value
                .observed_same_slot_target_token_delta_units,
            simulated_inner_instruction_count: value.simulated_inner_instruction_count,
            simulated_inner_programs: value.simulated_inner_programs,
            simulated_inner_payer_system_spend_lamports: value
                .simulated_inner_payer_system_spend_lamports,
        }
    }
}

impl From<BoundSolBuyQuote> for BoundQuote {
    fn from(value: BoundSolBuyQuote) -> Self {
        Self {
            quote_id: value.quote_id,
            wallet_id: value.wallet_id,
            wallet_pubkey: value.wallet_pubkey,
            quote_slippage_bps: value.quote_slippage_bps,
            expires_at_ms: value.expires_at_ms,
            quote: value.quote.into(),
        }
    }
}

impl From<BoundQuote> for BoundSolBuyQuote {
    fn from(value: BoundQuote) -> Self {
        Self {
            quote_id: value.quote_id,
            wallet_id: value.wallet_id,
            wallet_pubkey: value.wallet_pubkey,
            quote_slippage_bps: value.quote_slippage_bps,
            expires_at_ms: value.expires_at_ms,
            quote: value.quote.into(),
        }
    }
}

impl From<QuoteSubmissionResult> for SolBuySubmitResponse {
    fn from(value: QuoteSubmissionResult) -> Self {
        let status = match value.status {
            QuoteSubmissionOutcome::Pending => "submitting",
            QuoteSubmissionOutcome::Simulated => "simulated",
            QuoteSubmissionOutcome::Submitted => "submitted",
            QuoteSubmissionOutcome::Confirmed => "confirmed",
            QuoteSubmissionOutcome::Finalized => "finalized",
            QuoteSubmissionOutcome::Unknown => "unknown",
            QuoteSubmissionOutcome::Failed => "failed",
        };
        Self {
            quote_id: value.quote_id,
            wallet_id: value.wallet_id,
            status: status.into(),
            signature: value.signature,
            error: value.error,
            preflight: value.preflight.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_query_view_borrows_raw_request_strings_without_resolving_or_normalizing() {
        let request = SolBuyQuoteRequest {
            wallet_id: " wallet ".into(),
            mint: " raw mint ".into(),
            amount_lamports: u64::MAX,
            slippage_bps: u16::MAX,
        };
        let input = request.query_input();
        assert_eq!(input.wallet_id, request.wallet_id);
        assert_eq!(input.wallet_id.as_ptr(), request.wallet_id.as_ptr());
        assert_eq!(input.mint, request.mint);
        assert_eq!(input.mint.as_ptr(), request.mint.as_ptr());
        assert_eq!(input.amount_lamports, u64::MAX);
        assert_eq!(input.slippage_bps, u16::MAX);
    }
}
