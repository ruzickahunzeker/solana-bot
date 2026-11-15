use anyhow::{anyhow, Result};

use super::ports::quote_execution::QuoteSubmissionBackend;
use super::ports::submission_journal::SubmissionJournal;
use super::quote_submission_types::{
    QuoteSubmissionCommand, QuoteSubmissionOutcome, QuoteSubmissionResult,
};
use crate::domain::quote::BoundQuote;
use crate::domain::submission::{
    Reservation, SubmissionIntent, SubmissionKey, SubmissionRecord, SubmissionStatus,
};

#[derive(Debug)]
pub(crate) struct SubmissionError {
    pub kind: FailureKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    InvalidRequest,
    Forbidden,
    NotFound,
    Expired,
    Conflict,
    Unavailable,
    Backend,
}

impl SubmissionError {
    pub(super) fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub(super) fn storage(error: anyhow::Error) -> Self {
        tracing::error!(error = %crate::sanitize::redact_urls(&error.to_string()), "quote submission journal unavailable; do not retry with a new key");
        Self::new(
            FailureKind::Unavailable,
            "submission journal unavailable; transaction must not be retried with a new key",
        )
    }
    fn backend(error: anyhow::Error) -> Self {
        Self::new(
            FailureKind::Backend,
            crate::sanitize::redact_urls(&error.to_string()),
        )
    }
}

pub(crate) fn validate(request: &QuoteSubmissionCommand) -> Result<SubmissionKey, SubmissionError> {
    let key = SubmissionKey::new(&request.wallet_id, &request.idempotency_key)
        .map_err(|error| SubmissionError::new(FailureKind::InvalidRequest, error.to_string()))?;
    if !request.simulate && !request.confirmed {
        return Err(SubmissionError::new(
            FailureKind::InvalidRequest,
            "live SOL buy requires confirmed=true",
        ));
    }
    Ok(key)
}

/// Read before consulting ephemeral quotes, so a restart/expiry cannot hide a sent order.
pub(crate) async fn existing<J: SubmissionJournal>(
    journal: &J,
    key: &SubmissionKey,
    wallet_pubkey: &str,
    quote_id: &str,
) -> Result<Option<QuoteSubmissionResult>, SubmissionError> {
    let Some(record) = journal.get(key).await.map_err(SubmissionError::storage)? else {
        return Ok(None);
    };
    if record.intent.wallet_pubkey != wallet_pubkey || record.intent.quote_id != quote_id {
        return Err(conflict());
    }
    response(record).map(Some)
}

pub(crate) async fn execute<J: SubmissionJournal, B: QuoteSubmissionBackend>(
    journal: Option<&J>,
    backend: &B,
    bound: &BoundQuote,
    request: &QuoteSubmissionCommand,
) -> Result<QuoteSubmissionResult, SubmissionError> {
    let key = validate(request)?;
    if bound.wallet_id != request.wallet_id {
        return Err(SubmissionError::new(
            FailureKind::InvalidRequest,
            "quote wallet mismatch",
        ));
    }
    if !request.simulate {
        let journal = journal.ok_or_else(|| {
            SubmissionError::new(
                FailureKind::Unavailable,
                "live SOL quote submission requires durable trade persistence",
            )
        })?;
        if let Some(result) = existing(journal, &key, &bound.wallet_pubkey, &bound.quote_id).await?
        {
            return Ok(result);
        }
        backend
            .ensure_live_allowed()
            .map_err(|error| SubmissionError::new(FailureKind::Forbidden, error.to_string()))?;
    }
    let (prepared, preflight) = backend
        .prepare(bound)
        .await
        .map_err(SubmissionError::backend)?;
    if !preflight.executable
        || preflight.quote_id != bound.quote_id
        || preflight.wallet_id != bound.wallet_id
    {
        return Err(SubmissionError::backend(anyhow!(
            "aggregator preflight did not produce an executable transaction for this quote and wallet"
        )));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if u128::from(bound.expires_at_ms) <= now {
        return Err(SubmissionError::new(FailureKind::Expired, "quote expired"));
    }
    if request.simulate {
        return Ok(QuoteSubmissionResult {
            quote_id: bound.quote_id.clone(),
            wallet_id: request.wallet_id.clone(),
            status: QuoteSubmissionOutcome::Simulated,
            signature: None,
            error: None,
            preflight,
        });
    }
    let journal = journal.expect("live persistence checked before preparation");
    let intent = SubmissionIntent {
        key,
        wallet_pubkey: bound.wallet_pubkey.clone(),
        quote_id: bound.quote_id.clone(),
        quote: serde_json::to_value(bound).map_err(|e| SubmissionError::backend(e.into()))?,
    };
    let preflight_json =
        serde_json::to_value(&preflight).map_err(|e| SubmissionError::backend(e.into()))?;
    let record = match journal
        .reserve(&intent, &preflight_json)
        .await
        .map_err(SubmissionError::storage)?
    {
        Reservation::Existing(record) => return response(record),
        Reservation::Conflict => return Err(conflict()),
        Reservation::Created(record) => record,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if u128::from(bound.expires_at_ms) <= now {
        journal
            .fail_before_sign(&record.submission_id, "quote expired before signing")
            .await
            .map_err(SubmissionError::storage)?;
        return Err(SubmissionError::new(
            FailureKind::Expired,
            "quote expired before signing",
        ));
    }
    let signed = match backend.sign(prepared) {
        Ok(signed) => signed,
        Err(error) => {
            let message = crate::sanitize::redact_urls(&error.to_string());
            journal
                .fail_before_sign(&record.submission_id, &message)
                .await
                .map_err(SubmissionError::storage)?;
            return Err(SubmissionError::backend(error));
        }
    };
    journal
        .record_signed(&record.submission_id, &signed)
        .await
        .map_err(SubmissionError::storage)?;
    let (status, error) = match backend.broadcast(&signed).await {
        Ok(signature) if signature == signed.signature => (SubmissionStatus::Submitted, None),
        Ok(_) => (
            SubmissionStatus::Unknown,
            Some("RPC returned a different signature; reconcile original signature".into()),
        ),
        Err(error) => (
            SubmissionStatus::Unknown,
            Some(crate::sanitize::redact_urls(&error.to_string())),
        ),
    };
    journal
        .record_broadcast(&record.submission_id, status, error.as_deref())
        .await
        .map_err(SubmissionError::storage)?;
    let current = journal
        .get(&intent.key)
        .await
        .map_err(SubmissionError::storage)?
        .ok_or_else(|| {
            SubmissionError::new(
                FailureKind::Unavailable,
                "submission disappeared after broadcast",
            )
        })?;
    response(current)
}

fn conflict() -> SubmissionError {
    SubmissionError::new(
        FailureKind::Conflict,
        "idempotency key was used for another quote or wallet identity",
    )
}

fn response(record: SubmissionRecord) -> Result<QuoteSubmissionResult, SubmissionError> {
    Ok(QuoteSubmissionResult {
        quote_id: record.intent.quote_id,
        wallet_id: record.intent.key.wallet_id,
        status: record.status.into(),
        signature: record.signature,
        error: record.error,
        preflight: serde_json::from_value(record.preflight)
            .map_err(|e| SubmissionError::storage(e.into()))?,
    })
}

#[cfg(test)]
mod tests;
