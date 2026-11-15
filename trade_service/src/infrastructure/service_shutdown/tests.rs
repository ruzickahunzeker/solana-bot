use super::*;
use crate::{
    adapters::{
        ordinary_observation_worker::ObservationPlan, rpc::read_runtime::SharedReadRuntime,
    },
    application::ports::blockhash::LatestBlockhashReader,
    blockhash_cache::BlockhashCache,
    config::{
        ordinary_observation::OrdinaryObservationConfig, TradePersistenceConfig,
        TradePersistenceMode,
    },
    domain::blockhash::Blockhash,
    persistence::TradePersistence,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{oneshot, Notify};

struct Trace {
    entered: Notify,
    dropped: AtomicBool,
}
struct DropRead(Arc<Trace>);
impl Drop for DropRead {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}
struct PendingReader(Arc<Trace>, bool);
impl LatestBlockhashReader for PendingReader {
    async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
        let _guard = DropRead(self.0.clone());
        self.0.entered.notify_one();
        assert!(!self.1, "synthetic original refresher failure");
        std::future::pending().await
    }
}

async fn refresher(panic: bool) -> (BlockhashRefresher, Arc<Trace>) {
    let trace = Arc::new(Trace {
        entered: Notify::new(),
        dropped: AtomicBool::new(false),
    });
    let refresher = Arc::new(BlockhashCache::default())
        .spawn_refresher(PendingReader(trace.clone(), panic), 400);
    trace.entered.notified().await;
    (refresher, trace)
}

async fn resources(
    panic: bool,
) -> (
    BlockhashRefresher,
    Arc<Trace>,
    ObservationWorker,
    Arc<TradePersistence>,
) {
    let (refresher, trace) = refresher(panic).await;
    let journal = Arc::new(
        TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap(),
    );
    let settings = OrdinaryObservationConfig {
        enabled: true,
        ..Default::default()
    };
    let worker = ObservationPlan::prepare(
        &settings,
        &journal,
        Arc::new(SharedReadRuntime::new(&["http://127.0.0.1:9".into()]).unwrap()),
    )
    .unwrap()
    .unwrap()
    .start();
    (refresher, trace, worker, journal)
}

#[tokio::test]
async fn service_shutdown_preserves_original_workers_until_accepted_trade_finishes() {
    let tasks = TradeTasks::default();
    let (release, gate) = oneshot::channel();
    tasks
        .register()
        .unwrap()
        .spawn("original-working".into(), async move {
            gate.await.unwrap();
        });
    let (refresher, trace, worker, journal) = resources(false).await;
    let observation_stop = worker.stop_signal();
    let owner = tasks.clone();
    let shutdown = tokio::spawn(async move {
        finish(
            &owner,
            Duration::from_secs(3),
            refresher,
            Some(worker),
            journal.as_ref(),
            Duration::from_secs(1),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(ticket) = tasks.register() {
            drop(ticket);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!shutdown.is_finished());
    assert!(!trace.dropped.load(Ordering::SeqCst));
    assert!(!observation_stop.requested_for_tests());
    release.send(()).unwrap();
    shutdown.await.unwrap().unwrap();
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(observation_stop.requested_for_tests());
    assert!(tasks.drain(Duration::from_secs(1)).await.clean());
}

#[tokio::test]
async fn service_shutdown_timeout_is_explicit_and_does_not_abort_the_original_trade() {
    let tasks = TradeTasks::default();
    let (release, gate) = oneshot::channel();
    tasks
        .register()
        .unwrap()
        .spawn("original-unfinished".into(), async move {
            gate.await.unwrap();
        });
    let (refresher, trace, worker, journal) = resources(false).await;
    let observation_stop = worker.stop_signal();
    let error = finish(
        &tasks,
        Duration::from_millis(1),
        refresher,
        Some(worker),
        journal.as_ref(),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("active=1"));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(observation_stop.requested_for_tests());
    let report = tasks.drain(Duration::from_millis(1)).await;
    assert_eq!((report.active, report.abandoned), (1, 0));
    release.send(()).unwrap();
    assert!(tasks.drain(Duration::from_secs(1)).await.clean());
}

#[tokio::test]
async fn service_shutdown_refresher_error_still_joins_the_original_observer() {
    let (refresher, trace, worker, journal) = resources(true).await;
    let observation_stop = worker.stop_signal();
    let error = finish(
        &TradeTasks::default(),
        Duration::from_secs(1),
        refresher,
        Some(worker),
        journal.as_ref(),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("blockhash refresher task failed"));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(observation_stop.requested_for_tests());
}

#[tokio::test]
async fn service_shutdown_abandoned_task_is_not_a_clean_exit_even_with_no_active_work() {
    let tasks = TradeTasks::default();
    tasks
        .register()
        .unwrap()
        .spawn("original-panicked".into(), async {
            panic!("synthetic trade task failure");
        });
    let (refresher, trace, worker, journal) = resources(false).await;
    let stop = worker.stop_signal();
    let error = finish(
        &tasks,
        Duration::from_secs(1),
        refresher,
        Some(worker),
        journal.as_ref(),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("abandoned=1"));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(stop.requested_for_tests());
}

async fn shadow_fixture() -> crate::application::ordinary_claims::test_support::Fixture {
    crate::application::ordinary_claims::test_support::fixture(TradePersistenceMode::Shadow).await
}

#[tokio::test]
async fn shadow_shutdown_helper_joins_original_writer_before_reporting_success() {
    let fixture = shadow_fixture().await;
    let journal = fixture.state.persistence.clone();
    let pool = journal.pool_for_tests();
    let held = pool.acquire().await.unwrap();
    let mut record = fixture.record.clone();
    record.status = crate::domain::trade::TradeStatus::Simulated;
    record.updated_at_ms += 1;
    journal.update(&record).await.unwrap();
    let (refresher, trace) = refresher(false).await;
    let owner = journal.clone();
    let tasks = fixture.state.trade_tasks.clone();
    let shutdown = tokio::spawn(async move {
        finish(
            &tasks,
            Duration::from_secs(1),
            refresher,
            None,
            owner.as_ref(),
            Duration::from_secs(3),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !journal.shadow_is_closed_for_tests() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!shutdown.is_finished());
    assert!(trace.dropped.load(Ordering::SeqCst));
    drop(held);
    shutdown.await.unwrap().unwrap();
    assert_eq!(
        journal
            .get(&record.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        record.status
    );
}

#[tokio::test]
async fn shadow_shutdown_helper_reports_write_failure_without_changing_original_parent() {
    let fixture = shadow_fixture().await;
    let journal = fixture.state.persistence.clone();
    let mut invalid = fixture.record.clone();
    invalid.updated_at_ms = u128::MAX;
    journal.update(&invalid).await.unwrap();
    let (refresher, trace) = refresher(false).await;
    let error = finish(
        &fixture.state.trade_tasks,
        Duration::from_secs(1),
        refresher,
        None,
        journal.as_ref(),
        Duration::from_secs(3),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("failed=1"));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert_eq!(
        fixture
            .state
            .trades
            .get(&fixture.record.request_id)
            .unwrap()
            .status,
        fixture.record.status
    );
    assert_eq!(
        journal
            .get(&fixture.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        fixture.record.status
    );
}

#[tokio::test]
async fn shadow_shutdown_helper_drains_writer_even_when_ordinary_work_is_incomplete() {
    let fixture = shadow_fixture().await;
    let tasks = fixture.state.trade_tasks.clone();
    let ticket = tasks.register().unwrap();
    let journal = fixture.state.persistence.clone();
    let (refresher, trace) = refresher(false).await;
    let error = finish(
        &tasks,
        Duration::from_millis(1),
        refresher,
        None,
        journal.as_ref(),
        Duration::from_secs(3),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("active=1"));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(journal.shadow_is_closed_for_tests());
    assert!(journal
        .get(&fixture.record.request_id)
        .await
        .unwrap()
        .is_some());
    drop(ticket);
    assert!(tasks.drain(Duration::from_secs(1)).await.clean());
}

#[tokio::test]
async fn shadow_shutdown_helper_drains_writer_even_when_original_refresher_failed() {
    let fixture = shadow_fixture().await;
    let journal = fixture.state.persistence.clone();
    let (refresher, trace) = refresher(true).await;
    let error = finish(
        &fixture.state.trade_tasks,
        Duration::from_secs(1),
        refresher,
        None,
        journal.as_ref(),
        Duration::from_secs(3),
    )
    .await
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("blockhash refresher task failed"));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(journal
        .get(&fixture.record.request_id)
        .await
        .unwrap()
        .is_some());
    assert!(journal.shadow_is_closed_for_tests());
}
