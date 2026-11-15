use crate::{
    application::{
        ports::ordinary_claims::ExecutionOwner, trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::{
        quote::{BoundQuote, QuotePreflight},
        submission::SignedSubmission,
    },
};
use std::future::Future;

pub(crate) trait OrdinarySubmissionJournal: Sync {
    fn begin(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn prepare(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        bound: &BoundQuote,
        preflight: &QuotePreflight,
        message: &[u8],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn authorize_sign(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        bound: &BoundQuote,
        preflight: &QuotePreflight,
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
