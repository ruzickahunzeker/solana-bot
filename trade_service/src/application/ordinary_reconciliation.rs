use std::time::Duration;

use anyhow::{ensure, Result};

use crate::{
    application::{
        ordinary_claims::ensure_parent,
        ordinary_observation_context::OrdinaryObservationContext,
        ports::{
            ordinary_claims::ExecutionOwner,
            ordinary_reconciliation::{
                OrdinaryReconciliationJournal, RecoveredOrdinaryObservations,
            },
            reconciliation::SignatureStatusReader,
        },
        trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::{reconciliation::ChainCommitment, trade::TradeStatus},
};

pub(crate) struct OrdinaryReconciliationResult {
    pub(crate) record: TradeRecordV1,
    pub(crate) lookup_error: Option<String>,
}

fn validate_record(
    execution: &AdmittedTrade,
    signature: &str,
    record: &TradeRecordV1,
) -> Result<()> {
    ensure_parent(execution, record)?;
    ensure!(
        !signature.is_empty()
            && record.signature.as_deref() == Some(signature)
            && matches!(
                record.status,
                TradeStatus::Submitted
                    | TradeStatus::Unknown
                    | TradeStatus::Confirmed
                    | TradeStatus::Failed
            ),
        "ordinary observation signature or parent state mismatch"
    );
    Ok(())
}

pub(crate) async fn reconcile<
    J: OrdinaryReconciliationJournal + RecoveredOrdinaryObservations,
    R: SignatureStatusReader,
>(
    journal: &J,
    reader: &R,
    execution: &AdmittedTrade,
    owner: &ExecutionOwner,
    signature: &str,
    deadline: Duration,
) -> Result<OrdinaryReconciliationResult> {
    owner.ensure_binding(execution)?;
    let captured = journal
        .capture_live_observation(execution, owner, signature)
        .await?;
    validate_record(execution, signature, &captured.record)?;
    captured.context.ensure_record(&captured.record)?;
    ensure!(
        captured.context.signature() == signature,
        "ordinary captured signature changed"
    );
    captured.context.ensure_execution(execution)?;
    reconcile_captured(journal, reader, &captured.context, deadline).await
}

fn validate_captured_record(
    context: &OrdinaryObservationContext,
    record: &TradeRecordV1,
) -> Result<()> {
    context.ensure_record(record)?;
    ensure!(
        matches!(
            record.status,
            TradeStatus::Submitting
                | TradeStatus::Submitted
                | TradeStatus::Unknown
                | TradeStatus::Confirmed
                | TradeStatus::Failed
        ),
        "ordinary captured observation parent state invalid"
    );
    Ok(())
}

pub(crate) async fn reconcile_captured<
    J: RecoveredOrdinaryObservations,
    R: SignatureStatusReader,
>(
    journal: &J,
    reader: &R,
    context: &OrdinaryObservationContext,
    deadline: Duration,
) -> Result<OrdinaryReconciliationResult> {
    let current = journal.read_observation(context).await?;
    validate_captured_record(context, &current)?;
    let signature = context.signature();
    if matches!(current.status, TradeStatus::Confirmed | TradeStatus::Failed) {
        return Ok(OrdinaryReconciliationResult {
            record: current,
            lookup_error: None,
        });
    }
    let (observation, lookup_error) =
        match tokio::time::timeout(deadline, reader.lookup_signature(signature)).await {
            Ok(Ok(Some(mut observation)))
                if observation.signature == signature && observation.slot > 0 =>
            {
                observation.error = observation
                    .error
                    .map(|error| crate::sanitize::redact_urls(&error));
                (Some(observation), None)
            }
            Ok(Ok(Some(_))) => (
                None,
                Some("ordinary signature status identity or slot invalid".into()),
            ),
            Ok(Ok(None)) => (None, None),
            Ok(Err(error)) => (None, Some(crate::sanitize::redact_urls(&error.to_string()))),
            Err(_) => (
                None,
                Some(
                    "ordinary signature lookup timed out; original transaction was not retried"
                        .into(),
                ),
            ),
        };
    let record = if let Some(observation) =
        observation.filter(|value| value.commitment == ChainCommitment::Finalized)
    {
        journal
            .record_captured_finality(context, &observation)
            .await?
    } else {
        // A concurrent final observation may have won while this lookup failed or lagged.
        journal.read_observation(context).await?
    };
    validate_captured_record(context, &record)?;
    Ok(OrdinaryReconciliationResult {
        record,
        lookup_error,
    })
}
