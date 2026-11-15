use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{oneshot, Barrier};

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn trade_tasks_unused_admission_tickets_release_and_close_is_irreversible() {
    let tasks = TradeTasks::default();
    let ticket = tasks.register().unwrap();
    tasks.close();
    assert!(tasks.register().is_none());
    let report = tasks.drain(Duration::from_millis(1)).await;
    assert!(report.timed_out);
    assert_eq!(
        (report.active, report.admitting, report.abandoned),
        (1, 1, 0)
    );
    drop(ticket);
    assert!(tasks.drain(Duration::from_secs(1)).await.clean());
    assert!(tasks.register().is_none());
}

#[tokio::test]
async fn trade_tasks_preclose_ticket_can_dispatch_without_an_untracked_gap() {
    let tasks = TradeTasks::default();
    let ticket = tasks.register().unwrap();
    tasks.close();
    let (release, gate) = oneshot::channel();
    ticket.spawn("original-accepted".into(), async move {
        gate.await.unwrap();
    });
    let report = tasks.drain(Duration::from_millis(1)).await;
    assert!(report.timed_out);
    assert_eq!(
        (report.active, report.admitting, report.abandoned),
        (1, 0, 0)
    );
    assert_eq!(report.request_ids, ["original-accepted"]);
    release.send(()).unwrap();
    assert!(tasks.drain(Duration::from_secs(1)).await.clean());
}

#[tokio::test]
async fn trade_tasks_dropping_external_group_handles_does_not_abort_the_original_future() {
    let tasks = TradeTasks::default();
    let entered = Arc::new(Notify::new());
    let started = entered.clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let finished = dropped.clone();
    let (release, gate) = oneshot::channel();
    let (done, completion) = oneshot::channel();
    tasks
        .register()
        .unwrap()
        .spawn("detached-original".into(), async move {
            let _guard = Dropped(finished);
            started.notify_one();
            gate.await.unwrap();
            done.send(()).unwrap();
        });
    entered.notified().await;
    drop(tasks);
    assert!(!dropped.load(Ordering::SeqCst));
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), completion)
        .await
        .unwrap()
        .unwrap();
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn trade_tasks_last_completion_wakes_all_drain_observers_without_lost_notifications() {
    let tasks = TradeTasks::default();
    let barrier = Arc::new(Barrier::new(33));
    let completed = Arc::new(AtomicUsize::new(0));
    for i in 0..32 {
        let release = barrier.clone();
        let completed = completed.clone();
        tasks
            .register()
            .unwrap()
            .spawn(format!("original-{i}"), async move {
                release.wait().await;
                completed.fetch_add(1, Ordering::SeqCst);
            });
    }
    let mut waits = Vec::new();
    for _ in 0..8 {
        let tasks = tasks.clone();
        waits.push(tokio::spawn(async move {
            tasks.drain(Duration::from_secs(3)).await
        }));
    }
    barrier.wait().await;
    for wait in waits {
        assert!(wait.await.unwrap().clean());
    }
    assert_eq!(completed.load(Ordering::SeqCst), 32);
}

#[tokio::test]
async fn trade_tasks_cancelled_drain_waiter_does_not_cancel_or_release_running_work() {
    let tasks = TradeTasks::default();
    let (release, gate) = oneshot::channel();
    tasks
        .register()
        .unwrap()
        .spawn("original-pending".into(), async move {
            gate.await.unwrap();
        });
    let observer = tasks.clone();
    let waiter = tokio::spawn(async move { observer.drain(Duration::from_secs(3)).await });
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let report = tasks.drain(Duration::from_millis(1)).await;
    assert_eq!((report.active, report.abandoned), (1, 0));
    assert_eq!(report.request_ids, ["original-pending"]);
    release.send(()).unwrap();
    assert!(tasks.drain(Duration::from_secs(1)).await.clean());
}

#[tokio::test]
async fn trade_tasks_panics_cannot_be_reported_as_a_clean_drain() {
    let tasks = TradeTasks::default();
    tasks
        .register()
        .unwrap()
        .spawn("original-panicked".into(), async {
            panic!("synthetic tracked task panic");
        });
    let report = tasks.drain(Duration::from_secs(1)).await;
    assert_eq!((report.active, report.abandoned), (0, 1));
    assert!(!report.timed_out && !report.clean());
    assert_eq!(tasks.drain(Duration::from_secs(1)).await.abandoned, 1);
}

#[test]
fn trade_tasks_runtime_drop_accounts_for_a_dispatched_future_that_was_never_polled() {
    let tasks = TradeTasks::default();
    let started = Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tasks
            .register()
            .unwrap()
            .spawn("original-unpolled".into(), async move {
                flag.store(true, Ordering::SeqCst);
                std::future::pending::<()>().await;
            });
    });
    assert!(!started.load(Ordering::SeqCst));
    drop(runtime);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let report = runtime.block_on(tasks.drain(Duration::from_secs(1)));
    assert_eq!((report.active, report.abandoned), (0, 1));
    assert!(!report.clean());
}

#[tokio::test]
async fn trade_tasks_timeout_report_bounds_ids_without_forgetting_any_pending_ticket() {
    let tasks = TradeTasks::default();
    let admitting = [tasks.register().unwrap(), tasks.register().unwrap()];
    let (release, gate) = tokio::sync::watch::channel(false);
    for i in 0..40 {
        let mut gate = gate.clone();
        tasks
            .register()
            .unwrap()
            .spawn(format!("original-{i}"), async move {
                gate.changed().await.unwrap();
            });
    }
    let report = tasks.drain(Duration::from_millis(1)).await;
    assert!(report.timed_out && !report.clean());
    assert_eq!(
        (report.active, report.admitting, report.abandoned),
        (42, 2, 0)
    );
    assert_eq!(report.request_ids.len(), 32);
    drop(admitting);
    release.send(true).unwrap();
    assert!(tasks.drain(Duration::from_secs(1)).await.clean());
}
