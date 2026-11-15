use super::*;
use crate::adapters::test_support::IsolatedTestProcess;
use std::{future::Future, io::Write, task::Poll};

const CHILD_FLAG: &str = "SOL_BOT_ORDINARY_SCAN_SIGNAL_TEST";
const CHILD_TEST: &str =
    "adapters::ordinary_observation_worker::tests::signals::ordinary_scan_signal_child";
const READY: &str = "ORDINARY_SCAN_SIGNAL_READY";
const STOPPED: &str = "ORDINARY_SCAN_SIGNAL_STOPPED";

struct PendingHash {
    entered: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}
impl crate::application::ports::blockhash::LatestBlockhashReader for PendingHash {
    async fn latest_blockhash(&self) -> anyhow::Result<crate::domain::blockhash::Blockhash> {
        let _drop = PendingDrop(self.dropped.clone());
        self.entered.notify_one();
        std::future::pending().await
    }
}

// This helper is entered only by an exact-filtered test binary, never the service main function.
#[tokio::test]
async fn ordinary_scan_signal_child() {
    if std::env::var_os(CHILD_FLAG).is_none() {
        return;
    }
    let hash_entered = Arc::new(Notify::new());
    let hash_dropped = Arc::new(AtomicBool::new(false));
    let hash_worker = Arc::new(crate::blockhash_cache::BlockhashCache::default()).spawn_refresher(
        PendingHash {
            entered: hash_entered.clone(),
            dropped: hash_dropped.clone(),
        },
        400,
    );
    tokio::time::timeout(Duration::from_secs(10), hash_entered.notified())
        .await
        .unwrap();
    let store = Store::new(1, false);
    let reader = Arc::new(Reader::new(false, Some(1)));
    let worker = spawn_scanner(
        OrdinaryObservationScanner::new(store.clone(), reader.clone()),
        fast_config(),
        now,
        Duration::from_secs(20),
    );
    tokio::time::timeout(Duration::from_secs(10), reader.entered.notified())
        .await
        .unwrap();
    let mut signal = Box::pin(crate::shutdown_signal());
    std::future::poll_fn(|cx| {
        assert!(signal.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    println!("{READY}");
    std::io::stdout().flush().unwrap();
    tokio::time::timeout(Duration::from_secs(10), signal)
        .await
        .unwrap();
    hash_worker.abort_handle().abort();
    worker.stop_signal().request();
    worker.shutdown().await.unwrap();
    hash_worker.shutdown().await.unwrap();
    assert!(hash_dropped.load(Ordering::SeqCst));
    assert!(reader.dropped.load(Ordering::SeqCst));
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.0.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.0.rows.lock().unwrap()[0].1.status,
        TradeStatus::Unknown
    );
    println!("{STOPPED}");
}

fn exercise_signal(signal: &str) {
    let mut child = IsolatedTestProcess::spawn(CHILD_TEST, CHILD_FLAG, "1");
    child.await_line(READY);
    // Only the child we just spawned is targeted, after its signal handlers acknowledge readiness.
    child.signal(signal);
    child.await_line(STOPPED);
    child.finish();
}

#[test]
fn ordinary_scan_sigterm_stops_inflight_observation_without_touching_services() {
    exercise_signal("-TERM");
}

#[test]
fn ordinary_scan_sigint_stops_inflight_observation_without_touching_services() {
    exercise_signal("-INT");
}
