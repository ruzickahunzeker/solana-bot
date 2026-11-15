use anyhow::{ensure, Context, Result};
use sqlx::{Row, SqlitePool};
use std::sync::Arc;

use crate::{
    adapters::persistence::trade_snapshot::verify_stored_identity,
    application::{
        ordinary_claims::{ensure_parent, operation_for_claim, parent_can_start, ClaimOperation},
        ports::{
            ordinary_claims::{ExecutionClaim, ExecutionOwner, OrdinaryExecutionClaimer},
            ordinary_journal::{OrdinaryTradeJournal, TradeJournalMode},
        },
        trade_admission::AdmittedTrade,
    },
    idempotency::IdempotencyStore,
    persistence::{row_to_record, TradePersistence},
    state::TradeStore,
};

const MIGRATION: &str = include_str!("../../../migrations/004_ordinary_execution_claims.sql");
mod operation_upgrade;
mod sdk_observation_index;

pub(crate) struct RuntimeExecutionClaims {
    journal: Arc<TradePersistence>,
    index: Arc<TradeStore>,
    reservations: Arc<IdempotencyStore>,
}

impl RuntimeExecutionClaims {
    pub(crate) fn new(
        journal: Arc<TradePersistence>,
        index: Arc<TradeStore>,
        reservations: Arc<IdempotencyStore>,
    ) -> Self {
        Self {
            journal,
            index,
            reservations,
        }
    }
}

impl OrdinaryExecutionClaimer for RuntimeExecutionClaims {
    async fn claim(&self, execution: &AdmittedTrade) -> Result<ExecutionClaim> {
        let mode = OrdinaryTradeJournal::mode(self.journal.as_ref());
        let operation = operation_for_claim(execution, mode)?;
        if mode == TradeJournalMode::Durable {
            return self
                .journal
                .ordinary_execution_claims()
                .context("durable ordinary execution claims unavailable")?
                .claim(execution)
                .await;
        }
        ensure!(
            operation == ClaimOperation::SolBuy,
            "memory execution claim operation mismatch"
        );
        let parent = self
            .index
            .get(execution.reservation().request_id())
            .context("ordinary execution parent missing")?;
        ensure_parent(execution, &parent)?;
        self.reservations
            .claim_execution(execution, parent_can_start(&parent))
    }
}

pub(crate) struct SqliteOrdinaryExecutionClaims {
    pub(super) pool: SqlitePool,
}

impl SqliteOrdinaryExecutionClaims {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn initialize(&self) -> Result<()> {
        sqlx::raw_sql(MIGRATION).execute(&self.pool).await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS ordinary_submission_migrations(name TEXT PRIMARY KEY NOT NULL)").execute(&mut *transaction).await?;
        // Serialize migration decisions even when CREATE IF NOT EXISTS is a no-op.
        sqlx::query("UPDATE ordinary_submission_migrations SET name=name WHERE 0")
            .execute(&mut *transaction)
            .await?;
        for (name, sql) in [
            (
                "005_ordinary_signed_submissions",
                include_str!("../../../migrations/005_ordinary_signed_submissions.sql"),
            ),
            (
                "006_ordinary_final_observations",
                include_str!("../../../migrations/006_ordinary_final_observations.sql"),
            ),
            (
                "007_ordinary_observation_scan",
                include_str!("../../../migrations/007_ordinary_observation_scan.sql"),
            ),
            (
                "008_ordinary_claim_operations",
                include_str!("../../../migrations/008_ordinary_claim_operations.sql"),
            ),
            (
                "009_ordinary_sdk_prepared_candidate",
                include_str!("../../../migrations/009_ordinary_sdk_prepared_candidate.sql"),
            ),
            (
                "010_ordinary_sdk_observation_scan",
                include_str!("../../../migrations/010_ordinary_sdk_observation_scan.sql"),
            ),
        ] {
            let migrated: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?",
            )
            .bind(name)
            .fetch_one(&mut *transaction)
            .await?;
            if name == "010_ordinary_sdk_observation_scan" {
                sdk_observation_index::ensure_index(&mut transaction, migrated != 0, sql).await?;
            }
            if migrated == 0 {
                if name == "008_ordinary_claim_operations" {
                    operation_upgrade::apply(&mut transaction, sql).await?;
                } else if name != "010_ordinary_sdk_observation_scan" {
                    sqlx::raw_sql(sql).execute(&mut *transaction).await?;
                }
                sqlx::query("INSERT INTO ordinary_submission_migrations(name) VALUES(?)")
                    .bind(name)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn claim(&self, execution: &AdmittedTrade) -> Result<ExecutionClaim> {
        let operation = operation_for_claim(execution, TradeJournalMode::Durable)?;
        let anchor = execution.reservation();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let mut transaction = self.pool.begin().await?;
        // The first statement obtains SQLite's write lock; all subsequent checks roll back on error.
        let inserted = sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms,owner_nonce) SELECT request_id,1,?,request_hash,?,? FROM trades WHERE request_id=? AND wallet_id=? AND wallet_pubkey=? AND idempotency_key=? AND request_hash=? AND status='accepted' AND signature IS NULL ON CONFLICT(request_id) DO NOTHING")
            .bind(operation.storage_name())
            .bind(i64::try_from(crate::state::now_ms())?)
            .bind(&nonce)
            .bind(anchor.request_id()).bind(anchor.wallet_id()).bind(anchor.wallet_pubkey())
            .bind(anchor.idempotency_key()).bind(anchor.request_hash())
            .execute(&mut *transaction).await?.rows_affected();
        let row = sqlx::query("SELECT * FROM trades WHERE request_id=?")
            .bind(anchor.request_id())
            .fetch_optional(&mut *transaction)
            .await?
            .context("ordinary execution parent missing")?;
        ensure!(
            row.try_get::<&str, _>("idempotency_key")? == anchor.idempotency_key()
                && row.try_get::<&str, _>("request_hash")? == anchor.request_hash(),
            "ordinary execution parent key or hash mismatch"
        );
        let json: String = row.try_get("request_json")?;
        let parent = row_to_record(row)?;
        ensure_parent(execution, &parent)?;
        ensure!(
            verify_stored_identity(
                &parent,
                &json,
                anchor.idempotency_key(),
                operation.aggregator_sol()
            )?,
            "ordinary execution parent payment branch mismatch"
        );
        let claimed = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
            .bind(anchor.request_id())
            .fetch_optional(&mut *transaction)
            .await?
            .context("ordinary trade is not eligible for execution claim")?;
        ensure!(
            claimed.try_get::<i64, _>("schema_version")? == 1
                && claimed.try_get::<&str, _>("operation")? == operation.storage_name()
                && claimed.try_get::<&str, _>("request_hash")? == anchor.request_hash()
                && claimed.try_get::<i64, _>("created_at_ms")? >= 0,
            "ordinary execution claim identity mismatch"
        );
        if operation == ClaimOperation::SdkTrade {
            if let Some(candidate) = super::ordinary_sdk_preparation::validate_sdk_row(&claimed)? {
                crate::application::sdk_preparation::validate_origin(
                    execution,
                    &candidate.metadata,
                    &candidate.message,
                )?;
            }
            super::ordinary_sdk_preparation::validate_sdk_parent(&claimed, &parent)?;
        } else {
            ensure!(
                claimed
                    .try_get::<Option<&str>, _>("sdk_prepared_json")?
                    .is_none(),
                "aggregate claim cannot contain SDK prepared metadata"
            );
        }
        ensure!(
            inserted <= 1 && (inserted == 0 || parent_can_start(&parent)),
            "ordinary execution parent state changed during claim"
        );
        if inserted == 1 {
            ensure!(
                claimed.try_get::<Option<&str>, _>("owner_nonce")? == Some(nonce.as_str()),
                "ordinary execution owner readback mismatch"
            );
        }
        let result = if inserted == 1 {
            let owner = ExecutionOwner::new(execution, nonce)?;
            ensure!(
                owner.operation() == operation,
                "ordinary owner operation readback mismatch"
            );
            ExecutionClaim::Acquired(owner)
        } else {
            ExecutionClaim::AlreadyHeld
        };
        transaction.commit().await?;
        Ok(result)
    }
}
