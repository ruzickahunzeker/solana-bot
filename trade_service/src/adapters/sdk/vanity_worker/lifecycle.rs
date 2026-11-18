use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{
    sync::{Mutex, RwLock, RwLockReadGuard},
    task::{JoinError, JoinSet},
};

#[derive(Default)]
pub(super) struct VanityLifecycle {
    closed: AtomicBool,
    intake: RwLock<()>,
    jobs: Mutex<Jobs>,
}

#[derive(Default)]
struct Jobs {
    handles: JoinSet<()>,
    failed: usize,
}

impl Jobs {
    fn record(&mut self, result: Result<(), JoinError>) {
        if let Err(error) = result {
            self.failed = self.failed.saturating_add(1);
            tracing::error!(%error, "vanity task failed; retain original job for recovery");
        }
    }
}

/// Holds the intake-to-spawn interval, including the original database await.
pub(super) struct VanityAdmission<'a> {
    owner: &'a VanityLifecycle,
    _guard: RwLockReadGuard<'a, ()>,
}

impl VanityLifecycle {
    pub(super) async fn admit(&self) -> anyhow::Result<VanityAdmission<'_>> {
        let guard = self.intake.read().await;
        anyhow::ensure!(!self.closed(), "wallet generation service is stopping");
        Ok(VanityAdmission {
            owner: self,
            _guard: guard,
        })
    }

    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub(super) fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(super) fn intake_is_held_for_tests(&self) -> bool {
        self.intake.try_write().is_err()
    }

    pub(super) async fn drain(&self, deadline: Duration) -> anyhow::Result<()> {
        self.close();
        tokio::time::timeout(deadline, async {
            let _intake = self.intake.write().await;
            let mut jobs = self.jobs.lock().await;
            while let Some(result) = jobs.handles.join_next().await {
                jobs.record(result);
            }
            anyhow::ensure!(
                jobs.failed == 0,
                "vanity task shutdown failed: abandoned={}",
                jobs.failed
            );
            Ok(())
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("vanity task shutdown incomplete; original tasks retained, not aborted")
        })?
    }
}

impl VanityAdmission<'_> {
    pub(super) async fn spawn(self, future: impl Future<Output = ()> + Send + 'static) {
        let mut jobs = self.owner.jobs.lock().await;
        // Reap completed handles on intake as well as exit; no unbounded finished-handle history.
        while let Some(result) = jobs.handles.try_join_next() {
            jobs.record(result);
        }
        jobs.handles.spawn(future);
    }
}

#[cfg(test)]
mod tests;
