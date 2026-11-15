use crate::application::{
    trade_persistence_input::{RejectedTradeInput, ValidatedTradeInput},
    trade_record::TradeRecordV1,
};
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TradeJournalMode {
    Memory,
    Shadow,
    Durable,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SnapshotWrite {
    Reservation,
    Mirror,
}

pub enum TradeReservation {
    Created,
    Duplicate(TradeRecordV1),
    Conflict,
}

/// Ordinary V1 record storage, not the pre-broadcast signature submission journal.
pub(crate) trait OrdinaryTradeJournal: Sync {
    fn mode(&self) -> TradeJournalMode;
    fn ensure_snapshot_storage(&self, write: SnapshotWrite) -> anyhow::Result<()>;
    fn reserve_record(
        &self,
        record: &TradeRecordV1,
        input: &ValidatedTradeInput,
    ) -> impl Future<Output = anyhow::Result<TradeReservation>> + Send;
    fn mirror_accepted(
        &self,
        record: &TradeRecordV1,
        input: ValidatedTradeInput,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn mirror_rejection(
        &self,
        record: &TradeRecordV1,
        input: RejectedTradeInput,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn update_record(
        &self,
        record: &TradeRecordV1,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn read_record(
        &self,
        request_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<TradeRecordV1>>> + Send;
    fn recent_records(
        &self,
        limit: usize,
    ) -> impl Future<Output = anyhow::Result<Vec<TradeRecordV1>>> + Send;
}
