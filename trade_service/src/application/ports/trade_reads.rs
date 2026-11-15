use std::future::Future;

use crate::application::trade_record::TradeRecordV1;

/// Read-only ordinary V1 rows; no reservation, status mutation, signer or execution handle.
pub(crate) trait TradeRecordReadSource: Sync {
    fn read_record(
        &self,
        request_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<TradeRecordV1>>> + Send;
    fn recent_records(
        &self,
        limit: usize,
    ) -> impl Future<Output = anyhow::Result<Vec<TradeRecordV1>>> + Send;
}
