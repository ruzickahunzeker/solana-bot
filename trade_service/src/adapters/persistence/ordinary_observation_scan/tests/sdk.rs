use super::*;

async fn insert_sdk(pool: &sqlx::SqlitePool, id: &str, created: i64, updated: i64) {
    insert(pool, id, created, updated).await;
    sqlx::query("UPDATE trades SET side='sell',pool='meteora-damm-v2' WHERE request_id=?")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    // A bounded discovery hint, not cryptographic authorization; capture still rejects this fixture.
    sqlx::query("UPDATE ordinary_execution_claims SET operation='ordinary-sdk-trade',sdk_prepared_json='{}',unsigned_message=x'0102',unsigned_sha256='hint',signed_sha256='hint',submission_stage='broadcasting' WHERE request_id=?")
        .bind(id).execute(pool).await.unwrap();
}

#[tokio::test]
async fn sdk_scan_sql_merges_both_indexed_pages_under_one_global_cursor_and_limit() {
    let (store, pool) = setup().await;
    for (i, id) in ["tr_c", "tr_a", "tr_f", "tr_b", "tr_d", "tr_e"]
        .into_iter()
        .enumerate()
    {
        if i % 2 == 0 {
            insert(&pool, id, 100, 100).await;
        } else {
            insert_sdk(&pool, id, 100, 100).await;
        }
    }
    let mut after = None;
    let mut seen = Vec::new();
    for expected_more in [true, true, false] {
        let page = store
            .pending_observations(after.as_ref(), 200, 2)
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.has_more, expected_more);
        seen.extend(page.entries.iter().map(|v| v.request_id().to_owned()));
        after = page.entries.last().cloned();
    }
    assert_eq!(seen, ["tr_a", "tr_b", "tr_c", "tr_d", "tr_e", "tr_f"]);
    assert!(store
        .pending_observations(after.as_ref(), 200, 2)
        .await
        .unwrap()
        .entries
        .is_empty());
    assert!(!SDK_PAGE_SQL.contains("OFFSET"));
}

#[tokio::test]
async fn sdk_scan_sql_filters_incomplete_fresh_finalized_and_non_sdk_damm_hints() {
    let (store, pool) = setup().await;
    for id in [
        "tr_good",
        "tr_unsigned",
        "tr_candidate",
        "tr_digest",
        "tr_prepared",
        "tr_signing",
        "tr_terminal",
        "tr_finalized",
        "tr_wrong_pool",
        "tr_wrong_method",
        "tr_mismatch",
        "tr_fresh",
    ] {
        insert_sdk(&pool, id, 100, 100).await;
    }
    for sql in [
        "UPDATE ordinary_execution_claims SET signed_transaction=NULL WHERE request_id='tr_unsigned'",
        "UPDATE ordinary_execution_claims SET sdk_prepared_json=NULL WHERE request_id='tr_candidate'",
        "UPDATE ordinary_execution_claims SET signed_sha256=NULL WHERE request_id='tr_digest'",
        "UPDATE ordinary_execution_claims SET submission_stage='prepared' WHERE request_id='tr_prepared'",
        "UPDATE ordinary_execution_claims SET submission_stage='signing' WHERE request_id='tr_signing'",
        "UPDATE trades SET status='confirmed' WHERE request_id='tr_terminal'",
        "UPDATE ordinary_execution_claims SET final_observation_json='{}' WHERE request_id='tr_finalized'",
        "UPDATE trades SET pool='raydium-cpmm' WHERE request_id='tr_wrong_pool'",
        "UPDATE trades SET execution_method='pumpapi' WHERE request_id='tr_wrong_method'",
        "UPDATE trades SET signature='other' WHERE request_id='tr_mismatch'",
        "UPDATE trades SET updated_at_ms=201 WHERE request_id='tr_fresh'",
    ] { sqlx::query(sql).execute(&pool).await.unwrap(); }
    let page = store.pending_observations(None, 200, 64).await.unwrap();
    assert_eq!(
        page.entries
            .iter()
            .map(ObservationCursor::request_id)
            .collect::<Vec<_>>(),
        ["tr_good"]
    );
    assert!(store.capture_observation("tr_good").await.is_err());
}

#[tokio::test]
async fn sdk_scan_sql_uses_its_own_partial_index_without_a_sort_or_legacy_index_change() {
    let (store, pool) = setup().await;
    for _ in 0..2 {
        store.initialize().await.unwrap();
    }
    for (query, index) in [
        (PAGE_SQL, "idx_ordinary_signed_observation_scan"),
        (SDK_PAGE_SQL, "idx_ordinary_sdk_observation_scan"),
    ] {
        let plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {query}"))
            .bind(200_i64)
            .bind(200_i64)
            .bind(-1_i64)
            .bind("")
            .bind(65_i64)
            .fetch_all(&pool)
            .await
            .unwrap();
        let details = plan
            .iter()
            .map(|r| r.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(details.contains(index), "{details}");
        assert!(!details.contains("USE TEMP B-TREE"), "{details}");
    }
}

#[tokio::test]
async fn sdk_scan_sql_missing_new_index_fails_even_when_legacy_rows_exist_without_fallback() {
    let (store, pool) = setup().await;
    insert(&pool, "tr_legacy", 100, 100).await;
    insert_sdk(&pool, "tr_sdk", 100, 100).await;
    sqlx::query("DROP INDEX idx_ordinary_sdk_observation_scan")
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.pending_observations(None, 200, 64).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='idx_ordinary_sdk_observation_scan'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn sdk_scan_buy_sell_and_aggregate_share_the_same_cursor_cutoff_and_original_indexes() {
    let (store, pool) = setup().await;
    for (id, buy, created) in [
        ("tr_a", true, 99),
        ("tr_b", false, 100),
        ("tr_d", true, 100),
        ("tr_e", false, 101),
        ("tr_f", true, 101),
    ] {
        insert_sdk(&pool, id, created, 100).await;
        if buy {
            sqlx::query("UPDATE trades SET side='buy' WHERE request_id=?")
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    insert(&pool, "tr_c", 100, 100).await;
    insert_sdk(&pool, "tr_fresh", 100, 201).await;
    sqlx::query("UPDATE trades SET side='buy' WHERE request_id='tr_fresh'")
        .execute(&pool)
        .await
        .unwrap();
    let mut after = None;
    let mut seen = Vec::new();
    for more in [true, true, false] {
        let page = store
            .pending_observations(after.as_ref(), 200, 2)
            .await
            .unwrap();
        assert_eq!(page.has_more, more);
        assert_eq!(page.entries.len(), 2);
        seen.extend(page.entries.iter().map(|v| v.request_id().to_owned()));
        after = page.entries.last().cloned();
    }
    assert_eq!(seen, ["tr_a", "tr_b", "tr_c", "tr_d", "tr_e", "tr_f"]);
    for id in seen {
        assert!(
            store.capture_observation(&id).await.is_err(),
            "hint must not authorize {id}"
        );
    }
}
