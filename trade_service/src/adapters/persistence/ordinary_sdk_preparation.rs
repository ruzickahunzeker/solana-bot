use crate::{
    adapters::{
        persistence::{
            ordinary_claims::SqliteOrdinaryExecutionClaims, trade_snapshot::verify_stored_identity,
        },
        sdk::damm_sell_message::validate_candidate,
    },
    application::{
        ordinary_claims::{ensure_parent, ClaimOperation},
        ports::ordinary_claims::ExecutionOwner,
        sdk_preparation::{validate_origin, StoredSdkCandidate},
        trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::{sdk_candidate::SdkCandidateMetadata, trade::TradeStatus},
    persistence::row_to_record,
};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteRow, Row, Sqlite, Transaction, ValueRef};

mod submission;

fn sha(message: &[u8]) -> String {
    format!("{:x}", Sha256::digest(message))
}

pub(super) fn validate_sdk_row(row: &SqliteRow) -> Result<Option<StoredSdkCandidate>> {
    ensure!(
        row.try_get::<i64, _>("schema_version")? == 1
            && row.try_get::<&str, _>("operation")? == "ordinary-sdk-trade",
        "SDK candidate schema or operation mismatch"
    );
    let nonce = row
        .try_get::<Option<&str>, _>("owner_nonce")?
        .context("SDK claim owner is missing")?;
    ensure!(
        nonce.len() == 32 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "SDK claim owner is invalid"
    );
    for column in [
        "bound_quote_json",
        "preflight_json",
        "final_observation_json",
    ] {
        ensure!(
            row.try_get_raw(column)?.is_null(),
            "SDK candidate cannot reuse quote or final metadata"
        );
    }
    let stage = row.try_get::<&str, _>("submission_stage")?;
    let json: Option<&str> = row.try_get("sdk_prepared_json")?;
    let message: Option<Vec<u8>> = row.try_get("unsigned_message")?;
    let digest: Option<&str> = row.try_get("unsigned_sha256")?;
    let stored = match (stage, json, message, digest) {
        ("claimed", None, None, None) => None,
        (
            "prepared" | "signing" | "signed" | "broadcasting" | "submitted" | "unknown",
            Some(json),
            Some(message),
            Some(digest),
        ) => {
            ensure!(
                digest == sha(&message),
                "SDK candidate stored message digest mismatch"
            );
            let metadata = SdkCandidateMetadata::decode(json, &message)?;
            ensure!(
                metadata.encode(&message)? == json,
                "SDK candidate metadata is not the original canonical record"
            );
            validate_candidate(&metadata, &message)?;
            Some(StoredSdkCandidate { metadata, message })
        }
        _ => anyhow::bail!("SDK candidate stage or partial metadata is invalid"),
    };
    submission::validate_fields(row, stored.as_ref())?;
    Ok(stored)
}

pub(super) fn validate_sdk_parent(row: &SqliteRow, parent: &TradeRecordV1) -> Result<()> {
    submission::validate_parent(row, parent)
}

async fn checked(
    tx: &mut Transaction<'_, Sqlite>,
    execution: &AdmittedTrade,
    owner: &ExecutionOwner,
) -> Result<(SqliteRow, TradeRecordV1, Option<StoredSdkCandidate>)> {
    owner.ensure_binding(execution)?;
    ensure!(
        owner.operation() == ClaimOperation::SdkTrade,
        "SDK preparation has a different operation owner"
    );
    let a = execution.reservation();
    let raw = sqlx::query("SELECT * FROM trades WHERE request_id=?")
        .bind(a.request_id())
        .fetch_one(&mut **tx)
        .await?;
    ensure!(
        raw.try_get::<&str, _>("idempotency_key")? == a.idempotency_key()
            && raw.try_get::<&str, _>("request_hash")? == a.request_hash(),
        "SDK parent key or hash mismatch"
    );
    let json: String = raw.try_get("request_json")?;
    let parent = row_to_record(raw)?;
    ensure_parent(execution, &parent)?;
    ensure!(
        verify_stored_identity(&parent, &json, a.idempotency_key(), false)?,
        "SDK parent payment branch mismatch"
    );
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(a.request_id())
        .fetch_one(&mut **tx)
        .await?;
    ensure!(
        row.try_get::<i64, _>("schema_version")? == 1
            && row.try_get::<&str, _>("operation")? == "ordinary-sdk-trade"
            && row.try_get::<&str, _>("request_hash")? == a.request_hash()
            && row.try_get::<Option<&str>, _>("owner_nonce")? == Some(owner.nonce())
            && row.try_get::<i64, _>("created_at_ms")? >= 0,
        "SDK claim identity mismatch"
    );
    let stored = validate_sdk_row(&row)?;
    if let Some(candidate) = &stored {
        validate_origin(execution, &candidate.metadata, &candidate.message)?;
    }
    validate_sdk_parent(&row, &parent)?;
    Ok((row, parent, stored))
}

impl SqliteOrdinaryExecutionClaims {
    /// Only records a nominal candidate; it grants no business or signing authorization.
    pub(crate) async fn prepare_sdk_candidate(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> Result<()> {
        owner.ensure_binding(execution)?;
        validate_origin(execution, metadata, message)?;
        validate_candidate(metadata, message)?;
        let json = metadata.encode(message)?;
        let mut tx = self.pool.begin().await?;
        // Obtain the write lock before reading without overwriting an existing candidate.
        sqlx::query("UPDATE ordinary_execution_claims SET request_id=request_id WHERE 0")
            .execute(&mut *tx)
            .await?;
        let (_, parent, stored) = checked(&mut tx, execution, owner).await?;
        ensure!(
            parent.status == TradeStatus::BuildingTx && stored.is_none(),
            "SDK parent or candidate is not eligible for preparation"
        );
        let count = sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='prepared',sdk_prepared_json=?,unsigned_message=?,unsigned_sha256=? WHERE request_id=? AND operation='ordinary-sdk-trade' AND request_hash=? AND owner_nonce=? AND submission_stage='claimed' AND sdk_prepared_json IS NULL AND unsigned_message IS NULL AND unsigned_sha256 IS NULL AND signed_signature IS NULL")
            .bind(&json).bind(message).bind(sha(message)).bind(execution.reservation().request_id()).bind(execution.reservation().request_hash()).bind(owner.nonce()).execute(&mut *tx).await?.rows_affected();
        ensure!(count == 1, "SDK preparation state changed; do not retry");
        let (row, parent, stored) = checked(&mut tx, execution, owner).await?;
        let stored = stored.context("SDK prepared candidate missing on readback")?;
        ensure!(
            parent.status == TradeStatus::BuildingTx
                && stored.metadata == *metadata
                && stored.message == message
                && row.try_get::<Option<&str>, _>("sdk_prepared_json")? == Some(json.as_str()),
            "SDK candidate write readback mismatch"
        );
        tx.commit().await?;
        Ok(())
    }

    /// Independent readback, not a sign/broadcast permit or a recovered execution owner.
    pub(crate) async fn read_sdk_candidate(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<StoredSdkCandidate> {
        let mut tx = self.pool.begin().await?;
        let (_, parent, stored) = checked(&mut tx, execution, owner).await?;
        ensure!(
            matches!(
                parent.status,
                TradeStatus::BuildingTx | TradeStatus::Unknown
            ),
            "SDK prepared parent status is invalid"
        );
        let stored = stored.context("SDK prepared candidate is missing")?;
        tx.commit().await?;
        Ok(stored)
    }
}

#[cfg(test)]
mod tests;
