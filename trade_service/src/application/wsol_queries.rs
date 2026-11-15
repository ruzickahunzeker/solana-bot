use crate::domain::{reconciliation::ChainCommitment, wsol_progress::WsolState};
use anyhow::{bail, Context};
use std::time::Duration;

const WSOL_STATUS_TIMEOUT: Duration = Duration::from_secs(3);

use crate::application::{
    ports::{
        address_parser::TradeAddressParser,
        wsol_queries::{WsolQueryJournal, WsolQuerySource, WsolQuoteReader, WsolStatusReader},
    },
    wsol_failure::{WsolFailure, WsolFailureScope, WsolFailureStage as Stage},
    wsol_recovery::{WsolRecoveryFailure, WsolRecoveryInput},
    wsol_submission::{current_time_ms, WsolFuture},
    wsol_submission_types::{WsolDirection, WsolQuoteSnapshot, WsolSubmissionOutcome},
};

#[derive(Debug, Clone)]
pub(crate) struct WsolQuoteInput {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub direction: WsolDirection,
    pub amount_lamports: Option<u64>,
    pub all: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct WsolQuoteEstimate {
    pub amount_lamports: u64,
    pub source_balance_lamports: u64,
    pub ata_exists: bool,
    pub estimated_network_fee_lamports: u64,
    pub temporary_rent_lamports: u64,
    pub rent_refund_lamports: u64,
}

impl WsolQuoteEstimate {
    pub(crate) fn validate_for(&self, input: &WsolQuoteInput) -> anyhow::Result<()> {
        if self.amount_lamports == 0
            || self.amount_lamports > self.source_balance_lamports
            || (input.all && self.amount_lamports != self.source_balance_lamports)
            || (!input.all && input.amount_lamports != Some(self.amount_lamports))
        {
            bail!(WsolFailure::integrity(
                "WSOL quote estimate amount does not match the request and source balance"
            ));
        }
        let minimum_sol_cost = self
            .estimated_network_fee_lamports
            .checked_add(self.temporary_rent_lamports)
            .and_then(|value| value.checked_add(100_000))
            .ok_or_else(|| WsolFailure::integrity("WSOL quote estimate SOL budget overflows"))?;
        match input.direction {
            WsolDirection::Wrap => {
                if self.temporary_rent_lamports != 0 || self.rent_refund_lamports != 0 {
                    bail!(WsolFailure::integrity(
                        "WSOL wrap estimate contains unauthorized temporary rent or refund"
                    ));
                }
                let required = self.amount_lamports.checked_add(minimum_sol_cost);
                if required.map_or(true, |value| value > self.source_balance_lamports) {
                    bail!(WsolFailure::integrity(
                        "WSOL wrap estimate exceeds the source SOL budget"
                    ));
                }
            }
            WsolDirection::Unwrap => {
                let full = self.amount_lamports == self.source_balance_lamports;
                if !self.ata_exists
                    || (full && self.temporary_rent_lamports != 0)
                    || (!full && self.rent_refund_lamports != 0)
                {
                    bail!(WsolFailure::integrity(
                        "WSOL unwrap estimate has inconsistent source account, rent or refund"
                    ));
                }
            }
        }
        Ok(())
    }
}

pub(crate) trait WsolQueries: Send + Sync {
    fn recover(&self, input: WsolRecoveryInput) -> WsolFuture<'_, WsolSubmissionOutcome>;
    fn quote(&self, input: WsolQuoteInput) -> WsolFuture<'_, WsolQuoteSnapshot>;
    fn status<'a>(
        &'a self,
        id: &'a str,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> WsolFuture<'a, WsolSubmissionOutcome>;
}

pub(crate) struct WsolQueryService<S, J, P> {
    source: S,
    journal: Option<J>,
    parser: P,
}
impl<S, J, P> WsolQueryService<S, J, P> {
    pub(crate) fn new(source: S, journal: Option<J>, parser: P) -> Self {
        Self {
            source,
            journal,
            parser,
        }
    }
}
impl<S: WsolQuerySource, J: WsolQueryJournal, P: TradeAddressParser + Send + Sync> WsolQueries
    for WsolQueryService<S, J, P>
{
    fn recover(&self, input: WsolRecoveryInput) -> WsolFuture<'_, WsolSubmissionOutcome> {
        Box::pin(async move {
            input.validate()?;
            let journal = self
                .journal
                .as_ref()
                .ok_or(WsolRecoveryFailure::Unavailable)?;
            let mut record = journal
                .recovery(&input)
                .await
                .map_err(|_| WsolRecoveryFailure::Unavailable)?
                .ok_or(WsolRecoveryFailure::NotFound)?;
            input.validate_record(&record)?;
            // Return a ledger snapshot only; even status lookup may mutate observation state.
            record.error = record
                .error
                .map(|error| crate::sanitize::redact_urls(&error));
            Ok(WsolSubmissionOutcome {
                record,
                duplicate: false,
            })
        })
    }

    fn quote(&self, input: WsolQuoteInput) -> WsolFuture<'_, WsolQuoteSnapshot> {
        Box::pin(async move {
            let mut scope = WsolFailureScope::new();
            scope.stage(Stage::QuoteStore);
            let result: anyhow::Result<WsolQuoteSnapshot> = async {
                let journal = self
                    .journal
                    .as_ref()
                    .context("conversion quotes are disabled")?;
                scope.stage(Stage::QuoteMetadata);
                let metadata = self.source.metadata(&input.wallet_id).await?;
                if !metadata.enabled
                    || metadata.owner_type != input.owner_type
                    || metadata.owner_id != input.owner_id
                {
                    bail!(WsolFailure::rejected(
                        "wallet is disabled or does not belong to this owner"
                    ));
                }
                if metadata.wallet_id != input.wallet_id
                    || metadata.wallet_id.is_empty()
                    || metadata.owner_type.is_empty()
                    || metadata.owner_id.is_empty()
                {
                    bail!(WsolFailure::integrity(
                        "wallet metadata does not match requested wallet"
                    ));
                }
                if input.all && input.amount_lamports.is_some() {
                    bail!(WsolFailure::rejected(
                        "provide either all or amount_lamports"
                    ));
                }
                if input.all && input.direction != WsolDirection::Unwrap {
                    bail!(WsolFailure::rejected(
                        "all is only supported for WSOL unwrap"
                    ));
                }
                let owner = self
                    .parser
                    .parse(&metadata.pubkey)
                    .map_err(WsolFailure::integrity)?;
                if owner.to_string() != metadata.pubkey {
                    bail!(WsolFailure::integrity(
                        "wallet metadata public key is not canonical"
                    ));
                }
                scope.stage(Stage::QuoteEstimate);
                let reader = self
                    .source
                    .quote_reader(&input.wallet_id)
                    .context("wallet is not loaded")?;
                let estimate = reader.estimate(owner, &input).await?;
                estimate.validate_for(&input)?;
                let now = current_time_ms()?;
                let quote = WsolQuoteSnapshot {
                    quote_id: format!("cq_{}", uuid::Uuid::new_v4().simple()),
                    wallet_id: input.wallet_id,
                    wallet_pubkey: metadata.pubkey,
                    owner_type: input.owner_type,
                    owner_id: input.owner_id,
                    direction: input.direction,
                    amount_lamports: estimate.amount_lamports,
                    source_balance_lamports: estimate.source_balance_lamports,
                    ata_exists: estimate.ata_exists,
                    estimated_network_fee_lamports: estimate.estimated_network_fee_lamports,
                    temporary_rent_lamports: estimate.temporary_rent_lamports,
                    rent_refund_lamports: estimate.rent_refund_lamports,
                    expires_at_ms: now
                        .checked_add(60_000)
                        .context("WSOL quote expiry overflow")?,
                };
                scope.stage(Stage::QuoteSave);
                journal.save_quote(&quote).await?;
                Ok(quote)
            }
            .await;
            result.map_err(|error| scope.classify(error))
        })
    }

    fn status<'a>(
        &'a self,
        id: &'a str,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> WsolFuture<'a, WsolSubmissionOutcome> {
        Box::pin(async move {
            let mut scope = WsolFailureScope::new();
            scope.stage(Stage::StatusRead);
            let result: anyhow::Result<WsolSubmissionOutcome> = async {
                let journal = self
                    .journal
                    .as_ref()
                    .context("conversion quotes are disabled")?;
                let record = journal
                    .submission(id)
                    .await?
                    .ok_or_else(|| WsolFailure::rejected("conversion submission not found"))?;
                if record.owner_type != owner_type || record.owner_id != owner_id {
                    bail!(WsolFailure::rejected(
                        "conversion submission does not belong to this owner"
                    ));
                }
                let status = record
                    .validate_status_record(id)
                    .map_err(WsolFailure::integrity)?;
                scope.remember_record(&record);
                if let Some(signature) = &record.signature {
                    scope.remember_signature(signature);
                }
                if matches!(
                    status,
                    WsolState::Signed | WsolState::Submitted | WsolState::Unknown
                ) {
                    if let Some(signature) = record.signature.as_ref() {
                        scope.stage(Stage::StatusLookup);
                        let reader = self.source.status_reader(&record.wallet_id, signature)?;
                        if let Ok(Ok(Some(observation))) =
                            tokio::time::timeout(WSOL_STATUS_TIMEOUT, reader.lookup()).await
                        {
                            if let Ok(status) = observation.outcome_for(signature) {
                                let error = observation.chain.error.as_ref().map(|error| {
                                    if observation.chain.commitment == ChainCommitment::Finalized {
                                        error.clone()
                                    } else {
                                        format!(
                                            "non-final {} observation: {error}",
                                            observation.chain.commitment.as_str()
                                        )
                                    }
                                });
                                scope.stage(Stage::StatusObservation);
                                journal
                                    .record_observation(
                                        &record,
                                        status,
                                        error.as_deref(),
                                        current_time_ms()?,
                                    )
                                    .await?;
                            }
                        }
                    }
                }
                scope.stage(Stage::StatusFinalRead);
                let mut current = journal
                    .submission(id)
                    .await?
                    .context("conversion record missing")?;
                record
                    .validate_status_readback(&current)
                    .map_err(WsolFailure::integrity)?;
                current.error = current
                    .error
                    .map(|error| crate::sanitize::redact_urls(&error));
                Ok(WsolSubmissionOutcome {
                    record: current,
                    duplicate: false,
                })
            }
            .await;
            result.map_err(|error| scope.classify(error))
        })
    }
}

#[cfg(test)]
pub(crate) mod quote_boundary_tests;
#[cfg(test)]
pub(crate) mod status_boundary_tests;
#[cfg(test)]
pub(crate) mod tests;
