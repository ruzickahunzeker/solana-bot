use std::future::Future;

use crate::{
    application::sol_transfer_record::{SolTransferJournalEntry, SolTransferReservation},
    domain::{
        sol_transfer::{SolTransferAttemptState, SolTransferIntent, SolTransferSnapshot},
        submission::SignedSubmission,
    },
};

pub(crate) trait SolTransferJournal: Sync {
    fn lookup(
        &self,
        key: &str,
    ) -> impl Future<Output = anyhow::Result<Option<SolTransferJournalEntry>>> + Send;
    fn reserve(
        &self,
        intent: &SolTransferIntent,
    ) -> impl Future<Output = anyhow::Result<SolTransferReservation>> + Send;
    fn record_signed(
        &self,
        id: &str,
        snapshot: &SolTransferSnapshot,
        signed: &SignedSubmission,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn fail_before_sign(
        &self,
        id: &str,
        error: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn record_broadcast(
        &self,
        id: &str,
        state: SolTransferAttemptState,
        error: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn record_confirmed(&self, id: &str) -> impl Future<Output = anyhow::Result<()>> + Send;
}
