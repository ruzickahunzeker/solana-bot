use super::submission::SubmissionStatus;
use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChainCommitment {
    Processed = 1,
    Confirmed = 2,
    Finalized = 3,
}

impl ChainCommitment {
    pub fn rank(self) -> i64 {
        self as i64
    }
    pub fn from_rank(rank: i64) -> Result<Option<Self>> {
        Ok(match rank {
            0 => None,
            1 => Some(Self::Processed),
            2 => Some(Self::Confirmed),
            3 => Some(Self::Finalized),
            _ => bail!("invalid chain confirmation rank"),
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChainObservation {
    pub signature: String,
    pub commitment: ChainCommitment,
    pub slot: u64,
    pub error: Option<String>,
}

impl ChainObservation {
    pub fn submission_status(&self) -> SubmissionStatus {
        match (self.commitment, self.error.is_some()) {
            (ChainCommitment::Processed, true) => SubmissionStatus::Unknown,
            (_, true) => SubmissionStatus::Failed,
            (ChainCommitment::Processed, false) => SubmissionStatus::Submitted,
            (ChainCommitment::Confirmed, false) => SubmissionStatus::Confirmed,
            (ChainCommitment::Finalized, false) => SubmissionStatus::Finalized,
        }
    }
}
