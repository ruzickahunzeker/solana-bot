use super::{test_support::*, *};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Waker},
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Clone)]
struct Runtime {
    backend: Backend,
    reached: Arc<Notify>,
    release: Arc<Notify>,
    installations: Arc<AtomicUsize>,
    installed: Arc<AtomicBool>,
}

impl Runtime {
    fn new(backend: &Backend) -> Self {
        Self {
            backend: backend.clone(),
            reached: Arc::default(),
            release: Arc::default(),
            installations: Arc::default(),
            installed: Arc::default(),
        }
    }
}

impl WalletRuntimePort<Material> for Runtime {
    type Mutation<'a> = tokio::sync::MutexGuard<'a, ()>;

    async fn begin_mutation(&self) -> Self::Mutation<'_> {
        self.backend.mutation.lock().await
    }

    fn loaded(&self) -> LoadedWallets {
        self.backend.loaded()
    }

    async fn install(
        &self,
        _mutation: &mut Self::Mutation<'_>,
        id: &str,
        material: Material,
    ) -> anyhow::Result<()> {
        let _material = material;
        self.installations.fetch_add(1, Ordering::SeqCst);
        self.backend
            .state
            .lock()
            .unwrap()
            .events
            .push(format!("install-start:{id}"));
        self.reached.notify_one();
        self.release.notified().await;
        self.installed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn remove(&self, mutation: &mut Self::Mutation<'_>, id: &str) -> anyhow::Result<()> {
        self.backend.remove(mutation, id)?;
        self.installed.store(false, Ordering::SeqCst);
        Ok(())
    }
}

fn pending<F: Future + ?Sized>(future: Pin<&mut F>) {
    assert!(future
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
}

#[tokio::test]
async fn wallet_mutation_holds_repository_and_installation_before_competing_delete() {
    for secret in [None, Some("synthetic-import-not-a-key")] {
        let backend = Backend::new();
        let runtime = Runtime::new(&backend);
        let service = WalletManagementService::new(backend.clone(), runtime.clone());
        let mut create = service.create(input(secret));
        pending(create.as_mut());
        assert_eq!(runtime.installations.load(Ordering::SeqCst), 1);
        assert!(backend.mutation.try_lock().is_err());
        let before = backend.events();
        let mut delete = service.delete("managed-wallet");
        pending(delete.as_mut());
        assert_eq!(backend.events(), before);
        assert!(backend.state.lock().unwrap().wallet.enabled);
        runtime.release.notify_one();
        let created = create.await.unwrap();
        assert!(created.wallet.enabled);
        assert!(runtime.installed.load(Ordering::SeqCst));
        let deleted = delete.await.unwrap();
        assert!(!deleted.wallet.enabled);
        assert!(!runtime.installed.load(Ordering::SeqCst));
        assert_eq!(backend.drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            backend.events(),
            [
                if secret.is_some() { "import" } else { "create" },
                "install-start:managed-wallet",
                "disable:managed-wallet",
                "remove:managed-wallet"
            ]
        );
        assert!(backend.mutation.try_lock().is_ok());
    }
}

#[tokio::test]
async fn wallet_mutation_serializes_same_wallet_imports_before_any_repository_side_effect() {
    let backend = Backend::new();
    let runtime = Runtime::new(&backend);
    let first = WalletManagementService::new(backend.clone(), runtime.clone());
    // Distinct service facades still coordinate through the original runtime instance.
    let second = WalletManagementService::new(backend.clone(), runtime.clone());
    let mut a = first.create(input(Some("synthetic-import-not-a-key")));
    pending(a.as_mut());
    let before = backend.events();
    let mut b = second.create(input(Some("synthetic-import-not-a-key")));
    pending(b.as_mut());
    assert_eq!(backend.events(), before);
    assert_eq!(backend.state.lock().unwrap().inputs.len(), 1);
    runtime.release.notify_one();
    assert!(!a.await.unwrap().created);
    pending(b.as_mut());
    assert_eq!(runtime.installations.load(Ordering::SeqCst), 2);
    assert_eq!(backend.state.lock().unwrap().inputs.len(), 2);
    runtime.release.notify_one();
    assert!(!b.await.unwrap().created);
    assert_eq!(backend.drops.load(Ordering::SeqCst), 2);
    assert!(backend.mutation.try_lock().is_ok());
}

#[tokio::test]
async fn wallet_mutation_original_failures_release_coordination_without_changing_compensation() {
    for (operation, created, compensation_failed) in [
        ("create", true, false),
        ("import", false, false),
        ("install", false, false),
        ("install", true, false),
        ("install", true, true),
        ("disable", false, false),
        ("remove", false, false),
    ] {
        let backend = Backend::new();
        backend.state.lock().unwrap().import_created = created;
        backend.fail(operation);
        if compensation_failed {
            backend.fail("disable");
        }
        let service = WalletManagementService::new(backend.clone(), backend.clone());
        let result = match operation {
            "create" => service.create(input(None)).await,
            "disable" | "remove" => service.delete("managed-wallet").await,
            _ => {
                service
                    .create(input(Some("synthetic-import-not-a-key")))
                    .await
            }
        };
        let error = result.unwrap_err();
        assert_eq!(error.message, format!("synthetic {operation} failure"));
        assert_eq!(
            error.kind,
            match operation {
                "install" => WalletFailureKind::LoadFailed,
                "remove" => WalletFailureKind::RemoveFailed,
                _ => WalletFailureKind::InvalidRequest,
            }
        );
        assert!(backend.mutation.try_lock().is_ok());
        let disabled =
            operation == "remove" || (operation == "install" && created && !compensation_failed);
        assert_eq!(backend.state.lock().unwrap().wallet.enabled, !disabled);
        let events = backend.events();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.starts_with("disable:"))
                .count(),
            usize::from(
                operation == "disable"
                    || operation == "remove"
                    || (operation == "install" && created)
            )
        );
    }
}

#[tokio::test]
async fn wallet_mutation_cancelled_waiter_never_enters_repository_or_acquires_material() {
    for operation in ["create", "import", "delete"] {
        let backend = Backend::new();
        let guard = backend.mutation.lock().await;
        let service = Arc::new(WalletManagementService::new(
            backend.clone(),
            backend.clone(),
        ));
        let started = Arc::new(Notify::new());
        let signal = started.clone();
        let task = tokio::spawn(async move {
            signal.notify_one();
            match operation {
                "create" => service.create(input(None)).await,
                "import" => {
                    service
                        .create(input(Some("synthetic-import-not-a-key")))
                        .await
                }
                _ => service.delete("managed-wallet").await,
            }
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();
        assert!(backend.events().is_empty());
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert!(backend.events().is_empty());
        assert_eq!(backend.drops.load(Ordering::SeqCst), 0);
        drop(guard);
        assert!(backend.mutation.try_lock().is_ok());
    }
}

#[tokio::test]
async fn wallet_mutation_active_cancellation_preserves_partial_state_and_releases_original_gate() {
    let backend = Backend::new();
    let runtime = Runtime::new(&backend);
    let service = Arc::new(WalletManagementService::new(
        backend.clone(),
        runtime.clone(),
    ));
    let create_service = service.clone();
    let task = tokio::spawn(async move { create_service.create(input(None)).await });
    tokio::time::timeout(Duration::from_secs(1), runtime.reached.notified())
        .await
        .unwrap();
    assert!(backend.mutation.try_lock().is_err());
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(backend.drops.load(Ordering::SeqCst), 1);
    assert!(backend.state.lock().unwrap().wallet.enabled);
    assert!(!runtime.installed.load(Ordering::SeqCst));
    assert_eq!(backend.events(), ["create", "install-start:managed-wallet"]);
    assert!(backend.mutation.try_lock().is_ok());
    assert!(
        !service
            .delete("managed-wallet")
            .await
            .unwrap()
            .wallet
            .enabled
    );
    let before = backend.events();
    runtime.release.notify_one();
    tokio::task::yield_now().await;
    assert_eq!(backend.events(), before);
    assert!(!runtime.installed.load(Ordering::SeqCst));
}
