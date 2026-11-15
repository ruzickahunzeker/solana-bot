use crate::domain::sol_transfer::{SolTransferAttemptState, SolTransferIntent};

#[derive(Debug, Clone)]
pub struct SolTransferRecordV1 {
    pub transfer_id: String,
    pub idempotency_key: String,
    pub owner_type: String,
    pub owner_id: String,
    pub wallet_id: String,
    pub from_pubkey: String,
    pub to_pubkey: String,
    pub amount_mode: String,
    pub requested_amount_lamports: Option<u64>,
    pub requested_percent: Option<u64>,
    pub actual_amount_lamports: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub balance_before_lamports: Option<u64>,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SolTransferJournalEntry {
    pub record: SolTransferRecordV1,
    pub intent: Option<SolTransferIntent>,
    pub state: Option<SolTransferAttemptState>,
}

impl SolTransferRecordV1 {
    pub(crate) fn matches(&self, intent: &SolTransferIntent) -> bool {
        let (fixed, percent) = intent.amount.legacy_params();
        self.idempotency_key == intent.idempotency_key
            && self.owner_type == intent.owner_type
            && self.owner_id == intent.owner_id
            && self.wallet_id == intent.wallet_id
            && self.from_pubkey == intent.source.to_string()
            && self.to_pubkey == intent.destination.to_string()
            && self.amount_mode == (if fixed.is_some() { "fixed" } else { "percent" })
            && self.requested_amount_lamports == fixed
            && self.requested_percent == percent
    }
}

#[derive(Debug)]
pub(crate) enum SolTransferReservation {
    Created(SolTransferJournalEntry),
    Existing(SolTransferJournalEntry),
    Conflict,
}
