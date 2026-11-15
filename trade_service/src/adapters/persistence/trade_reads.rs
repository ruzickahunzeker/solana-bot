use crate::{
    application::{
        ports::{ordinary_journal::OrdinaryTradeJournal, trade_reads::TradeRecordReadSource},
        trade_record::TradeRecordV1,
    },
    persistence::TradePersistence,
    state::TradeStore,
};

/// Borrows the original request's stores; does not open a connection or create a second cache.
pub(crate) struct StoredTradeReadSource<'a> {
    persistence: &'a TradePersistence,
    trades: &'a TradeStore,
}

impl<'a> StoredTradeReadSource<'a> {
    pub(crate) fn new(persistence: &'a TradePersistence, trades: &'a TradeStore) -> Self {
        Self {
            persistence,
            trades,
        }
    }
}

impl TradeRecordReadSource for StoredTradeReadSource<'_> {
    async fn read_record(&self, request_id: &str) -> anyhow::Result<Option<TradeRecordV1>> {
        if self.persistence.is_durable() {
            return self.persistence.read_record(request_id).await;
        }
        Ok(self.trades.get(request_id))
    }

    async fn recent_records(&self, limit: usize) -> anyhow::Result<Vec<TradeRecordV1>> {
        if self.persistence.is_durable() {
            return self.persistence.recent_records(limit).await;
        }
        Ok(self.trades.recent(limit))
    }
}
