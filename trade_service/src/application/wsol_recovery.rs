use std::{error::Error, fmt};

use super::wsol_submission_types::WsolSubmissionRecord;

#[derive(Debug, Clone)]
pub(crate) struct WsolRecoveryInput {
    pub idempotency_key: String,
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
}

impl WsolRecoveryInput {
    pub(crate) fn validate(&self) -> Result<(), WsolRecoveryFailure> {
        if self.idempotency_key.trim().is_empty()
            || self.idempotency_key.len() > 128
            || self.wallet_id.trim().is_empty()
            || self.owner_type.trim().is_empty()
            || self.owner_id.trim().is_empty()
        {
            return Err(WsolRecoveryFailure::Rejected);
        }
        Ok(())
    }

    pub(crate) fn validate_record(
        &self,
        record: &WsolSubmissionRecord,
    ) -> Result<(), WsolRecoveryFailure> {
        if record.idempotency_key != self.idempotency_key
            || record.wallet_id != self.wallet_id
            || record.owner_type != self.owner_type
            || record.owner_id != self.owner_id
        {
            return Err(WsolRecoveryFailure::NotFound);
        }
        record
            .validate_status_record(&record.submission_id)
            .map_err(|_| WsolRecoveryFailure::Integrity)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsolRecoveryFailure {
    Rejected,
    NotFound,
    Unavailable,
    Integrity,
}

impl WsolRecoveryFailure {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Rejected => "wsol_recovery_rejected",
            Self::NotFound => "wsol_recovery_not_found",
            Self::Unavailable => "wsol_recovery_unavailable",
            Self::Integrity => "wsol_recovery_integrity_error",
        }
    }
}

impl fmt::Display for WsolRecoveryFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rejected => "WSOL recovery requires a 1..128 byte non-blank idempotency_key and non-blank wallet_id, owner_type and owner_id",
            Self::NotFound => "WSOL operation not found for this wallet and owner; absence is not permission to resubmit",
            Self::Unavailable => "WSOL recovery unavailable; do not automatically retry submission or use a new idempotency_key",
            Self::Integrity => "WSOL recovery record is inconsistent; reconcile the original operation without resubmitting",
        })
    }
}

impl Error for WsolRecoveryFailure {}
