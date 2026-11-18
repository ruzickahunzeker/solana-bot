use super::*;
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};

const WAIT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn vanity_lifecycle_empty_close_and_repeated_drain_reject_new_intake() {
    let owner = VanityLifecycle::default();
    owner.close();
    assert_eq!(
        owner.admit().await.err().unwrap().to_string(),
        "wallet generation service is stopping"
    );
    owner.drain(WAIT).await.unwrap();
    owner.drain(WAIT).await.unwrap();
    assert_eq!(owner.jobs.lock().await.handles.len(), 0);
}

#[tokio::test]
async fn vanity_lifecycle_waits_for_admission_to_spawn_after_close() {
    let owner = Arc::new(VanityLifecycle::default());
    let admission = owner.admit().await.unwrap();
    owner.close();
    let other = owner.clone();
    let drain = tokio::spawn(async move { other.drain(WAIT).await });
    assert!(owner.intake.try_write().is_err());
    let (release, wait) = oneshot::channel();
    admission.spawn(async move { wait.await.unwrap() }).await;
    assert!(!drain.is_finished());
    release.send(()).unwrap();
    drain.await.unwrap().unwrap();
    assert!(owner.admit().await.is_err());
}

#[tokio::test]
async fn vanity_lifecycle_dropped_admission_has_no_worker_or_abandoned_job() {
    let owner = VanityLifecycle::default();
    let admission = owner.admit().await.unwrap();
    assert!(owner.intake.try_write().is_err());
    drop(admission);
    owner.drain(WAIT).await.unwrap();
    let jobs = owner.jobs.lock().await;
    assert_eq!((jobs.handles.len(), jobs.failed), (0, 0));
}

#[tokio::test]
async fn vanity_lifecycle_timeout_keeps_original_async_task_for_later_join() {
    let owner = VanityLifecycle::default();
    let (release, wait) = oneshot::channel();
    let (finished, witness) = oneshot::channel();
    owner
        .admit()
        .await
        .unwrap()
        .spawn(async move {
            wait.await.unwrap();
            finished.send(()).unwrap();
        })
        .await;
    let error = owner.drain(Duration::from_millis(1)).await.unwrap_err();
    assert!(error.to_string().contains("not aborted"));
    assert_eq!(owner.jobs.lock().await.handles.len(), 1);
    release.send(()).unwrap();
    owner.drain(WAIT).await.unwrap();
    witness.await.unwrap();
}

#[tokio::test]
async fn vanity_lifecycle_blocking_work_is_joined_not_falsely_aborted_on_timeout() {
    let owner = VanityLifecycle::default();
    let entered = Arc::new(Notify::new());
    let signal = entered.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let (finished, witness) = oneshot::channel();
    owner
        .admit()
        .await
        .unwrap()
        .spawn(async move {
            tokio::task::spawn_blocking(move || {
                signal.notify_one();
                wait.recv().unwrap();
                finished.send(()).unwrap();
            })
            .await
            .unwrap();
        })
        .await;
    tokio::time::timeout(WAIT, entered.notified())
        .await
        .unwrap();
    assert!(owner.drain(Duration::from_millis(1)).await.is_err());
    assert_eq!(owner.jobs.lock().await.handles.len(), 1);
    release.send(()).unwrap();
    owner.drain(WAIT).await.unwrap();
    witness.await.unwrap();
}

#[tokio::test]
async fn vanity_lifecycle_cancelled_drain_keeps_handles_and_closed_intake() {
    let owner = Arc::new(VanityLifecycle::default());
    let (release, wait) = oneshot::channel();
    owner
        .admit()
        .await
        .unwrap()
        .spawn(async move { wait.await.unwrap() })
        .await;
    let other = owner.clone();
    let drain = tokio::spawn(async move { other.drain(WAIT).await });
    tokio::time::timeout(WAIT, async {
        while !owner.closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drain.abort();
    assert!(drain.await.unwrap_err().is_cancelled());
    assert!(owner.admit().await.is_err());
    assert_eq!(owner.jobs.lock().await.handles.len(), 1);
    release.send(()).unwrap();
    owner.drain(WAIT).await.unwrap();
}

#[tokio::test]
async fn vanity_lifecycle_panics_are_retained_across_drain_attempts() {
    let owner = VanityLifecycle::default();
    owner
        .admit()
        .await
        .unwrap()
        .spawn(async { panic!("public fixture vanity task failure") })
        .await;
    for _ in 0..2 {
        assert!(owner
            .drain(WAIT)
            .await
            .unwrap_err()
            .to_string()
            .contains("abandoned=1"));
    }
    assert!(owner.jobs.lock().await.handles.is_empty());
}

#[tokio::test]
async fn vanity_lifecycle_new_intake_reaps_finished_handles_without_losing_failure() {
    let owner = VanityLifecycle::default();
    let (finished, witness) = oneshot::channel();
    owner
        .admit()
        .await
        .unwrap()
        .spawn(async move {
            finished.send(()).unwrap();
            panic!("public fixture failure before next intake");
        })
        .await;
    // On this current-thread runtime the panic runs to completion before witness resumes.
    tokio::time::timeout(WAIT, witness).await.unwrap().unwrap();
    assert_eq!(owner.jobs.lock().await.handles.len(), 1);
    let (release, wait) = oneshot::channel();
    owner
        .admit()
        .await
        .unwrap()
        .spawn(async move { wait.await.unwrap() })
        .await;
    {
        let jobs = owner.jobs.lock().await;
        assert_eq!((jobs.handles.len(), jobs.failed), (1, 1));
    }
    release.send(()).unwrap();
    assert!(owner.drain(WAIT).await.is_err());
    assert_eq!(owner.jobs.lock().await.failed, 1);
}
