use anyhow::bail;
use dashmap::DashMap;
use solana_sdk::signature::{Keypair, Signer};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::sync::Semaphore;

use crate::{
    config::WalletConfig,
    models::CreateVanityJobRequest,
    sanitize::redact_urls,
    wallet_repository::{VanityJob, WalletRepository},
    wallets::WalletRegistry,
};

const DEFAULT_MAX_ATTEMPTS: u64 = 30_000_000;
const HARD_MAX_ATTEMPTS: u64 = 30_000_000;

struct VanityRuntime {
    attempts: AtomicU64,
    cancelled: AtomicBool,
}

pub struct WalletOps {
    repository: Arc<WalletRepository>,
    wallets: Arc<WalletRegistry>,
    vanity_slot: Arc<Semaphore>,
    vanity_jobs: DashMap<String, Arc<VanityRuntime>>,
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
        })
    }

    pub async fn create_vanity_job(
        self: &Arc<Self>,
        request: CreateVanityJobRequest,
    ) -> anyhow::Result<VanityJob> {
        validate_vanity_pattern(&request.pattern)?;
        let max_attempts = request
            .max_attempts
            .unwrap_or(DEFAULT_MAX_ATTEMPTS)
            .clamp(1, HARD_MAX_ATTEMPTS);
        let job = self
            .repository
            .create_vanity_job(
                &request.owner_type,
                &request.owner_id,
                request.label.trim(),
                request.match_side.as_str(),
                &request.pattern,
                max_attempts,
            )
            .await?;
        let runtime = Arc::new(VanityRuntime {
            attempts: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
        });
        self.vanity_jobs.insert(job.job_id.clone(), runtime.clone());
        let service = self.clone();
        let queued_job = job.clone();
        tokio::spawn(async move {
            service.run_vanity_job(queued_job, runtime).await;
        });
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
        if job.owner_type != owner_type || job.owner_id != owner_id {
            bail!("wallet generation job does not belong to this owner");
        }
        if !matches!(
            job.status.as_str(),
            "queued" | "running" | "cancel_requested"
        ) {
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
                        match self.wallets.add(&config).await {
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
                    Some(&redact_urls(&error.to_string())),
                )
                .await;
        }
        self.vanity_jobs.remove(&job.job_id);
    }
}

fn validate_vanity_pattern(pattern: &str) -> anyhow::Result<()> {
    if pattern.is_empty() || pattern.len() > 4 || !pattern.is_ascii() {
        bail!("pattern must contain 1..4 Base58 characters");
    }
    const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if !pattern.chars().all(|character| BASE58.contains(character)) {
        bail!("pattern contains invalid Base58 characters (0, O, I and l are not allowed)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_base58_vanity_patterns() {
        assert!(validate_vanity_pattern("9999").is_ok());
        assert!(validate_vanity_pattern("8888").is_ok());
        assert!(validate_vanity_pattern("0abc").is_err());
        assert!(validate_vanity_pattern("abcde").is_err());
    }
}
