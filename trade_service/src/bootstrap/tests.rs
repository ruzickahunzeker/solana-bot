use super::*;
use crate::{
    application::ports::blockhash::{CachedBlockhashRead, LatestBlockhashReader},
    config::{TradePersistenceMode, WalletStorageMode},
    domain::blockhash::Blockhash,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;

async fn resources() -> Resources {
    let (state, blockhash_cache) = crate::http_contract_tests::state_with_blockhash().await;
    Resources {
        config: state.config.clone(),
        wallets: state.wallets.clone(),
        wallet_repository: state.wallet_repository.clone(),
        read_rpc: Arc::new(SharedReadRuntime::new(&[]).unwrap()),
        blockhash_cache,
        persistence: state.persistence.clone(),
        wallet_ops: Arc::new(
            WalletOps::new(state.wallet_repository.clone(), state.wallets.clone())
                .await
                .unwrap(),
        ),
        conversions: None,
    }
}

#[derive(Default)]
struct Trace {
    calls: AtomicUsize,
    entered: Notify,
    dropped: AtomicBool,
}
struct Dropped(Arc<Trace>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}
struct PendingRead(Arc<Trace>);
impl LatestBlockhashReader for PendingRead {
    async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
        let _drop = Dropped(self.0.clone());
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.entered.notify_one();
        std::future::pending().await
    }
}

async fn refresher(cache: Arc<BlockhashCache>) -> (BlockhashRefresher, Arc<Trace>) {
    let trace = Arc::new(Trace::default());
    let task = cache.spawn_refresher(PendingRead(trace.clone()), 400);
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
        .await
        .unwrap();
    (task, trace)
}

#[tokio::test]
async fn bootstrap_assembly_uses_original_resources_and_hands_off_live_refresher() {
    let resources = resources().await;
    let config = Arc::downgrade(&resources.config);
    let wallets = Arc::downgrade(&resources.wallets);
    let repository = Arc::downgrade(&resources.wallet_repository);
    let persistence = Arc::downgrade(&resources.persistence);
    let cache = Arc::downgrade(&resources.blockhash_cache);
    let runtime = Arc::downgrade(&resources.read_rpc);
    let operations = Arc::downgrade(&resources.wallet_ops);
    let (refresher, trace) = refresher(resources.blockhash_cache.clone()).await;
    let boot = complete(resources, refresher).unwrap();
    assert!(boot.observation_plan.is_none());
    assert_eq!(boot.wallets.wallet_count(), 0);
    assert!(Arc::ptr_eq(&config.upgrade().unwrap(), &boot.state.config));
    assert!(Arc::ptr_eq(&wallets.upgrade().unwrap(), &boot.wallets));
    assert!(Arc::ptr_eq(
        &repository.upgrade().unwrap(),
        &boot.wallet_repository
    ));
    assert!(Arc::ptr_eq(
        &persistence.upgrade().unwrap(),
        &boot.state.persistence
    ));
    {
        let expected: Arc<dyn CachedBlockhashRead> = cache.upgrade().unwrap();
        assert!(Arc::ptr_eq(&expected, &boot.state.cached_blockhash));
    }
    let loaded = boot.state.wallet_management.loaded();
    assert_eq!(loaded.default_wallet_id, "fixture-wallet");
    assert!(loaded.wallets.is_empty());
    assert!(!trace.dropped.load(Ordering::SeqCst));
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    let Bootstrapped {
        state,
        ordinary_shutdown,
        wallets: held_wallets,
        wallet_repository: held_repository,
        blockhash_refresher,
        observation_plan,
        vanity_worker,
    } = boot;
    drop((
        state,
        ordinary_shutdown,
        held_wallets,
        held_repository,
        observation_plan,
        vanity_worker,
    ));
    assert!(wallets.upgrade().is_none());
    assert!(repository.upgrade().is_none());
    assert!(persistence.upgrade().is_none());
    assert!(runtime.upgrade().is_none());
    assert!(operations.upgrade().is_none());
    assert!(config.upgrade().is_none());
    assert!(cache.upgrade().is_some());
    blockhash_refresher.shutdown().await.unwrap();
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(cache.upgrade().is_none());
}

#[tokio::test]
async fn bootstrap_assembly_retains_only_supplied_conversion_store_until_state_drop() {
    for enabled in [false, true] {
        let mut resources = resources().await;
        Arc::get_mut(&mut resources.config)
            .unwrap()
            .conversion_quotes_enabled = enabled;
        resources.conversions = if enabled {
            Some(Arc::new(
                ConversionStore::new("sqlite::memory:").await.unwrap(),
            ))
        } else {
            None
        };
        let store = resources.conversions.as_ref().map(Arc::downgrade);
        let (refresher, trace) = refresher(resources.blockhash_cache.clone()).await;
        let boot = complete(resources, refresher).unwrap();
        assert_eq!(boot.state.conversions.is_some(), enabled);
        if let Some(store) = &store {
            assert!(Arc::ptr_eq(
                &store.upgrade().unwrap(),
                boot.state.conversions.as_ref().unwrap()
            ));
        }
        let Bootstrapped {
            state,
            ordinary_shutdown,
            wallets,
            wallet_repository,
            blockhash_refresher,
            observation_plan,
            vanity_worker,
        } = boot;
        drop((
            state,
            ordinary_shutdown,
            wallets,
            wallet_repository,
            observation_plan,
            vanity_worker,
        ));
        if let Some(store) = store {
            assert!(store.upgrade().is_none());
        }
        blockhash_refresher.shutdown().await.unwrap();
        assert!(trace.dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn bootstrap_assembly_failure_aborts_original_refresher_and_releases_resources() {
    let mut resources = resources().await;
    Arc::get_mut(&mut resources.config)
        .unwrap()
        .ordinary_observation
        .enabled = true;
    let runtime = Arc::downgrade(&resources.read_rpc);
    let wallets = Arc::downgrade(&resources.wallets);
    let cache = Arc::downgrade(&resources.blockhash_cache);
    let (refresher, trace) = refresher(resources.blockhash_cache.clone()).await;
    let error = complete(resources, refresher)
        .err()
        .expect("inconsistent observation storage must fail");
    assert_eq!(
        error.to_string(),
        "ordinary observation worker requires durable persistence"
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !trace.dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    assert!(runtime.upgrade().is_none());
    assert!(wallets.upgrade().is_none());
    assert!(cache.upgrade().is_none());
}

#[tokio::test]
async fn trade_shutdown_handoff_drains_original_resources_after_request_state_drop() {
    use std::time::Duration;

    let mut resources = resources().await;
    resources.persistence = Arc::new(
        TradePersistence::new(&crate::config::TradePersistenceConfig {
            mode: TradePersistenceMode::Shadow,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap(),
    );
    let persistence = Arc::downgrade(&resources.persistence);
    let (refresher, trace) = refresher(resources.blockhash_cache.clone()).await;
    let Bootstrapped {
        state,
        ordinary_shutdown,
        blockhash_refresher,
        ..
    } = complete(resources, refresher).unwrap();

    let ticket = state.trade_task_admission.register().unwrap();
    ordinary_shutdown.stop_intake()();
    assert!(state.trade_task_admission.register().is_none());
    drop(state);
    let original = persistence.upgrade().unwrap();
    assert!(!original.shadow_is_closed_for_tests());
    let incomplete = ordinary_shutdown
        .finish(
            Duration::ZERO,
            blockhash_refresher,
            None,
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
    assert!(incomplete.to_string().contains("active=1, admitting=1"));
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert!(original.shadow_is_closed_for_tests());
    drop((ticket, original, ordinary_shutdown));
    assert!(persistence.upgrade().is_none());
}

#[tokio::test]
async fn bootstrap_initializer_rejects_observation_mode_before_opening_wallet_database() {
    let state = crate::http_contract_tests::state().await;
    let dir = std::env::temp_dir().join(format!("bootstrap-early-reject-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("never-opened.sqlite");
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Shadow] {
        let mut config = (*state.config).clone();
        config.ordinary_observation.enabled = true;
        config.trade_persistence.mode = mode;
        config.wallet_storage.mode = WalletStorageMode::Database;
        config.wallet_storage.database_url = format!("sqlite://{}?mode=rwc", path.display());
        config.wallet_storage.passphrase = Some("unused-fixture-passphrase".into());
        let error = initialize(Arc::new(config))
            .await
            .err()
            .expect("reject before any initialization");
        assert_eq!(
            error.to_string(),
            "ordinary observation worker requires durable persistence"
        );
        assert!(!path.exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn vanity_shutdown_bootstrap_handoff_closes_the_same_original_worker() {
    let resources = resources().await;
    let original = resources.wallet_ops.clone();
    let (refresher, trace) = refresher(resources.blockhash_cache.clone()).await;
    let boot = complete(resources, refresher).unwrap();
    assert!(Arc::ptr_eq(&original, &boot.vanity_worker));
    boot.vanity_worker.request_stop();
    let error = boot
        .state
        .vanity_tasks
        .create(crate::application::vanity::VanityCreateInput {
            owner_type: "telegram".into(),
            owner_id: "public-owner".into(),
            label: "public handoff".into(),
            match_side: crate::domain::wallet::VanityMatchSide::Prefix,
            pattern: "9999".into(),
            max_attempts: Some(1),
        })
        .await
        .unwrap_err();
    assert_eq!(error.message, "wallet generation service is stopping");
    assert_eq!(boot.wallets.wallet_count(), 0);
    boot.vanity_worker
        .shutdown(std::time::Duration::from_secs(2))
        .await
        .unwrap();
    assert!(!trace.dropped.load(Ordering::SeqCst));
    boot.blockhash_refresher.shutdown().await.unwrap();
    assert!(trace.dropped.load(Ordering::SeqCst));
}
