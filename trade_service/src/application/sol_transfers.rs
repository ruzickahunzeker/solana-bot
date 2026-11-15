use std::{future::Future, pin::Pin};

use crate::{
    application::{
        ports::{
            address_parser::TradeAddressParser,
            sol_transfer_execution::{
                SolTransferArtifact, SolTransferExecutionWallet, SolTransferRuntime,
                TransferWalletMetadata,
            },
            sol_transfer_journal::SolTransferJournal,
        },
        sol_transfer_policy::{self, TransferAmountInput},
        sol_transfer_record::{
            SolTransferJournalEntry, SolTransferRecordV1, SolTransferReservation,
        },
    },
    domain::{
        execution_policy::BroadcastPolicy,
        sol_transfer::{SolTransferAttemptState as State, SolTransferIntent},
    },
};

pub(crate) struct SolTransferInput<'a> {
    pub owner_type: &'a str,
    pub owner_id: &'a str,
    pub to_wallet_id: Option<&'a str>,
    pub to_address: Option<&'a str>,
    pub amount: TransferAmountInput,
    pub idempotency_key: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) struct SolTransferPolicy {
    pub broadcast: BroadcastPolicy,
    pub allow_system_wallet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferFailureKind {
    BadRequest,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
}

#[derive(Debug)]
pub(crate) struct SolTransferError {
    pub kind: TransferFailureKind,
    pub message: String,
}
impl SolTransferError {
    fn new(kind: TransferFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    fn storage(error: anyhow::Error) -> Self {
        tracing::error!(error=%crate::sanitize::redact_urls(&error.to_string()), "SOL transfer journal unavailable; do not resubmit");
        Self::new(
            TransferFailureKind::Unavailable,
            "SOL transfer journal unavailable; do not resubmit or use a new key",
        )
    }
}

#[derive(Debug)]
pub(crate) struct SolTransferOutcome {
    pub record: SolTransferRecordV1,
    pub duplicate: bool,
}

pub(crate) type TransferFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SolTransferOutcome, SolTransferError>> + Send + 'a>>;
pub(crate) trait SolTransfers: Send + Sync {
    fn submit<'a>(
        &'a self,
        wallet_id: &'a str,
        input: SolTransferInput<'a>,
        policy: SolTransferPolicy,
    ) -> TransferFuture<'a>;
}

pub(crate) struct SolTransferService<R, P> {
    runtime: R,
    parser: P,
}
impl<R, P> SolTransferService<R, P> {
    pub(crate) fn new(runtime: R, parser: P) -> Self {
        Self { runtime, parser }
    }
}

impl<R: SolTransferRuntime, P: TradeAddressParser + Send + Sync> SolTransfers
    for SolTransferService<R, P>
{
    fn submit<'a>(
        &'a self,
        wallet_id: &'a str,
        input: SolTransferInput<'a>,
        policy: SolTransferPolicy,
    ) -> TransferFuture<'a> {
        Box::pin(async move {
            use TransferFailureKind as Kind;
            policy
                .broadcast
                .ensure_allowed()
                .map_err(|error| SolTransferError::new(Kind::Forbidden, error.to_string()))?;
            sol_transfer_policy::validate_key(input.idempotency_key)
                .map_err(|error| SolTransferError::new(Kind::BadRequest, error.to_string()))?;
            let source = self
                .runtime
                .metadata(wallet_id)
                .await
                .map_err(|error| SolTransferError::new(Kind::NotFound, error.to_string()))?;
            if !owned(
                &source,
                input.owner_type,
                input.owner_id,
                policy.allow_system_wallet,
            ) {
                return Err(SolTransferError::new(
                    Kind::Forbidden,
                    "source wallet is disabled or does not belong to this owner",
                ));
            }
            let destination = match (input.to_wallet_id, input.to_address) {
                (Some(id), None) => {
                    let target = self.runtime.metadata(id).await.map_err(|error| {
                        SolTransferError::new(Kind::BadRequest, error.to_string())
                    })?;
                    if !owned(
                        &target,
                        input.owner_type,
                        input.owner_id,
                        policy.allow_system_wallet,
                    ) {
                        return Err(SolTransferError::new(
                            Kind::Forbidden,
                            "target wallet does not belong to this owner",
                        ));
                    }
                    target.pubkey
                }
                (None, Some(address)) => address.trim().to_owned(),
                _ => {
                    return Err(SolTransferError::new(
                        Kind::BadRequest,
                        "provide exactly one destination",
                    ))
                }
            };
            let destination = self.parser.parse(&destination).map_err(|_| {
                SolTransferError::new(
                    Kind::BadRequest,
                    "destination is not a valid Solana address",
                )
            })?;
            let amount = sol_transfer_policy::select_amount(input.amount)
                .map_err(|error| SolTransferError::new(Kind::BadRequest, error.to_string()))?;
            let source = self.parser.parse(&source.pubkey).map_err(|_| {
                SolTransferError::new(Kind::Unavailable, "source wallet public key is invalid")
            })?;
            let intent = SolTransferIntent {
                idempotency_key: input.idempotency_key.into(),
                owner_type: input.owner_type.into(),
                owner_id: input.owner_id.into(),
                wallet_id: wallet_id.into(),
                source,
                destination,
                amount,
            };
            let journal = self.runtime.journal().ok_or_else(|| {
                SolTransferError::new(
                    Kind::Unavailable,
                    "SOL transfer requires durable wallet storage",
                )
            })?;
            // Only the creator may execute; a replay never becomes a recovery send.
            let entry = match journal
                .reserve(&intent)
                .await
                .map_err(SolTransferError::storage)?
            {
                SolTransferReservation::Created(entry) => entry,
                SolTransferReservation::Existing(entry) => return replay(entry, &intent),
                SolTransferReservation::Conflict => {
                    let existing = journal
                        .lookup(&intent.idempotency_key)
                        .await
                        .map_err(SolTransferError::storage)?;
                    let other_owner = existing.is_some_and(|entry| {
                        entry.record.owner_type != intent.owner_type
                            || entry.record.owner_id != intent.owner_id
                    });
                    return Err(SolTransferError::new(
                        Kind::Conflict,
                        if other_owner {
                            "idempotency key belongs to another owner"
                        } else {
                            "idempotency key was used for another SOL transfer intent"
                        },
                    ));
                }
            };
            if entry.state != Some(State::Reserved)
                || entry.intent.as_ref() != Some(&intent)
                || !entry.record.matches(&intent)
            {
                return Err(SolTransferError::storage(anyhow::anyhow!(
                    "invalid SOL transfer reservation"
                )));
            }
            let id = entry.record.transfer_id;
            let Some(wallet) = self.runtime.resolve(wallet_id) else {
                journal
                    .fail_before_sign(&id, "source wallet is not loaded")
                    .await
                    .map_err(SolTransferError::storage)?;
                return Err(SolTransferError::new(
                    Kind::Unavailable,
                    "source wallet is not loaded",
                ));
            };
            if wallet.pubkey() != intent.source {
                journal
                    .fail_before_sign(&id, "source wallet public key mismatch")
                    .await
                    .map_err(SolTransferError::storage)?;
                return Err(SolTransferError::new(
                    Kind::Unavailable,
                    "source wallet public key mismatch",
                ));
            }
            let prepared = match wallet.prepare(intent.destination, intent.amount).await {
                Ok(prepared) => prepared,
                Err(error) => return fail(&journal, &intent, &id, error).await,
            };
            let snapshot = wallet.prepared_snapshot(&prepared).clone();
            if let Err(error) = sol_transfer_policy::validate_snapshot(&intent, &snapshot) {
                return fail(&journal, &intent, &id, error).await;
            }
            let signed = match wallet.sign(prepared) {
                Ok(signed) => signed,
                Err(error) => return fail(&journal, &intent, &id, error).await,
            };
            if signed.snapshot() != &snapshot {
                return fail(
                    &journal,
                    &intent,
                    &id,
                    anyhow::anyhow!("signed SOL transfer snapshot changed"),
                )
                .await;
            }
            journal
                .record_signed(&id, signed.snapshot(), signed.submission())
                .await
                .map_err(SolTransferError::storage)?;
            let (state,error) = match wallet.broadcast(&signed).await {
                Ok(signature) if signature == signed.submission().signature => (State::Submitted,None),
                Ok(_) => (State::Unknown,Some("RPC returned a different signature; reconcile original SOL transfer signature".into())),
                Err(error) => (State::Unknown,Some(crate::sanitize::redact_urls(&error.to_string()))),
            };
            journal
                .record_broadcast(&id, state, error.as_deref())
                .await
                .map_err(SolTransferError::storage)?;
            if state == State::Submitted && wallet.confirm(&signed).await {
                journal
                    .record_confirmed(&id)
                    .await
                    .map_err(SolTransferError::storage)?;
            }
            result(&journal, &intent, &id).await
        })
    }
}

fn owned(
    wallet: &TransferWalletMetadata,
    owner_type: &str,
    owner_id: &str,
    allow_system: bool,
) -> bool {
    wallet.enabled
        && ((wallet.owner_type == owner_type && wallet.owner_id == owner_id)
            || (allow_system && wallet.owner_type == "system" && wallet.owner_id == "global"))
}

fn replay(
    entry: SolTransferJournalEntry,
    intent: &SolTransferIntent,
) -> Result<SolTransferOutcome, SolTransferError> {
    if !entry.record.matches(intent)
        || !entry
            .intent
            .as_ref()
            .map_or(true, |stored| stored == intent)
    {
        return Err(SolTransferError::new(
            TransferFailureKind::Conflict,
            "idempotency key was used for another SOL transfer intent",
        ));
    }
    if entry.state.is_none()
        && !(matches!(entry.record.status.as_str(), "submitted" | "confirmed")
            && entry
                .record
                .signature
                .as_ref()
                .is_some_and(|sig| !sig.is_empty()))
    {
        return Err(SolTransferError::new(
            TransferFailureKind::Unavailable,
            "legacy SOL transfer outcome cannot be verified; do not resubmit or use a new key",
        ));
    }
    Ok(SolTransferOutcome {
        record: entry.record,
        duplicate: true,
    })
}

async fn fail<J: SolTransferJournal>(
    journal: &J,
    intent: &SolTransferIntent,
    id: &str,
    error: anyhow::Error,
) -> Result<SolTransferOutcome, SolTransferError> {
    journal
        .fail_before_sign(id, &crate::sanitize::redact_urls(&error.to_string()))
        .await
        .map_err(SolTransferError::storage)?;
    result(journal, intent, id).await
}

async fn result<J: SolTransferJournal>(
    journal: &J,
    intent: &SolTransferIntent,
    id: &str,
) -> Result<SolTransferOutcome, SolTransferError> {
    let entry = journal
        .lookup(&intent.idempotency_key)
        .await
        .map_err(SolTransferError::storage)?
        .ok_or_else(|| SolTransferError::storage(anyhow::anyhow!("SOL transfer disappeared")))?;
    if entry.record.transfer_id != id || !entry.record.matches(intent) {
        return Err(SolTransferError::storage(anyhow::anyhow!(
            "SOL transfer identity changed"
        )));
    }
    Ok(SolTransferOutcome {
        record: entry.record,
        duplicate: false,
    })
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
