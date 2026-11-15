use anyhow::{ensure, Context, Result};
use sqlx::{sqlite::SqliteRow, Row, Sqlite, SqlitePool, Transaction};

use super::sol_transfer_codec as codec;
use crate::{
    application::{
        ports::sol_transfer_journal::SolTransferJournal,
        sol_transfer_record::{SolTransferJournalEntry, SolTransferReservation},
    },
    domain::{
        sol_transfer::{SolTransferAttemptState as State, SolTransferIntent, SolTransferSnapshot},
        submission::SignedSubmission,
    },
};

const MIGRATION: &str = include_str!("../../../migrations/wallet_001_sol_transfer_attempts.sql");
const RESTARTED: &str = "service restarted during SOL transfer; do not resubmit";
const SELECT: &str = "SELECT p.*,a.transfer_id AS attempt_id,a.fingerprint,a.intent_json,a.state AS attempt_state,a.snapshot_json,a.signature AS attempt_signature,a.signed_transaction,a.error AS attempt_error FROM wallet_sol_transfers p LEFT JOIN wallet_sol_transfer_attempts a ON a.transfer_id=p.transfer_id";

pub(crate) struct SqliteSolTransferJournal {
    pool: SqlitePool,
}

impl SqliteSolTransferJournal {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn begin_write(&self) -> Result<Transaction<'static, Sqlite>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut *tx)
            .await?;
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            synchronous >= 2 && foreign_keys == 1,
            "SOL transfer journal requires synchronous=FULL and foreign_keys=ON"
        );
        Ok(tx)
    }

    pub(crate) async fn initialize(&self) -> Result<()> {
        let mut tx = self.begin_write().await?;
        sqlx::raw_sql(MIGRATION).execute(&mut *tx).await?;
        let rows = sqlx::query(&format!("{SELECT} WHERE a.state IN ('reserved','signed')"))
            .fetch_all(&mut *tx)
            .await?;
        for row in rows {
            let entry = decode(&row)?;
            let state = entry.state.context("expected managed SOL transfer")?;
            require_one(sqlx::query("UPDATE wallet_sol_transfer_attempts SET state='unknown',error=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND state=?")
                .bind(RESTARTED).bind(&entry.record.transfer_id).bind(state.as_str()).execute(&mut *tx).await?.rows_affected())?;
            require_one(sqlx::query("UPDATE wallet_sol_transfers SET status='unknown',error=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=?")
                .bind(RESTARTED).bind(&entry.record.transfer_id).execute(&mut *tx).await?.rows_affected())?;
        }
        tx.commit().await?;
        Ok(())
    }
}

impl SolTransferJournal for SqliteSolTransferJournal {
    async fn lookup(&self, key: &str) -> Result<Option<SolTransferJournalEntry>> {
        sqlx::query(&format!("{SELECT} WHERE p.idempotency_key=?"))
            .bind(key)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(decode)
            .transpose()
    }

    async fn reserve(&self, intent: &SolTransferIntent) -> Result<SolTransferReservation> {
        let (json, fingerprint) = codec::encode_intent(intent)?;
        let (fixed, percent) = intent.amount.legacy_params();
        let id = format!("transfer_{}", uuid::Uuid::new_v4().simple());
        let mut tx = self.begin_write().await?;
        let created = sqlx::query("INSERT INTO wallet_sol_transfers(transfer_id,idempotency_key,owner_type,owner_id,wallet_id,from_pubkey,to_pubkey,amount_mode,requested_amount_lamports,requested_percent,status) VALUES(?,?,?,?,?,?,?,?,?,?,'processing') ON CONFLICT(idempotency_key) DO NOTHING")
            .bind(&id).bind(&intent.idempotency_key).bind(&intent.owner_type).bind(&intent.owner_id).bind(&intent.wallet_id)
            .bind(intent.source.to_string()).bind(intent.destination.to_string())
            .bind(if fixed.is_some() { "fixed" } else { "percent" })
            .bind(fixed.map(|value| value as i64)).bind(percent.map(|value| value as i64))
            .execute(&mut *tx).await?.rows_affected() == 1;
        if created {
            sqlx::query("INSERT INTO wallet_sol_transfer_attempts(transfer_id,fingerprint,intent_json,state) VALUES(?,?,?,'reserved')")
                .bind(&id).bind(fingerprint).bind(json).execute(&mut *tx).await?;
        }
        let row = sqlx::query(&format!("{SELECT} WHERE p.idempotency_key=?"))
            .bind(&intent.idempotency_key)
            .fetch_one(&mut *tx)
            .await?;
        let entry = decode(&row)?;
        if !entry.record.matches(intent)
            || !entry
                .intent
                .as_ref()
                .map_or(true, |stored| stored == intent)
        {
            tx.rollback().await?;
            return Ok(SolTransferReservation::Conflict);
        }
        tx.commit().await?;
        Ok(if created {
            SolTransferReservation::Created(entry)
        } else {
            SolTransferReservation::Existing(entry)
        })
    }

    async fn record_signed(
        &self,
        id: &str,
        snapshot: &SolTransferSnapshot,
        signed: &SignedSubmission,
    ) -> Result<()> {
        ensure!(
            !signed.signature.is_empty() && !signed.bytes.is_empty(),
            "signed transaction and signature are required before broadcast"
        );
        let amount = i64::try_from(snapshot.amount_lamports)
            .context("SOL transfer amount exceeds projection range")?;
        let fee = i64::try_from(snapshot.fee_lamports)
            .context("SOL transfer fee exceeds projection range")?;
        let balance = i64::try_from(snapshot.balance_before_lamports)
            .context("SOL transfer balance exceeds projection range")?;
        let mut tx = self.begin_write().await?;
        let entry = managed(&mut tx, id, State::Reserved).await?;
        codec::validate_snapshot(
            entry
                .intent
                .as_ref()
                .context("missing SOL transfer intent")?,
            snapshot,
        )?;
        require_one(sqlx::query("UPDATE wallet_sol_transfer_attempts SET state='signed',snapshot_json=?,signature=?,signed_transaction=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND state='reserved' AND signature IS NULL")
            .bind(codec::encode_snapshot(snapshot)?).bind(&signed.signature).bind(&signed.bytes).bind(id)
            .execute(&mut *tx).await?.rows_affected())?;
        require_one(sqlx::query("UPDATE wallet_sol_transfers SET status='unknown',actual_amount_lamports=?,fee_lamports=?,balance_before_lamports=?,signature=?,error=NULL,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND status='processing' AND signature IS NULL")
            .bind(amount).bind(fee).bind(balance).bind(&signed.signature).bind(id)
            .execute(&mut *tx).await?.rows_affected())?;
        tx.commit().await?;
        Ok(())
    }

    async fn fail_before_sign(&self, id: &str, error: &str) -> Result<()> {
        ensure!(!error.is_empty(), "SOL transfer failure reason is required");
        let error = crate::sanitize::redact_urls(error);
        let mut tx = self.begin_write().await?;
        managed(&mut tx, id, State::Reserved).await?;
        require_one(sqlx::query("UPDATE wallet_sol_transfer_attempts SET state='failed',error=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND state='reserved' AND signature IS NULL")
            .bind(&error).bind(id).execute(&mut *tx).await?.rows_affected())?;
        require_one(sqlx::query("UPDATE wallet_sol_transfers SET status='failed',error=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND status='processing' AND signature IS NULL")
            .bind(&error).bind(id).execute(&mut *tx).await?.rows_affected())?;
        tx.commit().await?;
        Ok(())
    }

    async fn record_broadcast(&self, id: &str, state: State, error: Option<&str>) -> Result<()> {
        ensure!(
            matches!(state, State::Submitted | State::Unknown),
            "invalid SOL transfer broadcast state"
        );
        ensure!(
            state != State::Submitted || error.is_none(),
            "submitted SOL transfer cannot have a send error"
        );
        let error = error.map(crate::sanitize::redact_urls);
        let mut tx = self.begin_write().await?;
        let entry = managed(&mut tx, id, State::Signed).await?;
        require_one(sqlx::query("UPDATE wallet_sol_transfer_attempts SET state=?,error=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND state='signed' AND signature IS NOT NULL")
            .bind(state.as_str()).bind(&error).bind(id).execute(&mut *tx).await?.rows_affected())?;
        require_one(sqlx::query("UPDATE wallet_sol_transfers SET status=?,error=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND status='unknown' AND signature=?")
            .bind(state.projection_status()).bind(&error).bind(id).bind(entry.record.signature)
            .execute(&mut *tx).await?.rows_affected())?;
        tx.commit().await?;
        Ok(())
    }

    async fn record_confirmed(&self, id: &str) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let entry = managed(&mut tx, id, State::Submitted).await?;
        require_one(sqlx::query("UPDATE wallet_sol_transfer_attempts SET state='confirmed',updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND state='submitted'")
            .bind(id).execute(&mut *tx).await?.rows_affected())?;
        require_one(sqlx::query("UPDATE wallet_sol_transfers SET status='confirmed',updated_at=CURRENT_TIMESTAMP WHERE transfer_id=? AND status='submitted' AND signature=?")
            .bind(id).bind(entry.record.signature).execute(&mut *tx).await?.rows_affected())?;
        tx.commit().await?;
        Ok(())
    }
}

async fn managed(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    expected: State,
) -> Result<SolTransferJournalEntry> {
    let row = sqlx::query(&format!("{SELECT} WHERE p.transfer_id=?"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .context("SOL transfer not found")?;
    let entry = decode(&row)?;
    ensure!(
        entry.state == Some(expected),
        "SOL transfer state changed; do not retry automatically"
    );
    Ok(entry)
}

fn require_one(rows: u64) -> Result<()> {
    ensure!(
        rows == 1,
        "SOL transfer state changed; do not retry automatically"
    );
    Ok(())
}

fn decode(row: &SqliteRow) -> Result<SolTransferJournalEntry> {
    let record = codec::decode_projection(row)?;
    if row.try_get::<Option<String>, _>("attempt_id")?.is_none() {
        return Ok(SolTransferJournalEntry {
            record,
            intent: None,
            state: None,
        });
    }
    let intent = codec::decode_intent(row.try_get("intent_json")?)?;
    let fingerprint: String = row.try_get("fingerprint")?;
    ensure!(
        codec::encode_intent(&intent)?.1 == fingerprint && record.matches(&intent),
        "SOL transfer intent or projection is inconsistent"
    );
    let state = State::parse(row.try_get("attempt_state")?)?;
    let signature: Option<String> = row.try_get("attempt_signature")?;
    let error: Option<String> = row.try_get("attempt_error")?;
    let bytes: Option<Vec<u8>> = row.try_get("signed_transaction")?;
    let snapshot: Option<String> = row.try_get("snapshot_json")?;
    ensure!(
        record.status == state.projection_status()
            && record.signature == signature
            && record.error == error,
        "SOL transfer outcome projection is inconsistent"
    );
    if let Some(json) = snapshot {
        ensure!(
            signature.as_ref().is_some_and(|value| !value.is_empty())
                && bytes.as_ref().is_some_and(|value| !value.is_empty())
                && matches!(
                    state,
                    State::Signed | State::Submitted | State::Confirmed | State::Unknown
                ),
            "invalid signed SOL transfer state"
        );
        let snapshot = codec::decode_snapshot(&json)?;
        codec::validate_snapshot(&intent, &snapshot)?;
        ensure!(
            record.actual_amount_lamports == Some(snapshot.amount_lamports)
                && record.fee_lamports == Some(snapshot.fee_lamports)
                && record.balance_before_lamports == Some(snapshot.balance_before_lamports),
            "SOL transfer preparation projection is inconsistent"
        );
    } else {
        ensure!(
            signature.is_none()
                && bytes.is_none()
                && record.actual_amount_lamports.is_none()
                && record.fee_lamports.is_none()
                && record.balance_before_lamports.is_none()
                && matches!(state, State::Reserved | State::Failed | State::Unknown),
            "invalid unsigned SOL transfer state"
        );
    }
    ensure!(
        !matches!(
            state,
            State::Reserved | State::Signed | State::Submitted | State::Confirmed
        ) || error.is_none(),
        "unexpected SOL transfer state error"
    );
    ensure!(
        state != State::Failed || error.as_ref().is_some_and(|value| !value.is_empty()),
        "missing SOL transfer failure reason"
    );
    Ok(SolTransferJournalEntry {
        record,
        intent: Some(intent),
        state: Some(state),
    })
}

#[cfg(test)]
mod tests;
