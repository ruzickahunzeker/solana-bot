use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Waker};
use std::time::Duration;

#[derive(Clone)]
struct DatabaseTestRuntime {
    adapter: SdkWalletManagement,
    reached: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    installed: Arc<AtomicBool>,
    fail_install: bool,
}

impl WalletRuntimePort<WalletMaterial> for DatabaseTestRuntime {
    type Mutation<'a> = crate::wallets::WalletManagementMutation<'a>;

    async fn begin_mutation(&self) -> Self::Mutation<'_> {
        self.adapter.begin_mutation().await
    }

    fn loaded(&self) -> LoadedWallets {
        self.adapter.loaded()
    }

    async fn install(
        &self,
        _mutation: &mut Self::Mutation<'_>,
        _id: &str,
        material: WalletMaterial,
    ) -> anyhow::Result<()> {
        let _material = material;
        self.reached.notify_one();
        self.release.notified().await;
        if self.fail_install {
            anyhow::bail!("synthetic runtime installation failed");
        }
        self.installed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn remove(&self, _mutation: &mut Self::Mutation<'_>, _id: &str) -> anyhow::Result<()> {
        self.installed.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn wallet_mutation_real_repository_preserves_restore_compensation_and_disable_install_order()
{
    use crate::application::wallet_management::{WalletManagement, WalletManagementService};
    use solana_sdk::signature::{Keypair, Signer};

    for initially_enabled in [false, true] {
        for fail_install in [false, true] {
            let state = crate::http_contract_tests::state().await;
            let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
            let repository = Arc::new(repository);
            // Only the public key is inserted. This synthetic secret is never stored/exported.
            let fixture = Keypair::new();
            sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES('restore-fixture','telegram','42','old',?,'not-ciphertext-test-only',?)")
                .bind(fixture.pubkey().to_string()).bind(i64::from(initially_enabled))
                .execute(&pool).await.unwrap();
            let adapter = SdkWalletManagement::new(repository.clone(), state.wallets.clone());
            let runtime = DatabaseTestRuntime {
                adapter: adapter.clone(),
                reached: Arc::default(),
                release: Arc::default(),
                installed: Arc::default(),
                fail_install,
            };
            let service = Arc::new(WalletManagementService::new(adapter, runtime.clone()));
            let installing = service.clone();
            let input = CreateWalletInput {
                owner_type: "telegram".into(),
                owner_id: "42".into(),
                label: "restored".into(),
                private_key_base58: Some(bs58::encode(fixture.to_bytes()).into_string()),
            };
            let task = tokio::spawn(async move { installing.create(input).await });
            tokio::time::timeout(Duration::from_secs(1), runtime.reached.notified())
                .await
                .unwrap();
            assert!(repository.get("restore-fixture").await.unwrap().enabled);
            let mut deleting = service.delete("restore-fixture");
            assert!(deleting
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending());
            assert!(repository.get("restore-fixture").await.unwrap().enabled);
            assert!(!runtime.installed.load(Ordering::SeqCst));
            runtime.release.notify_one();
            let result = task.await.unwrap();
            if fail_install {
                let error = result.unwrap_err();
                assert_eq!(
                    error.kind,
                    crate::application::wallet_management::WalletFailureKind::LoadFailed
                );
                assert_eq!(error.message, "synthetic runtime installation failed");
                assert_eq!(
                    repository.get("restore-fixture").await.unwrap().enabled,
                    initially_enabled
                );
            } else {
                let result = result.unwrap();
                assert_eq!(result.created, !initially_enabled);
                assert!(result.wallet.enabled);
                assert_eq!(result.wallet.pubkey, fixture.pubkey().to_string());
                assert!(runtime.installed.load(Ordering::SeqCst));
            }
            assert!(!deleting.await.unwrap().wallet.enabled);
            assert!(!repository.get("restore-fixture").await.unwrap().enabled);
            assert!(!runtime.installed.load(Ordering::SeqCst));
            assert_eq!(state.wallets.wallet_count(), 0);
            let secret_marker: String = sqlx::query_scalar(
                "SELECT encrypted_private_key FROM wallets WHERE wallet_id='restore-fixture'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(secret_marker, "not-ciphertext-test-only");
            pool.close().await;
        }
    }
}

#[tokio::test]
async fn wallet_mutation_wrong_registry_scope_rejects_before_initialization_or_removal() {
    let first = crate::http_contract_tests::state().await;
    let second = crate::http_contract_tests::state().await;
    let mut wrong = second.wallets.begin_management_mutation().await;
    let input = WalletConfig {
        wallet_id: "literal-public-fixture".into(),
        private_key_base58: "synthetic-not-a-key".into(),
    };
    assert_eq!(
        first
            .wallets
            .add(&mut wrong, &input)
            .await
            .err()
            .unwrap()
            .to_string(),
        "wallet mutation belongs to another registry"
    );
    assert_eq!(
        first
            .wallets
            .remove(&mut wrong, "fixture-wallet")
            .unwrap_err()
            .to_string(),
        "wallet mutation belongs to another registry"
    );
    let mut original = first.wallets.begin_management_mutation().await;
    assert_eq!(
        first
            .wallets
            .remove(&mut original, "fixture-wallet")
            .unwrap_err()
            .to_string(),
        "default wallet cannot be removed"
    );
    assert_eq!(
        first
            .wallets
            .remove(&mut original, "literal-public-fixture")
            .unwrap_err()
            .to_string(),
        "wallet literal-public-fixture is not loaded"
    );
    let error = first
        .wallets
        .add(&mut original, &input)
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "initialize wallet literal-public-fixture"
    );
    assert_eq!(
        error.root_cause().to_string(),
        "at least one RPC URL is required"
    );
    assert_eq!(first.wallets.wallet_count(), 0);
    assert_eq!(second.wallets.wallet_count(), 0);
}

#[tokio::test]
async fn wallet_mutation_adapter_clones_share_registry_but_independent_registries_do_not() {
    let first = crate::http_contract_tests::state().await;
    let second = crate::http_contract_tests::state().await;
    assert!(!Arc::ptr_eq(&first.wallets, &second.wallets));
    let adapter = SdkWalletManagement::new(first.wallet_repository.clone(), first.wallets.clone());
    let cloned = adapter.clone();
    let held = adapter.begin_mutation().await;
    let mut waiting = Box::pin(cloned.begin_mutation());
    assert!(waiting
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    let independent = tokio::time::timeout(
        Duration::from_secs(1),
        second.wallets.begin_management_mutation(),
    )
    .await
    .unwrap();
    drop(independent);
    drop(held);
    let acquired = tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap();
    let mut registry_waiter = Box::pin(first.wallets.begin_management_mutation());
    assert!(registry_waiter
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    drop(acquired);
    let registry_guard = registry_waiter.await;
    drop(registry_guard);
    assert_eq!(first.wallets.wallet_count(), 0);
    assert_eq!(second.wallets.wallet_count(), 0);

    let held = adapter.begin_mutation().await;
    let entered = Arc::new(AtomicUsize::new(0));
    let count = entered.clone();
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let task = tokio::spawn(async move {
        signal.notify_one();
        let _guard = cloned.begin_mutation().await;
        count.fetch_add(1, Ordering::SeqCst);
    });
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    drop(held);
    let _guard = adapter.begin_mutation().await;
    assert_eq!(entered.load(Ordering::SeqCst), 0);
    assert_eq!(first.wallets.wallet_count(), 0);
}
