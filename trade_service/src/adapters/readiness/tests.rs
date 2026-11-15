use super::*;
use crate::{domain::blockhash::Blockhash, state::now_ms};
use std::time::Duration;
use tokio::sync::oneshot;

#[tokio::test]
async fn readiness_health_samples_expired_cache_after_pending_rpc_not_before() {
    let cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([7; 32]),
        now_ms(),
    ));
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let task = {
        let cache = cache.clone();
        tokio::spawn(async move {
            observe_health(
                &cache,
                100,
                async {
                    entered_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                    true
                },
                async { true },
            )
            .await
        })
    };
    entered_rx.await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    release_tx.send(()).unwrap();
    let (fresh, age, rpc, database) = task.await.unwrap();
    assert!(age.unwrap() > 100);
    assert!(!fresh, "stale pre-RPC freshness must not authorize ready");
    assert!(rpc && database);
}

#[tokio::test]
async fn readiness_health_samples_after_database_wait_and_preserves_diagnostics() {
    let cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([7; 32]),
        now_ms(),
    ));
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let task = {
        let cache = cache.clone();
        tokio::spawn(async move {
            observe_health(&cache, 100, async { true }, async {
                entered_tx.send(()).unwrap();
                release_rx.await.unwrap();
                false
            })
            .await
        })
    };
    entered_rx.await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    release_tx.send(()).unwrap();
    let (fresh, age, rpc, database) = task.await.unwrap();
    assert!(age.unwrap() > 100, "age must be sampled after all awaits");
    assert!(!fresh);
    assert!(rpc);
    assert!(!database, "database health remains a separate diagnostic");
}

#[tokio::test]
async fn readiness_health_cancellation_drops_probe_without_polling_database() {
    let cache = Arc::new(BlockhashCache::default());
    let (entered_tx, entered_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    struct Guard(Option<oneshot::Sender<()>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let task = tokio::spawn(async move {
        observe_health(
            &cache,
            2000,
            async {
                let _guard = Guard(Some(dropped_tx));
                entered_tx.send(()).unwrap();
                std::future::pending::<bool>().await
            },
            async { panic!("database must not be polled after canceled probe") },
        )
        .await
    });
    entered_rx.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    dropped_rx.await.unwrap();
}

#[tokio::test]
async fn readiness_health_empty_or_old_cache_never_invents_success_or_a_timestamp() {
    let empty = BlockhashCache::default();
    assert_eq!(
        observe_health(&empty, 2000, async { false }, async { false }).await,
        (false, None, false, false)
    );
    let old = BlockhashCache::with_snapshot_for_tests(Blockhash::from_bytes([9; 32]), 100);
    let (fresh, age, rpc, database) =
        observe_health(&old, 2000, async { true }, async { true }).await;
    assert!(!fresh && age.unwrap() > 2000 && rpc && database);
    assert_eq!(old.get_fresh(u64::MAX).unwrap().updated_at_ms, 100);
}

#[tokio::test]
async fn readiness_health_sees_a_successful_refresh_completed_during_probe() {
    use crate::application::ports::blockhash::LatestBlockhashReader;
    use tokio::sync::Notify;
    struct Reader(Arc<Notify>);
    impl LatestBlockhashReader for Reader {
        async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
            self.0.notify_one();
            Ok(Blockhash::from_bytes([4; 32]))
        }
    }
    let cache = Arc::new(BlockhashCache::default());
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let task = {
        let cache = cache.clone();
        tokio::spawn(async move {
            observe_health(
                &cache,
                60_000,
                async {
                    entered_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                    true
                },
                async { false },
            )
            .await
        })
    };
    entered_rx.await.unwrap();
    let refreshed = Arc::new(Notify::new());
    let guard = cache
        .clone()
        .spawn_refresher(Reader(refreshed.clone()), 60_000);
    tokio::time::timeout(Duration::from_secs(1), refreshed.notified())
        .await
        .unwrap();
    guard.shutdown().await.unwrap();
    release_tx.send(()).unwrap();
    let (fresh, age, rpc, database) = task.await.unwrap();
    assert!(fresh && age.unwrap() <= 60_000 && rpc);
    assert!(!database);
    assert_eq!(cache.get_fresh(60_000).unwrap().hash.to_bytes(), [4; 32]);
}
