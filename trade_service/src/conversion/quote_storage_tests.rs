//! Corrupt-row and transactional quote evidence in disposable SQLite only.
use super::*;
use std::sync::Arc;

#[tokio::test]
async fn wsol_quote_storage_corrupt_row_does_not_panic_or_publish_raw_payload() {
    let store = Arc::new(ConversionStore::new("sqlite::memory:").await.unwrap());
    store.save_quote(&contract_tests::quote()).await.unwrap();
    sqlx::query("UPDATE conversion_quotes SET quote_json=X'FF'")
        .execute(&store.pool)
        .await
        .unwrap();
    let reader = store.clone();
    let result = tokio::spawn(async move { reader.get_quote("cq_fixture").await }).await;
    let error = result
        .expect("corrupt storage must return an error, not panic")
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<WsolFailure>().unwrap().kind,
        crate::application::wsol_failure::WsolFailureKind::Integrity
    );
    assert_eq!(error.to_string(), "stored WSOL quote could not be decoded");
}

#[tokio::test]
async fn wsol_quote_storage_requires_row_identity_and_expiry_but_keeps_expired_history_readable() {
    for assignment in [
        "quote_json='null'",
        "quote_json='{'",
        "quote_json=CAST(X'FF' AS TEXT)",
        "expires_at_ms='not-an-integer'",
        "expires_at_ms=-1",
        "expires_at_ms=999",
    ] {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        let quote = contract_tests::quote();
        store.save_quote(&quote).await.unwrap();
        assert_eq!(
            store.get_quote(&quote.quote_id).await.unwrap(),
            Some(quote.clone())
        );
        sqlx::query(&format!("UPDATE conversion_quotes SET {assignment}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let error = store.get_quote(&quote.quote_id).await.unwrap_err();
        assert!(
            error.downcast_ref::<WsolFailure>().is_some(),
            "{assignment}"
        );
    }
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let quote = contract_tests::quote();
    store.save_quote(&quote).await.unwrap();
    let mut foreign = quote.clone();
    foreign.quote_id = "other-quote".into();
    sqlx::query("UPDATE conversion_quotes SET quote_json=?")
        .bind(serde_json::to_string(&foreign).unwrap())
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store.get_quote(&quote.quote_id).await.is_err());
    assert!(store.get_quote("absent").await.unwrap().is_none());
}

#[tokio::test]
async fn wsol_quote_storage_insert_and_readback_faults_rollback_without_publishing_a_quote() {
    for (timing, body) in [
        ("BEFORE", "SELECT RAISE(IGNORE);"),
        (
            "BEFORE",
            "SELECT RAISE(FAIL,'synthetic quote insert failure');",
        ),
        (
            "AFTER",
            "DELETE FROM conversion_quotes WHERE quote_id=NEW.quote_id;",
        ),
        ("AFTER", "UPDATE conversion_quotes SET quote_id='corrupt';"),
        ("AFTER", "UPDATE conversion_quotes SET expires_at_ms=-1;"),
        ("AFTER", "UPDATE conversion_quotes SET quote_json=X'FF';"),
        ("AFTER", "UPDATE conversion_quotes SET quote_json='null';"),
        (
            "AFTER",
            "UPDATE conversion_quotes SET quote_json=quote_json || ' ';",
        ),
    ] {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(&format!(
            "CREATE TRIGGER quote_fault {timing} INSERT ON conversion_quotes BEGIN {body} END;"
        ))
        .execute(&store.pool)
        .await
        .unwrap();
        assert!(
            store.save_quote(&contract_tests::quote()).await.is_err(),
            "{body}"
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversion_quotes")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{body}");
    }
    let original_json = serde_json::to_value(contract_tests::quote()).unwrap();
    for field in original_json.as_object().unwrap().keys() {
        let store = ConversionStore::new("sqlite::memory:").await.unwrap();
        let quote = contract_tests::quote();
        let mut changed = original_json.clone();
        changed[field] = match &original_json[field] {
            serde_json::Value::Number(number) => serde_json::json!(number.as_u64().unwrap() + 1),
            serde_json::Value::Bool(value) => serde_json::json!(!value),
            serde_json::Value::String(_) if field == "direction" => serde_json::json!("wrap"),
            serde_json::Value::String(value) => serde_json::json!(format!("{value}x")),
            _ => unreachable!(),
        };
        // Bind altered JSON through a fixture table, not SQL string interpolation.
        sqlx::query("CREATE TABLE altered_quote(payload TEXT NOT NULL)")
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO altered_quote VALUES(?)")
            .bind(serde_json::to_string(&changed).unwrap())
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TRIGGER quote_fault AFTER INSERT ON conversion_quotes BEGIN UPDATE conversion_quotes SET quote_json=(SELECT payload FROM altered_quote); END;")
            .execute(&store.pool).await.unwrap();
        assert!(store.save_quote(&quote).await.is_err(), "{field}");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversion_quotes")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{field}");
    }
}

#[tokio::test]
async fn wsol_quote_storage_keeps_original_json_and_duplicate_id_and_overflow_behavior() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let quote = contract_tests::quote();
    store.save_quote(&quote).await.unwrap();
    let raw: String = sqlx::query_scalar("SELECT quote_json FROM conversion_quotes")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(raw, serde_json::to_string(&quote).unwrap());
    assert!(store.save_quote(&quote).await.is_err());
    let mut oversized = quote.clone();
    oversized.quote_id = "overflow".into();
    oversized.expires_at_ms = u64::MAX;
    assert!(store.save_quote(&oversized).await.is_err());
    assert!(store.get_quote("overflow").await.unwrap().is_none());
    assert_eq!(
        store.get_quote(&quote.quote_id).await.unwrap(),
        Some(quote.clone())
    );
    let mut legacy = serde_json::to_value(&quote).unwrap();
    legacy["unknown_historical_field"] = serde_json::json!("ignored as before");
    let json = serde_json::to_string_pretty(&legacy).unwrap();
    sqlx::query("UPDATE conversion_quotes SET quote_json=?")
        .bind(&json)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_eq!(store.get_quote(&quote.quote_id).await.unwrap(), Some(quote));
    let stored: String = sqlx::query_scalar("SELECT quote_json FROM conversion_quotes")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(stored, json);
}

#[tokio::test]
async fn wsol_quote_storage_file_reopen_read_only_checks_do_not_rewrite_bad_history() {
    let dir = std::env::temp_dir().join(format!("wsol-quote-storage-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let store = ConversionStore::new(&url).await.unwrap();
    let quote = contract_tests::quote();
    store.save_quote(&quote).await.unwrap();
    let mut bad = quote.clone();
    bad.quote_id = "bad-quote".into();
    store.save_quote(&bad).await.unwrap();
    sqlx::query("UPDATE conversion_quotes SET expires_at_ms=0 WHERE quote_id='bad-quote'")
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;
    let options = SqliteConnectOptions::from_str(&url)
        .unwrap()
        .read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let readonly = ConversionStore { pool };
    for _ in 0..3 {
        assert_eq!(
            readonly.get_quote(&quote.quote_id).await.unwrap(),
            Some(quote.clone())
        );
        assert!(readonly.get_quote("bad-quote").await.is_err());
    }
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT quote_id,quote_json,expires_at_ms FROM conversion_quotes ORDER BY quote_id",
    )
    .fetch_all(&readonly.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                bad.quote_id.clone(),
                serde_json::to_string(&bad).unwrap(),
                0
            ),
            (
                quote.quote_id.clone(),
                serde_json::to_string(&quote).unwrap(),
                1000
            ),
        ]
    );
    readonly.pool.close().await;
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
