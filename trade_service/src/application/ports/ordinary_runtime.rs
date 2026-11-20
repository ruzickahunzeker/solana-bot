use crate::application::{
    ports::{
        ordinary_journal::OrdinaryTradeJournal,
        ordinary_sdk_submission::OrdinarySdkSubmissionJournal, trade_admission::TradeRecordIndex,
    },
    trade_record::TradeRecordV1,
};
use crate::domain::trade::TradeStatus;

/// Access to the same ordinary-order authority, not a new reservation or journal.
pub(crate) trait OrdinaryExecutionStorage: OrdinaryTradeJournal {
    // Ordinary background execution owns this handle across awaits.
    type SdkJournal: OrdinarySdkSubmissionJournal + Send;
    fn sdk_journal(&self) -> Option<Self::SdkJournal>;

    /// No owner exists yet. Return a record only after the original durable CAS commits.
    fn fail_unclaimed_preparation(
        &self,
        _execution: &crate::application::trade_admission::AdmittedTrade,
        _error: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<TradeRecordV1>>> + Send {
        async { anyhow::bail!("unclaimed SDK failure storage unavailable") }
    }
}

pub(crate) trait TradeExecutionIndex: TradeRecordIndex {
    fn update_status(
        &self,
        request_id: &str,
        status: TradeStatus,
        signature: Option<String>,
        error: Option<String>,
    ) -> Option<TradeRecordV1>;
}
