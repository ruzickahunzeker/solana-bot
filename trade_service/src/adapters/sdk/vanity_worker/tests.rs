use super::*;
use sqlx::SqlitePool;

async fn seed(pool: &SqlitePool, id: &str, status: &str) {
    sqlx::query("INSERT INTO wallet_generation_jobs(job_id,owner_type,owner_id,label,match_side,pattern,status,attempts,max_attempts) VALUES(?,'telegram','42','public fixture','prefix','9999',?,2,100)")
        .bind(id).bind(status).execute(pool).await.unwrap();
}

#[tokio::test]
async fn constructor_keeps_resources_one_slot_and_original_restart_status_scope_without_generating()
{
    let state = crate::http_contract_tests::state().await;
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    let repository = Arc::new(repository);
    for status in ["queued", "running", "cancel_requested", "completed"] {
        seed(&pool, status, status).await;
    }
    let worker = Arc::new(
        WalletOps::new(repository.clone(), state.wallets.clone())
            .await
            .unwrap(),
    );
    assert!(Arc::ptr_eq(&worker.repository, &repository));
    assert!(Arc::ptr_eq(&worker.wallets, &state.wallets));
    assert_eq!(worker.vanity_slot.available_permits(), 1);
    assert!(worker.vanity_jobs.is_empty());
    for status in ["queued", "running", "cancel_requested", "completed"] {
        let job = VanityWorkerPort::get(&worker, status).await.unwrap();
        if ["queued", "running"].contains(&status) {
            assert_eq!(job.status, "interrupted");
            assert_eq!(job.error.as_deref(), Some("service restarted"));
        } else {
            assert_eq!(job.status, status);
            assert!(job.error.is_none());
        }
    }
    assert_eq!(state.wallets.wallet_count(), 0);
    pool.close().await;
}

#[tokio::test]
async fn get_and_cancel_keep_existing_owner_status_and_missing_job_contracts_without_a_worker_task()
{
    let state = crate::http_contract_tests::state().await;
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    let worker = Arc::new(
        WalletOps::new(Arc::new(repository), state.wallets.clone())
            .await
            .unwrap(),
    );
    for status in [
        "queued",
        "running",
        "cancel_requested",
        "completed",
        "failed",
        "cancelled",
        "exhausted",
        "interrupted",
        "unexpected",
    ] {
        seed(&pool, status, status).await;
        let before = VanityWorkerPort::get(&worker, status).await.unwrap();
        assert_eq!(before.attempts, 2);
        assert_eq!(
            VanityWorkerPort::cancel(&worker, status, "telegram", "99")
                .await
                .unwrap_err()
                .to_string(),
            "wallet generation job does not belong to this owner"
        );
        assert_eq!(
            VanityWorkerPort::get(&worker, status).await.unwrap().status,
            status
        );
        let after = VanityWorkerPort::cancel(&worker, status, "telegram", "42")
            .await
            .unwrap();
        assert_eq!(
            after.status,
            if ["queued", "running", "cancel_requested"].contains(&status) {
                "cancel_requested"
            } else {
                status
            }
        );
        assert_eq!(after.attempts, 2);
    }
    assert_eq!(
        VanityWorkerPort::get(&worker, "missing")
            .await
            .unwrap_err()
            .to_string(),
        "wallet generation job not found"
    );
    assert_eq!(
        VanityWorkerPort::cancel(&worker, "missing", "telegram", "42")
            .await
            .unwrap_err()
            .to_string(),
        "wallet generation job not found"
    );
    assert!(worker.vanity_jobs.is_empty());
    assert_eq!(state.wallets.wallet_count(), 0);
    pool.close().await;
}

#[tokio::test]
async fn active_counter_overlay_and_cancel_flag_use_original_record_without_running_generation() {
    let state = crate::http_contract_tests::state().await;
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    let worker = Arc::new(
        WalletOps::new(Arc::new(repository), state.wallets.clone())
            .await
            .unwrap(),
    );
    seed(&pool, "public-job", "running").await;
    let runtime = Arc::new(VanityRuntime {
        attempts: AtomicU64::new(9),
        cancelled: AtomicBool::new(false),
    });
    worker
        .vanity_jobs
        .insert("public-job".into(), runtime.clone());
    assert_eq!(
        VanityWorkerPort::get(&worker, "public-job")
            .await
            .unwrap()
            .attempts,
        9
    );
    let cancelled = VanityWorkerPort::cancel(&worker, "public-job", "telegram", "42")
        .await
        .unwrap();
    assert_eq!(cancelled.status, "cancel_requested");
    assert_eq!(cancelled.attempts, 9);
    assert!(runtime.cancelled.load(Ordering::Relaxed));
    let row: (String, i64) = sqlx::query_as(
        "SELECT status,attempts FROM wallet_generation_jobs WHERE job_id='public-job'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, ("cancel_requested".into(), 9));
    assert_eq!(worker.vanity_jobs.len(), 1);
    assert_eq!(state.wallets.wallet_count(), 0);
    pool.close().await;
}
