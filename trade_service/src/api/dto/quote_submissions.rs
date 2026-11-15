use serde::{Deserialize, Serialize};

use crate::{
    application::submission_reconciler::{LookupOutcome, ReconciliationResult},
    domain::submission::SubmissionStatus,
};

#[derive(Deserialize)]
pub(crate) struct QuoteSubmissionQuery {
    pub wallet_id: String,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct QuoteSubmissionStatusResponse {
    pub submission_id: String,
    pub quote_id: String,
    pub wallet_id: String,
    pub status: &'static str,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub commitment: Option<&'static str>,
    pub observed_slot: Option<u64>,
    pub reconciliation_status: &'static str,
    pub reconciliation_error: Option<String>,
}

impl From<ReconciliationResult> for QuoteSubmissionStatusResponse {
    fn from(result: ReconciliationResult) -> Self {
        let record = result.record;
        Self {
            submission_id: record.submission_id,
            quote_id: record.intent.quote_id,
            wallet_id: record.intent.key.wallet_id,
            status: match record.status {
                SubmissionStatus::Reserved => "submitting",
                SubmissionStatus::Signed => "unknown",
                status => status.as_str(),
            },
            signature: record.signature,
            error: record.error,
            commitment: record.commitment.map(|commitment| commitment.as_str()),
            observed_slot: record.observed_slot,
            reconciliation_status: match result.outcome {
                LookupOutcome::Updated => "updated",
                LookupOutcome::Unchanged => "unchanged",
                LookupOutcome::NotFound => "not_found",
                LookupOutcome::Unavailable => "unavailable",
                LookupOutcome::NotRequired => "not_required",
            },
            reconciliation_error: result.lookup_error,
        }
    }
}
