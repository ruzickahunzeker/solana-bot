use crate::application::wsol_submission_types::{
    WsolDirection as InternalDirection, WsolQuoteSnapshot, WsolSimulationOutcome,
    WsolSubmissionInput, WsolSubmissionOutcome, WsolSubmissionRecord as ConversionRecord,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct WsolRecoveryQuery {
    pub idempotency_key: String,
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
}

impl WsolRecoveryQuery {
    pub(crate) fn input(self) -> crate::application::wsol_recovery::WsolRecoveryInput {
        crate::application::wsol_recovery::WsolRecoveryInput {
            idempotency_key: self.idempotency_key,
            wallet_id: self.wallet_id,
            owner_type: self.owner_type,
            owner_id: self.owner_id,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WsolQuoteRequest {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub direction: WsolDirection,
    pub amount_lamports: Option<u64>,
    #[serde(default)]
    pub all: bool,
}

impl WsolQuoteRequest {
    pub(crate) fn input(&self) -> crate::application::wsol_queries::WsolQuoteInput {
        crate::application::wsol_queries::WsolQuoteInput {
            wallet_id: self.wallet_id.clone(),
            owner_type: self.owner_type.clone(),
            owner_id: self.owner_id.clone(),
            direction: match self.direction {
                WsolDirection::Wrap => {
                    crate::application::wsol_submission_types::WsolDirection::Wrap
                }
                WsolDirection::Unwrap => {
                    crate::application::wsol_submission_types::WsolDirection::Unwrap
                }
            },
            amount_lamports: self.amount_lamports,
            all: self.all,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WsolQuotePreview {
    pub quote: WsolQuote,
    pub executable: bool,
}

#[derive(Debug, Serialize)]
pub struct WsolSimulationResult {
    pub status: &'static str,
    pub quote_id: String,
    pub network_fee_lamports: Option<u64>,
    pub units_consumed: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct WsolSubmissionResult {
    pub submission_id: String,
    pub quote_id: String,
    pub wallet_id: String,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
}

impl WsolSubmissionResult {
    pub(crate) fn from_record(record: ConversionRecord, duplicate: bool) -> Self {
        Self {
            submission_id: record.submission_id,
            quote_id: record.quote_id,
            wallet_id: record.wallet_id,
            status: record.status,
            signature: record.signature,
            error: record.error,
            duplicate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsolDirection {
    Wrap,
    Unwrap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsolQuote {
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub owner_type: String,
    pub owner_id: String,
    pub direction: WsolDirection,
    pub amount_lamports: u64,
    pub source_balance_lamports: u64,
    pub ata_exists: bool,
    pub estimated_network_fee_lamports: u64,
    pub temporary_rent_lamports: u64,
    pub rent_refund_lamports: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsolSubmission {
    pub quote_id: String,
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub simulate: bool,
    #[serde(default)]
    pub confirmed: bool,
    pub idempotency_key: String,
}

impl WsolSubmission {
    pub(crate) fn input(&self) -> crate::application::wsol_submission_types::WsolSubmissionInput {
        crate::application::wsol_submission_types::WsolSubmissionInput {
            quote_id: self.quote_id.clone(),
            wallet_id: self.wallet_id.clone(),
            owner_type: self.owner_type.clone(),
            owner_id: self.owner_id.clone(),
            simulate: self.simulate,
            confirmed: self.confirmed,
            idempotency_key: self.idempotency_key.clone(),
        }
    }
}
impl From<&WsolQuote> for WsolQuoteSnapshot {
    fn from(quote: &WsolQuote) -> Self {
        Self {
            quote_id: quote.quote_id.clone(),
            wallet_id: quote.wallet_id.clone(),
            wallet_pubkey: quote.wallet_pubkey.clone(),
            owner_type: quote.owner_type.clone(),
            owner_id: quote.owner_id.clone(),
            direction: match quote.direction {
                WsolDirection::Wrap => InternalDirection::Wrap,
                WsolDirection::Unwrap => InternalDirection::Unwrap,
            },
            amount_lamports: quote.amount_lamports,
            source_balance_lamports: quote.source_balance_lamports,
            ata_exists: quote.ata_exists,
            estimated_network_fee_lamports: quote.estimated_network_fee_lamports,
            temporary_rent_lamports: quote.temporary_rent_lamports,
            rent_refund_lamports: quote.rent_refund_lamports,
            expires_at_ms: quote.expires_at_ms,
        }
    }
}
impl From<&WsolQuoteSnapshot> for WsolQuote {
    fn from(quote: &WsolQuoteSnapshot) -> Self {
        Self {
            quote_id: quote.quote_id.clone(),
            wallet_id: quote.wallet_id.clone(),
            wallet_pubkey: quote.wallet_pubkey.clone(),
            owner_type: quote.owner_type.clone(),
            owner_id: quote.owner_id.clone(),
            direction: match quote.direction {
                InternalDirection::Wrap => WsolDirection::Wrap,
                InternalDirection::Unwrap => WsolDirection::Unwrap,
            },
            amount_lamports: quote.amount_lamports,
            source_balance_lamports: quote.source_balance_lamports,
            ata_exists: quote.ata_exists,
            estimated_network_fee_lamports: quote.estimated_network_fee_lamports,
            temporary_rent_lamports: quote.temporary_rent_lamports,
            rent_refund_lamports: quote.rent_refund_lamports,
            expires_at_ms: quote.expires_at_ms,
        }
    }
}
impl From<&WsolSubmissionInput> for WsolSubmission {
    fn from(input: &WsolSubmissionInput) -> Self {
        Self {
            quote_id: input.quote_id.clone(),
            wallet_id: input.wallet_id.clone(),
            owner_type: input.owner_type.clone(),
            owner_id: input.owner_id.clone(),
            simulate: input.simulate,
            confirmed: input.confirmed,
            idempotency_key: input.idempotency_key.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct WsolOwnerQuery {
    pub owner_type: String,
    pub owner_id: String,
}

impl From<WsolQuoteSnapshot> for WsolQuotePreview {
    fn from(quote: WsolQuoteSnapshot) -> Self {
        Self {
            quote: (&quote).into(),
            executable: false,
        }
    }
}
impl From<WsolSubmissionOutcome> for WsolSubmissionResult {
    fn from(outcome: WsolSubmissionOutcome) -> Self {
        Self::from_record(outcome.record, outcome.duplicate)
    }
}
impl From<WsolSimulationOutcome> for WsolSimulationResult {
    fn from(outcome: WsolSimulationOutcome) -> Self {
        Self {
            status: "simulated",
            quote_id: outcome.quote_id,
            network_fee_lamports: Some(outcome.network_fee_lamports),
            units_consumed: outcome.units_consumed,
        }
    }
}
