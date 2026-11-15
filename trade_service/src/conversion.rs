use crate::{
    application::wsol_failure::WsolFailure,
    application::wsol_submission_types::{
        WsolQuoteSnapshot, WsolSignedOperation, WsolSubmissionInput,
    },
    domain::wsol_progress::{should_advance, WsolState, WsolUpdate},
};
use anyhow::{bail, Result};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::{str::FromStr, time::Duration};

#[cfg(test)]
pub(crate) use crate::api::dto::wsol_conversions::{
    WsolDirection, WsolQuotePreview, WsolQuoteRequest, WsolSimulationResult, WsolSubmissionResult,
};
pub use crate::api::dto::wsol_conversions::{WsolQuote, WsolSubmission};
#[cfg(test)]
pub(crate) use crate::sdk_client::wsol_conversion::UnsignedWsolTransaction;

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS conversion_quotes (
  quote_id TEXT PRIMARY KEY,
  quote_json TEXT NOT NULL,
  expires_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS conversion_submissions (
  submission_id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  fingerprint TEXT NOT NULL,
  quote_id TEXT NOT NULL,
  wallet_id TEXT NOT NULL,
  wallet_pubkey TEXT NOT NULL,
  owner_type TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  status TEXT NOT NULL,
  signature TEXT,
  signed_transaction BLOB,
  error TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_conversion_submissions_owner
  ON conversion_submissions(owner_type, owner_id, wallet_id, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_conversion_one_pending_per_wallet
  ON conversion_submissions(wallet_id)
  WHERE status IN ('reserved', 'signed', 'submitted', 'unknown');
"#;

pub use crate::application::wsol_submission_types::WsolSubmissionRecord as ConversionRecord;

pub struct ConversionStore {
    pool: SqlitePool,
}

impl ConversionStore {
    pub async fn new(database_url: &str) -> Result<Self> {
        if let Some(path) = database_url.strip_prefix("sqlite://") {
            if let Some(parent) = std::path::Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(MIGRATION).execute(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn save_quote(&self, quote: &WsolQuote) -> Result<()> {
        let quote_json = serde_json::to_string(quote)?;
        let expires_at_ms = i64::try_from(quote.expires_at_ms)?;
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO conversion_quotes(quote_id,quote_json,expires_at_ms) VALUES(?,?,?)",
        )
        .bind(&quote.quote_id)
        .bind(&quote_json)
        .bind(expires_at_ms)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            bail!(WsolFailure::integrity(
                "WSOL quote insertion was not acknowledged"
            ));
        }
        let row = sqlx::query(
            "SELECT quote_id,quote_json,expires_at_ms FROM conversion_quotes WHERE quote_id=?",
        )
        .bind(&quote.quote_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| WsolFailure::integrity("WSOL quote missing after insertion"))?;
        let persisted = decode_quote(&row, &quote.quote_id)?;
        let persisted_json: &str = row.try_get("quote_json")?;
        if persisted != *quote || persisted_json != quote_json {
            bail!(WsolFailure::integrity(
                "WSOL quote changed during persistence"
            ));
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_quote(&self, quote_id: &str) -> Result<Option<WsolQuote>> {
        let row = sqlx::query(
            "SELECT quote_id,quote_json,expires_at_ms FROM conversion_quotes WHERE quote_id=?",
        )
        .bind(quote_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref()
            .map(|row| decode_quote(row, quote_id))
            .transpose()
    }

    #[cfg(test)]
    pub async fn reserve_live(
        &self,
        submission: &WsolSubmission,
        quote: &WsolQuote,
        now_ms: u64,
    ) -> Result<(ConversionRecord, bool)> {
        let input = submission.input();
        let quote = WsolQuoteSnapshot::from(quote);
        let fingerprint =
            crate::adapters::persistence::wsol_conversion::reservation_fingerprint(&input, &quote)?;
        self.reserve_bound(&input, &quote, &fingerprint, now_ms)
            .await
    }

    pub(crate) async fn reserve_bound(
        &self,
        input: &WsolSubmissionInput,
        quote: &WsolQuoteSnapshot,
        fingerprint: &str,
        now_ms: u64,
    ) -> Result<(ConversionRecord, bool)> {
        if input.simulate {
            bail!(WsolFailure::rejected(
                "simulation does not reserve a live idempotency key"
            ));
        }
        input.validate_quote(quote).map_err(WsolFailure::rejected)?;
        if fingerprint.is_empty() {
            bail!("WSOL reservation fingerprint is empty");
        }
        if let Some(existing) = self.get_by_key(&input.idempotency_key).await? {
            validate_reserved_record(&existing, input, quote, fingerprint)?;
            return Ok((existing, false));
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(row) =
            sqlx::query("SELECT * FROM conversion_submissions WHERE idempotency_key=?")
                .bind(&input.idempotency_key)
                .fetch_optional(&mut *tx)
                .await?
        {
            let existing = decode_record(&row)?;
            validate_reserved_record(&existing, input, quote, fingerprint)?;
            tx.commit().await?;
            return Ok((existing, false));
        }
        quote
            .validate_binding(
                &input.wallet_id,
                &quote.wallet_pubkey,
                &input.owner_type,
                &input.owner_id,
                now_ms,
            )
            .map_err(WsolFailure::rejected)?;
        let stamp = i64::try_from(now_ms)?;
        let submission_id = format!("conv_{}", uuid::Uuid::new_v4().simple());
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO conversion_submissions(submission_id,idempotency_key,fingerprint,quote_id,wallet_id,wallet_pubkey,owner_type,owner_id,status,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?,'reserved',?,?)",
        )
        .bind(&submission_id)
        .bind(&input.idempotency_key)
        .bind(fingerprint)
        .bind(&quote.quote_id)
        .bind(&quote.wallet_id)
        .bind(&quote.wallet_pubkey)
        .bind(&quote.owner_type)
        .bind(&quote.owner_id)
        .bind(stamp)
        .bind(stamp)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        let row = match sqlx::query("SELECT * FROM conversion_submissions WHERE idempotency_key=?")
            .bind(&input.idempotency_key)
            .fetch_optional(&mut *tx)
            .await?
        {
            Some(row) => row,
            None => {
                bail!(WsolFailure::conflict(
                    "wallet has a pending conversion; reconcile it before submitting another"
                ))
            }
        };
        let record = decode_record(&row)?;
        validate_reserved_record(&record, input, quote, fingerprint)?;
        record.validate_unsigned_reservation(&record)?;
        if !inserted
            || record.submission_id != submission_id
            || row.try_get::<i64, _>("created_at_ms")? != stamp
            || row.try_get::<i64, _>("updated_at_ms")? != stamp
        {
            bail!("WSOL reservation changed during persistence");
        }
        tx.commit().await?;
        Ok((record, true))
    }

    pub(crate) async fn record_signed_bound(
        &self,
        operation: WsolSignedOperation<'_>,
        now_ms: u64,
    ) -> Result<ConversionRecord> {
        let stamp = i64::try_from(now_ms)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE submission_id=?")
            .bind(&operation.record.submission_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("WSOL reservation missing before signed evidence write")
            })?;
        let current = decode_record(&row)?;
        let created_at = row.try_get::<i64, _>("created_at_ms")?;
        operation.validate_reservation(&current)?;
        let result = sqlx::query(
            "UPDATE conversion_submissions SET status='signed',signature=?,signed_transaction=?,updated_at_ms=? WHERE submission_id=? AND idempotency_key=? AND fingerprint=? AND quote_id=? AND wallet_id=? AND wallet_pubkey=? AND owner_type=? AND owner_id=? AND status='reserved' AND signature IS NULL AND signed_transaction IS NULL AND error IS NULL",
        )
        .bind(operation.signature).bind(operation.signed_bytes).bind(stamp)
        .bind(&current.submission_id).bind(&current.idempotency_key).bind(&current.fingerprint)
        .bind(&current.quote_id).bind(&current.wallet_id).bind(&current.wallet_pubkey)
        .bind(&current.owner_type).bind(&current.owner_id)
        .execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            bail!("WSOL signed evidence compare-and-set failed");
        }
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE submission_id=?")
            .bind(&current.submission_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("WSOL reservation missing after signed evidence write")
            })?;
        let signed = decode_record(&row)?;
        operation.validate(&signed)?;
        if signed.status != "signed"
            || signed.error.is_some()
            || row.try_get::<i64, _>("created_at_ms")? != created_at
            || row.try_get::<i64, _>("updated_at_ms")? != stamp
        {
            bail!("WSOL signed evidence changed during persistence");
        }
        tx.commit().await?;
        Ok(signed)
    }

    #[cfg(test)]
    pub async fn record_signed(
        &self,
        submission_id: &str,
        signature: &str,
        signed_transaction: &[u8],
        now_ms: u64,
    ) -> Result<()> {
        let record = self.get_by_id(submission_id).await?.ok_or_else(|| {
            anyhow::anyhow!("WSOL reservation missing before signed evidence write")
        })?;
        self.record_signed_bound(
            WsolSignedOperation {
                record: &record,
                signature,
                signed_bytes: signed_transaction,
            },
            now_ms,
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn fail_before_sign_bound(
        &self,
        expected: &ConversionRecord,
        error: &str,
        now_ms: u64,
    ) -> Result<ConversionRecord> {
        let stamp = i64::try_from(now_ms)?;
        let error = crate::sanitize::redact_urls(error);
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE submission_id=?")
            .bind(&expected.submission_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("WSOL reservation missing before unsigned failure"))?;
        let current = decode_record(&row)?;
        let created_at = row.try_get::<i64, _>("created_at_ms")?;
        expected.validate_unsigned_reservation(&current)?;
        let result = sqlx::query("UPDATE conversion_submissions SET status='failed',error=?,updated_at_ms=? WHERE submission_id=? AND idempotency_key=? AND fingerprint=? AND quote_id=? AND wallet_id=? AND wallet_pubkey=? AND owner_type=? AND owner_id=? AND status='reserved' AND signature IS NULL AND signed_transaction IS NULL AND error IS NULL")
            .bind(&error).bind(stamp).bind(&expected.submission_id).bind(&expected.idempotency_key)
            .bind(&expected.fingerprint).bind(&expected.quote_id).bind(&expected.wallet_id)
            .bind(&expected.wallet_pubkey).bind(&expected.owner_type).bind(&expected.owner_id)
            .execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            bail!("WSOL unsigned failure compare-and-set failed");
        }
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE submission_id=?")
            .bind(&expected.submission_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("WSOL reservation missing after unsigned failure"))?;
        let failed = decode_record(&row)?;
        expected.validate_readback(&failed)?;
        if failed.status != "failed"
            || failed.signature.is_some()
            || failed.signed_transaction.is_some()
            || failed.error.as_deref() != Some(error.as_str())
            || row.try_get::<i64, _>("created_at_ms")? != created_at
            || row.try_get::<i64, _>("updated_at_ms")? != stamp
        {
            bail!("WSOL unsigned failure changed during persistence");
        }
        tx.commit().await?;
        Ok(failed)
    }

    #[cfg(test)]
    pub async fn mark_failed_before_sign(
        &self,
        submission_id: &str,
        error: &str,
        now_ms: u64,
    ) -> Result<()> {
        let expected = self
            .get_by_id(submission_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("WSOL reservation missing before unsigned failure"))?;
        self.fail_before_sign_bound(&expected, error, now_ms)
            .await?;
        Ok(())
    }

    pub(crate) async fn apply_signed_update(
        &self,
        operation: WsolSignedOperation<'_>,
        update: WsolUpdate,
        error: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let stamp = i64::try_from(now_ms)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE submission_id=?")
            .bind(&operation.record.submission_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("WSOL submission missing during outcome update"))?;
        let current = decode_record(&row)?;
        operation.validate(&current)?;
        let state = WsolState::parse(&current.status)?;
        if !should_advance(state, update)? {
            tx.commit().await?;
            return Ok(());
        }
        let next = update.target().as_str();
        let error = error.map(crate::sanitize::redact_urls);
        let result=sqlx::query("UPDATE conversion_submissions SET status=?,error=?,updated_at_ms=? WHERE submission_id=? AND idempotency_key=? AND fingerprint=? AND quote_id=? AND wallet_id=? AND wallet_pubkey=? AND owner_type=? AND owner_id=? AND status=? AND signature=? AND signed_transaction=?")
            .bind(next).bind(error.as_deref()).bind(stamp)
            .bind(&current.submission_id).bind(&current.idempotency_key).bind(&current.fingerprint)
            .bind(&current.quote_id).bind(&current.wallet_id).bind(&current.wallet_pubkey)
            .bind(&current.owner_type).bind(&current.owner_id).bind(state.as_str())
            .bind(operation.signature).bind(operation.signed_bytes).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            bail!("WSOL outcome compare-and-set failed");
        }
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE submission_id=?")
            .bind(&current.submission_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("WSOL submission missing after outcome update"))?;
        let updated = decode_record(&row)?;
        operation.validate(&updated)?;
        if updated.status != next
            || updated.error != error
            || row.try_get::<i64, _>("updated_at_ms")? != stamp
        {
            bail!("WSOL outcome changed during persistence");
        }
        tx.commit().await?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn mark_broadcast_result(
        &self,
        submission_id: &str,
        status: &str,
        error: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        use crate::domain::wsol_progress::{WsolBroadcastOutcome as B, WsolObservedOutcome as O};
        let update = match status {
            "submitted" => WsolUpdate::Broadcast(B::Submitted),
            "unknown" => WsolUpdate::Broadcast(B::Unknown),
            "confirmed" => WsolUpdate::Observation(O::Confirmed),
            "failed" => WsolUpdate::Observation(O::Failed),
            _ => bail!("invalid conversion broadcast status"),
        };
        let record = self
            .get_by_id(submission_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("conversion submission not found"))?;
        if matches!(record.status.as_str(), "confirmed" | "failed") {
            bail!("conversion has no persisted signature or is already terminal");
        }
        self.apply_signed_update(
            WsolSignedOperation::from_record(&record)?,
            update,
            error,
            now_ms,
        )
        .await
    }

    pub async fn get_by_key(&self, idempotency_key: &str) -> Result<Option<ConversionRecord>> {
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE idempotency_key=?")
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(decode_record).transpose()
    }

    pub async fn get_by_id(&self, submission_id: &str) -> Result<Option<ConversionRecord>> {
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE submission_id=?")
            .bind(submission_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(decode_record).transpose()
    }

    pub(crate) async fn recover_by_key(
        &self,
        input: &crate::application::wsol_recovery::WsolRecoveryInput,
    ) -> Result<Option<ConversionRecord>> {
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE idempotency_key=? AND wallet_id=? AND owner_type=? AND owner_id=?")
            .bind(&input.idempotency_key)
            .bind(&input.wallet_id)
            .bind(&input.owner_type)
            .bind(&input.owner_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(decode_record).transpose()
    }
}

fn decode_quote(row: &sqlx::sqlite::SqliteRow, expected_id: &str) -> Result<WsolQuote> {
    let decoded = (|| -> Result<(&str, i64, WsolQuote)> {
        Ok((
            row.try_get("quote_id")?,
            row.try_get("expires_at_ms")?,
            serde_json::from_str(row.try_get::<&str, _>("quote_json")?)?,
        ))
    })();
    let (id, expires_at_ms, quote) =
        decoded.map_err(|_| WsolFailure::integrity("stored WSOL quote could not be decoded"))?;
    if id != expected_id
        || quote.quote_id != id
        || u64::try_from(expires_at_ms).ok() != Some(quote.expires_at_ms)
    {
        bail!(WsolFailure::integrity(
            "stored WSOL quote identity or expiry does not match its row"
        ));
    }
    Ok(quote)
}

fn validate_reserved_record(
    record: &ConversionRecord,
    input: &WsolSubmissionInput,
    quote: &WsolQuoteSnapshot,
    fingerprint: &str,
) -> Result<()> {
    if record.fingerprint != fingerprint {
        bail!(WsolFailure::conflict(
            "idempotency key was used with different conversion parameters"
        ));
    }
    record.validate_request(input, quote)
}

fn decode_record(row: &sqlx::sqlite::SqliteRow) -> Result<ConversionRecord> {
    Ok(ConversionRecord {
        submission_id: row.try_get("submission_id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        fingerprint: row.try_get("fingerprint")?,
        quote_id: row.try_get("quote_id")?,
        wallet_id: row.try_get("wallet_id")?,
        wallet_pubkey: row.try_get("wallet_pubkey")?,
        owner_type: row.try_get("owner_type")?,
        owner_id: row.try_get("owner_id")?,
        status: row.try_get("status")?,
        signature: row.try_get("signature")?,
        signed_transaction: row.try_get("signed_transaction")?,
        error: row.try_get("error")?,
    })
}

#[cfg(test)]
pub(crate) mod contract_tests;
#[cfg(test)]
mod progress_tests;
#[cfg(test)]
mod quote_storage_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod signing_tests;
#[cfg(test)]
mod unsigned_tests;

#[cfg(test)]
mod tests {
    use super::{WsolDirection, WsolQuote, WsolSubmission};

    fn quote() -> WsolQuote {
        WsolQuote {
            quote_id: "q1".into(),
            wallet_id: "w1".into(),
            wallet_pubkey: "pubkey".into(),
            owner_type: "telegram".into(),
            owner_id: "123".into(),
            direction: WsolDirection::Unwrap,
            amount_lamports: 10,
            source_balance_lamports: 10,
            ata_exists: true,
            estimated_network_fee_lamports: 1,
            temporary_rent_lamports: 0,
            rent_refund_lamports: 2,
            expires_at_ms: 100,
        }
    }

    #[test]
    fn quote_is_bound_to_owner_wallet_and_expiry() {
        let quote = quote();
        assert!(quote
            .validate_binding("w1", "pubkey", "telegram", "123", 99)
            .is_ok());
        assert!(quote
            .validate_binding("w1", "pubkey", "telegram", "other", 99)
            .is_err());
        assert!(quote
            .validate_binding("w1", "changed", "telegram", "123", 99)
            .is_err());
        assert!(quote
            .validate_binding("w1", "pubkey", "telegram", "123", 100)
            .is_err());
        assert!(quote.validate_current_balance(10).is_ok());
        assert!(quote.validate_current_balance(9).is_err());
        assert!(quote.validate_current_balance(11).is_err());
    }

    #[test]
    fn simulation_and_live_have_distinct_fingerprints() {
        let quote = quote();
        let simulation = WsolSubmission {
            quote_id: "q1".into(),
            wallet_id: "w1".into(),
            owner_type: "telegram".into(),
            owner_id: "123".into(),
            simulate: true,
            confirmed: false,
            idempotency_key: "same-key".into(),
        };
        let mut live = simulation.clone();
        live.simulate = false;
        assert_ne!(
            simulation.fingerprint(&quote).unwrap(),
            live.fingerprint(&quote).unwrap()
        );
        live.wallet_id = "other".into();
        assert!(live.fingerprint(&quote).is_err());
    }

    #[tokio::test]
    async fn durable_reservation_rejects_changed_request_and_persists_signature() {
        let store = super::ConversionStore::new("sqlite::memory:")
            .await
            .unwrap();
        let quote = quote();
        store.save_quote(&quote).await.unwrap();
        assert_eq!(store.get_quote("q1").await.unwrap(), Some(quote.clone()));
        let submission = WsolSubmission {
            quote_id: "q1".into(),
            wallet_id: "w1".into(),
            owner_type: "telegram".into(),
            owner_id: "123".into(),
            simulate: false,
            confirmed: true,
            idempotency_key: "key".into(),
        };
        let (record, inserted) = store.reserve_live(&submission, &quote, 50).await.unwrap();
        assert!(inserted);
        assert_eq!(record.status, "reserved");
        let (again, inserted) = store.reserve_live(&submission, &quote, 101).await.unwrap();
        assert!(!inserted);
        assert_eq!(again.submission_id, record.submission_id);
        let mut changed = submission.clone();
        changed.quote_id = "other".into();
        assert!(store.reserve_live(&changed, &quote, 50).await.is_err());
        let mut changed_amount = quote.clone();
        changed_amount.amount_lamports = 9;
        assert!(store
            .reserve_live(&submission, &changed_amount, 50)
            .await
            .is_err());
        let mut simulation = submission.clone();
        simulation.simulate = true;
        assert!(store.reserve_live(&simulation, &quote, 50).await.is_err());
        let mut concurrent = submission.clone();
        concurrent.idempotency_key = "another-key".into();
        assert!(store.reserve_live(&concurrent, &quote, 50).await.is_err());
        assert!(store.get_by_key("another-key").await.unwrap().is_none());
        store
            .record_signed(&record.submission_id, "sig", &[1, 2, 3], 60)
            .await
            .unwrap();
        let signed = store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(
            store.get_by_id(&record.submission_id).await.unwrap(),
            Some(signed.clone())
        );
        assert_eq!(signed.status, "signed");
        assert_eq!(signed.signature.as_deref(), Some("sig"));
        assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        assert!(store
            .record_signed(&record.submission_id, "new", &[4], 61)
            .await
            .is_err());
        store
            .mark_broadcast_result(&record.submission_id, "unknown", None, 62)
            .await
            .unwrap();
        let unknown = store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(unknown.signature.as_deref(), Some("sig"));
        assert_eq!(unknown.status, "unknown");
        assert!(store.reserve_live(&concurrent, &quote, 63).await.is_err());
        store
            .mark_broadcast_result(&record.submission_id, "confirmed", None, 63)
            .await
            .unwrap();
        assert!(store
            .mark_broadcast_result(&record.submission_id, "submitted", None, 64)
            .await
            .is_err());
        let mut unsigned = submission.clone();
        unsigned.idempotency_key = "unsigned".into();
        let (unsigned_record, _) = store.reserve_live(&unsigned, &quote, 50).await.unwrap();
        store
            .mark_failed_before_sign(&unsigned_record.submission_id, "preflight failed", 51)
            .await
            .unwrap();
        assert_eq!(
            store.get_by_key("unsigned").await.unwrap().unwrap().status,
            "failed"
        );
        assert!(store
            .record_signed(&unsigned_record.submission_id, "sig2", &[1], 52)
            .await
            .is_err());
    }
}
