use anyhow::Result;
use serde_json::Value;
use std::future::Future;

use crate::domain::submission::{
    Reservation, SignedSubmission, SubmissionIntent, SubmissionKey, SubmissionRecord,
    SubmissionStatus,
};

pub(crate) trait SubmissionJournal: Sync {
    fn get(
        &self,
        key: &SubmissionKey,
    ) -> impl Future<Output = Result<Option<SubmissionRecord>>> + Send;
    fn reserve(
        &self,
        intent: &SubmissionIntent,
        preflight: &Value,
    ) -> impl Future<Output = Result<Reservation>> + Send;
    fn record_signed(
        &self,
        submission_id: &str,
        signed: &SignedSubmission,
    ) -> impl Future<Output = Result<()>> + Send;
    fn fail_before_sign(
        &self,
        submission_id: &str,
        error: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn record_broadcast(
        &self,
        submission_id: &str,
        status: SubmissionStatus,
        error: Option<&str>,
    ) -> impl Future<Output = Result<()>> + Send;
}
