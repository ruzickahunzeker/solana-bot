use anyhow::Result;
use std::future::Future;

use crate::domain::{
    reconciliation::ChainObservation,
    submission::{SubmissionKey, SubmissionRecord},
};

/// Deliberately offers no signer, signed bytes, reservation or broadcasting capability.
pub(crate) trait SignatureStatusReader: Sync {
    fn lookup_signature(
        &self,
        signature: &str,
    ) -> impl Future<Output = Result<Option<ChainObservation>>> + Send;
}

pub(crate) trait ReconciliationJournal: Sync {
    fn lookup(
        &self,
        key: &SubmissionKey,
    ) -> impl Future<Output = Result<Option<SubmissionRecord>>> + Send;
    fn record_observation(
        &self,
        submission_id: &str,
        observation: &ChainObservation,
    ) -> impl Future<Output = Result<bool>> + Send;
}
