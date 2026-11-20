use super::SqliteOrdinaryExecutionClaims;
use crate::{
    adapters::persistence::trade_snapshot::verify_stored_identity,
    application::{
        ordinary_claims::{ensure_parent, pump_prepared_scope},
        ports::ordinary_journal::TradeJournalMode,
        trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::trade::TradeStatus,
    persistence::row_to_record,
};
use anyhow::{ensure, Context, Result};
use sqlx::Row;

impl SqliteOrdinaryExecutionClaims {
    /// Uses the original parent and claims table; never creates an execution owner.
    pub(crate) async fn fail_unclaimed_preparation(
        &self,
        execution: &AdmittedTrade,
        error: &str,
    ) -> Result<Option<TradeRecordV1>> {
        ensure!(
            pump_prepared_scope(execution, TradeJournalMode::Durable),
            "unclaimed preparation failure is outside the native Pump profile"
        );
        let a = execution.reservation();
        let error = crate::sanitize::redact_urls(error);
        ensure!(
            error.len() <= 65_536,
            "unclaimed preparation error exceeds limit"
        );
        let mut tx = self.pool.begin().await?;
        // Lock before identity checks. Another claimant must see either Accepted or Failed.
        sqlx::query("UPDATE trades SET request_id=request_id WHERE 0")
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT * FROM trades WHERE request_id=?")
            .bind(a.request_id())
            .fetch_optional(&mut *tx)
            .await?
            .context("unclaimed preparation parent missing")?;
        ensure!(
            row.try_get::<&str, _>("idempotency_key")? == a.idempotency_key()
                && row.try_get::<&str, _>("request_hash")? == a.request_hash(),
            "unclaimed preparation parent key or hash mismatch"
        );
        let json: String = row.try_get("request_json")?;
        let parent = row_to_record(row)?;
        ensure_parent(execution, &parent)?;
        ensure!(
            verify_stored_identity(&parent, &json, a.idempotency_key(), false)?,
            "unclaimed preparation parent payment branch mismatch"
        );
        let count = sqlx::query("UPDATE trades SET status='failed',error=?,updated_at_ms=? WHERE request_id=? AND wallet_id=? AND wallet_pubkey=? AND idempotency_key=? AND request_hash=? AND request_json=? AND status='accepted' AND signature IS NULL AND NOT EXISTS(SELECT 1 FROM ordinary_execution_claims WHERE request_id=trades.request_id)")
            .bind(&error).bind(i64::try_from(crate::state::now_ms())?)
            .bind(a.request_id()).bind(a.wallet_id()).bind(a.wallet_pubkey())
            .bind(a.idempotency_key()).bind(a.request_hash()).bind(&json)
            .execute(&mut *tx).await?.rows_affected();
        ensure!(
            count <= 1,
            "unclaimed preparation failure changed multiple parents"
        );
        let result = if count == 1 {
            let row = sqlx::query("SELECT * FROM trades WHERE request_id=?")
                .bind(a.request_id())
                .fetch_one(&mut *tx)
                .await?;
            ensure!(
                row.try_get::<&str, _>("request_hash")? == a.request_hash()
                    && row.try_get::<&str, _>("idempotency_key")? == a.idempotency_key()
                    && row.try_get::<&str, _>("request_json")? == json,
                "unclaimed preparation failure identity readback mismatch"
            );
            let saved = row_to_record(row)?;
            ensure_parent(execution, &saved)?;
            ensure!(
                saved.status == TradeStatus::Failed
                    && saved.signature.is_none()
                    && saved.error.as_deref() == Some(error.as_str())
                    && saved.created_at_ms == parent.created_at_ms,
                "unclaimed preparation failure readback mismatch"
            );
            Some(saved)
        } else {
            None
        };
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
