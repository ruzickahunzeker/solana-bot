use crate::domain::{quote::QuotePreflight, submission::SubmissionStatus};

#[derive(Debug, Clone)]
pub(crate) struct QuoteSubmissionCommand {
    pub wallet_id: String,
    pub confirmed: bool,
    pub simulate: bool,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuoteSubmissionOutcome {
    Pending,
    Simulated,
    Submitted,
    Confirmed,
    Finalized,
    Unknown,
    Failed,
}

impl From<SubmissionStatus> for QuoteSubmissionOutcome {
    fn from(status: SubmissionStatus) -> Self {
        match status {
            SubmissionStatus::Reserved => Self::Pending,
            SubmissionStatus::Signed | SubmissionStatus::Unknown => Self::Unknown,
            SubmissionStatus::Submitted => Self::Submitted,
            SubmissionStatus::Confirmed => Self::Confirmed,
            SubmissionStatus::Finalized => Self::Finalized,
            SubmissionStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct QuoteSubmissionResult {
    pub quote_id: String,
    pub wallet_id: String,
    pub status: QuoteSubmissionOutcome,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub preflight: QuotePreflight,
}
