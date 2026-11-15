use crate::{
    adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
    application::ordinary_observation_scan::{
        ObservationCursor, ObservationPage, PendingOrdinaryObservations,
    },
};
use anyhow::{ensure, Result};
use sqlx::Row;

const PAGE_SQL: &str = "SELECT c.created_at_ms,c.request_id FROM ordinary_execution_claims AS c INDEXED BY idx_ordinary_signed_observation_scan JOIN trades AS t ON t.request_id=c.request_id WHERE c.schema_version=1 AND c.operation='ordinary-sol-buy' AND c.owner_nonce IS NOT NULL AND c.signed_signature IS NOT NULL AND c.signed_transaction IS NOT NULL AND c.final_observation_json IS NULL AND c.submission_stage IN ('signed','submitted','unknown') AND typeof(c.created_at_ms)='integer' AND typeof(c.request_id)='text' AND c.created_at_ms>=0 AND c.created_at_ms<=? AND t.updated_at_ms<=? AND t.status IN ('submitting','submitted','unknown') AND t.signature=c.signed_signature AND length(c.request_id) BETWEEN 1 AND 128 AND c.request_id NOT GLOB '*[^A-Za-z0-9_-]*' AND (c.created_at_ms,c.request_id)>(?,?) ORDER BY c.created_at_ms,c.request_id LIMIT ?";

impl PendingOrdinaryObservations for SqliteOrdinaryExecutionClaims {
    async fn pending_observations(
        &self,
        after: Option<&ObservationCursor>,
        cutoff_ms: i64,
        limit: usize,
    ) -> Result<ObservationPage> {
        ensure!(
            cutoff_ms >= 0 && (1..=64).contains(&limit),
            "invalid observation page bounds"
        );
        let rows = sqlx::query(PAGE_SQL)
            .bind(cutoff_ms)
            .bind(cutoff_ms)
            .bind(after.map_or(-1, ObservationCursor::created_at_ms))
            .bind(after.map_or("", ObservationCursor::request_id))
            .bind(i64::try_from(limit + 1)?)
            .fetch_all(&self.pool)
            .await?;
        let has_more = rows.len() > limit;
        let entries = rows
            .into_iter()
            .take(limit)
            .map(|row| {
                ObservationCursor::new(row.try_get("created_at_ms")?, row.try_get("request_id")?)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ObservationPage { entries, has_more })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::ordinary_reconciliation::RecoveredOrdinaryObservations;
    use crate::{
        config::{TradePersistenceConfig, TradePersistenceMode},
        persistence::TradePersistence,
    };

    async fn setup() -> (SqliteOrdinaryExecutionClaims, sqlx::SqlitePool) {
        let store = TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap();
        let pool = store.pool_for_tests();
        (store.ordinary_execution_claims().unwrap(), pool)
    }
    async fn insert(pool: &sqlx::SqlitePool, id: &str, created: i64, updated: i64) {
        // Discovery hints only: deliberately not complete signed/authorized execution evidence.
        sqlx::query("INSERT INTO trades(request_id,wallet_id,wallet_pubkey,idempotency_key,request_hash,request_json,side,pool,mint,execution_method,status,signature,created_at_ms,updated_at_ms) VALUES(?,'fixture-wallet','public-fixture',?,'hash','{}','buy','pump-amm','public-mint','sdk','unknown','public-signature',?,?)")
            .bind(id).bind(id).bind(created).bind(updated).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms,owner_nonce,submission_stage,signed_signature,signed_transaction) VALUES(?,1,'ordinary-sol-buy','hash',?,?,'signed','public-signature',x'0102')")
            .bind(id).bind(created).bind("a".repeat(32)).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn ordinary_scan_sql_keyset_orders_ties_filters_age_and_never_uses_offset() {
        let (store, pool) = setup().await;
        for (id, time) in [("tr_c", 100), ("tr_a", 100), ("tr_b", 100), ("tr_d", 101)] {
            insert(&pool, id, time, time).await;
        }
        insert(&pool, "tr_fresh", 100, 300).await;
        let first = store.pending_observations(None, 200, 2).await.unwrap();
        assert_eq!(
            first
                .entries
                .iter()
                .map(ObservationCursor::request_id)
                .collect::<Vec<_>>(),
            vec!["tr_a", "tr_b"]
        );
        assert!(first.has_more);
        let second = store
            .pending_observations(first.entries.last(), 200, 2)
            .await
            .unwrap();
        assert_eq!(
            second
                .entries
                .iter()
                .map(ObservationCursor::request_id)
                .collect::<Vec<_>>(),
            vec!["tr_c", "tr_d"]
        );
        assert!(!second.has_more);
        assert!(!PAGE_SQL.contains("OFFSET"));
    }

    #[tokio::test]
    async fn ordinary_scan_sql_excludes_unsigned_terminal_unowned_and_inconsistent_signature_rows()
    {
        let (store, pool) = setup().await;
        for id in [
            "tr_good",
            "tr_unsigned",
            "tr_terminal",
            "tr_unowned",
            "tr_mismatch",
            "tr_recorded",
            "tr_invalid/id",
        ] {
            insert(&pool, id, 100, 100).await;
        }
        for sql in [
            "UPDATE ordinary_execution_claims SET signed_transaction=NULL WHERE request_id='tr_unsigned'",
            "UPDATE trades SET status='confirmed' WHERE request_id='tr_terminal'",
            "UPDATE ordinary_execution_claims SET owner_nonce=NULL WHERE request_id='tr_unowned'",
            "UPDATE trades SET signature='different' WHERE request_id='tr_mismatch'",
            "UPDATE ordinary_execution_claims SET final_observation_json='{}' WHERE request_id='tr_recorded'",
        ] { sqlx::query(sql).execute(&pool).await.unwrap(); }
        let page = store.pending_observations(None, 200, 64).await.unwrap();
        assert_eq!(
            page.entries
                .iter()
                .map(ObservationCursor::request_id)
                .collect::<Vec<_>>(),
            vec!["tr_good"]
        );
        assert!(store.capture_observation("tr_good").await.is_err());
    }

    #[tokio::test]
    async fn ordinary_scan_sql_index_is_used_and_migration_is_idempotent() {
        let (store, pool) = setup().await;
        for _ in 0..2 {
            store.initialize().await.unwrap();
        }
        let plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {PAGE_SQL}"))
            .bind(200_i64)
            .bind(200_i64)
            .bind(-1_i64)
            .bind("")
            .bind(3_i64)
            .fetch_all(&pool)
            .await
            .unwrap();
        let details = plan
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            details.contains("idx_ordinary_signed_observation_scan"),
            "{details}"
        );
        assert!(!details.contains("USE TEMP B-TREE"), "{details}");
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name='007_ordinary_observation_scan'").fetch_one(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn ordinary_scan_sql_rejects_bad_bounds_and_missing_index_without_schema_repair() {
        let (store, pool) = setup().await;
        for (cutoff, limit) in [(-1, 1), (200, 0), (200, 65)] {
            assert!(store
                .pending_observations(None, cutoff, limit)
                .await
                .is_err());
        }
        assert!(ObservationCursor::new(1, "tr_/invalid".into()).is_err());
        sqlx::query("DROP INDEX idx_ordinary_signed_observation_scan")
            .execute(&pool)
            .await
            .unwrap();
        assert!(store.pending_observations(None, 200, 2).await.is_err());
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_ordinary_signed_observation_scan'").fetch_one(&pool).await.unwrap(), 0);
    }
}
