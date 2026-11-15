use crate::{
    application::{
        ports::ordinary_runtime::TradeExecutionIndex,
        ports::trade_admission::{MemoryReservation, MemoryTradeReservations, TradeRecordIndex},
        trade_record::TradeRecordV1,
    },
    idempotency::IdempotencyStore,
    state::TradeStore,
};

impl MemoryTradeReservations for IdempotencyStore {
    fn reserve_key(
        &self,
        wallet_id: &str,
        key: &str,
        request_id: &str,
        hash: &str,
        aggregator_sol: bool,
    ) -> MemoryReservation {
        self.reserve(wallet_id, key, request_id, hash, aggregator_sol)
    }
}

impl TradeExecutionIndex for TradeStore {
    fn update_status(
        &self,
        request_id: &str,
        status: crate::domain::trade::TradeStatus,
        signature: Option<String>,
        error: Option<String>,
    ) -> Option<TradeRecordV1> {
        self.update(request_id, status, signature, error)
    }
}

impl TradeRecordIndex for TradeStore {
    fn insert_record(&self, record: &TradeRecordV1) {
        TradeStore::insert_record(self, record)
    }
    fn get_record(&self, request_id: &str) -> Option<TradeRecordV1> {
        self.get(request_id)
    }
    fn remove_record(&self, request_id: &str) {
        self.remove(request_id)
    }
}
