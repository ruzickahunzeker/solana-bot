use super::*;
use crate::{
    adapters::sdk::damm_sell_message::validate_signed_candidate,
    domain::submission::SignedSubmission,
};

pub(super) fn validate_fields(
    row: &SqliteRow,
    candidate: Option<&StoredSdkCandidate>,
) -> Result<()> {
    let stage: &str = row.try_get("submission_stage")?;
    let signature: Option<String> = row.try_get("signed_signature")?;
    let bytes: Option<Vec<u8>> = row.try_get("signed_transaction")?;
    let digest: Option<&str> = row.try_get("signed_sha256")?;
    match (stage, signature, bytes, digest) {
        ("claimed" | "prepared" | "signing" | "failed" | "rejected", None, None, None) => {}
        (
            "signed" | "broadcasting" | "submitted" | "unknown",
            Some(signature),
            Some(bytes),
            Some(digest),
        ) => {
            ensure!(digest == sha(&bytes), "SDK signed digest mismatch");
            let candidate = candidate.context("SDK signed candidate is missing")?;
            validate_signed_candidate(
                &candidate.metadata,
                &candidate.message,
                &SignedSubmission { signature, bytes },
            )?;
        }
        _ => anyhow::bail!("SDK signed stage or partial evidence is invalid"),
    }
    let error: Option<&str> = row.try_get("submission_error")?;
    ensure!(
        ["unknown", "failed", "rejected"].contains(&stage) || error.is_none(),
        "SDK submission error belongs to a different stage"
    );
    if let Some(error) = error {
        ensure!(
            error.len() <= 65_536 && crate::sanitize::redact_urls(error) == error,
            "SDK submission error is oversized or not redacted"
        );
    }
    ensure!(
        !["failed", "rejected"].contains(&stage) || error.is_some(),
        "SDK failed journal requires pre-sign error evidence"
    );
    Ok(())
}

pub(super) fn validate_parent(row: &SqliteRow, parent: &TradeRecordV1) -> Result<()> {
    if let Some(evidence) = super::observation::validate_finality_fields(row)? {
        ensure!(
            parent.status == evidence.status()
                && parent.error.as_deref() == evidence.error()
                && parent.signature.as_deref()
                    == row.try_get::<Option<&str>, _>("signed_signature")?,
            "SDK finalized parent differs from its original evidence"
        );
        return Ok(());
    }
    let stage: &str = row.try_get("submission_stage")?;
    let expected = match stage {
        "claimed" => matches!(
            parent.status,
            TradeStatus::Accepted | TradeStatus::BuildingTx | TradeStatus::Unknown
        ),
        "prepared" | "signing" => matches!(
            parent.status,
            TradeStatus::BuildingTx | TradeStatus::Unknown
        ),
        "signed" | "broadcasting" => matches!(
            parent.status,
            TradeStatus::Submitting | TradeStatus::Unknown
        ),
        "submitted" => matches!(parent.status, TradeStatus::Submitted | TradeStatus::Unknown),
        "unknown" => parent.status == TradeStatus::Unknown,
        "failed" => {
            parent.status == TradeStatus::Failed
                && parent.error.as_deref() == row.try_get::<Option<&str>, _>("submission_error")?
        }
        "rejected" => {
            parent.status == TradeStatus::Rejected
                && parent.error.as_deref() == row.try_get::<Option<&str>, _>("submission_error")?
        }
        _ => false,
    };
    ensure!(expected, "SDK parent status differs from its journal stage");
    ensure!(
        parent.signature.as_deref() == row.try_get::<Option<&str>, _>("signed_signature")?,
        "SDK parent signature differs from its journal"
    );
    Ok(())
}

fn one(count: u64) -> Result<()> {
    ensure!(count == 1, "SDK submission state changed; do not retry");
    Ok(())
}

async fn write_lock(tx: &mut Transaction<'_, Sqlite>) -> Result<()> {
    sqlx::query("UPDATE ordinary_execution_claims SET request_id=request_id WHERE 0")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn same_candidate(
    stored: Option<StoredSdkCandidate>,
    metadata: &SdkCandidateMetadata,
    message: &[u8],
) -> Result<()> {
    let stored = stored.context("SDK prepared candidate missing")?;
    ensure!(
        stored.metadata == *metadata && stored.message == message,
        "SDK prepared candidate differs from the original"
    );
    Ok(())
}

fn signed_from_row(row: &SqliteRow) -> Result<SignedSubmission> {
    Ok(SignedSubmission {
        signature: row
            .try_get::<Option<String>, _>("signed_signature")?
            .context("SDK signature is missing")?,
        bytes: row
            .try_get::<Option<Vec<u8>>, _>("signed_transaction")?
            .context("SDK signed bytes are missing")?,
    })
}

impl SqliteOrdinaryExecutionClaims {
    pub(crate) async fn fail_sdk_before_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        error: &str,
    ) -> Result<()> {
        self.terminal_sdk_before_sign(execution, owner, error, false)
            .await
    }

    pub(crate) async fn reject_sdk_before_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        error: &str,
    ) -> Result<()> {
        self.terminal_sdk_before_sign(execution, owner, error, true)
            .await
    }

    async fn terminal_sdk_before_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        error: &str,
        rejected: bool,
    ) -> Result<()> {
        let error = crate::sanitize::redact_urls(error);
        let stage = if rejected { "rejected" } else { "failed" };
        let status = if rejected {
            TradeStatus::Rejected
        } else {
            TradeStatus::Failed
        };
        ensure!(error.len() <= 65_536, "SDK pre-sign error exceeds limit");
        let mut tx = self.pool.begin().await?;
        write_lock(&mut tx).await?;
        let (row, parent, _) = checked(&mut tx, execution, owner).await?;
        ensure!(
            ["claimed", "prepared"].contains(&row.try_get::<&str, _>("submission_stage")?)
                && parent.status == TradeStatus::BuildingTx,
            "SDK signing may have started; do not downgrade to a pre-sign failure"
        );
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage=?,submission_error=? WHERE request_id=? AND operation='ordinary-sdk-trade' AND owner_nonce=? AND request_hash=? AND submission_stage IN ('claimed','prepared') AND signed_signature IS NULL AND signed_transaction IS NULL")
            .bind(stage).bind(&error).bind(execution.reservation().request_id()).bind(owner.nonce()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        one(sqlx::query("UPDATE trades SET status=?,error=?,updated_at_ms=? WHERE request_id=? AND request_hash=? AND status='building_tx' AND signature IS NULL")
            .bind(stage).bind(&error).bind(i64::try_from(crate::state::now_ms())?).bind(execution.reservation().request_id()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent, _) = checked(&mut tx, execution, owner).await?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == stage
                && parent.status == status
                && parent.error.as_deref() == Some(error.as_str()),
            "SDK pre-sign failure readback mismatch"
        );
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn observe_sdk_submission(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<TradeRecordV1> {
        let mut tx = self.pool.begin().await?;
        let (_, parent, _) = checked(&mut tx, execution, owner).await?;
        tx.commit().await?;
        Ok(parent)
    }
    /// Single-use journal transition only. The caller still needs live business authorization.
    /// A stored nominal candidate cannot authorize a signer or create a recovery owner.
    pub(crate) async fn begin_sdk_signing(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        write_lock(&mut tx).await?;
        let (row, parent, candidate) = checked(&mut tx, execution, owner).await?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "prepared"
                && parent.status == TradeStatus::BuildingTx,
            "SDK original candidate cannot begin signing"
        );
        same_candidate(candidate, metadata, message)?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='signing' WHERE request_id=? AND operation='ordinary-sdk-trade' AND owner_nonce=? AND request_hash=? AND submission_stage='prepared' AND signed_signature IS NULL")
            .bind(execution.reservation().request_id()).bind(owner.nonce()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent, candidate) = checked(&mut tx, execution, owner).await?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "signing"
                && parent.status == TradeStatus::BuildingTx,
            "SDK signing transition readback mismatch"
        );
        same_candidate(candidate, metadata, message)?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn record_sdk_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        write_lock(&mut tx).await?;
        let (row, parent, candidate) = checked(&mut tx, execution, owner).await?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "signing"
                && parent.status == TradeStatus::BuildingTx,
            "SDK signature has no original signing transition"
        );
        let candidate = candidate.context("SDK signed candidate is missing")?;
        validate_signed_candidate(&candidate.metadata, &candidate.message, signed)?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='signed',signed_signature=?,signed_transaction=?,signed_sha256=? WHERE request_id=? AND operation='ordinary-sdk-trade' AND owner_nonce=? AND request_hash=? AND submission_stage='signing' AND signed_signature IS NULL")
            .bind(&signed.signature).bind(&signed.bytes).bind(sha(&signed.bytes)).bind(execution.reservation().request_id()).bind(owner.nonce()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        one(sqlx::query("UPDATE trades SET status='submitting',signature=?,error=NULL,updated_at_ms=? WHERE request_id=? AND request_hash=? AND status='building_tx' AND signature IS NULL")
            .bind(&signed.signature).bind(i64::try_from(crate::state::now_ms())?).bind(execution.reservation().request_id()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent, _) = checked(&mut tx, execution, owner).await?;
        let stored = signed_from_row(&row)?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "signed"
                && parent.status == TradeStatus::Submitting
                && stored.signature == signed.signature
                && stored.bytes == signed.bytes,
            "SDK signed write readback mismatch"
        );
        tx.commit().await?;
        Ok(())
    }

    /// Read-only evidence, not a broadcast permit. Recovery cannot reconstruct the owner.
    pub(crate) async fn read_sdk_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<SignedSubmission> {
        let mut tx = self.pool.begin().await?;
        let (row, _, _) = checked(&mut tx, execution, owner).await?;
        let signed = signed_from_row(&row)?;
        tx.commit().await?;
        Ok(signed)
    }

    /// Atomically consumes this original owner's single broadcast opportunity.
    pub(crate) async fn take_sdk_broadcast(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        expected: &SignedSubmission,
    ) -> Result<SignedSubmission> {
        let mut tx = self.pool.begin().await?;
        write_lock(&mut tx).await?;
        let (row, parent, _) = checked(&mut tx, execution, owner).await?;
        let signed = signed_from_row(&row)?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "signed"
                && parent.status == TradeStatus::Submitting
                && signed.signature == expected.signature
                && signed.bytes == expected.bytes,
            "SDK signed bytes cannot be taken for broadcast"
        );
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='broadcasting' WHERE request_id=? AND operation='ordinary-sdk-trade' AND owner_nonce=? AND request_hash=? AND submission_stage='signed' AND signed_signature=? AND signed_sha256=?")
            .bind(execution.reservation().request_id()).bind(owner.nonce()).bind(execution.reservation().request_hash()).bind(&signed.signature).bind(sha(&signed.bytes)).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent, _) = checked(&mut tx, execution, owner).await?;
        let read = signed_from_row(&row)?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "broadcasting"
                && parent.status == TradeStatus::Submitting
                && read.signature == signed.signature
                && read.bytes == signed.bytes,
            "SDK broadcast consumption readback mismatch"
        );
        tx.commit().await?;
        Ok(signed)
    }

    pub(crate) async fn record_sdk_broadcast(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
        submitted: bool,
        error: Option<&str>,
    ) -> Result<()> {
        ensure!(
            !submitted || error.is_none(),
            "SDK successful receipt cannot contain a submission error"
        );
        let stage = if submitted { "submitted" } else { "unknown" };
        let error = error.map(crate::sanitize::redact_urls);
        ensure!(
            error.as_ref().is_none_or(|s| s.len() <= 65_536),
            "SDK submission error exceeds limit"
        );
        let mut tx = self.pool.begin().await?;
        write_lock(&mut tx).await?;
        let (row, parent, _) = checked(&mut tx, execution, owner).await?;
        let stored = signed_from_row(&row)?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "broadcasting"
                && parent.status == TradeStatus::Submitting
                && stored.signature == signed.signature
                && stored.bytes == signed.bytes,
            "SDK receipt differs from the consumed original bytes"
        );
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage=?,submission_error=? WHERE request_id=? AND operation='ordinary-sdk-trade' AND owner_nonce=? AND request_hash=? AND submission_stage='broadcasting' AND signed_signature=? AND signed_sha256=?")
            .bind(stage).bind(&error).bind(execution.reservation().request_id()).bind(owner.nonce()).bind(execution.reservation().request_hash()).bind(&signed.signature).bind(sha(&signed.bytes)).execute(&mut *tx).await?.rows_affected())?;
        one(sqlx::query("UPDATE trades SET status=?,error=?,updated_at_ms=? WHERE request_id=? AND request_hash=? AND status='submitting' AND signature=?")
            .bind(stage).bind(&error).bind(i64::try_from(crate::state::now_ms())?).bind(execution.reservation().request_id()).bind(execution.reservation().request_hash()).bind(&signed.signature).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent, _) = checked(&mut tx, execution, owner).await?;
        let read = signed_from_row(&row)?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == stage
                && parent.status
                    == if submitted {
                        TradeStatus::Submitted
                    } else {
                        TradeStatus::Unknown
                    }
                && parent.error == error
                && row.try_get::<Option<&str>, _>("submission_error")? == error.as_deref()
                && read.signature == signed.signature
                && read.bytes == signed.bytes,
            "SDK receipt write readback mismatch"
        );
        tx.commit().await?;
        Ok(())
    }
}
