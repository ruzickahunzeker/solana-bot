use anyhow::{bail, Context, Result};
use std::{
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    application::{
        ports::wsol_conversion::{
            WsolArtifact, WsolExecutionWallet, WsolJournal, WsolRuntime, WsolWalletMetadata,
        },
        wsol_failure::{WsolFailure, WsolFailureScope, WsolFailureStage as Stage},
        wsol_submission_types::{
            WsolQuoteSnapshot, WsolSignedOperation, WsolSimulationOutcome, WsolSubmissionInput,
            WsolSubmissionOutcome, WsolSubmissionRecord,
        },
    },
    domain::{
        execution_policy::BroadcastPolicy,
        wsol_progress::{WsolBroadcastOutcome, WsolState},
    },
};

#[derive(Clone, Copy)]
pub(crate) struct WsolSubmitPolicy {
    pub broadcast: BroadcastPolicy,
    pub live_enabled: bool,
}

pub(crate) type WsolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
pub(crate) trait WsolSubmissions: Send + Sync {
    fn submit(
        &self,
        input: WsolSubmissionInput,
        policy: WsolSubmitPolicy,
    ) -> WsolFuture<'_, WsolSubmissionOutcome>;
    fn simulate(&self, input: WsolSubmissionInput) -> WsolFuture<'_, WsolSimulationOutcome>;
}

pub(crate) struct WsolSubmissionService<R, J> {
    runtime: R,
    journal: Option<J>,
}
impl<R, J> WsolSubmissionService<R, J> {
    pub(crate) fn new(runtime: R, journal: Option<J>) -> Self {
        Self { runtime, journal }
    }
}

pub(crate) fn current_time_ms() -> Result<u64> {
    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

impl<R: WsolRuntime, J: WsolJournal> WsolSubmissionService<R, J> {
    async fn wallet(&self, quote: &WsolQuoteSnapshot) -> Result<R::Wallet> {
        let metadata = self.runtime.metadata(&quote.wallet_id).await?;
        self.wallet_from_metadata(quote, metadata)
    }

    fn wallet_from_metadata(
        &self,
        quote: &WsolQuoteSnapshot,
        metadata: WsolWalletMetadata,
    ) -> Result<R::Wallet> {
        if !metadata.enabled {
            bail!(WsolFailure::rejected("wallet is disabled"));
        }
        if metadata.wallet_id != quote.wallet_id {
            bail!(WsolFailure::integrity(
                "wallet metadata does not match requested wallet"
            ));
        }
        quote
            .validate_binding(
                &metadata.wallet_id,
                &metadata.pubkey,
                &metadata.owner_type,
                &metadata.owner_id,
                current_time_ms()?,
            )
            .map_err(WsolFailure::rejected)?;
        let wallet = self
            .runtime
            .resolve(&quote.wallet_id)
            .context("wallet is not loaded")?;
        if wallet.pubkey().to_string() != metadata.pubkey {
            bail!(WsolFailure::integrity(
                "loaded wallet signer does not match quote"
            ));
        }
        Ok(wallet)
    }

    async fn failed(
        &self,
        journal: &J,
        expected: &WsolSubmissionRecord,
        error: &str,
    ) -> Result<WsolSubmissionOutcome> {
        let error = crate::sanitize::redact_urls(error);
        let failed = journal
            .fail_before_sign(expected, &error, current_time_ms()?)
            .await?;
        expected
            .validate_readback(&failed)
            .map_err(WsolFailure::integrity)?;
        validate_unsigned_failure(&failed, &error).map_err(WsolFailure::integrity)?;
        let current = journal
            .by_id(&expected.submission_id)
            .await?
            .context("failed conversion record missing")?;
        failed
            .validate_readback(&current)
            .map_err(WsolFailure::integrity)?;
        validate_unsigned_failure(&current, &error).map_err(WsolFailure::integrity)?;
        Ok(WsolSubmissionOutcome {
            record: current,
            duplicate: false,
        })
    }
}

impl<R: WsolRuntime, J: WsolJournal> WsolSubmissions for WsolSubmissionService<R, J> {
    fn submit(
        &self,
        input: WsolSubmissionInput,
        policy: WsolSubmitPolicy,
    ) -> WsolFuture<'_, WsolSubmissionOutcome> {
        Box::pin(async move {
            let mut scope = WsolFailureScope::new();
            let result: Result<WsolSubmissionOutcome> = async {
            policy.broadcast.ensure_allowed()?;
            if input.simulate || !input.confirmed || !policy.live_enabled {
                bail!("live WSOL conversion is disabled or request is simulation");
            }
            let journal = self
                .journal
                .as_ref()
                .context("conversion quotes are disabled")?;
            scope.stage(Stage::QuoteLookup);
            let quote = journal
                .quote(&input.quote_id)
                .await?
                .ok_or_else(|| WsolFailure::rejected("conversion quote not found"))?;
            scope.stage(Stage::Reservation);
            let (record, created) = journal.reserve(&input, &quote, current_time_ms()?).await?;
            record.validate_request(&input, &quote).map_err(WsolFailure::integrity)?;
            scope.remember_record(&record);
            if !created {
                return Ok(WsolSubmissionOutcome {
                    record,
                    duplicate: true,
                });
            }
            record.validate_unsigned_reservation(&record).map_err(WsolFailure::integrity)?;
            // Resolve once after reservation; all remaining stages retain this original wallet.
            let (wallet, signed) = match async {
                let wallet = self.wallet(&quote).await?;
                let prepared = wallet.prepare(&quote).await?;
                let signed = wallet.sign(prepared, &quote)?;
                Ok::<_, anyhow::Error>((wallet, signed))
            }
            .await
            {
                Ok(result) => result,
                Err(error) => {
                    scope.stage(Stage::UnsignedFailure);
                    return self.failed(journal, &record, &error.to_string()).await;
                },
            };
            let operation = WsolSignedOperation {
                record: &record,
                signature: signed.signature(),
                signed_bytes: signed.signed_bytes(),
            };
            scope.stage(Stage::SignedPersistence);
            let persisted = journal.record_signed(operation, current_time_ms()?).await?;
            operation.validate(&persisted).map_err(WsolFailure::integrity)?;
            if persisted.status != "signed" || persisted.error.is_some() {
                bail!(WsolFailure::integrity("WSOL signed evidence receipt is not a clean signed record"));
            }
            scope.remember_signature(signed.signature());
            let simulation =
                tokio::time::timeout(Duration::from_secs(8), wallet.simulate_signed(&signed)).await;
            if !matches!(simulation, Ok(Ok(true))) {
                scope.stage(Stage::SimulationReceipt);
                journal.record_simulation_failure(
                    operation,
                    "pre-submit simulation failed or timed out; signed evidence retained; do not resubmit with a new key",
                    current_time_ms()?,
                ).await?;
                scope.stage(Stage::FinalRead);
                let current = journal
                    .by_key(&input.idempotency_key)
                    .await?
                    .context("conversion record missing after simulation failure")?;
                operation.validate(&current).map_err(WsolFailure::integrity)?;
                return Ok(WsolSubmissionOutcome {
                    record: current,
                    duplicate: false,
                });
            }
            scope.stage(Stage::PreBroadcastRead);
            policy.broadcast.ensure_allowed()?;
            let current = journal
                .by_id(&record.submission_id)
                .await?
                .context("conversion record missing before broadcast")?;
            operation.validate(&current).map_err(WsolFailure::integrity)?;
            match WsolState::parse(&current.status).map_err(WsolFailure::integrity)? {
                WsolState::Signed => {}
                WsolState::Unknown
                | WsolState::Submitted
                | WsolState::Confirmed
                | WsolState::Failed => {
                    return Ok(WsolSubmissionOutcome {
                        record: current,
                        duplicate: false,
                    });
                }
                WsolState::Reserved => {
                    bail!(WsolFailure::integrity("WSOL signed record reverted to an unsigned reservation"))
                }
            }
            let sent =
                tokio::time::timeout(Duration::from_secs(12), wallet.broadcast(&signed)).await;
            let (status, error) = match sent {
                Ok(Ok(signature)) if signature == signed.signature() => {
                    (WsolBroadcastOutcome::Submitted, None)
                }
                Ok(Ok(_)) => (
                    WsolBroadcastOutcome::Unknown,
                    Some("RPC returned a different signature; reconcile original signature".into()),
                ),
                Ok(Err(error)) => (
                    WsolBroadcastOutcome::Unknown,
                    Some(format!(
                        "broadcast result uncertain: {}",
                        crate::sanitize::redact_urls(&error.to_string())
                    )),
                ),
                Err(_) => (
                    WsolBroadcastOutcome::Unknown,
                    Some("broadcast timed out; reconcile original signature".into()),
                ),
            };
            scope.stage(Stage::BroadcastReceipt);
            journal
                .record_broadcast(operation, status, error.as_deref(), current_time_ms()?)
                .await?;
            scope.stage(Stage::FinalRead);
            let current = journal
                .by_key(&input.idempotency_key)
                .await?
                .context("conversion record missing after submission")?;
            operation.validate(&current).map_err(WsolFailure::integrity)?;
            Ok(WsolSubmissionOutcome {
                record: current,
                duplicate: false,
            })
            }.await;
            result.map_err(|error| scope.classify(error))
        })
    }

    fn simulate(&self, input: WsolSubmissionInput) -> WsolFuture<'_, WsolSimulationOutcome> {
        Box::pin(async move {
            let mut scope = WsolFailureScope::new();
            scope.stage(Stage::UnsignedAdmission);
            let result: Result<WsolSimulationOutcome> = async {
                if !input.simulate {
                    bail!(WsolFailure::rejected(
                        "simulation endpoint requires simulate=true"
                    ));
                }
                let journal = self
                    .journal
                    .as_ref()
                    .context("conversion quotes are disabled")?;
                scope.stage(Stage::UnsignedQuote);
                let quote = journal
                    .quote(&input.quote_id)
                    .await?
                    .ok_or_else(|| WsolFailure::rejected("conversion quote not found"))?;
                scope.stage(Stage::UnsignedInput);
                input
                    .validate_quote(&quote)
                    .map_err(WsolFailure::rejected)?;
                scope.stage(Stage::UnsignedMetadata);
                let metadata = self.runtime.metadata(&quote.wallet_id).await?;
                scope.stage(Stage::UnsignedWallet);
                let wallet = self.wallet_from_metadata(&quote, metadata)?;
                scope.stage(Stage::UnsignedPrepare);
                let prepared = wallet.prepare(&quote).await?;
                scope.stage(Stage::UnsignedRpc);
                let units_consumed = wallet.simulate_unsigned(&prepared, &quote).await?;
                Ok(WsolSimulationOutcome {
                    quote_id: quote.quote_id,
                    network_fee_lamports: quote.estimated_network_fee_lamports,
                    units_consumed,
                })
            }
            .await;
            result.map_err(|error| scope.classify(error))
        })
    }
}

fn validate_unsigned_failure(record: &WsolSubmissionRecord, error: &str) -> Result<()> {
    if record.status != "failed"
        || record.signature.is_some()
        || record.signed_transaction.is_some()
        || record.error.as_deref() != Some(error)
    {
        bail!("WSOL unsigned failure receipt does not match the original failure");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
pub(crate) mod unsigned_boundary_tests;
