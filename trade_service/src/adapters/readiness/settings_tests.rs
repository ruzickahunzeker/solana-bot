use super::*;
use crate::{
    application::readiness::{ReadinessQuery, ReadinessService},
    config::{TradePersistenceConfig, TradePersistenceMode},
    domain::{
        blockhash::Blockhash,
        readiness::{ReadinessWalletMetadata, WalletReadinessObservation},
        trade::Address,
        wallet::WalletDescriptor,
    },
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;

fn observation(rpc_ok: bool) -> WalletReadinessObservation {
    let wallet = Address::from_bytes([4; 32]);
    WalletReadinessObservation {
        default_wallet_id: "diagnostic-only".into(),
        wallet_count: 1,
        wallets: vec![WalletDescriptor {
            wallet_id: "diagnostic-only".into(),
            pubkey: wallet,
        }],
        wallet_metadata: ReadinessWalletMetadata {
            wallet,
            rpc_lanes: 3,
            swqos_lanes: vec!["Default".into()],
            active_swqos_lanes: vec![],
            mev_protection: false,
            require_mev_submit: true,
        },
        rpc_ok,
    }
}
struct FixedObserver {
    value: WalletReadinessObservation,
    calls: Arc<AtomicUsize>,
}
impl ReadinessWalletObserver for FixedObserver {
    async fn observe_wallets(&self) -> WalletReadinessObservation {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.value.clone()
    }
}
fn fixed(rpc_ok: bool) -> (FixedObserver, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        FixedObserver {
            value: observation(rpc_ok),
            calls: calls.clone(),
        },
        calls,
    )
}
async fn persistence() -> Arc<TradePersistence> {
    Arc::new(
        TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap(),
    )
}

#[tokio::test]
async fn readiness_settings_projection_copies_only_three_values_without_resources() {
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.rpc_urls.clear();
    config.wallets.clear();
    for age in [0, u64::MAX] {
        for broadcast in [BroadcastPolicy::Enabled, BroadcastPolicy::Disabled] {
            for enabled in [false, true] {
                config.blockhash_max_age_ms = age;
                config.security.broadcast = broadcast;
                config.pumpapi.enabled = enabled;
                let settings = config.readiness_settings();
                config.blockhash_max_age_ms = age.wrapping_add(1);
                assert_eq!(settings.blockhash_max_age_ms, age);
                assert_eq!(settings.broadcast, broadcast);
                assert_eq!(settings.pumpapi_enabled, enabled);
            }
        }
    }
}

#[tokio::test]
async fn readiness_settings_actual_source_keeps_original_cache_and_store_without_retaining_config()
{
    let state = crate::http_contract_tests::state().await;
    let config = Arc::downgrade(&state.config);
    let cache = Arc::downgrade(&state.blockhash_cache);
    let store = Arc::downgrade(&state.persistence);
    let (observer, calls) = fixed(false);
    let source = RuntimeReadinessSource::new(
        state.config.readiness_settings(),
        observer,
        state.blockhash_cache.clone(),
        state.persistence.clone(),
    );
    assert!(Arc::ptr_eq(&source.blockhash_cache, &state.blockhash_cache));
    assert!(Arc::ptr_eq(&source.persistence, &state.persistence));
    drop(state);
    assert!(config.upgrade().is_none());
    let snapshot = source.read().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(snapshot.default_wallet_id, "diagnostic-only");
    assert!(!snapshot.has_blockhash && !snapshot.rpc_ok);
    assert_eq!(snapshot.blockhash_age_ms, None);
    assert!(snapshot.trade_database_healthy);
    drop(source);
    assert!(cache.upgrade().is_none());
    assert!(store.upgrade().is_none());
}

#[tokio::test]
async fn readiness_settings_actual_report_preserves_ready_rule_and_independent_diagnostics() {
    for fresh in [false, true] {
        for rpc_ok in [false, true] {
            for database_ok in [false, true] {
                let store = persistence().await;
                if !database_ok {
                    store.pool_for_tests().close().await;
                }
                for broadcast in [BroadcastPolicy::Enabled, BroadcastPolicy::Disabled] {
                    for pumpapi_enabled in [false, true] {
                        let cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
                            Blockhash::from_bytes([7; 32]),
                            100,
                        ));
                        let (observer, calls) = fixed(rpc_ok);
                        let source = RuntimeReadinessSource::new(
                            ReadinessSettings {
                                blockhash_max_age_ms: if fresh { u64::MAX } else { 0 },
                                broadcast,
                                pumpapi_enabled,
                            },
                            observer,
                            cache,
                            store.clone(),
                        );
                        let report = ReadinessService::new(source).inspect().await;
                        assert_eq!(report.ready, fresh && rpc_ok);
                        let value = report.snapshot;
                        assert_eq!(value.paused, broadcast == BroadcastPolicy::Disabled);
                        assert_eq!(value.pumpapi_executor, pumpapi_enabled);
                        assert_eq!(value.trade_database_healthy, database_ok);
                        assert_eq!(value.trade_persistence_mode, "durable");
                        assert_eq!(value.has_blockhash, fresh);
                        assert_eq!(value.rpc_ok, rpc_ok);
                        assert_eq!(value.wallets, observation(rpc_ok).wallets);
                        assert_eq!(value.wallet_count, 1);
                        assert_eq!(value.wallet_metadata.wallet, Address::from_bytes([4; 32]));
                        assert_eq!(value.wallet_metadata.rpc_lanes, 3);
                        assert_eq!(value.wallet_metadata.swqos_lanes, ["Default"]);
                        assert!(value.wallet_metadata.active_swqos_lanes.is_empty());
                        assert!(!value.wallet_metadata.mev_protection);
                        assert!(value.wallet_metadata.require_mev_submit);
                        assert_eq!(calls.load(Ordering::SeqCst), 1);
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct Trace {
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
    dropped: AtomicBool,
}
struct Dropped(Arc<Trace>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}
struct WaitingObserver(Arc<Trace>);
impl ReadinessWalletObserver for WaitingObserver {
    async fn observe_wallets(&self) -> WalletReadinessObservation {
        let _drop = Dropped(self.0.clone());
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.entered.notify_one();
        self.0.release.notified().await;
        observation(true)
    }
}

#[tokio::test]
async fn readiness_settings_actual_source_cancellation_drops_probe_without_retry() {
    let trace = Arc::new(Trace::default());
    let cache = Arc::new(BlockhashCache::default());
    let source = RuntimeReadinessSource::new(
        ReadinessSettings {
            blockhash_max_age_ms: 2000,
            broadcast: BroadcastPolicy::Disabled,
            pumpapi_enabled: false,
        },
        WaitingObserver(trace.clone()),
        cache.clone(),
        persistence().await,
    );
    let task = tokio::spawn(async move { source.read().await });
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(trace.dropped.load(Ordering::SeqCst));
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    assert!(cache.age_ms().is_none());
}

#[tokio::test]
async fn readiness_settings_actual_source_samples_cache_after_pending_wallet_observation() {
    struct Reader;
    impl crate::application::ports::blockhash::LatestBlockhashReader for Reader {
        async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
            Ok(Blockhash::from_bytes([9; 32]))
        }
    }
    let trace = Arc::new(Trace::default());
    let cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([1; 32]),
        100,
    ));
    let source = RuntimeReadinessSource::new(
        ReadinessSettings {
            blockhash_max_age_ms: 60_000,
            broadcast: BroadcastPolicy::Disabled,
            pumpapi_enabled: false,
        },
        WaitingObserver(trace.clone()),
        cache.clone(),
        persistence().await,
    );
    let task = tokio::spawn(async move { source.read().await });
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
        .await
        .unwrap();
    let refresh = cache.clone().spawn_refresher(Reader, 60_000);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while cache.get_fresh(60_000).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    refresh.shutdown().await.unwrap();
    trace.release.notify_one();
    let value = task.await.unwrap();
    assert!(value.has_blockhash && value.rpc_ok && value.paused);
    assert!(value.blockhash_age_ms.unwrap() <= 60_000);
    assert_eq!(cache.get_fresh(60_000).unwrap().hash.to_bytes(), [9; 32]);
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
}
