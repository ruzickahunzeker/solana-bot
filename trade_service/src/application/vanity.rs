use anyhow::{bail, Result};
use std::{future::Future, pin::Pin};

use crate::{
    application::ports::vanity::VanityWorkerPort,
    domain::wallet::{VanityJob, VanityMatchSide},
};

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

const DEFAULT_MAX_ATTEMPTS: u64 = 30_000_000;
const HARD_MAX_ATTEMPTS: u64 = 30_000_000;

pub(crate) struct VanityCreateInput {
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub match_side: VanityMatchSide,
    pub pattern: String,
    pub max_attempts: Option<u64>,
}

pub(crate) struct VanityCreateCommand {
    owner_type: String,
    owner_id: String,
    label: String,
    match_side: VanityMatchSide,
    pattern: String,
    max_attempts: u64,
}

impl VanityCreateCommand {
    pub(crate) fn owner_type(&self) -> &str {
        &self.owner_type
    }
    pub(crate) fn owner_id(&self) -> &str {
        &self.owner_id
    }
    pub(crate) fn label(&self) -> &str {
        &self.label
    }
    pub(crate) fn match_side(&self) -> &str {
        self.match_side.as_str()
    }
    pub(crate) fn pattern(&self) -> &str {
        &self.pattern
    }
    pub(crate) fn max_attempts(&self) -> u64 {
        self.max_attempts
    }
}

fn validate_pattern(pattern: &str) -> Result<()> {
    if pattern.is_empty() || pattern.len() > 4 || !pattern.is_ascii() {
        bail!("pattern must contain 1..4 Base58 characters");
    }
    const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if !pattern.chars().all(|character| BASE58.contains(character)) {
        bail!("pattern contains invalid Base58 characters (0, O, I and l are not allowed)");
    }
    Ok(())
}

fn prepare(input: VanityCreateInput) -> Result<VanityCreateCommand> {
    validate_pattern(&input.pattern)?;
    Ok(VanityCreateCommand {
        owner_type: input.owner_type,
        owner_id: input.owner_id,
        label: input.label.trim().into(),
        match_side: input.match_side,
        pattern: input.pattern,
        max_attempts: input
            .max_attempts
            .unwrap_or(DEFAULT_MAX_ATTEMPTS)
            .clamp(1, HARD_MAX_ATTEMPTS),
    })
}

/// Called at the worker's original post-read guard; no additional query or mutation.
pub(crate) fn cancellation_allowed(
    job: &VanityJob,
    owner_type: &str,
    owner_id: &str,
) -> Result<bool> {
    if job.owner_type != owner_type || job.owner_id != owner_id {
        bail!("wallet generation job does not belong to this owner");
    }
    Ok(matches!(
        job.status.as_str(),
        "queued" | "running" | "cancel_requested"
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VanityFailureKind {
    InvalidRequest,
    NotFound,
    OwnerMismatch,
}

#[derive(Debug)]
pub(crate) struct VanityError {
    pub kind: VanityFailureKind,
    pub message: String,
}
impl VanityError {
    fn new(kind: VanityFailureKind, error: anyhow::Error) -> Self {
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

pub(crate) type VanityFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VanityJob, VanityError>> + Send + 'a>>;

pub(crate) trait VanityTasks: Send + Sync {
    fn create(&self, input: VanityCreateInput) -> VanityFuture<'_>;
    fn get<'a>(&'a self, id: &'a str, owner_type: &'a str, owner_id: &'a str) -> VanityFuture<'a>;
    fn cancel<'a>(
        &'a self,
        id: &'a str,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> VanityFuture<'a>;
}

pub(crate) struct VanityService<W> {
    worker: W,
}
impl<W> VanityService<W> {
    pub(crate) fn new(worker: W) -> Self {
        Self { worker }
    }
}

impl<W: VanityWorkerPort> VanityTasks for VanityService<W> {
    fn create(&self, input: VanityCreateInput) -> VanityFuture<'_> {
        Box::pin(async move {
            let command = prepare(input)
                .map_err(|error| VanityError::new(VanityFailureKind::InvalidRequest, error))?;
            self.worker
                .create(command)
                .await
                .map_err(|error| VanityError::new(VanityFailureKind::InvalidRequest, error))
        })
    }
    fn get<'a>(&'a self, id: &'a str, owner_type: &'a str, owner_id: &'a str) -> VanityFuture<'a> {
        Box::pin(async move {
            let job = self
                .worker
                .get(id)
                .await
                .map_err(|error| VanityError::new(VanityFailureKind::NotFound, error))?;
            if job.owner_type != owner_type || job.owner_id != owner_id {
                return Err(VanityError {
                    kind: VanityFailureKind::OwnerMismatch,
                    message: "wallet generation job owner mismatch".into(),
                });
            }
            Ok(job)
        })
    }
    fn cancel<'a>(
        &'a self,
        id: &'a str,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> VanityFuture<'a> {
        Box::pin(async move {
            self.worker
                .cancel(id, owner_type, owner_id)
                .await
                .map_err(|error| VanityError::new(VanityFailureKind::InvalidRequest, error))
        })
    }
}
