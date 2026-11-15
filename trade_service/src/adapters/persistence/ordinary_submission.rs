use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteRow, Row, Sqlite, Transaction};

use crate::{
    adapters::persistence::{
        ordinary_claims::SqliteOrdinaryExecutionClaims, trade_snapshot::verify_stored_identity,
    },
    application::{
        ordinary_claims::{ensure_parent, ensure_scope},
        ports::{
            ordinary_claims::ExecutionOwner, ordinary_journal::TradeJournalMode,
            ordinary_submission::OrdinarySubmissionJournal,
        },
        trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::{
        quote::{BoundQuote, QuotePreflight},
        submission::SignedSubmission,
        trade::TradeStatus,
    },
    persistence::row_to_record,
};

pub(super) mod final_observation;
mod observation_context;

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn now() -> Result<i64> {
    Ok(i64::try_from(crate::state::now_ms())?)
}
fn one(count: u64) -> Result<()> {
    ensure!(
        count == 1,
        "ordinary submission state changed; do not retry"
    );
    Ok(())
}
fn metadata<T: serde::Serialize>(value: &T) -> Result<String> {
    let json = serde_json::to_string(value)?;
    ensure!(
        json.len() <= 65_536,
        "ordinary submission metadata exceeds limit"
    );
    Ok(json)
}
fn message(bytes: &[u8]) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= 1232,
        "ordinary transaction message size invalid"
    );
    Ok(())
}

fn validate_metadata(row: &SqliteRow, execution: &AdmittedTrade) -> Result<()> {
    validate_metadata_bound(row, |bound| {
        crate::application::ordinary_submission::validate_bound(execution, bound)
    })
}

fn validate_metadata_bound(
    row: &SqliteRow,
    validate_bound: impl Fn(&BoundQuote) -> Result<()>,
) -> Result<()> {
    ensure!(
        row.try_get::<Option<&str>, _>("sdk_prepared_json")?
            .is_none(),
        "aggregate journal cannot contain SDK candidate metadata"
    );
    let stage: &str = row.try_get("submission_stage")?;
    ensure!(
        [
            "claimed",
            "preparing",
            "prepared",
            "signing",
            "signed",
            "submitted",
            "unknown",
            "failed"
        ]
        .contains(&stage),
        "ordinary submission stage invalid"
    );
    let quote: Option<String> = row.try_get("bound_quote_json")?;
    let report: Option<String> = row.try_get("preflight_json")?;
    let unsigned: Option<Vec<u8>> = row.try_get("unsigned_message")?;
    let unsigned_sha: Option<String> = row.try_get("unsigned_sha256")?;
    match (quote, report, unsigned, unsigned_sha) {
        (None, None, None, None) => ensure!(
            ["claimed", "preparing", "failed"].contains(&stage),
            "ordinary prepared evidence missing"
        ),
        (Some(quote), Some(report), Some(unsigned), Some(hash)) => {
            ensure!(
                quote.len() <= 65_536 && report.len() <= 65_536,
                "ordinary metadata exceeds limit"
            );
            let bound: BoundQuote = serde_json::from_str(&quote)?;
            let preflight: QuotePreflight = serde_json::from_str(&report)?;
            validate_bound(&bound)?;
            crate::application::ordinary_submission::validate_preflight(&bound, &preflight)?;
            message(&unsigned)?;
            ensure!(
                hash == sha(&unsigned),
                "ordinary unsigned evidence hash mismatch"
            );
        }
        _ => anyhow::bail!("ordinary partial prepared evidence"),
    }
    let signed: Option<&str> = row.try_get("signed_signature")?;
    let bytes: Option<Vec<u8>> = row.try_get("signed_transaction")?;
    let hash: Option<&str> = row.try_get("signed_sha256")?;
    match (signed, bytes, hash) {
        (None, None, None) => ensure!(
            !["signed", "submitted", "unknown"].contains(&stage),
            "ordinary signed evidence missing"
        ),
        (Some(signature), Some(bytes), Some(hash)) => {
            ensure!(
                ["signed", "submitted", "unknown"].contains(&stage) && !signature.is_empty(),
                "ordinary signed stage inconsistent"
            );
            message(&bytes)?;
            ensure!(
                hash == sha(&bytes),
                "ordinary signed evidence hash mismatch"
            );
        }
        _ => anyhow::bail!("ordinary partial signed evidence"),
    }
    Ok(())
}

async fn checked(
    tx: &mut Transaction<'_, Sqlite>,
    execution: &AdmittedTrade,
    owner: &ExecutionOwner,
) -> Result<(SqliteRow, TradeRecordV1)> {
    owner.ensure_binding(execution)?;
    ensure_scope(execution, TradeJournalMode::Durable)?;
    ensure!(
        owner.mode() == TradeJournalMode::Durable,
        "ordinary owner is not durable"
    );
    let anchor = execution.reservation();
    let parent = sqlx::query("SELECT * FROM trades WHERE request_id=?")
        .bind(anchor.request_id())
        .fetch_optional(&mut **tx)
        .await?
        .context("ordinary parent missing")?;
    ensure!(
        parent.try_get::<&str, _>("idempotency_key")? == anchor.idempotency_key()
            && parent.try_get::<&str, _>("request_hash")? == anchor.request_hash(),
        "ordinary parent key or hash changed"
    );
    let json: String = parent.try_get("request_json")?;
    let parent = row_to_record(parent)?;
    ensure_parent(execution, &parent)?;
    ensure!(
        verify_stored_identity(&parent, &json, anchor.idempotency_key(), true)?,
        "ordinary payment branch changed"
    );
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(anchor.request_id())
        .fetch_optional(&mut **tx)
        .await?
        .context("ordinary claim missing")?;
    ensure!(
        row.try_get::<i64, _>("schema_version")? == 1
            && row.try_get::<&str, _>("operation")? == "ordinary-sol-buy"
            && row.try_get::<&str, _>("request_hash")? == anchor.request_hash()
            && row.try_get::<Option<&str>, _>("owner_nonce")? == Some(owner.nonce()),
        "ordinary claim owner or identity changed"
    );
    ensure!(
        row.try_get::<i64, _>("created_at_ms")? >= 0,
        "ordinary claim timestamp invalid"
    );
    validate_metadata(&row, execution)?;
    final_observation::validate(&row, &parent)?;
    Ok((row, parent))
}

fn read_signature(row: &SqliteRow, parent: &TradeRecordV1) -> Result<SignedSubmission> {
    let signature: String = row
        .try_get::<Option<String>, _>("signed_signature")?
        .context("ordinary signature missing")?;
    let bytes: Vec<u8> = row
        .try_get::<Option<Vec<u8>>, _>("signed_transaction")?
        .context("ordinary transaction bytes missing")?;
    message(&bytes)?;
    ensure!(
        !signature.is_empty()
            && parent.signature.as_deref() == Some(signature.as_str())
            && row.try_get::<Option<&str>, _>("signed_sha256")? == Some(sha(&bytes).as_str()),
        "ordinary signed evidence mismatch"
    );
    Ok(SignedSubmission { signature, bytes })
}

impl OrdinarySubmissionJournal for SqliteOrdinaryExecutionClaims {
    async fn begin(&self, execution: &AdmittedTrade, owner: &ExecutionOwner) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='preparing' WHERE request_id=? AND owner_nonce=? AND request_hash=? AND submission_stage='claimed' AND signed_signature IS NULL")
            .bind(execution.reservation().request_id()).bind(owner.nonce()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        let (_, parent) = checked(&mut tx, execution, owner).await?;
        ensure!(
            parent.status == TradeStatus::BuildingTx && parent.signature.is_none(),
            "ordinary parent cannot begin preparation"
        );
        tx.commit().await?;
        Ok(())
    }

    async fn prepare(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        bound: &BoundQuote,
        preflight: &QuotePreflight,
        unsigned: &[u8],
    ) -> Result<()> {
        crate::application::ordinary_submission::validate_bound(execution, bound)?;
        crate::application::ordinary_submission::validate_preflight(bound, preflight)?;
        message(unsigned)?;
        let mut tx = self.pool.begin().await?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='prepared',bound_quote_json=?,preflight_json=?,unsigned_message=?,unsigned_sha256=? WHERE request_id=? AND owner_nonce=? AND submission_stage='preparing' AND signed_signature IS NULL")
            .bind(metadata(bound)?).bind(metadata(preflight)?).bind(unsigned).bind(sha(unsigned))
            .bind(execution.reservation().request_id()).bind(owner.nonce()).execute(&mut *tx).await?.rows_affected())?;
        let (_, parent) = checked(&mut tx, execution, owner).await?;
        ensure!(
            parent.status == TradeStatus::BuildingTx && parent.signature.is_none(),
            "ordinary parent cannot prepare"
        );
        tx.commit().await?;
        Ok(())
    }

    async fn authorize_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        bound: &BoundQuote,
        preflight: &QuotePreflight,
        unsigned: &[u8],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='signing' WHERE request_id=? AND owner_nonce=? AND submission_stage='prepared' AND signed_signature IS NULL")
            .bind(execution.reservation().request_id()).bind(owner.nonce()).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent) = checked(&mut tx, execution, owner).await?;
        ensure!(
            parent.status == TradeStatus::BuildingTx
                && parent.signature.is_none()
                && row.try_get::<Option<&str>, _>("bound_quote_json")?
                    == Some(metadata(bound)?.as_str())
                && row.try_get::<Option<&str>, _>("preflight_json")?
                    == Some(metadata(preflight)?.as_str())
                && row
                    .try_get::<Option<Vec<u8>>, _>("unsigned_message")?
                    .as_deref()
                    == Some(unsigned)
                && row.try_get::<Option<&str>, _>("unsigned_sha256")?
                    == Some(sha(unsigned).as_str()),
            "ordinary prepared readback mismatch"
        );
        ensure!(
            u128::from(bound.expires_at_ms) > crate::state::now_ms(),
            "ordinary quote expired before signing"
        );
        tx.commit().await?;
        Ok(())
    }

    async fn record_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
    ) -> Result<()> {
        message(&signed.bytes)?;
        ensure!(!signed.signature.is_empty(), "ordinary signature missing");
        let mut tx = self.pool.begin().await?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='signed',signed_signature=?,signed_transaction=?,signed_sha256=? WHERE request_id=? AND owner_nonce=? AND submission_stage='signing' AND signed_signature IS NULL AND unsigned_message IS NOT NULL")
            .bind(&signed.signature).bind(&signed.bytes).bind(sha(&signed.bytes)).bind(execution.reservation().request_id()).bind(owner.nonce()).execute(&mut *tx).await?.rows_affected())?;
        one(sqlx::query("UPDATE trades SET status='submitting',signature=?,error=NULL,updated_at_ms=? WHERE request_id=? AND request_hash=? AND status='building_tx' AND signature IS NULL")
            .bind(&signed.signature).bind(now()?).bind(execution.reservation().request_id()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent) = checked(&mut tx, execution, owner).await?;
        let read = read_signature(&row, &parent)?;
        ensure!(
            read.signature == signed.signature && read.bytes == signed.bytes,
            "ordinary signed write readback mismatch"
        );
        tx.commit().await?;
        Ok(())
    }

    async fn read_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<SignedSubmission> {
        let mut tx = self.pool.begin().await?;
        let (row, parent) = checked(&mut tx, execution, owner).await?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "signed"
                && parent.status == TradeStatus::Submitting,
            "ordinary signed record cannot authorize broadcast"
        );
        let signed = read_signature(&row, &parent)?;
        tx.commit().await?;
        Ok(signed)
    }

    async fn record_broadcast(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
        submitted: bool,
        error: Option<&str>,
    ) -> Result<()> {
        let stage = if submitted { "submitted" } else { "unknown" };
        let error = error.map(crate::sanitize::redact_urls);
        let mut tx = self.pool.begin().await?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage=?,submission_error=? WHERE request_id=? AND owner_nonce=? AND submission_stage='signed' AND signed_signature=? AND signed_sha256=?")
            .bind(stage).bind(&error).bind(execution.reservation().request_id()).bind(owner.nonce()).bind(&signed.signature).bind(sha(&signed.bytes)).execute(&mut *tx).await?.rows_affected())?;
        one(sqlx::query("UPDATE trades SET status=?,error=?,updated_at_ms=? WHERE request_id=? AND request_hash=? AND status='submitting' AND signature=?")
            .bind(stage).bind(&error).bind(now()?).bind(execution.reservation().request_id()).bind(execution.reservation().request_hash()).bind(&signed.signature).execute(&mut *tx).await?.rows_affected())?;
        let (row, parent) = checked(&mut tx, execution, owner).await?;
        let read = read_signature(&row, &parent)?;
        ensure!(
            read.bytes == signed.bytes && read.signature == signed.signature,
            "ordinary broadcast evidence mismatch"
        );
        tx.commit().await?;
        Ok(())
    }

    async fn fail_before_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        error: &str,
    ) -> Result<()> {
        let error = crate::sanitize::redact_urls(error);
        let mut tx = self.pool.begin().await?;
        one(sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='failed',submission_error=? WHERE request_id=? AND owner_nonce=? AND submission_stage IN ('preparing','prepared','signing') AND signed_signature IS NULL AND signed_transaction IS NULL")
            .bind(&error).bind(execution.reservation().request_id()).bind(owner.nonce()).execute(&mut *tx).await?.rows_affected())?;
        one(sqlx::query("UPDATE trades SET status='failed',error=?,updated_at_ms=? WHERE request_id=? AND request_hash=? AND status IN ('accepted','building_tx') AND signature IS NULL")
            .bind(&error).bind(now()?).bind(execution.reservation().request_id()).bind(execution.reservation().request_hash()).execute(&mut *tx).await?.rows_affected())?;
        checked(&mut tx, execution, owner).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn observe(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<TradeRecordV1> {
        let mut tx = self.pool.begin().await?;
        let (row, parent) = checked(&mut tx, execution, owner).await?;
        let stage: &str = row.try_get("submission_stage")?;
        ensure!(
            [
                "claimed",
                "preparing",
                "prepared",
                "signing",
                "signed",
                "submitted",
                "unknown",
                "failed"
            ]
            .contains(&stage),
            "ordinary submission stage invalid"
        );
        if row
            .try_get::<Option<&str>, _>("signed_signature")?
            .is_some()
        {
            read_signature(&row, &parent)?;
        } else {
            ensure!(
                row.try_get::<Option<Vec<u8>>, _>("signed_transaction")?
                    .is_none()
                    && parent.signature.is_none(),
                "ordinary partial signature evidence"
            );
        }
        tx.commit().await?;
        Ok(parent)
    }
}
