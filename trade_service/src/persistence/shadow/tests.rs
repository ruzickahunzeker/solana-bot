use super::*;
use crate::{
    config::{TradePersistenceConfig, TradePersistenceMode},
    domain::trade::TradeStatus,
    persistence::{
        tests::{normalized, record, request},
        TradePersistence,
    },
};

async fn store() -> Arc<TradePersistence> {
    Arc::new(
        TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Shadow,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap(),
    )
}
async fn insert(store: &TradePersistence, id: &str) -> TradeRecord {
    let request = request(" shadow-literal-key ", 1_000_000);
    let record = record(id, &request);
    store
        .mirror(&record, &request, Some(&normalized(&request)))
        .await
        .unwrap();
    record
}
fn counts(store: &TradePersistence) -> Counts {
    lock(&store.shadow.as_ref().unwrap().gate).counts
}
async fn closed(store: &TradePersistence) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !store.shadow_is_closed_for_tests() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shadow_drain_fifo_preserves_original_hash_json_key_and_last_update() {
    let store = store().await;
    let request = request(" literal-key ", 1_000_000);
    let mut record = record("shadow-fifo", &request);
    store
        .mirror(&record, &request, Some(&normalized(&request)))
        .await
        .unwrap();
    record.status = TradeStatus::Submitted;
    record.signature = Some("synthetic-stored-shadow-signature".into());
    record.updated_at_ms = 2;
    store.update(&record).await.unwrap();
    record.status = TradeStatus::Confirmed;
    record.updated_at_ms = 3;
    store.update(&record).await.unwrap();
    store.shutdown_shadow(Duration::from_secs(3)).await.unwrap();
    let saved = store.get(&record.request_id).await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(&record).unwrap()
    );
    let raw: (String, String, String) = sqlx::query_as(
        "SELECT idempotency_key,request_hash,request_json FROM trades WHERE request_id=?",
    )
    .bind(&record.request_id)
    .fetch_one(&store.pool_for_tests())
    .await
    .unwrap();
    assert_eq!(raw.0, request.idempotency_key);
    assert_eq!(
        raw.1,
        crate::persistence::request_hash(&record.wallet_id, &request, &normalized(&request))
            .unwrap()
    );
    assert_eq!(raw.2, serde_json::to_string(&request).unwrap());
    assert_eq!(
        counts(&store),
        Counts {
            accepted: 3,
            completed: 3,
            ..Default::default()
        }
    );
    store.shutdown_shadow(Duration::from_secs(1)).await.unwrap();
    assert_eq!(counts(&store).completed, 3);
}

#[tokio::test]
async fn shadow_drain_timeout_retains_original_writer_and_later_wait_finishes_once() {
    let store = store().await;
    let pool = store.pool_for_tests();
    let held = pool.acquire().await.unwrap();
    let record = insert(&store, "shadow-timeout").await;
    assert!(store
        .shutdown_shadow(Duration::from_millis(1))
        .await
        .unwrap_err()
        .to_string()
        .contains("timed out"));
    assert_eq!((counts(&store).accepted, counts(&store).completed), (1, 0));
    assert!(store
        .shadow
        .as_ref()
        .unwrap()
        .completion
        .lock()
        .await
        .task
        .is_some());
    drop(held);
    store.shutdown_shadow(Duration::from_secs(3)).await.unwrap();
    assert_eq!(
        store
            .get(&record.request_id)
            .await
            .unwrap()
            .unwrap()
            .request_id,
        record.request_id
    );
    assert_eq!(counts(&store).completed, 1);
}

#[tokio::test]
async fn shadow_drain_cancelled_waiter_does_not_discard_handle_or_restart_write() {
    let store = store().await;
    let pool = store.pool_for_tests();
    let held = pool.acquire().await.unwrap();
    let record = insert(&store, "shadow-cancelled").await;
    let owner = store.clone();
    let waiter = tokio::spawn(async move { owner.shutdown_shadow(Duration::from_secs(3)).await });
    closed(&store).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(store
        .shadow
        .as_ref()
        .unwrap()
        .completion
        .lock()
        .await
        .task
        .is_some());
    drop(held);
    store.shutdown_shadow(Duration::from_secs(3)).await.unwrap();
    assert!(store.get(&record.request_id).await.unwrap().is_some());
    assert_eq!(
        counts(&store),
        Counts {
            accepted: 1,
            completed: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn shadow_drain_concurrent_wait_budget_includes_the_existing_join_lock() {
    let store = store().await;
    let pool = store.pool_for_tests();
    let held = pool.acquire().await.unwrap();
    insert(&store, "shadow-concurrent").await;
    let owner = store.clone();
    let first = tokio::spawn(async move { owner.shutdown_shadow(Duration::from_secs(3)).await });
    closed(&store).await;
    let second = tokio::time::timeout(
        Duration::from_millis(200),
        store.shutdown_shadow(Duration::from_millis(1)),
    )
    .await
    .unwrap();
    assert!(second.unwrap_err().to_string().contains("timed out"));
    drop(held);
    first.await.unwrap().unwrap();
    store.shutdown_shadow(Duration::from_secs(1)).await.unwrap();
    assert_eq!(counts(&store).completed, 1);
}

#[tokio::test]
async fn shadow_drain_write_failure_remains_visible_after_original_health_recovers() {
    let store = store().await;
    let request = request("same-key", 1_000_000);
    store
        .update(&record("missing-shadow-row", &request))
        .await
        .unwrap();
    let mut valid = insert(&store, "valid-shadow-row").await;
    valid.status = TradeStatus::Simulated;
    valid.updated_at_ms = 2;
    store.update(&valid).await.unwrap();
    let error = store
        .shutdown_shadow(Duration::from_secs(3))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("failed=1"));
    assert!(
        store.healthy().await,
        "last successful write retains the original health rule"
    );
    assert_eq!(
        store.get(&valid.request_id).await.unwrap().unwrap().status,
        TradeStatus::Simulated
    );
    assert_eq!(
        counts(&store),
        Counts {
            accepted: 3,
            completed: 3,
            failed: 1,
            ..Default::default()
        }
    );
    assert!(store.shutdown_shadow(Duration::from_secs(1)).await.is_err());
}

#[tokio::test]
async fn shadow_drain_original_full_queue_rejection_cannot_be_erased_by_later_success() {
    let store = store().await;
    let mut record = insert(&store, "shadow-full").await;
    // No yield occurs in the Shadow enqueue path, so the original 1024-slot queue is deterministic.
    for i in 1..1_024 {
        record.status = TradeStatus::Simulated;
        record.updated_at_ms = i + 1;
        store.update(&record).await.unwrap();
    }
    let error = store.update(&record).await.unwrap_err();
    assert!(error
        .to_string()
        .starts_with("shadow persistence queue rejected update:"));
    assert_eq!(counts(&store).rejected, 1);
    assert!(store
        .shutdown_shadow(Duration::from_secs(10))
        .await
        .unwrap_err()
        .to_string()
        .contains("rejected=1"));
    assert_eq!(
        (counts(&store).accepted, counts(&store).completed),
        (1_024, 1_024)
    );
    assert_eq!(
        store
            .get(&record.request_id)
            .await
            .unwrap()
            .unwrap()
            .updated_at_ms,
        1_024
    );
    assert!(store.healthy().await);
}

#[tokio::test]
async fn shadow_drain_drop_closes_but_does_not_abort_accepted_fifo_writes() {
    let store = store().await;
    let pool = store.pool_for_tests();
    let held = pool.acquire().await.unwrap();
    let mut record = insert(&store, "shadow-drop").await;
    record.status = TradeStatus::Simulated;
    record.updated_at_ms = 2;
    store.update(&record).await.unwrap();
    let gate = store.shadow.as_ref().unwrap().gate.clone();
    drop(store);
    assert!(lock(&gate).sender.is_none());
    assert_eq!(lock(&gate).counts.completed, 0);
    drop(held);
    tokio::time::timeout(Duration::from_secs(3), async {
        while lock(&gate).counts.completed != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let saved: String = sqlx::query_scalar("SELECT status FROM trades WHERE request_id=?")
        .bind(&record.request_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved, "simulated");
}

#[tokio::test]
async fn shadow_drain_aborted_unpolled_writer_cannot_become_clean_on_repeated_wait() {
    let store = store().await;
    let record = insert(&store, "shadow-aborted").await;
    store
        .shadow
        .as_ref()
        .unwrap()
        .completion
        .lock()
        .await
        .task
        .as_ref()
        .unwrap()
        .abort();
    for _ in 0..2 {
        assert!(store
            .shutdown_shadow(Duration::from_secs(1))
            .await
            .unwrap_err()
            .to_string()
            .contains("task failed"));
    }
    assert_eq!(counts(&store).completed, 0);
    assert!(store.get(&record.request_id).await.unwrap().is_none());
    assert!(!store.healthy().await);
}

#[tokio::test]
async fn shadow_drain_close_rejects_new_inserts_and_updates_without_new_sql_work() {
    let store = store().await;
    store.shutdown_shadow(Duration::from_secs(1)).await.unwrap();
    let request = request("after-close", 1_000_000);
    let record = record("shadow-after-close", &request);
    let error = store
        .mirror(&record, &request, Some(&normalized(&request)))
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .starts_with("shadow persistence queue rejected write:"));
    assert!(store
        .update(&record)
        .await
        .unwrap_err()
        .to_string()
        .starts_with("shadow persistence queue rejected update:"));
    assert_eq!(
        counts(&store),
        Counts {
            rejected: 2,
            ..Default::default()
        }
    );
    assert!(store.get(&record.request_id).await.unwrap().is_none());
    assert!(store.shutdown_shadow(Duration::from_secs(1)).await.is_err());
}

#[tokio::test]
async fn shadow_drain_memory_and_durable_modes_do_not_close_or_replace_the_original_authority() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
        let store = TradePersistence::new(&TradePersistenceConfig {
            mode,
            database_url: if mode == TradePersistenceMode::Memory {
                "not-a-database"
            } else {
                "sqlite::memory:"
            }
            .into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap();
        store.shutdown_shadow(Duration::ZERO).await.unwrap();
        assert!(store.shadow.is_none());
        assert!(store.healthy().await);
        if mode == TradePersistenceMode::Durable {
            let request = request("original-durable", 1_000_000);
            let record = record("original-durable-row", &request);
            assert!(matches!(
                store
                    .reserve(&record, &request, &normalized(&request))
                    .await
                    .unwrap(),
                crate::persistence::DurableReserve::Created
            ));
            assert!(matches!(
                store
                    .reserve(&record, &request, &normalized(&request))
                    .await
                    .unwrap(),
                crate::persistence::DurableReserve::Duplicate(_)
            ));
        }
    }
}

#[tokio::test]
async fn shadow_drain_file_reopen_retains_terminal_row_and_original_snapshot() {
    let dir = std::env::temp_dir().join(format!("shadow-drain-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Shadow,
        database_url: format!("sqlite://{}", dir.join("public.sqlite").display()),
        busy_timeout_ms: 100,
    };
    let store = TradePersistence::new(&config).await.unwrap();
    let mut record = insert(&store, "shadow-file").await;
    record.status = TradeStatus::Simulated;
    record.updated_at_ms = 2;
    store.update(&record).await.unwrap();
    store.shutdown_shadow(Duration::from_secs(3)).await.unwrap();
    let pool = store.pool_for_tests();
    let before: (String, String, String) = sqlx::query_as(
        "SELECT idempotency_key,request_hash,request_json FROM trades WHERE request_id=?",
    )
    .bind(&record.request_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    pool.close().await;
    drop(store);
    let reopened = TradePersistence::new(&config).await.unwrap();
    assert_eq!(
        serde_json::to_value(reopened.get(&record.request_id).await.unwrap().unwrap()).unwrap(),
        serde_json::to_value(&record).unwrap()
    );
    let after: (String, String, String) = sqlx::query_as(
        "SELECT idempotency_key,request_hash,request_json FROM trades WHERE request_id=?",
    )
    .bind(&record.request_id)
    .fetch_one(&reopened.pool_for_tests())
    .await
    .unwrap();
    assert_eq!(before, after);
    reopened
        .shutdown_shadow(Duration::from_secs(1))
        .await
        .unwrap();
    reopened.pool_for_tests().close().await;
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn shadow_drain_counter_overflow_cannot_report_clean_completion() {
    let mut counts = Counts {
        accepted: u64::MAX,
        completed: u64::MAX,
        ..Default::default()
    };
    counts.accepted();
    assert!(counts.overflowed && !counts.clean());
}
