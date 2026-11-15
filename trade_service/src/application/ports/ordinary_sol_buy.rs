use crate::{
    application::{
        ports::ordinary_claims::ExecutionOwner,
        quote_submission_types::{QuoteSubmissionCommand, QuoteSubmissionResult},
        trade_admission::AdmittedTrade,
        trade_execution::TradeExecutionResult,
        trade_record::TradeRecordV1,
    },
    domain::{
        quote::{BoundQuote, QuoteEstimate},
        trade::Address,
    },
};
use std::{future::Future, pin::Pin};

pub(crate) struct JournaledSolBuyOutcome {
    pub record: Option<TradeRecordV1>,
    pub signature: Option<String>,
    pub error: Option<String>,
}

pub(crate) enum SolBuyExecutionOutcome {
    Legacy(TradeExecutionResult),
    Journaled(JournaledSolBuyOutcome),
}

pub(crate) type SolBuyExecutionFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<SolBuyExecutionOutcome>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SolBuyIntent {
    pub wallet_id: String,
    pub mint: Address,
    pub amount_lamports: u64,
    pub slippage_bps: u16,
}

/// The ordinary aggregator path retains its own lookup and precedes SDK/PumpAPI selection.
pub(crate) trait OrdinarySolBuyBackend: Send + Sync {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinarySolBuyWallet>>;
}

pub(crate) trait OrdinarySolBuyWallet: Send + Sync {
    fn buy<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        owner: Option<&'a ExecutionOwner>,
    ) -> SolBuyExecutionFuture<'a>;
}

/// Only the existing quote -> bind -> legacy-submit stages, not a new submission journal.
pub(crate) trait OrdinarySolBuyPipeline: Send + Sync {
    type Request: Send + Sync;

    fn map_request(&self, input: SolBuyIntent) -> Self::Request;
    fn validate_request(&self, request: &Self::Request) -> anyhow::Result<()>;
    fn quote(
        &self,
        request: &Self::Request,
    ) -> impl Future<Output = anyhow::Result<QuoteEstimate>> + Send;
    fn bind(&self, request: &Self::Request, quote: QuoteEstimate) -> anyhow::Result<BoundQuote>;
    fn submit(
        &self,
        bound: &BoundQuote,
        request: &QuoteSubmissionCommand,
    ) -> impl Future<Output = anyhow::Result<QuoteSubmissionResult>> + Send;
}
