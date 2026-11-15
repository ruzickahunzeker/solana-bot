use super::{checked, metadata, now, one, read_signature};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Row};

use crate::{
    adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
    application::{
        ordinary_observation_context::{CapturedOrdinaryObservation, OrdinaryObservationContext},
        ports::{
            ordinary_claims::ExecutionOwner,
            ordinary_reconciliation::OrdinaryReconciliationJournal,
            ordinary_reconciliation::RecoveredOrdinaryObservations,
        },
        trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        trade::TradeStatus,
    },
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::adapters::persistence) struct FinalObservationV1 {
    schema_version: u8,
    signature: String,
    commitment: String,
    slot: u64,
    error: Option<String>,
}

impl FinalObservationV1 {
    pub(in crate::adapters::persistence) fn new(
        signature: &str,
        observation: &ChainObservation,
    ) -> Result<Self> {
        ensure!(
            !signature.is_empty()
                && observation.signature == signature
                && observation.commitment == ChainCommitment::Finalized
                && observation.slot > 0,
            "ordinary final observation identity or finality invalid"
        );
        Ok(Self {
            schema_version: 1,
            signature: signature.into(),
            commitment: "finalized".into(),
            slot: observation.slot,
            error: observation
                .error
                .as_deref()
                .map(crate::sanitize::redact_urls),
        })
    }

    pub(in crate::adapters::persistence) fn status(&self) -> TradeStatus {
        if self.error.is_some() {
            TradeStatus::Failed
        } else {
            TradeStatus::Confirmed
        }
    }

    pub(in crate::adapters::persistence) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(in crate::adapters::persistence) fn decode(json: &str, signature: &str) -> Result<Self> {
        ensure!(
            json.len() <= 65_536,
            "ordinary final observation exceeds limit"
        );
        let value: Self = serde_json::from_str(json)?;
        ensure!(
            value.schema_version == 1
                && value.commitment == "finalized"
                && value.slot > 0
                && value.signature == signature
                && value.error == value.error.as_deref().map(crate::sanitize::redact_urls),
            "ordinary final observation or parent evidence mismatch"
        );
        Ok(value)
    }
}

pub(super) fn validate(row: &SqliteRow, parent: &TradeRecordV1) -> Result<()> {
    let Some(json) = row.try_get::<Option<String>, _>("final_observation_json")? else {
        return Ok(());
    };
    ensure!(
        json.len() <= 65_536,
        "ordinary final observation exceeds limit"
    );
    let observation: FinalObservationV1 = serde_json::from_str(&json)?;
    let signed = read_signature(row, parent)?;
    ensure!(
        observation.schema_version == 1
            && observation.commitment == "finalized"
            && observation.slot > 0
            && observation.signature == signed.signature
            && observation.error
                == observation
                    .error
                    .as_deref()
                    .map(crate::sanitize::redact_urls)
            && parent.status == observation.status()
            && parent.error == observation.error
            && matches!(
                row.try_get::<&str, _>("submission_stage")?,
                "signed" | "submitted" | "unknown"
            ),
        "ordinary final observation or parent evidence mismatch"
    );
    Ok(())
}

fn eligible(row: &SqliteRow, parent: &TradeRecordV1, signature: &str) -> Result<()> {
    ensure!(
        matches!(
            row.try_get::<&str, _>("submission_stage")?,
            "submitted" | "unknown"
        ) && matches!(
            parent.status,
            TradeStatus::Submitted
                | TradeStatus::Unknown
                | TradeStatus::Confirmed
                | TradeStatus::Failed
        ) && read_signature(row, parent)?.signature == signature,
        "ordinary operation cannot authorize signature observation"
    );
    Ok(())
}

impl OrdinaryReconciliationJournal for SqliteOrdinaryExecutionClaims {
    async fn capture_live_observation(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
    ) -> Result<CapturedOrdinaryObservation> {
        let mut tx = self.pool.begin().await?;
        let (row, parent) = checked(&mut tx, execution, owner).await?;
        eligible(&row, &parent, signature)?;
        let (_, captured) = super::observation_context::capture_checked(
            &mut tx,
            execution.reservation().request_id(),
        )
        .await?;
        captured.context.ensure_execution(execution)?;
        tx.commit().await?;
        Ok(captured)
    }

    #[cfg(test)]
    async fn observation_record(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
    ) -> Result<TradeRecordV1> {
        Ok(self
            .capture_live_observation(execution, owner, signature)
            .await?
            .record)
    }

    #[cfg(test)]
    async fn record_final_observation(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
        observation: &ChainObservation,
    ) -> Result<TradeRecordV1> {
        let captured = self
            .capture_live_observation(execution, owner, signature)
            .await?;
        self.record_captured_finality(&captured.context, observation)
            .await
    }
}

impl RecoveredOrdinaryObservations for SqliteOrdinaryExecutionClaims {
    async fn capture_observation(&self, request_id: &str) -> Result<CapturedOrdinaryObservation> {
        let mut tx = self.pool.begin().await?;
        let (_, captured) =
            super::observation_context::capture_checked(&mut tx, request_id).await?;
        tx.commit().await?;
        Ok(captured)
    }

    async fn read_observation(
        &self,
        context: &OrdinaryObservationContext,
    ) -> Result<TradeRecordV1> {
        let captured = self.capture_observation(context.request_id()).await?;
        context.ensure_same(&captured.context)?;
        Ok(captured.record)
    }

    async fn record_captured_finality(
        &self,
        context: &OrdinaryObservationContext,
        observation: &ChainObservation,
    ) -> Result<TradeRecordV1> {
        if context.operation() == crate::application::ordinary_claims::ClaimOperation::SdkTrade {
            return crate::adapters::persistence::ordinary_sdk_preparation::observation::record_finality(
                self, context, observation,
            ).await;
        }
        let signature = context.signature();
        let evidence = FinalObservationV1::new(signature, observation)?;
        let json = metadata(&evidence)?;
        let status = if evidence.error.is_some() {
            "failed"
        } else {
            "confirmed"
        };
        let mut tx = self.pool.begin().await?;
        // Acquire the writer lock before reading; a duplicate cannot replace finalized evidence.
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage=submission_stage WHERE request_id=? AND owner_nonce IS NOT NULL AND request_hash=? AND signed_signature=? AND submission_stage IN ('signed','submitted','unknown')")
            .bind(context.request_id())
            .bind(context.request_hash()).bind(signature)
            .execute(&mut *tx).await?.rows_affected())?;
        let (row, parent) = super::observation_context::checked_context(&mut tx, context).await?;
        if let Some(previous) = row.try_get::<Option<String>, _>("final_observation_json")? {
            ensure!(previous == json, "ordinary finalized observation conflict");
            tx.commit().await?;
            return Ok(parent);
        }
        if matches!(parent.status, TradeStatus::Confirmed | TradeStatus::Failed) {
            // Preserve a terminal result written by another authority; do not invent its evidence.
            tx.commit().await?;
            return Ok(parent);
        }
        one(sqlx::query("UPDATE ordinary_execution_claims SET final_observation_json=? WHERE request_id=? AND owner_nonce IS NOT NULL AND request_hash=? AND signed_signature=? AND final_observation_json IS NULL AND submission_stage IN ('signed','submitted','unknown')")
            .bind(&json).bind(context.request_id())
            .bind(context.request_hash()).bind(signature)
            .execute(&mut *tx).await?.rows_affected())?;
        one(sqlx::query("UPDATE trades SET status=?,error=?,updated_at_ms=? WHERE request_id=? AND request_hash=? AND signature=? AND status IN ('submitting','submitted','unknown')")
            .bind(status).bind(&evidence.error).bind(now()?)
            .bind(context.request_id()).bind(context.request_hash())
            .bind(signature).execute(&mut *tx).await?.rows_affected())?;
        let (read, parent) = super::observation_context::checked_context(&mut tx, context).await?;
        ensure!(
            read.try_get::<Option<&str>, _>("final_observation_json")? == Some(json.as_str())
                && parent.status == evidence.status()
                && parent.error == evidence.error,
            "ordinary final observation readback mismatch"
        );
        tx.commit().await?;
        self.read_observation(context).await
    }
}
