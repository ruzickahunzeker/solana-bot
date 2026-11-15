use tokio::time::{sleep, Duration, Instant};

use crate::{
    application::{ports::trade_reads::TradeRecordReadSource, trade_record::TradeRecordV1},
    domain::trade::TradeStatus,
};

pub(crate) struct TradeQueryService<S> {
    source: S,
}

impl<S: TradeRecordReadSource> TradeQueryService<S> {
    pub(crate) fn new(source: S) -> Self {
        Self { source }
    }

    pub(crate) async fn get(&self, request_id: &str) -> anyhow::Result<Option<TradeRecordV1>> {
        self.source.read_record(request_id).await
    }

    pub(crate) async fn recent(&self, limit: Option<usize>) -> anyhow::Result<Vec<TradeRecordV1>> {
        self.source
            .recent_records(limit.unwrap_or(20).clamp(1, 100))
            .await
    }

    /// V1 wait failures return no snapshot; admission retains its original accepted row.
    pub(crate) async fn wait(&self, request_id: &str, wait_ms: u64) -> Option<TradeRecordV1> {
        if wait_ms == 0 {
            return self.source.read_record(request_id).await.ok().flatten();
        }
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            let record = self.source.read_record(request_id).await.ok().flatten()?;
            if !is_pending_status(record.status) || Instant::now() >= deadline {
                return Some(record);
            }
            sleep(Duration::from_millis(20)).await;
        }
    }
}

pub(crate) fn is_pending_status(status: TradeStatus) -> bool {
    matches!(
        status,
        TradeStatus::Accepted
            | TradeStatus::WarmingParams
            | TradeStatus::BuildingTx
            | TradeStatus::Submitting
    )
}

#[cfg(test)]
mod tests;
