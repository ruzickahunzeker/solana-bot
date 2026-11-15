use anyhow::{ensure, Context, Result};
use sqlx::{Connection, Row, Sqlite, SqliteConnection, Transaction};

const TABLE: &str = "ordinary_execution_claims";
const STAGING: &str = "ordinary_execution_claims_operation_upgrade";
const INDEX: &str = "idx_ordinary_signed_observation_scan";
const COLUMNS: &[&str] = &[
    "request_id",
    "schema_version",
    "operation",
    "request_hash",
    "created_at_ms",
    "owner_nonce",
    "submission_stage",
    "bound_quote_json",
    "preflight_json",
    "unsigned_message",
    "unsigned_sha256",
    "signed_signature",
    "signed_transaction",
    "signed_sha256",
    "submission_error",
    "final_observation_json",
];

pub(super) async fn apply(tx: &mut Transaction<'_, Sqlite>, sql: &str) -> Result<()> {
    let old_index = preflight(tx).await?;
    sqlx::raw_sql(sql).execute(&mut **tx).await?;
    // Compare storage class as well as value, before either original object is removed.
    let projection = COLUMNS
        .iter()
        .flat_map(|name| [format!("\"{name}\""), format!("typeof(\"{name}\")")])
        .collect::<Vec<_>>()
        .join(",");
    for (left, right) in [(TABLE, STAGING), (STAGING, TABLE)] {
        let difference: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM (SELECT {projection} FROM {left} EXCEPT SELECT {projection} FROM {right})"
        )).fetch_one(&mut **tx).await?;
        ensure!(
            difference == 0,
            "ordinary operation migration changed existing claim bytes or types"
        );
    }
    sqlx::query("DROP TABLE ordinary_execution_claims")
        .execute(&mut **tx)
        .await?;
    sqlx::query("ALTER TABLE ordinary_execution_claims_operation_upgrade RENAME TO ordinary_execution_claims").execute(&mut **tx).await?;
    sqlx::raw_sql(include_str!(
        "../../../../migrations/007_ordinary_observation_scan.sql"
    ))
    .execute(&mut **tx)
    .await?;
    let new_index: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='index' AND name=?")
            .bind(INDEX)
            .fetch_one(&mut **tx)
            .await?;
    ensure!(
        new_index == old_index,
        "ordinary operation migration changed the legacy observation index"
    );
    ensure!(
        sqlx::query("SELECT * FROM pragma_foreign_key_check(?)")
            .bind(TABLE)
            .fetch_all(&mut **tx)
            .await?
            .is_empty(),
        "ordinary operation migration has a foreign key violation"
    );
    Ok(())
}

async fn preflight(tx: &mut Transaction<'_, Sqlite>) -> Result<String> {
    let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut **tx)
        .await?;
    ensure!(
        enabled == 1,
        "ordinary operation migration requires foreign keys enabled"
    );
    let staging: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name=?")
        .bind(STAGING)
        .fetch_one(&mut **tx)
        .await?;
    ensure!(
        staging == 0,
        "ordinary operation migration staging object already exists"
    );
    // SQLite PRAGMAs omit CHECK clauses and table options. Reproduce the trusted
    // legacy DDL in an isolated connection rather than parsing sqlite_schema SQL.
    let mut reference = SqliteConnection::connect("sqlite::memory:").await?;
    for sql in [
        include_str!("../../../../migrations/001_trade_persistence.sql"),
        super::MIGRATION,
        include_str!("../../../../migrations/005_ordinary_signed_submissions.sql"),
        include_str!("../../../../migrations/006_ordinary_final_observations.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&mut reference).await?;
    }
    let expected: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='table' AND name=?")
            .bind(TABLE)
            .fetch_one(&mut reference)
            .await?;
    reference.close().await?;
    let actual: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='table' AND name=?")
            .bind(TABLE)
            .fetch_one(&mut **tx)
            .await?;
    ensure!(
        actual == expected,
        "ordinary operation migration encountered unsupported legacy table DDL"
    );
    let columns = sqlx::query("SELECT * FROM pragma_table_xinfo(?) ORDER BY cid")
        .bind(TABLE)
        .fetch_all(&mut **tx)
        .await?;
    ensure!(
        columns.len() == COLUMNS.len(),
        "ordinary operation migration encountered unknown columns"
    );
    for (index, row) in columns.iter().enumerate() {
        let name = COLUMNS[index];
        let kind = if ["schema_version", "created_at_ms"].contains(&name) {
            "INTEGER"
        } else if ["unsigned_message", "signed_transaction"].contains(&name) {
            "BLOB"
        } else {
            "TEXT"
        };
        let required = i64::from(index < 5 || name == "submission_stage");
        let default = if name == "submission_stage" {
            Some("'claimed'")
        } else {
            None
        };
        ensure!(
            row.try_get::<&str, _>("name")? == name
                && row.try_get::<&str, _>("type")? == kind
                && row.try_get::<i64, _>("notnull")? == required
                && row.try_get::<Option<&str>, _>("dflt_value")? == default
                && row.try_get::<i64, _>("pk")? == i64::from(index == 0)
                && row.try_get::<i64, _>("hidden")? == 0,
            "ordinary operation migration encountered an unsupported column definition"
        );
    }
    let foreign = sqlx::query("SELECT * FROM pragma_foreign_key_list(?)")
        .bind(TABLE)
        .fetch_all(&mut **tx)
        .await?;
    ensure!(
        foreign.len() == 1,
        "ordinary operation migration encountered unknown foreign keys"
    );
    let key = &foreign[0];
    ensure!(
        key.try_get::<&str, _>("table")? == "trades"
            && key.try_get::<&str, _>("from")? == "request_id"
            && key.try_get::<&str, _>("to")? == "request_id"
            && key.try_get::<&str, _>("on_delete")? == "RESTRICT"
            && key.try_get::<&str, _>("on_update")? == "NO ACTION",
        "ordinary operation migration parent link changed"
    );
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&mut **tx)
    .await?;
    for table in tables {
        let incoming = sqlx::query("SELECT * FROM pragma_foreign_key_list(?)")
            .bind(table)
            .fetch_all(&mut **tx)
            .await?;
        for key in incoming {
            ensure!(
                key.try_get::<&str, _>("table")? != TABLE,
                "ordinary operation migration cannot rebuild a referenced claims table"
            );
        }
    }
    let triggers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND tbl_name=?",
    )
    .bind(TABLE)
    .fetch_one(&mut **tx)
    .await?;
    ensure!(
        triggers == 0,
        "ordinary operation migration cannot discard claim triggers"
    );
    let indexes = sqlx::query("SELECT * FROM pragma_index_list(?)")
        .bind(TABLE)
        .fetch_all(&mut **tx)
        .await?;
    ensure!(
        indexes.len() == 2,
        "ordinary operation migration encountered unknown indexes"
    );
    for row in indexes {
        let name: String = row.try_get("name")?;
        let origin: &str = row.try_get("origin")?;
        let expected: &[&str] = if origin == "pk" {
            ensure!(
                row.try_get::<i64, _>("unique")? == 1 && row.try_get::<i64, _>("partial")? == 0,
                "ordinary operation migration primary index changed"
            );
            &["request_id"]
        } else {
            ensure!(
                name == INDEX
                    && origin == "c"
                    && row.try_get::<i64, _>("unique")? == 0
                    && row.try_get::<i64, _>("partial")? == 1,
                "ordinary operation migration observation index changed"
            );
            &["created_at_ms", "request_id"]
        };
        let keys = sqlx::query("SELECT * FROM pragma_index_xinfo(?) WHERE key=1 ORDER BY seqno")
            .bind(&name)
            .fetch_all(&mut **tx)
            .await?;
        ensure!(
            keys.len() == expected.len(),
            "ordinary operation migration index keys changed"
        );
        for (row, expected) in keys.iter().zip(expected) {
            ensure!(
                row.try_get::<Option<&str>, _>("name")? == Some(*expected)
                    && row.try_get::<&str, _>("coll")? == "BINARY"
                    && row.try_get::<i64, _>("desc")? == 0,
                "ordinary operation migration index collation or expression changed"
            );
        }
    }
    sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='index' AND name=?")
        .bind(INDEX)
        .fetch_optional(&mut **tx)
        .await?
        .context("ordinary operation migration observation index is missing")
}

#[cfg(test)]
mod tests;
