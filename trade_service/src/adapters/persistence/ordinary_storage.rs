use std::sync::Arc;

use super::{ordinary_claims::RuntimeExecutionClaims, trade_reads::StoredTradeReadSource};
use crate::{
    application::ports::{
        ordinary_claims::OrdinaryExecutionClaimer,
        ordinary_journal::OrdinaryTradeJournal,
        ordinary_runtime::{OrdinaryExecutionStorage, TradeExecutionIndex},
        trade_admission::{MemoryTradeReservations, TradeRecordIndex},
        trade_reads::TradeRecordReadSource,
    },
    idempotency::IdempotencyStore,
    persistence::TradePersistence,
    state::TradeStore,
};

/// These handles share the existing ordinary-order authority; no storage is opened here.
#[derive(Clone)]
pub(crate) struct OrdinaryStorage {
    journal: Arc<TradePersistence>,
    index: Arc<TradeStore>,
    reservations: Arc<IdempotencyStore>,
}

impl OrdinaryStorage {
    pub(crate) fn new(
        journal: Arc<TradePersistence>,
        index: Arc<TradeStore>,
        reservations: Arc<IdempotencyStore>,
    ) -> Self {
        Self {
            journal,
            index,
            reservations,
        }
    }

    pub(crate) fn view(&self) -> OrdinaryStorageRef<'_> {
        OrdinaryStorageRef::new(&self.journal, &self.index, &self.reservations)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OrdinaryStorageRef<'a> {
    journal: &'a Arc<TradePersistence>,
    index: &'a Arc<TradeStore>,
    reservations: &'a Arc<IdempotencyStore>,
}

impl<'a> OrdinaryStorageRef<'a> {
    pub(crate) fn new(
        journal: &'a Arc<TradePersistence>,
        index: &'a Arc<TradeStore>,
        reservations: &'a Arc<IdempotencyStore>,
    ) -> Self {
        Self {
            journal,
            index,
            reservations,
        }
    }

    pub(crate) fn retain(self) -> OrdinaryStorage {
        OrdinaryStorage::new(
            Arc::clone(self.journal),
            Arc::clone(self.index),
            Arc::clone(self.reservations),
        )
    }

    pub(crate) fn admission_journal(self) -> &'a impl OrdinaryTradeJournal {
        self.journal.as_ref()
    }

    pub(crate) fn admission_index(self) -> &'a impl TradeRecordIndex {
        self.index.as_ref()
    }

    pub(crate) fn admission_reservations(self) -> &'a impl MemoryTradeReservations {
        self.reservations.as_ref()
    }

    pub(crate) fn reads(self) -> impl TradeRecordReadSource + 'a {
        StoredTradeReadSource::new(self.journal.as_ref(), self.index.as_ref())
    }

    pub(crate) fn execution_journal(self) -> &'a impl OrdinaryExecutionStorage {
        self.journal.as_ref()
    }

    pub(crate) fn execution_index(self) -> &'a impl TradeExecutionIndex {
        self.index.as_ref()
    }

    pub(crate) fn claims(self) -> impl OrdinaryExecutionClaimer + Send + 'static + use<> {
        RuntimeExecutionClaims::new(
            Arc::clone(self.journal),
            Arc::clone(self.index),
            Arc::clone(self.reservations),
        )
    }
}

#[cfg(test)]
mod tests;
