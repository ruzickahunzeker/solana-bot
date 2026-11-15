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
    type SdkJournal: OrdinarySdkSubmissionJournal;
    fn sdk_journal(&self) -> Option<Self::SdkJournal>;
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
