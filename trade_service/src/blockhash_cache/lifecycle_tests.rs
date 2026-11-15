use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use tokio::sync::Notify;

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
struct PendingReader {
    entered: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}
impl LatestBlockhashReader for PendingReader {
    async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
        let _drop = DropFlag(self.dropped.clone());
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn shared_read_refresher_shutdown_cancels_query_without_refreshing_old_hash() {
    let cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([9; 32]),
        100,
    ));
    let entered = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let worker = cache.clone().spawn_refresher(
        PendingReader {
            entered: entered.clone(),
            dropped: dropped.clone(),
        },
        400,
    );
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    worker.shutdown().await.unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    let snapshot = cache.get_fresh(u64::MAX).unwrap();
    assert_eq!(snapshot.hash.to_bytes(), [9; 32]);
    assert_eq!(snapshot.updated_at_ms, 100);
}

#[tokio::test]
async fn shared_read_refresher_owner_drop_aborts_query_on_bootstrap_failure() {
    let cache = Arc::new(BlockhashCache::default());
    let entered = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let worker = cache.clone().spawn_refresher(
        PendingReader {
            entered: entered.clone(),
            dropped: dropped.clone(),
        },
        400,
    );
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    drop(worker);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(cache.age_ms().is_none());
}

struct UnavailableReader {
    starts: Arc<Mutex<Vec<std::time::Instant>>>,
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
}
impl LatestBlockhashReader for UnavailableReader {
    async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
        self.starts.lock().unwrap().push(std::time::Instant::now());
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        anyhow::bail!("all endpoints cooling down")
    }
}
#[tokio::test]
async fn shared_read_zero_refresh_interval_is_paced_even_when_all_nodes_fail_fast() {
    let starts = Arc::new(Mutex::new(vec![]));
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let cache = Arc::new(BlockhashCache::default());
    let worker = cache.clone().spawn_refresher(
        UnavailableReader {
            starts: starts.clone(),
            calls: calls.clone(),
            entered: entered.clone(),
        },
        0,
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.load(Ordering::SeqCst) < 3 {
            entered.notified().await;
        }
    })
    .await
    .unwrap();
    worker.shutdown().await.unwrap();
    let starts = starts.lock().unwrap();
    for pair in starts.windows(2) {
        assert!(pair[1].duration_since(pair[0]) >= Duration::from_millis(45));
    }
    assert!(cache.age_ms().is_none());
}
