use super::*;
use crate::adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims;
use crate::adapters::persistence::ordinary_sdk_preparation::validate_sdk_row as validate_sdk_claim;
use sqlx::sqlite::SqlitePoolOptions;

const UPGRADE: &str = include_str!("../../../../../migrations/008_ordinary_claim_operations.sql");

async fn legacy_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for sql in [
        include_str!("../../../../../migrations/001_trade_persistence.sql"),
        super::super::MIGRATION,
        include_str!("../../../../../migrations/005_ordinary_signed_submissions.sql"),
        include_str!("../../../../../migrations/006_ordinary_final_observations.sql"),
        include_str!("../../../../../migrations/007_ordinary_observation_scan.sql"),
        "CREATE TABLE ordinary_submission_migrations(name TEXT PRIMARY KEY NOT NULL); \
         INSERT INTO ordinary_submission_migrations VALUES('005_ordinary_signed_submissions'), \
         ('006_ordinary_final_observations'),('007_ordinary_observation_scan');",
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    for (index, stage) in [
        "claimed",
        "preparing",
        "prepared",
        "signing",
        "signed",
        "submitted",
        "unknown",
        "failed",
        "finalized",
    ]
    .iter()
    .enumerate()
    {
        let id = format!("tr_legacy_{index}");
        sqlx::query("INSERT INTO trades(request_id,wallet_id,wallet_pubkey,idempotency_key,request_hash,request_json,side,pool,mint,execution_method,status,created_at_ms,updated_at_ms) VALUES(?,'fixture-wallet','public-fixture',?,'legacy-hash','{}','buy','pump-amm','public-mint','sdk','unknown',1,2)")
            .bind(&id).bind(format!(" key-{index} ")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms,owner_nonce,submission_stage,bound_quote_json,preflight_json,unsigned_message,unsigned_sha256,signed_signature,signed_transaction,signed_sha256,submission_error,final_observation_json) VALUES(?,1,'ordinary-sol-buy','legacy-hash',?,NULL,?,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL)")
            .bind(&id).bind(index as i64).bind(stage).execute(&pool).await.unwrap();
        if index > 0 {
            sqlx::query("UPDATE ordinary_execution_claims SET owner_nonce=?,bound_quote_json=?,preflight_json=?,unsigned_message=?,unsigned_sha256=?,signed_signature=?,signed_transaction=?,signed_sha256=?,submission_error=?,final_observation_json=? WHERE request_id=?")
                .bind("a".repeat(32)).bind(" {\"fixture\":1} \n").bind("{}").bind(vec![0_u8, 255, index as u8])
                .bind("old-unsigned-hash").bind("old-signature").bind(vec![255_u8, 0, index as u8])
                .bind("old-signed-hash").bind("fixture-error\0suffix").bind("{\"fixture\":true}")
                .bind(id).execute(&pool).await.unwrap();
        }
    }
    pool
}

async fn table_snapshot(pool: &sqlx::SqlitePool, table: &str) -> Vec<Vec<String>> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info(?) WHERE name != 'sdk_prepared_json' ORDER BY cid",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap();
    let projection = names
        .iter()
        .flat_map(|name| [format!("quote(\"{name}\")"), format!("typeof(\"{name}\")")])
        .collect::<Vec<_>>()
        .join(",");
    sqlx::query(&format!("SELECT {projection} FROM {table} ORDER BY 1"))
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (0..names.len() * 2)
                .map(|i| row.try_get(i).unwrap())
                .collect()
        })
        .collect()
}

async fn schema_snapshot(pool: &sqlx::SqlitePool) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn upgrade_preserves_every_legacy_column_storage_class_parent_and_index_and_runs_once() {
    let pool = legacy_pool().await;
    let claims = table_snapshot(&pool, TABLE).await;
    let parents = table_snapshot(&pool, "trades").await;
    let index: String = sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name=?")
        .bind(INDEX)
        .fetch_one(&pool)
        .await
        .unwrap();
    let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
    store.initialize().await.unwrap();
    let schema = schema_snapshot(&pool).await;
    for _ in 0..2 {
        store.initialize().await.unwrap();
        assert_eq!(schema_snapshot(&pool).await, schema);
        assert_eq!(table_snapshot(&pool, TABLE).await, claims);
        assert_eq!(table_snapshot(&pool, "trades").await, parents);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT sql FROM sqlite_schema WHERE name=?")
                .bind(INDEX)
                .fetch_one(&pool)
                .await
                .unwrap(),
            index
        );
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name='008_ordinary_claim_operations'")
            .fetch_one(&pool).await.unwrap(), 1);
        assert!(sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty());
    }
    assert!(
        sqlx::query("DELETE FROM trades WHERE request_id='tr_legacy_0'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms) VALUES('missing',1,'ordinary-sdk-trade','hash',1)")
        .execute(&pool).await.is_err());
}

#[tokio::test]
async fn unsupported_schema_dependencies_and_index_changes_roll_back_without_marking_upgrade() {
    for mutation in [
        "ALTER TABLE ordinary_execution_claims ADD COLUMN unknown_data BLOB",
        "CREATE TRIGGER keep_audit AFTER UPDATE ON ordinary_execution_claims BEGIN SELECT 1; END",
        "CREATE TABLE dependent(id TEXT REFERENCES ordinary_execution_claims(request_id))",
        "CREATE INDEX extra_claim_index ON ordinary_execution_claims(request_hash)",
        "CREATE TABLE ordinary_execution_claims_operation_upgrade(data BLOB)",
        "DROP INDEX idx_ordinary_signed_observation_scan",
        "DROP INDEX idx_ordinary_signed_observation_scan; CREATE INDEX idx_ordinary_signed_observation_scan ON ordinary_execution_claims(created_at_ms DESC,request_id) WHERE schema_version=1",
        "DROP INDEX idx_ordinary_signed_observation_scan; CREATE INDEX idx_ordinary_signed_observation_scan ON ordinary_execution_claims(created_at_ms,request_id) WHERE operation='ordinary-sdk-trade'",
        "CREATE VIEW dependent_view AS SELECT request_id FROM ordinary_execution_claims",
        "CREATE TRIGGER dependent_trigger AFTER INSERT ON trades BEGIN SELECT request_id FROM ordinary_execution_claims; END",
    ] {
        let pool = legacy_pool().await;
        sqlx::raw_sql(mutation).execute(&pool).await.unwrap();
        let before = schema_snapshot(&pool).await;
        let claims = table_snapshot(&pool, TABLE).await;
        let parents = table_snapshot(&pool, "trades").await;
        let markers = table_snapshot(&pool, "ordinary_submission_migrations").await;
        let result = SqliteOrdinaryExecutionClaims::new(pool.clone()).initialize().await;
        assert!(result.is_err(), "must refuse unsupported schema: {mutation}");
        assert_eq!(schema_snapshot(&pool).await, before, "{mutation}");
        assert_eq!(table_snapshot(&pool, TABLE).await, claims, "{mutation}");
        assert_eq!(table_snapshot(&pool, "trades").await, parents, "{mutation}");
        assert_eq!(table_snapshot(&pool, "ordinary_submission_migrations").await, markers, "{mutation}");
    }
}

#[tokio::test]
async fn copy_corruption_or_storage_class_changes_abort_before_removing_the_original_table() {
    for suffix in [
        "DELETE FROM ordinary_execution_claims_operation_upgrade WHERE request_id='tr_legacy_0';",
        "UPDATE ordinary_execution_claims_operation_upgrade SET signed_transaction=x'FF' WHERE request_id='tr_legacy_1';",
        "UPDATE ordinary_execution_claims_operation_upgrade SET submission_error=CAST(submission_error AS BLOB) WHERE request_id='tr_legacy_2';",
        "UPDATE ordinary_execution_claims_operation_upgrade SET created_at_ms=0.5 WHERE request_id='tr_legacy_3';",
    ] {
        let pool = legacy_pool().await;
        let before = schema_snapshot(&pool).await;
        let claims = table_snapshot(&pool, TABLE).await;
        let mut tx = pool.begin().await.unwrap();
        assert!(apply(&mut tx, &format!("{UPGRADE}\n{suffix}")).await.is_err());
        tx.rollback().await.unwrap();
        assert_eq!(schema_snapshot(&pool).await, before);
        assert_eq!(table_snapshot(&pool, TABLE).await, claims);
    }
}

#[tokio::test]
async fn migration_refuses_disabled_foreign_keys_and_unrecognized_table_constraints() {
    for mutation in [
        "PRAGMA foreign_keys=OFF",
        "PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=replace(sql,'CHECK(created_at_ms >= 0)','CHECK(created_at_ms >= 0 AND created_at_ms < 100)') WHERE name='ordinary_execution_claims'; PRAGMA writable_schema=OFF; PRAGMA schema_version=999",
    ] {
        let pool = legacy_pool().await;
        sqlx::raw_sql(mutation).execute(&pool).await.unwrap();
        let schema = schema_snapshot(&pool).await;
        let claims = table_snapshot(&pool, TABLE).await;
        assert!(SqliteOrdinaryExecutionClaims::new(pool.clone()).initialize().await.is_err());
        assert_eq!(schema_snapshot(&pool).await, schema);
        assert_eq!(table_snapshot(&pool, TABLE).await, claims);
    }
}

#[tokio::test]
async fn sdk_claim_validation_refuses_quote_metadata_signed_artifacts_and_unowned_rows() {
    let pool = legacy_pool().await;
    SqliteOrdinaryExecutionClaims::new(pool.clone())
        .initialize()
        .await
        .unwrap();
    sqlx::query("UPDATE ordinary_execution_claims SET operation='ordinary-sdk-trade',owner_nonce=? WHERE request_id='tr_legacy_0'")
        .bind("b".repeat(32)).execute(&pool).await.unwrap();
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id='tr_legacy_0'")
        .fetch_one(&pool)
        .await
        .unwrap();
    validate_sdk_claim(&row).unwrap();
    for column in &COLUMNS[7..] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("PRAGMA ignore_check_constraints=ON")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query(&format!(
            "UPDATE ordinary_execution_claims SET {column}=? WHERE request_id='tr_legacy_0'"
        ))
        .bind("unexpected-metadata")
        .execute(&mut *tx)
        .await
        .unwrap();
        let row =
            sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id='tr_legacy_0'")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(validate_sdk_claim(&row).is_err(), "{column}");
        tx.rollback().await.unwrap();
    }
    for (column, value) in [
        ("owner_nonce", None),
        ("owner_nonce", Some("bad")),
        ("submission_stage", Some("signed")),
    ] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(&format!(
            "UPDATE ordinary_execution_claims SET {column}=? WHERE request_id='tr_legacy_0'"
        ))
        .bind(value)
        .execute(&mut *tx)
        .await
        .unwrap();
        let row =
            sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id='tr_legacy_0'")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(validate_sdk_claim(&row).is_err(), "{column}");
        tx.rollback().await.unwrap();
    }
}
