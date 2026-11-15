use anyhow::{ensure, Context, Result};
use sqlx::{Connection, Row, Sqlite, SqliteConnection, Transaction};

const INDEX: &str = "idx_ordinary_sdk_observation_scan";
const TABLE: &str = "ordinary_execution_claims";

pub(super) async fn ensure_index(
    tx: &mut Transaction<'_, Sqlite>,
    marked: bool,
    sql: &str,
) -> Result<()> {
    let existing = sqlx::query("SELECT type,tbl_name,sql FROM sqlite_schema WHERE name=?")
        .bind(INDEX)
        .fetch_optional(&mut **tx)
        .await?;
    if !marked {
        ensure!(
            existing.is_none(),
            "unmarked SDK observation index already exists; refusing adoption"
        );
        sqlx::raw_sql(sql).execute(&mut **tx).await?;
    }
    let actual = sqlx::query("SELECT type,tbl_name,sql FROM sqlite_schema WHERE name=?")
        .bind(INDEX)
        .fetch_optional(&mut **tx)
        .await?
        .context("marked SDK observation index is missing; refusing automatic repair")?;
    // Let SQLite parse the trusted DDL; don't normalize or parse user schema text ourselves.
    let mut reference = SqliteConnection::connect("sqlite::memory:").await?;
    sqlx::raw_sql("CREATE TABLE ordinary_execution_claims(created_at_ms INTEGER,request_id TEXT,schema_version INTEGER,operation TEXT,owner_nonce TEXT,sdk_prepared_json TEXT,unsigned_message BLOB,unsigned_sha256 TEXT,signed_signature TEXT,signed_transaction BLOB,signed_sha256 TEXT,final_observation_json TEXT,submission_stage TEXT)")
        .execute(&mut reference).await?;
    sqlx::raw_sql(sql).execute(&mut reference).await?;
    let expected: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='index' AND name=?")
            .bind(INDEX)
            .fetch_one(&mut reference)
            .await?;
    reference.close().await?;
    ensure!(
        actual.try_get::<&str, _>("type")? == "index"
            && actual.try_get::<&str, _>("tbl_name")? == TABLE
            && actual.try_get::<Option<&str>, _>("sql")? == Some(expected.as_str()),
        "SDK observation index DDL changed; refusing automatic repair"
    );
    let index = sqlx::query("SELECT * FROM pragma_index_list(?) WHERE name=?")
        .bind(TABLE)
        .bind(INDEX)
        .fetch_one(&mut **tx)
        .await?;
    ensure!(
        index.try_get::<i64, _>("unique")? == 0
            && index.try_get::<i64, _>("partial")? == 1
            && index.try_get::<&str, _>("origin")? == "c",
        "SDK observation index properties changed"
    );
    let columns = sqlx::query("SELECT * FROM pragma_index_xinfo(?) ORDER BY seqno")
        .bind(INDEX)
        .fetch_all(&mut **tx)
        .await?;
    ensure!(columns.len() == 3, "SDK observation index columns changed");
    for (position, name) in [Some("created_at_ms"), Some("request_id"), None]
        .into_iter()
        .enumerate()
    {
        let row = &columns[position];
        ensure!(
            row.try_get::<Option<&str>, _>("name")? == name
                && row.try_get::<i64, _>("desc")? == 0
                && row.try_get::<&str, _>("coll")? == "BINARY"
                && row.try_get::<i64, _>("key")? == i64::from(position < 2)
                && if position < 2 {
                    row.try_get::<i64, _>("cid")? >= 0
                } else {
                    row.try_get::<i64, _>("cid")? == -1
                },
            "SDK observation index column metadata changed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
