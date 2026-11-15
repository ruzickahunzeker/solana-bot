use crate::application::ports::blockhash::LatestBlockhashReader;
use crate::domain::blockhash::Blockhash;
use crate::state::now_ms;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::warn;

#[cfg(test)]
mod lifecycle_tests;

pub(crate) struct BlockhashRefresher {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl BlockhashRefresher {
    pub(crate) fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.task
            .as_ref()
            .expect("refresher task missing")
            .abort_handle()
    }

    pub(crate) async fn shutdown(mut self) -> anyhow::Result<()> {
        if let Some(task) = self.task.take() {
            task.abort();
            if let Err(error) = task.await {
                anyhow::ensure!(error.is_cancelled(), "blockhash refresher task failed");
            }
        }
        Ok(())
    }
}

impl Drop for BlockhashRefresher {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[derive(Clone)]
pub struct CachedBlockhash {
    pub hash: Blockhash,
    pub updated_at_ms: u128,
}

#[derive(Default)]
pub struct BlockhashCache {
    inner: DashMap<&'static str, CachedBlockhash>,
}

impl BlockhashCache {
    #[cfg(test)]
    pub(crate) fn with_snapshot_for_tests(hash: Blockhash, updated_at_ms: u128) -> Self {
        let cache = Self::default();
        cache.inner.insert(
            "latest",
            CachedBlockhash {
                hash,
                updated_at_ms,
            },
        );
        cache
    }

    pub(crate) fn spawn_refresher<R: LatestBlockhashReader + Send + 'static>(
        self: Arc<Self>,
        reader: R,
        refresh_ms: u64,
    ) -> BlockhashRefresher {
        let interval = Duration::from_millis(refresh_ms.max(50));
        let task = tokio::spawn(async move {
            loop {
                self.refresh_once(&reader).await;
                sleep(interval).await;
            }
        });
        BlockhashRefresher { task: Some(task) }
    }

    async fn refresh_once(&self, reader: &impl LatestBlockhashReader) {
        match reader.latest_blockhash().await {
            Ok(hash) => {
                self.inner.insert(
                    "latest",
                    CachedBlockhash {
                        hash,
                        updated_at_ms: now_ms(),
                    },
                );
            }
            Err(_) => warn!("all RPC lanes failed to refresh blockhash"),
        }
    }

    pub fn get_fresh(&self, max_age_ms: u64) -> Option<CachedBlockhash> {
        self.get_fresh_with_clock(max_age_ms, now_ms)
    }

    fn get_fresh_with_clock(
        &self,
        max_age_ms: u64,
        clock: impl FnOnce() -> u128,
    ) -> Option<CachedBlockhash> {
        let cached = self.inner.get("latest")?.clone();
        if clock().saturating_sub(cached.updated_at_ms) > u128::from(max_age_ms) {
            return None;
        }
        Some(cached)
    }

    #[cfg(test)]
    pub fn age_ms(&self) -> Option<u128> {
        self.age_with_clock(now_ms)
    }

    pub(crate) fn freshness(&self, max_age_ms: u64) -> (bool, Option<u128>) {
        self.freshness_with_clock(max_age_ms, now_ms)
    }

    fn freshness_with_clock(
        &self,
        max_age_ms: u64,
        clock: impl FnOnce() -> u128,
    ) -> (bool, Option<u128>) {
        let Some(cached) = self.inner.get("latest") else {
            return (false, None);
        };
        let age = clock().saturating_sub(cached.updated_at_ms);
        (age <= u128::from(max_age_ms), Some(age))
    }

    #[cfg(test)]
    fn age_with_clock(&self, clock: impl FnOnce() -> u128) -> Option<u128> {
        self.inner
            .get("latest")
            .map(|cached| clock().saturating_sub(cached.updated_at_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Reader {
        value: Option<Blockhash>,
        calls: AtomicUsize,
    }

    impl LatestBlockhashReader for Reader {
        async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.value
                .ok_or_else(|| anyhow::anyhow!("synthetic unavailable blockhash"))
        }
    }

    fn cache() -> BlockhashCache {
        BlockhashCache::default()
    }

    fn cached() -> CachedBlockhash {
        CachedBlockhash {
            hash: Blockhash::from_bytes([42; 32]),
            updated_at_ms: 100,
        }
    }

    #[test]
    fn empty_cache_is_not_fresh_and_has_no_age() {
        let cache = cache();
        assert!(cache.get_fresh_with_clock(u64::MAX, || u128::MAX).is_none());
        assert_eq!(cache.age_with_clock(|| 100), None);
    }

    #[test]
    fn exact_age_boundary_is_fresh_and_expired_entries_are_retained() {
        let cache = cache();
        cache.inner.insert("latest", cached());
        assert_eq!(
            cache
                .get_fresh_with_clock(50, || 150)
                .unwrap()
                .hash
                .to_bytes(),
            [42; 32]
        );
        assert!(cache.get_fresh_with_clock(50, || 151).is_none());
        assert_eq!(cache.age_with_clock(|| 151), Some(51));
        assert_eq!(cache.inner.get("latest").unwrap().updated_at_ms, 100);
    }

    #[test]
    fn clock_rollback_saturates_and_zero_age_is_allowed() {
        let cache = cache();
        cache.inner.insert("latest", cached());
        assert_eq!(cache.age_with_clock(|| 99), Some(0));
        assert!(cache.get_fresh_with_clock(0, || 99).is_some());
        assert!(cache.get_fresh_with_clock(0, || 100).is_some());
        assert!(cache.get_fresh_with_clock(0, || 101).is_none());
    }

    #[test]
    fn readiness_cache_freshness_uses_one_entry_and_clock_sample() {
        let cache = cache();
        assert_eq!(cache.freshness_with_clock(50, || 150), (false, None));
        cache.inner.insert("latest", cached());
        for (max_age, now, expected) in [
            (50, 150, (true, Some(50))),
            (50, 151, (false, Some(51))),
            (0, 99, (true, Some(0))),
            (0, 100, (true, Some(0))),
            (0, 101, (false, Some(1))),
        ] {
            let calls = AtomicUsize::new(0);
            assert_eq!(
                cache.freshness_with_clock(max_age, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    now
                }),
                expected
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(cache.inner.get("latest").unwrap().updated_at_ms, 100);
        }
    }

    #[tokio::test]
    async fn refresh_uses_read_port_and_replaces_only_after_success() {
        let cache = cache();
        cache.inner.insert("latest", cached());
        let reader = Reader {
            value: Some(Blockhash::from_bytes([7; 32])),
            calls: AtomicUsize::new(0),
        };
        cache.refresh_once(&reader).await;
        assert_eq!(reader.calls.load(Ordering::Relaxed), 1);
        let result = cache.get_fresh(u64::MAX).unwrap();
        assert_eq!(result.hash.to_bytes(), [7; 32]);
        assert!(result.updated_at_ms > 100);
    }

    #[tokio::test]
    async fn failed_refresh_keeps_original_hash_age_and_empty_cache_unready() {
        for existing in [false, true] {
            let cache = cache();
            if existing {
                cache.inner.insert("latest", cached());
            }
            let reader = Reader {
                value: None,
                calls: AtomicUsize::new(0),
            };
            cache.refresh_once(&reader).await;
            assert_eq!(reader.calls.load(Ordering::Relaxed), 1);
            assert!(cache.get_fresh_with_clock(50, || 151).is_none());
            assert_eq!(
                cache.age_with_clock(|| 151),
                if existing { Some(51) } else { None }
            );
            if existing {
                assert_eq!(cache.inner.get("latest").unwrap().hash.to_bytes(), [42; 32]);
            }
        }
    }

    #[test]
    fn clock_is_sampled_after_snapshot_and_not_for_an_empty_cache() {
        let cache = cache();
        assert!(cache
            .get_fresh_with_clock(50, || panic!("empty cache must not query clock"))
            .is_none());
        assert_eq!(
            cache.age_with_clock(|| panic!("empty cache must not query clock")),
            None
        );
        cache.inner.insert("latest", cached());
        let result = cache
            .get_fresh_with_clock(50, || {
                cache.inner.insert(
                    "latest",
                    CachedBlockhash {
                        hash: Blockhash::from_bytes([9; 32]),
                        updated_at_ms: 150,
                    },
                );
                150
            })
            .unwrap();
        assert_eq!(result.hash.to_bytes(), [42; 32]);
        assert_eq!(result.updated_at_ms, 100);
        assert_eq!(cache.inner.get("latest").unwrap().hash.to_bytes(), [9; 32]);
    }
}
