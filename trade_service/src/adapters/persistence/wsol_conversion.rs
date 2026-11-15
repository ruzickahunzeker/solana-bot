use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::{
    application::wsol_failure::WsolFailure,
    application::wsol_submission_types::WsolSignedOperation,
    domain::wsol_progress::{WsolBroadcastOutcome, WsolObservedOutcome, WsolUpdate},
};
use crate::{
    application::{
        ports::wsol_conversion::WsolJournal,
        wsol_submission_types::{WsolQuoteSnapshot, WsolSubmissionInput, WsolSubmissionRecord},
    },
    conversion::{ConversionStore, WsolQuote, WsolSubmission},
};

#[derive(Clone)]
pub(crate) struct SqliteWsolJournal {
    store: Arc<ConversionStore>,
}
impl SqliteWsolJournal {
    pub(crate) fn new(store: Arc<ConversionStore>) -> Self {
        Self { store }
    }
}

impl WsolQuote {
    pub fn validate_current_balance(&self, current_balance_lamports: u64) -> Result<()> {
        if current_balance_lamports != self.source_balance_lamports {
            bail!("conversion source balance changed; request a new quote");
        }
        Ok(())
    }

    pub fn validate_binding(
        &self,
        wallet_id: &str,
        wallet_pubkey: &str,
        owner_type: &str,
        owner_id: &str,
        now_ms: u64,
    ) -> Result<()> {
        if now_ms >= self.expires_at_ms {
            bail!("conversion quote expired");
        }
        if self.wallet_id != wallet_id
            || self.wallet_pubkey != wallet_pubkey
            || self.owner_type != owner_type
            || self.owner_id != owner_id
        {
            bail!("conversion quote does not belong to this wallet and owner");
        }
        if self.amount_lamports == 0 || self.amount_lamports > self.source_balance_lamports {
            bail!("conversion quote amount is invalid for source balance");
        }
        Ok(())
    }
}

impl WsolSubmission {
    pub fn fingerprint(&self, quote: &WsolQuote) -> Result<String> {
        if self.idempotency_key.trim().is_empty() || self.idempotency_key.len() > 128 {
            bail!("idempotency_key must be 1..128 characters");
        }
        if self.quote_id != quote.quote_id
            || self.wallet_id != quote.wallet_id
            || self.owner_type != quote.owner_type
            || self.owner_id != quote.owner_id
        {
            bail!("submission does not match conversion quote");
        }
        let encoded = serde_json::to_vec(&(self, quote))?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}

pub(crate) fn reservation_fingerprint(
    input: &WsolSubmissionInput,
    quote: &WsolQuoteSnapshot,
) -> Result<String> {
    if input.simulate {
        bail!(WsolFailure::rejected(
            "simulation does not reserve a live idempotency key"
        ));
    }
    WsolSubmission::from(input)
        .fingerprint(&quote.into())
        .map_err(|error| WsolFailure::rejected(error).into())
}

impl WsolJournal for SqliteWsolJournal {
    async fn quote(&self, id: &str) -> anyhow::Result<Option<WsolQuoteSnapshot>> {
        Ok(self.store.get_quote(id).await?.as_ref().map(Into::into))
    }
    async fn reserve(
        &self,
        input: &WsolSubmissionInput,
        quote: &WsolQuoteSnapshot,
        now: u64,
    ) -> anyhow::Result<(WsolSubmissionRecord, bool)> {
        let fingerprint = reservation_fingerprint(input, quote)?;
        self.store
            .reserve_bound(input, quote, &fingerprint, now)
            .await
    }
    async fn record_signed(
        &self,
        operation: WsolSignedOperation<'_>,
        now: u64,
    ) -> anyhow::Result<WsolSubmissionRecord> {
        self.store.record_signed_bound(operation, now).await
    }
    async fn record_simulation_failure(
        &self,
        operation: WsolSignedOperation<'_>,
        error: &str,
        now: u64,
    ) -> anyhow::Result<()> {
        self.store
            .apply_signed_update(operation, WsolUpdate::SimulationUncertain, Some(error), now)
            .await
    }
    async fn fail_before_sign(
        &self,
        expected: &WsolSubmissionRecord,
        error: &str,
        now: u64,
    ) -> anyhow::Result<WsolSubmissionRecord> {
        self.store
            .fail_before_sign_bound(expected, error, now)
            .await
    }
    async fn record_broadcast(
        &self,
        operation: WsolSignedOperation<'_>,
        status: WsolBroadcastOutcome,
        error: Option<&str>,
        now: u64,
    ) -> anyhow::Result<()> {
        self.store
            .apply_signed_update(operation, WsolUpdate::Broadcast(status), error, now)
            .await
    }
    async fn by_id(&self, id: &str) -> anyhow::Result<Option<WsolSubmissionRecord>> {
        self.store.get_by_id(id).await
    }
    async fn by_key(&self, key: &str) -> anyhow::Result<Option<WsolSubmissionRecord>> {
        self.store.get_by_key(key).await
    }
}

impl crate::application::ports::wsol_queries::WsolQueryJournal for SqliteWsolJournal {
    async fn recovery(
        &self,
        input: &crate::application::wsol_recovery::WsolRecoveryInput,
    ) -> anyhow::Result<Option<WsolSubmissionRecord>> {
        self.store.recover_by_key(input).await
    }
    async fn save_quote(&self, quote: &WsolQuoteSnapshot) -> anyhow::Result<()> {
        self.store.save_quote(&quote.into()).await
    }
    async fn submission(&self, id: &str) -> anyhow::Result<Option<WsolSubmissionRecord>> {
        self.store.get_by_id(id).await
    }
    async fn record_observation(
        &self,
        expected: &WsolSubmissionRecord,
        status: WsolObservedOutcome,
        error: Option<&str>,
        now: u64,
    ) -> anyhow::Result<()> {
        self.store
            .apply_signed_update(
                WsolSignedOperation::from_record(expected)?,
                WsolUpdate::Observation(status),
                error,
                now,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wsol_adapter_round_trip_preserves_wire_fingerprint_and_shared_store() {
        let store = Arc::new(ConversionStore::new("sqlite::memory:").await.unwrap());
        let journal = SqliteWsolJournal::new(store.clone());
        assert!(Arc::ptr_eq(&store, &journal.store));
        let mut quote = crate::conversion::contract_tests::quote();
        quote.expires_at_ms = 4_000_000_000_000;
        let wire = crate::conversion::contract_tests::submission();
        let input = wire.input();
        let snapshot = WsolQuoteSnapshot::from(&quote);
        assert_eq!(WsolQuote::from(&snapshot), quote);
        assert_eq!(WsolSubmission::from(&input), wire);
        store.save_quote(&quote).await.unwrap();
        assert_eq!(
            journal.quote(&quote.quote_id).await.unwrap(),
            Some(snapshot.clone())
        );
        let (record, inserted) = journal.reserve(&input, &snapshot, 1).await.unwrap();
        assert!(inserted);
        assert_eq!(record.fingerprint, wire.fingerprint(&quote).unwrap());
        assert_eq!(
            store.get_by_key(&wire.idempotency_key).await.unwrap(),
            Some(record.clone())
        );
        let (duplicate, inserted) = store.reserve_live(&wire, &quote, 2).await.unwrap();
        assert!(!inserted);
        assert_eq!(duplicate, record);
    }
}
