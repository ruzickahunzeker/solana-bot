use crate::{
    application::{
        ordinary_claims::ClaimOperation,
        ports::{
            ordinary_claims::ExecutionOwner,
            ordinary_sdk_submission::{OrdinarySdkCandidateBackend, OrdinarySdkSubmissionJournal},
        },
        sdk_preparation::validate_origin,
        trade_admission::AdmittedTrade,
        trade_execution::JournaledTradeOutcome,
        trade_queries::is_pending_status,
    },
    domain::trade::TradeStatus,
};
use anyhow::{ensure, Result};

mod signing_permit;
pub(crate) use signing_permit::SdkSigningPermit;

/// Consumes the original owner and candidate. There is no retry, restored owner, or Quote.
pub(crate) async fn execute<J: OrdinarySdkSubmissionJournal, B: OrdinarySdkCandidateBackend>(
    journal: &J,
    backend: &B,
    execution: &AdmittedTrade,
    owner: ExecutionOwner,
    candidate: B::Candidate,
) -> JournaledTradeOutcome {
    let mut signing_attempted = false;
    let mut signature = None;
    let result: Result<()> = async {
        owner.ensure_binding(execution)?;
        ensure!(
            owner.operation() == ClaimOperation::SdkTrade,
            "SDK submission requires its original operation owner"
        );
        backend.ensure_live_allowed()?;
        let metadata = backend.metadata(&candidate);
        let message = backend.unsigned_message(&candidate)?;
        validate_origin(execution, metadata, &message)?;
        backend.authorize(execution, &candidate)?;
        journal
            .prepare(execution, &owner, metadata, &message)
            .await?;
        let stored = journal.read_prepared(execution, &owner).await?;
        ensure!(
            stored.metadata == *metadata && stored.message == message,
            "SDK independent prepared readback differs from original"
        );
        let metadata = stored.metadata;
        backend.ensure_live_allowed()?;
        backend.authorize(execution, &candidate)?;
        // An error may lose the commit acknowledgement. Never retry the signing CAS.
        signing_attempted = true;
        journal
            .begin_signing(execution, &owner, &metadata, &message)
            .await?;
        let permit = SdkSigningPermit::new(execution, &owner, &metadata, &message)?;
        backend.ensure_live_allowed()?;
        backend.authorize(execution, &candidate)?;
        let signed_artifact = backend.sign(candidate, permit)?;
        let signed = backend.signed_submission(&signed_artifact);
        // Preserve a reported signature diagnostically even when subsequent evidence rejects it.
        if !signed.signature.is_empty() && signed.signature.len() <= 128 {
            signature = Some(signed.signature.clone());
        }
        backend.verify_signed(&signed_artifact, &metadata, &message)?;
        ensure!(
            signature.is_some(),
            "SDK signer returned no bounded signature"
        );
        journal.record_signed(execution, &owner, signed).await?;
        let persisted = journal.read_signed(execution, &owner).await?;
        ensure!(
            persisted.signature == signed.signature && persisted.bytes == signed.bytes,
            "SDK independent signed readback differs from original"
        );
        backend.ensure_live_allowed()?;
        backend.authorize_broadcast(&signed_artifact)?;
        let taken = journal
            .take_broadcast(execution, &owner, &persisted)
            .await?;
        ensure!(
            taken.signature == persisted.signature && taken.bytes == persisted.bytes,
            "SDK consumed broadcast bytes differ from original"
        );
        backend.ensure_live_allowed()?;
        backend.authorize_broadcast(&signed_artifact)?;
        let (submitted, error) = match backend.broadcast(&signed_artifact, &taken).await {
            Ok(receipt) if receipt == taken.signature => (true, None),
            Ok(_) => (
                false,
                Some(
                    "SDK sender returned a different signature; reconcile original signature"
                        .to_owned(),
                ),
            ),
            Err(error) => (
                false,
                Some(crate::sanitize::redact_urls(&error.to_string())),
            ),
        };
        journal
            .record_broadcast(execution, &owner, &taken, submitted, error.as_deref())
            .await?;
        Ok(())
    }
    .await;
    let failure = result.err();
    if signature.is_none() && signing_attempted {
        if let Some(known) = failure.as_ref().and_then(|error| {
            error.downcast_ref::<crate::application::trade_execution::UncertainSdkSubmission>()
        }) {
            signature = known
                .signatures()
                .first()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .cloned();
        }
    }
    let error = failure.map(|e| crate::sanitize::redact_urls(&e.to_string()));
    finish(
        journal,
        execution,
        &owner,
        signing_attempted,
        signature,
        error,
    )
    .await
}

pub(crate) async fn reject_preparation<J: OrdinarySdkSubmissionJournal>(
    journal: &J,
    execution: &AdmittedTrade,
    owner: ExecutionOwner,
    error: anyhow::Error,
) -> JournaledTradeOutcome {
    finish(
        journal,
        execution,
        &owner,
        false,
        None,
        Some(crate::sanitize::redact_urls(&error.to_string())),
    )
    .await
}

pub(crate) async fn reject_context<J: OrdinarySdkSubmissionJournal>(
    journal: &J,
    execution: &AdmittedTrade,
    owner: ExecutionOwner,
    error: &str,
) -> JournaledTradeOutcome {
    let error = crate::sanitize::redact_urls(error);
    let result = async {
        owner.ensure_binding(execution)?;
        journal
            .reject_before_sign(execution, &owner, &error)
            .await?;
        let record = journal.observe(execution, &owner).await?;
        crate::application::ordinary_claims::ensure_parent(execution, &record)?;
        ensure!(
            record.status == TradeStatus::Rejected && record.signature.is_none(),
            "SDK pre-sign rejection readback mismatch"
        );
        Ok::<_, anyhow::Error>(record)
    }
    .await;
    match result {
        Ok(record) => JournaledTradeOutcome {
            record: Some(record),
            signature: None,
            error: Some(error),
        },
        Err(storage_error) => JournaledTradeOutcome {
            record: None,
            signature: None,
            error: Some(format!(
                "{error}; original SDK rejection unavailable: {}",
                crate::sanitize::redact_urls(&storage_error.to_string())
            )),
        },
    }
}

async fn finish<J: OrdinarySdkSubmissionJournal>(
    journal: &J,
    execution: &AdmittedTrade,
    owner: &ExecutionOwner,
    signing_attempted: bool,
    mut signature: Option<String>,
    mut error: Option<String>,
) -> JournaledTradeOutcome {
    if !signing_attempted {
        if let Some(reason) = error.as_deref() {
            if let Err(storage_error) = journal.fail_before_sign(execution, owner, reason).await {
                tracing::error!(error = %crate::sanitize::redact_urls(&storage_error.to_string()), "SDK pre-sign failure could not be journaled; do not resubmit");
            }
        }
    }
    let record = match journal.observe(execution, owner).await {
        Ok(record)
            if crate::application::ordinary_claims::ensure_parent(execution, &record).is_ok() =>
        {
            if signature.is_none() {
                signature = record.signature.clone();
            }
            let mut signed_failure = signing_attempted && record.status == TradeStatus::Failed;
            if signed_failure {
                if let Some(signature) = signature.as_deref() {
                    signed_failure = !matches!(
                        journal.finalized_failure(execution, owner, signature).await,
                        Ok(Some(ref verified))
                            if crate::application::ordinary_claims::ensure_parent(execution, verified).is_ok()
                                && verified.status == TradeStatus::Failed
                                && verified.signature == record.signature
                                && verified.error == record.error
                    );
                }
            }
            if !signed_failure && !is_pending_status(record.status) && record.signature == signature
            {
                error = error.or_else(|| record.error.clone());
                Some(record)
            } else {
                None
            }
        }
        Ok(_) => {
            tracing::error!("SDK journal returned a different parent; no resubmission");
            None
        }
        Err(storage_error) => {
            tracing::error!(error = %crate::sanitize::redact_urls(&storage_error.to_string()), "SDK original journal readback unavailable; no resubmission");
            None
        }
    };
    if record.is_none() && error.is_none() {
        error = Some(
            "SDK journal outcome unavailable; reconcile original order without resubmission".into(),
        );
    }
    JournaledTradeOutcome {
        record,
        signature,
        error,
    }
}

#[cfg(test)]
mod tests;
