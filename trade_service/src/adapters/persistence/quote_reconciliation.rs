use crate::{
    adapters::persistence::submission_journal::SqliteSubmissionJournal,
    application::ports::reconciliation::ReconciliationJournal,
    domain::{
        reconciliation::ChainObservation,
        submission::{SubmissionKey, SubmissionRecord},
    },
};

/// Observation-only view: no migration, reservation, signed-byte write or broadcast methods.
pub(crate) struct QuoteReconciliationJournal {
    inner: SqliteSubmissionJournal,
}

impl QuoteReconciliationJournal {
    pub(crate) fn new(inner: SqliteSubmissionJournal) -> Self {
        Self { inner }
    }
}

impl ReconciliationJournal for QuoteReconciliationJournal {
    async fn lookup(&self, key: &SubmissionKey) -> anyhow::Result<Option<SubmissionRecord>> {
        self.inner.lookup(key).await
    }

    async fn record_observation(
        &self,
        submission_id: &str,
        observation: &ChainObservation,
    ) -> anyhow::Result<bool> {
        self.inner
            .record_observation(submission_id, observation)
            .await
    }
}
