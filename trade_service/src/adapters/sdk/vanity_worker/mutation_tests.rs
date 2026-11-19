use super::*;
use crate::{
    adapters::sdk::wallet_management::SdkWalletManagement,
    application::ports::wallet_management::WalletRuntimePort,
};
use std::future::Future;
use std::task::{Context, Waker};
use std::time::Duration;

#[tokio::test]
async fn wallet_mutation_vanity_and_management_use_same_registry_without_generation() {
    let state = crate::http_contract_tests::state().await;
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    let repository = Arc::new(repository);
    let worker = WalletOps::new(repository.clone(), state.wallets.clone())
        .await
        .unwrap();
    let adapter = SdkWalletManagement::new(repository.clone(), state.wallets.clone());
    assert!(Arc::ptr_eq(&worker.repository, &repository));
    assert!(Arc::ptr_eq(&worker.wallets, &state.wallets));
    let management = adapter.begin_mutation().await;
    let mut vanity = Box::pin(worker.wallets.begin_management_mutation());
    assert!(vanity
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    drop(management);
    let vanity = tokio::time::timeout(Duration::from_secs(1), vanity)
        .await
        .unwrap();
    let mut management = Box::pin(adapter.begin_mutation());
    assert!(management
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    drop(vanity);
    let management = tokio::time::timeout(Duration::from_secs(1), management)
        .await
        .unwrap();
    drop(management);
    worker.shutdown(Duration::from_secs(1)).await.unwrap();
    for table in ["wallets", "wallet_generation_jobs"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    assert_eq!(state.wallets.wallet_count(), 0);
    pool.close().await;
}
