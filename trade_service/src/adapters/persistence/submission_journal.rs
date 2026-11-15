use anyhow::{bail, Context, Result};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::application::ports::reconciliation::ReconciliationJournal;
use crate::application::ports::submission_journal::SubmissionJournal;
use crate::domain::reconciliation::{ChainCommitment, ChainObservation};
use crate::domain::submission::{
    Reservation, SignedSubmission, SubmissionIntent, SubmissionKey, SubmissionRecord,
    SubmissionStatus,
};

const MIGRATION: &str = include_str!("../../../migrations/002_quote_submissions.sql");
const RECONCILIATION_MIGRATION: &str =
    include_str!("../../../migrations/003_quote_reconciliation.sql");

pub(crate) struct SqliteSubmissionJournal {
    pool: SqlitePool,
}

impl SqliteSubmissionJournal {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn initialize(&self) -> Result<()> {
        sqlx::raw_sql(MIGRATION).execute(&self.pool).await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS quote_submission_migrations(name TEXT PRIMARY KEY NOT NULL)").execute(&mut *transaction).await?;
        let migrated: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_submission_migrations WHERE name='003_quote_reconciliation'").fetch_one(&mut *transaction).await?;
        if migrated == 0 {
            sqlx::raw_sql(RECONCILIATION_MIGRATION)
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "INSERT INTO quote_submission_migrations(name) VALUES('003_quote_reconciliation')",
            )
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        // Recovery is observation-only: never re-sign or re-broadcast an interrupted order.
        sqlx::query("UPDATE quote_submissions SET status='unknown',error='service restarted during submission; do not resubmit',updated_at_ms=? WHERE status IN ('reserved','signed')")
            .bind(now_ms()).execute(&self.pool).await?;
        Ok(())
    }
}

impl SubmissionJournal for SqliteSubmissionJournal {
    async fn get(&self, key: &SubmissionKey) -> Result<Option<SubmissionRecord>> {
        sqlx::query("SELECT * FROM quote_submissions WHERE wallet_id=? AND idempotency_key=?")
            .bind(&key.wallet_id)
            .bind(&key.idempotency_key)
            .fetch_optional(&self.pool)
            .await?
            .map(decode)
            .transpose()
    }

    async fn reserve(&self, intent: &SubmissionIntent, preflight: &Value) -> Result<Reservation> {
        let hash = intent.fingerprint()?;
        let id = format!("qs_{}", uuid::Uuid::new_v4().simple());
        let inserted = sqlx::query("INSERT INTO quote_submissions(submission_id,wallet_id,idempotency_key,fingerprint,intent_json,preflight_json,status,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,'reserved',?,?) ON CONFLICT(wallet_id,idempotency_key) DO NOTHING")
            .bind(&id).bind(&intent.key.wallet_id).bind(&intent.key.idempotency_key)
            .bind(&hash).bind(serde_json::to_string(intent)?).bind(serde_json::to_string(preflight)?)
            .bind(now_ms()).bind(now_ms()).execute(&self.pool).await?.rows_affected() == 1;
        let record = self
            .get(&intent.key)
            .await?
            .context("submission disappeared after reserve")?;
        if record.intent.fingerprint()? != hash {
            return Ok(Reservation::Conflict);
        }
        Ok(if inserted {
            Reservation::Created(record)
        } else {
            Reservation::Existing(record)
        })
    }

    async fn record_signed(&self, submission_id: &str, signed: &SignedSubmission) -> Result<()> {
        if signed.signature.is_empty() || signed.bytes.is_empty() {
            bail!("signed transaction and signature are required before broadcast");
        }
        let count = sqlx::query("UPDATE quote_submissions SET status='signed',signature=?,signed_transaction=?,updated_at_ms=? WHERE submission_id=? AND status='reserved' AND signature IS NULL")
            .bind(&signed.signature).bind(&signed.bytes).bind(now_ms()).bind(submission_id)
            .execute(&self.pool).await?.rows_affected();
        require_one(count)
    }

    async fn fail_before_sign(&self, submission_id: &str, error: &str) -> Result<()> {
        let count = sqlx::query("UPDATE quote_submissions SET status='failed',error=?,updated_at_ms=? WHERE submission_id=? AND status='reserved' AND signature IS NULL")
            .bind(error).bind(now_ms()).bind(submission_id).execute(&self.pool).await?.rows_affected();
        require_one(count)
    }

    async fn record_broadcast(
        &self,
        submission_id: &str,
        status: SubmissionStatus,
        error: Option<&str>,
    ) -> Result<()> {
        if !matches!(
            status,
            SubmissionStatus::Submitted | SubmissionStatus::Unknown
        ) {
            bail!("invalid broadcast result status");
        }
        let count = sqlx::query("UPDATE quote_submissions SET status=?,error=?,updated_at_ms=? WHERE submission_id=? AND status='signed' AND signature IS NOT NULL AND signed_transaction IS NOT NULL")
            .bind(status.as_str()).bind(error).bind(now_ms()).bind(submission_id)
            .execute(&self.pool).await?.rows_affected();
        if count == 0 {
            let observed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_submissions WHERE submission_id=? AND confirmation_rank > 0 AND signature IS NOT NULL")
                .bind(submission_id).fetch_one(&self.pool).await?;
            if observed == 1 {
                return Ok(());
            }
        }
        require_one(count)
    }
}

impl ReconciliationJournal for SqliteSubmissionJournal {
    async fn lookup(&self, key: &SubmissionKey) -> Result<Option<SubmissionRecord>> {
        self.get(key).await
    }

    async fn record_observation(
        &self,
        submission_id: &str,
        observation: &ChainObservation,
    ) -> Result<bool> {
        let slot =
            i64::try_from(observation.slot).context("observation slot exceeds storage range")?;
        let count = sqlx::query("UPDATE quote_submissions SET status=?,error=?,confirmation_rank=?,observed_slot=?,updated_at_ms=? WHERE submission_id=? AND signature=? AND signed_transaction IS NOT NULL AND confirmation_rank < ? AND status IN ('signed','submitted','unknown','confirmed','failed')")
            .bind(observation.submission_status().as_str()).bind(&observation.error)
            .bind(observation.commitment.rank()).bind(slot).bind(now_ms())
            .bind(submission_id).bind(&observation.signature).bind(observation.commitment.rank())
            .execute(&self.pool).await?.rows_affected();
        Ok(count == 1)
    }
}

fn require_one(count: u64) -> Result<()> {
    if count != 1 {
        bail!("submission state changed; operation must not be retried automatically");
    }
    Ok(())
}

fn decode(row: sqlx::sqlite::SqliteRow) -> Result<SubmissionRecord> {
    let record = SubmissionRecord {
        submission_id: row.try_get("submission_id")?,
        intent: serde_json::from_str(row.try_get("intent_json")?)?,
        status: SubmissionStatus::parse(row.try_get("status")?)?,
        preflight: serde_json::from_str(row.try_get("preflight_json")?)?,
        signature: row.try_get("signature")?,
        error: row.try_get("error")?,
        commitment: ChainCommitment::from_rank(row.try_get("confirmation_rank")?)?,
        observed_slot: row
            .try_get::<Option<i64>, _>("observed_slot")?
            .map(u64::try_from)
            .transpose()?,
    };
    if record.intent.fingerprint()? != row.try_get::<&str, _>("fingerprint")?
        || record.intent.key.wallet_id != row.try_get::<&str, _>("wallet_id")?
        || record.intent.key.idempotency_key != row.try_get::<&str, _>("idempotency_key")?
    {
        bail!("submission identity or fingerprint is inconsistent");
    }
    Ok(record)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
