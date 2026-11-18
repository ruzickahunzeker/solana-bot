use super::*;
use crate::{
    application::vanity::{VanityCreateInput, VanityService, VanityTasks},
    domain::wallet::VanityMatchSide,
};
use std::{future::Future, task::Poll, time::Duration};

const WAIT: Duration = Duration::from_secs(2);

fn input() -> VanityCreateInput {
    VanityCreateInput {
        owner_type: "telegram".into(),
        owner_id: "public-lifecycle-owner".into(),
        label: "public lifecycle fixture".into(),
        match_side: VanityMatchSide::Prefix,
        pattern: "9999".into(),
        max_attempts: Some(1),
    }
}

async fn fixture() -> (Arc<WalletOps>, sqlx::SqlitePool) {
    let state = crate::http_contract_tests::state().await;
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    let worker = Arc::new(
        WalletOps::new(Arc::new(repository), state.wallets.clone())
            .await
            .unwrap(),
    );
    (worker, pool)
}

async fn no_wallets(worker: &WalletOps, pool: &sqlx::SqlitePool) {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallets")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(worker.wallets.wallet_count(), 0);
}

#[tokio::test]
async fn vanity_shutdown_closed_create_keeps_database_and_wallets_empty() {
    let (worker, pool) = fixture().await;
    worker.request_stop();
    let service = VanityService::new(worker.clone());
    assert_eq!(
        service.create(input()).await.unwrap_err().message,
        "wallet generation service is stopping"
    );
    worker.shutdown(WAIT).await.unwrap();
    worker.shutdown(WAIT).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallet_generation_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(worker.vanity_jobs.is_empty());
    no_wallets(&worker, &pool).await;
    pool.close().await;
}

#[tokio::test]
async fn vanity_shutdown_queued_job_keeps_original_single_permit_and_cancelled_record() {
    let (worker, pool) = fixture().await;
    let held = worker.vanity_slot.clone().acquire_owned().await.unwrap();
    let service = VanityService::new(worker.clone());
    let job = service.create(input()).await.unwrap();
    assert_eq!(job.status, "queued");
    worker.request_stop();
    assert!(worker
        .vanity_jobs
        .get(&job.job_id)
        .unwrap()
        .cancelled
        .load(Ordering::Relaxed));
    assert!(worker.shutdown(Duration::from_millis(1)).await.is_err());
    assert_eq!(worker.vanity_jobs.len(), 1);
    assert_eq!(worker.vanity_slot.available_permits(), 0);
    drop(held);
    worker.shutdown(WAIT).await.unwrap();
    let stopped = worker.get_vanity_job(&job.job_id).await.unwrap();
    assert_eq!(
        (stopped.status.as_str(), stopped.attempts),
        ("cancelled", 0)
    );
    assert!(stopped.wallet_id.is_none());
    assert!(stopped.pubkey.is_none());
    assert!(worker.vanity_jobs.is_empty());
    assert_eq!(worker.vanity_slot.available_permits(), 1);
    no_wallets(&worker, &pool).await;
    pool.close().await;
}

#[tokio::test]
async fn vanity_shutdown_intake_await_is_owned_and_late_spawn_is_cancelled_without_generation() {
    let (worker, pool) = fixture().await;
    let held_slot = worker.vanity_slot.clone().acquire_owned().await.unwrap();
    let held_database = pool.acquire().await.unwrap();
    let service = VanityService::new(worker.clone());
    let mut create = Box::pin(service.create(input()));
    assert!(std::future::poll_fn(|cx| Poll::Ready(create.as_mut().poll(cx).is_pending())).await);
    assert!(worker.lifecycle.intake_is_held_for_tests());
    worker.request_stop();
    assert!(worker.shutdown(Duration::from_millis(1)).await.is_err());
    assert!(worker.vanity_jobs.is_empty());
    drop(held_database);
    let job = tokio::time::timeout(WAIT, create).await.unwrap().unwrap();
    assert!(worker
        .vanity_jobs
        .get(&job.job_id)
        .unwrap()
        .cancelled
        .load(Ordering::Relaxed));
    drop(held_slot);
    worker.shutdown(WAIT).await.unwrap();
    let stopped = worker.get_vanity_job(&job.job_id).await.unwrap();
    assert_eq!(
        (stopped.status.as_str(), stopped.attempts),
        ("cancelled", 0)
    );
    assert!(worker.vanity_jobs.is_empty());
    no_wallets(&worker, &pool).await;
    pool.close().await;
}

#[tokio::test]
async fn vanity_shutdown_dropped_create_await_releases_admission_without_spawning() {
    let (worker, pool) = fixture().await;
    let held_database = pool.acquire().await.unwrap();
    let service = VanityService::new(worker.clone());
    let mut create = Box::pin(service.create(input()));
    assert!(std::future::poll_fn(|cx| Poll::Ready(create.as_mut().poll(cx).is_pending())).await);
    assert!(worker.lifecycle.intake_is_held_for_tests());
    worker.request_stop();
    drop(create);
    worker.shutdown(WAIT).await.unwrap();
    drop(held_database);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallet_generation_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(worker.vanity_jobs.is_empty());
    no_wallets(&worker, &pool).await;
    pool.close().await;
}
