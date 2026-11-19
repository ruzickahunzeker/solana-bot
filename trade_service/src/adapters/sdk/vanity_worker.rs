use dashmap::DashMap;
use solana_sdk::signature::{Keypair, Signer};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::sync::Semaphore;

mod lifecycle;

#[cfg(test)]
mod mutation_tests;
#[cfg(test)]
mod shutdown_tests;
#[cfg(test)]
mod tests;

use crate::{
    application::{
        ports::vanity::VanityWorkerPort,
        vanity::{cancellation_allowed, VanityCreateCommand},
    },
    config::WalletConfig,
    wallet_repository::{VanityJob, WalletRepository},
    wallets::WalletRegistry,
};

struct VanityRuntime {
    attempts: AtomicU64,
    cancelled: AtomicBool,
}

pub struct WalletOps {
    repository: Arc<WalletRepository>,
    wallets: Arc<WalletRegistry>,
    vanity_slot: Arc<Semaphore>,
    vanity_jobs: DashMap<String, Arc<VanityRuntime>>,
    lifecycle: lifecycle::VanityLifecycle,
}

impl WalletOps {
    pub async fn new(
        repository: Arc<WalletRepository>,
        wallets: Arc<WalletRegistry>,
    ) -> anyhow::Result<Self> {
        if repository.database_enabled() {
            repository.interrupt_active_vanity_jobs().await?;
        }
        Ok(Self {
            repository,
            wallets,
            vanity_slot: Arc::new(Semaphore::new(1)),
            vanity_jobs: DashMap::new(),
            lifecycle: Default::default(),
        })
    }

    pub(crate) async fn create_vanity_job(
        self: &Arc<Self>,
        request: VanityCreateCommand,
    ) -> anyhow::Result<VanityJob> {
        let admission = self.lifecycle.admit().await?;
        let job = self
            .repository
            .create_vanity_job(
                request.owner_type(),
                request.owner_id(),
                request.label(),
                request.match_side(),
                request.pattern(),
                request.max_attempts(),
            )
            .await?;
        let runtime = Arc::new(VanityRuntime {
            attempts: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
        });
        self.vanity_jobs.insert(job.job_id.clone(), runtime.clone());
        // Stop may have swept the map while this admitted request awaited its original row.
        if self.lifecycle.closed() {
            runtime.cancelled.store(true, Ordering::Relaxed);
        }
        let service = self.clone();
        let queued_job = job.clone();
        admission
            .spawn(async move {
                service.run_vanity_job(queued_job, runtime).await;
            })
            .await;
        Ok(job)
    }

    pub async fn get_vanity_job(&self, job_id: &str) -> anyhow::Result<VanityJob> {
        let mut job = self.repository.get_vanity_job(job_id).await?;
        if let Some(runtime) = self.vanity_jobs.get(job_id) {
            job.attempts = runtime.attempts.load(Ordering::Relaxed);
        }
        Ok(job)
    }

    pub async fn cancel_vanity_job(
        &self,
        job_id: &str,
        owner_type: &str,
        owner_id: &str,
    ) -> anyhow::Result<VanityJob> {
        let job = self.get_vanity_job(job_id).await?;
        if !cancellation_allowed(&job, owner_type, owner_id)? {
            return Ok(job);
        }
        if let Some(runtime) = self.vanity_jobs.get(job_id) {
            runtime.cancelled.store(true, Ordering::Relaxed);
        }
        self.repository
            .update_vanity_job(job_id, "cancel_requested", job.attempts, None, None, None)
            .await?;
        self.get_vanity_job(job_id).await
    }

    pub(crate) fn request_stop(&self) {
        self.lifecycle.close();
        for runtime in &self.vanity_jobs {
            runtime.cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) async fn shutdown(&self, deadline: std::time::Duration) -> anyhow::Result<()> {
        self.request_stop();
        let result = self.lifecycle.drain(deadline).await;
        if let Err(error) = &result {
            tracing::warn!(%error, "vanity shutdown incomplete; do not claim generation or persistence was cancelled");
        }
        result
    }

    async fn run_vanity_job(self: Arc<Self>, job: VanityJob, runtime: Arc<VanityRuntime>) {
        let _permit = match self.vanity_slot.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        if runtime.cancelled.load(Ordering::Relaxed) {
            let _ = self
                .repository
                .update_vanity_job(&job.job_id, "cancelled", 0, None, None, None)
                .await;
            self.vanity_jobs.remove(&job.job_id);
            return;
        }
        let _ = self
            .repository
            .update_vanity_job(&job.job_id, "running", 0, None, None, None)
            .await;

        let side = job.match_side.clone();
        let pattern = job.pattern.clone();
        let runtime_for_worker = runtime.clone();
        let max_attempts = job.max_attempts;
        let generated = tokio::task::spawn_blocking(move || {
            for attempt in 1..=max_attempts {
                if attempt % 1_000 == 0 {
                    runtime_for_worker
                        .attempts
                        .store(attempt, Ordering::Relaxed);
                    if runtime_for_worker.cancelled.load(Ordering::Relaxed) {
                        return None;
                    }
                }
                let keypair = Keypair::new();
                let pubkey = keypair.pubkey().to_string();
                let matched = if side == "prefix" {
                    pubkey.starts_with(&pattern)
                } else {
                    pubkey.ends_with(&pattern)
                };
                if matched {
                    runtime_for_worker
                        .attempts
                        .store(attempt, Ordering::Relaxed);
                    return Some((pubkey, bs58::encode(keypair.to_bytes()).into_string()));
                }
            }
            runtime_for_worker
                .attempts
                .store(max_attempts, Ordering::Relaxed);
            None
        })
        .await;

        let attempts = runtime.attempts.load(Ordering::Relaxed);
        let result = match generated {
            Ok(Some((pubkey, private_key))) => {
                let mut mutation = self.wallets.begin_management_mutation().await;
                match self
                    .repository
                    .import(&job.owner_type, &job.owner_id, &job.label, &private_key)
                    .await
                {
                    Ok((wallet, created)) => {
                        let config = WalletConfig {
                            wallet_id: wallet.wallet_id.clone(),
                            private_key_base58: private_key,
                        };
                        match self.wallets.add(&mut mutation, &config).await {
                            Ok(_) => {
                                self.repository
                                    .update_vanity_job(
                                        &job.job_id,
                                        "completed",
                                        attempts,
                                        Some(&wallet.wallet_id),
                                        Some(&pubkey),
                                        None,
                                    )
                                    .await
                            }
                            Err(error) => {
                                if created {
                                    let _ = self.repository.disable(&wallet.wallet_id).await;
                                }
                                Err(error)
                            }
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Ok(None) if runtime.cancelled.load(Ordering::Relaxed) => {
                self.repository
                    .update_vanity_job(&job.job_id, "cancelled", attempts, None, None, None)
                    .await
            }
            Ok(None) => {
                self.repository
                    .update_vanity_job(
                        &job.job_id,
                        "exhausted",
                        attempts,
                        None,
                        None,
                        Some("maximum attempts reached"),
                    )
                    .await
            }
            Err(error) => Err(anyhow::anyhow!("vanity worker failed: {error}")),
        };
        if let Err(error) = result {
            let _ = self
                .repository
                .update_vanity_job(
                    &job.job_id,
                    "failed",
                    attempts,
                    None,
                    None,
                    Some(&error.to_string()),
                )
                .await;
        }
        self.vanity_jobs.remove(&job.job_id);
    }
}

impl VanityWorkerPort for Arc<WalletOps> {
    async fn create(&self, command: VanityCreateCommand) -> anyhow::Result<VanityJob> {
        self.create_vanity_job(command).await
    }
    async fn get(&self, id: &str) -> anyhow::Result<VanityJob> {
        self.get_vanity_job(id).await
    }
    async fn cancel(
        &self,
        id: &str,
        owner_type: &str,
        owner_id: &str,
    ) -> anyhow::Result<VanityJob> {
        self.cancel_vanity_job(id, owner_type, owner_id).await
    }
}
