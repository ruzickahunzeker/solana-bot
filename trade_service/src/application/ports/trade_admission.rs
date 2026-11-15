use crate::application::{
    trade_command::TradeCommand,
    trade_persistence_input::{RejectedTradeInput, ValidatedTradeInput},
    trade_record::TradeRecordV1,
};

pub enum MemoryReservation {
    Reserved,
    Duplicate(String),
    Conflict,
}

pub(crate) trait MemoryTradeReservations: Sync {
    fn reserve_key(
        &self,
        wallet_id: &str,
        key: &str,
        request_id: &str,
        hash: &str,
        aggregator_sol: bool,
    ) -> MemoryReservation;
}

pub(crate) trait TradeRecordIndex: Sync {
    fn insert_record(&self, record: &TradeRecordV1);
    fn get_record(&self, request_id: &str) -> Option<TradeRecordV1>;
    fn remove_record(&self, request_id: &str);
}

/// Lazy legacy mapping: do not serialize persistence JSON for memory/replayed requests.
pub(crate) trait ValidatedAdmissionPayload: Sync {
    fn wallet_id(&self) -> &str;
    fn idempotency_key(&self) -> &str;
    fn aggregator_sol(&self) -> bool;
    fn fingerprint(&self) -> anyhow::Result<String>;
    fn new_record(&self, request_id: String) -> TradeRecordV1;
    fn snapshot(&self) -> anyhow::Result<ValidatedTradeInput>;
    fn execution_command(&self) -> TradeCommand;
}

/// Rejection metadata only; deliberately no normal idempotency reservation or execution API.
pub(crate) trait RejectedAdmissionPayload: Sync {
    fn new_record(&self, request_id: String) -> TradeRecordV1;
    fn snapshot(&self, record: &TradeRecordV1) -> anyhow::Result<RejectedTradeInput>;
}
