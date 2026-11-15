use std::time::Duration;

use super::{
    ports::reconciliation::{ReconciliationJournal, SignatureStatusReader},
    quote_submission::{FailureKind, SubmissionError},
};
use crate::domain::{
    reconciliation::ChainCommitment,
    submission::{SubmissionKey, SubmissionRecord},
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LookupOutcome {
    Updated,
    Unchanged,
    NotFound,
    Unavailable,
    NotRequired,
}

pub(crate) struct ReconciliationResult {
    pub record: SubmissionRecord,
    pub outcome: LookupOutcome,
    pub lookup_error: Option<String>,
}

pub(crate) async fn reconcile<J: ReconciliationJournal, R: SignatureStatusReader>(
    journal: &J,
    reader: &R,
    key: &SubmissionKey,
    wallet_pubkey: &str,
    quote_id: &str,
    timeout: Duration,
) -> Result<ReconciliationResult, SubmissionError> {
    let record = journal
        .lookup(key)
        .await
        .map_err(SubmissionError::storage)?
        .ok_or_else(|| SubmissionError::new(FailureKind::NotFound, "submission not found"))?;
    if record.intent.wallet_pubkey != wallet_pubkey || record.intent.quote_id != quote_id {
        return Err(SubmissionError::new(
            FailureKind::Conflict,
            "submission quote or wallet identity mismatch",
        ));
    }
    let Some(signature) = record.signature.as_deref() else {
        return Ok(result(record, LookupOutcome::NotRequired, None));
    };
    if record.commitment == Some(ChainCommitment::Finalized) {
        return Ok(result(record, LookupOutcome::NotRequired, None));
    }
    let mut observation =
        match tokio::time::timeout(timeout, reader.lookup_signature(signature)).await {
            Ok(Ok(Some(observation))) => observation,
            Ok(Ok(None)) => {
                return refresh(
                    journal,
                    key,
                    &record.submission_id,
                    LookupOutcome::NotFound,
                    None,
                )
                .await
            }
            Ok(Err(error)) => {
                return refresh(
                    journal,
                    key,
                    &record.submission_id,
                    LookupOutcome::Unavailable,
                    Some(crate::sanitize::redact_urls(&error.to_string())),
                )
                .await
            }
            Err(_) => {
                return refresh(
                    journal,
                    key,
                    &record.submission_id,
                    LookupOutcome::Unavailable,
                    Some(
                        "signature status lookup timed out; original transaction was not retried"
                            .into(),
                    ),
                )
                .await
            }
        };
    if observation.signature != signature {
        return refresh(
            journal,
            key,
            &record.submission_id,
            LookupOutcome::Unavailable,
            Some("signature status response did not match requested signature".into()),
        )
        .await;
    }
    observation.error = observation
        .error
        .map(|error| crate::sanitize::redact_urls(&error));
    let updated = journal
        .record_observation(&record.submission_id, &observation)
        .await
        .map_err(SubmissionError::storage)?;
    refresh(
        journal,
        key,
        &record.submission_id,
        if updated {
            LookupOutcome::Updated
        } else {
            LookupOutcome::Unchanged
        },
        None,
    )
    .await
}

// Refresh even after RPC errors/misses: another query may already have finalized this order.
async fn refresh<J: ReconciliationJournal>(
    journal: &J,
    key: &SubmissionKey,
    expected_id: &str,
    outcome: LookupOutcome,
    lookup_error: Option<String>,
) -> Result<ReconciliationResult, SubmissionError> {
    let current = journal
        .lookup(key)
        .await
        .map_err(SubmissionError::storage)?
        .ok_or_else(|| {
            SubmissionError::new(
                FailureKind::Unavailable,
                "submission disappeared during reconciliation",
            )
        })?;
    if current.submission_id != expected_id {
        return Err(SubmissionError::new(
            FailureKind::Unavailable,
            "submission identity changed during reconciliation",
        ));
    }
    Ok(result(current, outcome, lookup_error))
}

fn result(
    record: SubmissionRecord,
    outcome: LookupOutcome,
    lookup_error: Option<String>,
) -> ReconciliationResult {
    ReconciliationResult {
        record,
        outcome,
        lookup_error,
    }
}

#[cfg(test)]
mod tests;
