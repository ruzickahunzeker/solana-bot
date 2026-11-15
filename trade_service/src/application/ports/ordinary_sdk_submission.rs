use crate::{
    application::{
        ports::ordinary_claims::ExecutionOwner, sdk_preparation::StoredSdkCandidate,
        trade_admission::AdmittedTrade, trade_record::TradeRecordV1,
    },
    domain::{sdk_candidate::SdkCandidateMetadata, submission::SignedSubmission},
};
use std::future::Future;

/// Uses the original ordinary-order authority, never an independent Quote reservation.
pub(crate) trait OrdinarySdkSubmissionJournal: Sync {
    fn prepare(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn read_prepared(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> impl Future<Output = anyhow::Result<StoredSdkCandidate>> + Send;
    fn begin_signing(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn record_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn read_signed(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> impl Future<Output = anyhow::Result<SignedSubmission>> + Send;
    fn take_broadcast(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
    ) -> impl Future<Output = anyhow::Result<SignedSubmission>> + Send;
    fn record_broadcast(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signed: &SignedSubmission,
        submitted: bool,
        error: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn fail_before_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        error: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn observe(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> impl Future<Output = anyhow::Result<TradeRecordV1>> + Send;
}

/// Candidate and signed artifact retain backend-owned live evidence, not a recovered DTO.
/// Implementations must not spawn an opaque sign/send worker or retry with another route/key.
pub(crate) trait OrdinarySdkCandidateBackend: Sync {
    type Candidate: Send + Sync;
    type Signed: Send + Sync;

    fn ensure_live_allowed(&self) -> anyhow::Result<()>;
    fn metadata<'a>(&self, candidate: &'a Self::Candidate) -> &'a SdkCandidateMetadata;
    fn unsigned_message(&self, candidate: &Self::Candidate) -> anyhow::Result<Vec<u8>>;
    fn authorize(
        &self,
        execution: &AdmittedTrade,
        candidate: &Self::Candidate,
    ) -> anyhow::Result<()>;
    fn sign(&self, candidate: Self::Candidate) -> anyhow::Result<Self::Signed>;
    fn signed_submission<'a>(&self, signed: &'a Self::Signed) -> &'a SignedSubmission;
    fn verify_signed(
        &self,
        signed: &Self::Signed,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> anyhow::Result<()>;
    fn authorize_broadcast(&self, signed: &Self::Signed) -> anyhow::Result<()>;
    fn broadcast(
        &self,
        signed: &SignedSubmission,
    ) -> impl Future<Output = anyhow::Result<String>> + Send;
}
