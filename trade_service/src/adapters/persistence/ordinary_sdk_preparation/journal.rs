use super::*;
use crate::{
    application::ports::ordinary_sdk_submission::OrdinarySdkSubmissionJournal,
    domain::submission::SignedSubmission,
};

impl OrdinarySdkSubmissionJournal for SqliteOrdinaryExecutionClaims {
    async fn prepare(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> Result<()> {
        self.prepare_sdk_candidate(execution, owner, metadata, message)
            .await
    }
    async fn read_prepared(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<StoredSdkCandidate> {
        self.read_sdk_candidate(execution, owner).await
    }
    async fn begin_signing(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> Result<()> {
        self.begin_sdk_signing(execution, owner, metadata, message)
            .await
    }
    async fn record_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
    ) -> Result<()> {
        self.record_sdk_signed(execution, owner, signed).await
    }
    async fn read_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<SignedSubmission> {
        self.read_sdk_signed(execution, owner).await
    }
    async fn take_broadcast(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
    ) -> Result<SignedSubmission> {
        self.take_sdk_broadcast(execution, owner, signed).await
    }
    async fn record_broadcast(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
        submitted: bool,
        error: Option<&str>,
    ) -> Result<()> {
        self.record_sdk_broadcast(execution, owner, signed, submitted, error)
            .await
    }
    async fn fail_before_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        error: &str,
    ) -> Result<()> {
        self.fail_sdk_before_sign(execution, owner, error).await
    }
    async fn observe(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> Result<TradeRecordV1> {
        self.observe_sdk_submission(execution, owner).await
    }
}
