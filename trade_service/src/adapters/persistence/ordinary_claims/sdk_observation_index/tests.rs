use super::*;
use crate::{
    adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
    config::{TradePersistenceConfig, TradePersistenceMode},
    persistence::TradePersistence,
};
use sqlx::SqlitePool;

const MARKER: &str = "010_ordinary_sdk_observation_scan";
const DDL: &str = include_str!("../../../../../migrations/010_ordinary_sdk_observation_scan.sql");

async fn setup() -> (SqliteOrdinaryExecutionClaims, SqlitePool) {
    let store = TradePersistence::new(&TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: "sqlite::memory:".into(),
        busy_timeout_ms: 2000,
    })
    .await
    .unwrap();
    let pool = store.pool_for_tests();
    sqlx::query(&format!("DROP INDEX {INDEX}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM ordinary_submission_migrations WHERE name=?")
        .bind(MARKER)
        .execute(&pool)
        .await
        .unwrap();
    (store.ordinary_execution_claims().unwrap(), pool)
}
async fn schema(pool: &SqlitePool) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")
        .fetch_all(pool)
        .await
        .unwrap()
}
async fn rows(pool: &SqlitePool, table: &str) -> Vec<Vec<String>> {
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
            .bind(table)
            .fetch_all(pool)
            .await
            .unwrap();
    let projection = names
        .iter()
        .flat_map(|n| [format!("hex(\"{n}\")"), format!("typeof(\"{n}\")")])
        .collect::<Vec<_>>()
        .join(",");
    sqlx::query(&format!("SELECT {projection} FROM {table} ORDER BY 1"))
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (0..names.len() * 2).map(|i| row.get(i)).collect())
        .collect()
}

#[tokio::test]
async fn sdk_scan_migration_preserves_old_rows_types_parent_and_legacy_index_then_is_idempotent() {
    let (store, pool) = setup().await;
    for (i, operation) in ["ordinary-sol-buy", "ordinary-sdk-trade"]
        .into_iter()
        .enumerate()
    {
        let id = format!("tr_index_{i}");
        sqlx::query("INSERT INTO trades(request_id,wallet_id,wallet_pubkey,idempotency_key,request_hash,request_json,side,pool,mint,execution_method,status,created_at_ms,updated_at_ms) VALUES(?,'fixture-wallet','public-key',?,'hash','{}','sell','meteora-damm-v2','public-mint','sdk','unknown',1,2)")
            .bind(&id).bind(format!(" original-{i} ")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms,owner_nonce,submission_stage,sdk_prepared_json,unsigned_message,signed_transaction,submission_error) VALUES(?,1,?,'hash',1,?,'unknown','opaque original',?,?,?)")
            .bind(&id).bind(operation).bind("a".repeat(32)).bind(vec![0_u8, 255, i as u8]).bind(vec![255_u8, 0, i as u8])
            .bind("original error\0suffix").execute(&pool).await.unwrap();
    }
    let claims = rows(&pool, TABLE).await;
    let parents = rows(&pool, "trades").await;
    let legacy: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE name='idx_ordinary_signed_observation_scan'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    store.initialize().await.unwrap();
    let expected = schema(&pool).await;
    for _ in 0..3 {
        store.initialize().await.unwrap();
        assert_eq!(schema(&pool).await, expected);
        assert_eq!(rows(&pool, TABLE).await, claims);
        assert_eq!(rows(&pool, "trades").await, parents);
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT sql FROM sqlite_schema WHERE name='idx_ordinary_signed_observation_scan'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        legacy
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?"
        )
        .bind(MARKER)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn sdk_scan_migration_rejects_unmarked_existing_index_without_adopting_or_repairing() {
    for sql in [
        DDL,
        "CREATE INDEX idx_ordinary_sdk_observation_scan ON ordinary_execution_claims(request_hash)",
    ] {
        let (store, pool) = setup().await;
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        let before = schema(&pool).await;
        assert!(store.initialize().await.is_err());
        assert_eq!(schema(&pool).await, before);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?"
            )
            .bind(MARKER)
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn sdk_scan_migration_rejects_marked_missing_changed_or_foreign_index_without_repairs() {
    for replacement in [None,
        Some("CREATE INDEX idx_ordinary_sdk_observation_scan ON ordinary_execution_claims(created_at_ms DESC,request_id)"),
        Some("CREATE INDEX idx_ordinary_sdk_observation_scan ON ordinary_execution_claims(created_at_ms,request_id) WHERE operation='ordinary-sol-buy'"),
        Some("CREATE INDEX idx_ordinary_sdk_observation_scan ON trades(created_at_ms,request_id)"),
    ] {
        let (store, pool) = setup().await;
        store.initialize().await.unwrap();
        sqlx::query(&format!("DROP INDEX {INDEX}")).execute(&pool).await.unwrap();
        if let Some(sql) = replacement { sqlx::raw_sql(sql).execute(&pool).await.unwrap(); }
        let before = schema(&pool).await;
        let markers = rows(&pool, "ordinary_submission_migrations").await;
        assert!(store.initialize().await.is_err());
        assert_eq!(schema(&pool).await, before);
        assert_eq!(rows(&pool, "ordinary_submission_migrations").await, markers);
    }
}

#[tokio::test]
async fn sdk_scan_migration_marker_failure_rolls_back_new_index_then_can_retry_once() {
    let (store, pool) = setup().await;
    sqlx::raw_sql("CREATE TRIGGER deny_sdk_scan_marker BEFORE INSERT ON ordinary_submission_migrations WHEN NEW.name='010_ordinary_sdk_observation_scan' BEGIN SELECT RAISE(ABORT,'synthetic SDK scan marker failure'); END;")
        .execute(&pool).await.unwrap();
    let before = schema(&pool).await;
    let markers = rows(&pool, "ordinary_submission_migrations").await;
    assert!(store.initialize().await.is_err());
    assert_eq!(schema(&pool).await, before);
    assert_eq!(rows(&pool, "ordinary_submission_migrations").await, markers);
    sqlx::query("DROP TRIGGER deny_sdk_scan_marker")
        .execute(&pool)
        .await
        .unwrap();
    store.initialize().await.unwrap();
    store.initialize().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?"
        )
        .bind(MARKER)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn sdk_scan_migration_concurrent_initializers_share_one_original_index_and_marker() {
    let (store, pool) = setup().await;
    let other = SqliteOrdinaryExecutionClaims::new(pool.clone());
    let (a, b) = tokio::join!(store.initialize(), other.initialize());
    a.unwrap();
    b.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE name=?")
            .bind(INDEX)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?"
        )
        .bind(MARKER)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
}
