use crate::application::{
    ports::{
        ordinary_journal::{
            OrdinaryTradeJournal, SnapshotWrite, TradeJournalMode, TradeReservation,
        },
        trade_admission::{
            MemoryReservation, MemoryTradeReservations, RejectedAdmissionPayload, TradeRecordIndex,
            ValidatedAdmissionPayload,
        },
    },
    trade_command::TradeCommand,
    trade_record::TradeRecordV1,
};
use crate::domain::trade::{Address, TradeStatus};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(crate) enum AdmissionDecision {
    Created {
        record: TradeRecordV1,
        shadow_error: Option<anyhow::Error>,
        execution: AdmittedTrade,
    },
    Duplicate(TradeRecordV1),
}

/// Original reservation identity, not a second key, chain authorization or signed journal.
#[derive(Debug)]
pub(crate) struct OrdinaryReservation {
    request_id: String,
    wallet_id: String,
    wallet_pubkey: String,
    idempotency_key: String,
    request_hash: String,
    mode: TradeJournalMode,
}

impl OrdinaryReservation {
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
    pub(crate) fn wallet_id(&self) -> &str {
        &self.wallet_id
    }
    pub(crate) fn wallet_pubkey(&self) -> &str {
        &self.wallet_pubkey
    }
    pub(crate) fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
    pub(crate) fn request_hash(&self) -> &str {
        &self.request_hash
    }
    pub(crate) fn mode(&self) -> TradeJournalMode {
        self.mode
    }

    pub(crate) fn ensure_wallet(&self, wallet_id: &str, pubkey: Address) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.wallet_id == wallet_id && self.wallet_pubkey == pubkey.to_string(),
            "ordinary execution wallet identity changed after admission"
        );
        Ok(())
    }
}

/// Only a newly created reservation produces this owned execution input. No Clone or wire codec.
#[derive(Debug)]
pub(crate) struct AdmittedTrade {
    reservation: OrdinaryReservation,
    command: TradeCommand,
}

impl AdmittedTrade {
    fn created<P: ValidatedAdmissionPayload>(
        record: &TradeRecordV1,
        payload: &P,
        request_hash: String,
        mode: TradeJournalMode,
    ) -> anyhow::Result<Self> {
        let command = payload.execution_command();
        anyhow::ensure!(
            record.wallet_id == payload.wallet_id()
                && command.wallet_id == record.wallet_id
                && command.idempotency_key == payload.idempotency_key()
                && command.aggregator_sol == payload.aggregator_sol()
                && command.route.pool == record.pool
                && command.amount.side() == record.side
                && command.execution.method == record.execution_method
                && record.status == TradeStatus::Accepted
                && !record.duplicate
                && !record.request_id.is_empty()
                && !request_hash.is_empty(),
            "ordinary execution does not match the created reservation"
        );
        Ok(Self {
            reservation: OrdinaryReservation {
                request_id: record.request_id.clone(),
                wallet_id: record.wallet_id.clone(),
                wallet_pubkey: record.wallet_pubkey.clone(),
                idempotency_key: payload.idempotency_key().into(),
                request_hash,
                mode,
            },
            command,
        })
    }
    pub(crate) fn reservation(&self) -> &OrdinaryReservation {
        &self.reservation
    }
    pub(crate) fn command(&self) -> &TradeCommand {
        &self.command
    }

    #[cfg(test)]
    pub(crate) fn unreserved_fixture(record: &TradeRecordV1, command: TradeCommand) -> Self {
        Self {
            reservation: OrdinaryReservation {
                request_id: record.request_id.clone(),
                wallet_id: command.wallet_id.clone(),
                wallet_pubkey: record.wallet_pubkey.clone(),
                idempotency_key: command.idempotency_key.clone(),
                request_hash: "test-only-unreserved-fixture".into(),
                mode: TradeJournalMode::Memory,
            },
            command,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_command_for_test(&self, command: TradeCommand) -> Self {
        let anchor = self.reservation();
        Self {
            reservation: OrdinaryReservation {
                request_id: anchor.request_id.clone(),
                wallet_id: anchor.wallet_id.clone(),
                wallet_pubkey: anchor.wallet_pubkey.clone(),
                idempotency_key: anchor.idempotency_key.clone(),
                request_hash: anchor.request_hash.clone(),
                mode: anchor.mode,
            },
            command,
        }
    }
}

#[derive(Debug)]
pub(crate) enum AdmissionError {
    Internal(anyhow::Error),
    Conflict(&'static str),
    PersistenceUnavailable(anyhow::Error),
}

const CONFLICT: &str = "idempotency key was already used for a different trade";

/// Keep the legacy two index writes and the second timestamp's position unchanged.
pub(crate) fn reject<I, P>(
    index: &I,
    payload: &P,
    request_id: String,
    error: String,
    now_ms: impl FnOnce() -> u128,
) -> TradeRecordV1
where
    I: TradeRecordIndex,
    P: RejectedAdmissionPayload,
{
    let mut record = payload.new_record(request_id);
    index.insert_record(&record);
    record.status = TradeStatus::Rejected;
    record.error = Some(error);
    record.updated_at_ms = now_ms();
    index.insert_record(&record);
    record
}

/// Separate from record creation so the Handler can retain its original log timing.
pub(crate) async fn mirror_rejection<J, P>(
    journal: &J,
    payload: &P,
    record: &TradeRecordV1,
) -> anyhow::Result<()>
where
    J: OrdinaryTradeJournal,
    P: RejectedAdmissionPayload,
{
    if journal.mode() == TradeJournalMode::Memory {
        return Ok(());
    }
    journal.ensure_snapshot_storage(SnapshotWrite::Mirror)?;
    journal
        .mirror_rejection(record, payload.snapshot(record)?)
        .await
}

/// Only the already validated admission stage. No auth, wire DTO, clock or task spawning.
pub(crate) async fn accept<J, I, R, P>(
    journal: &J,
    index: &I,
    reservations: &R,
    payload: &P,
    request_id: String,
) -> Result<AdmissionDecision, AdmissionError>
where
    J: OrdinaryTradeJournal,
    I: TradeRecordIndex,
    R: MemoryTradeReservations,
    P: ValidatedAdmissionPayload,
{
    let mode = journal.mode();
    let mut request_hash = None;
    if mode != TradeJournalMode::Durable {
        let hash = payload.fingerprint().map_err(AdmissionError::Internal)?;
        match reservations.reserve_key(
            payload.wallet_id(),
            payload.idempotency_key(),
            &request_id,
            &hash,
            payload.aggregator_sol(),
        ) {
            MemoryReservation::Duplicate(id) => {
                let mut existing = index.get_record(&id).ok_or(AdmissionError::Conflict(
                    "duplicate idempotency key points to missing trade",
                ))?;
                existing.duplicate = true;
                return Ok(AdmissionDecision::Duplicate(existing));
            }
            MemoryReservation::Conflict => return Err(AdmissionError::Conflict(CONFLICT)),
            MemoryReservation::Reserved => {}
        }
        request_hash = Some(hash);
    }
    let record = payload.new_record(request_id);
    index.insert_record(&record);
    if mode == TradeJournalMode::Durable {
        let reservation = async {
            journal.ensure_snapshot_storage(SnapshotWrite::Reservation)?;
            let snapshot = payload.snapshot()?;
            let hash = snapshot.snapshot().hash().to_owned();
            let reservation = journal.reserve_record(&record, &snapshot).await?;
            Ok::<_, anyhow::Error>((reservation, hash))
        }
        .await;
        match reservation {
            Ok((TradeReservation::Created, hash)) => request_hash = Some(hash),
            Ok((TradeReservation::Duplicate(mut existing), _)) => {
                index.remove_record(&record.request_id);
                existing.duplicate = true;
                return Ok(AdmissionDecision::Duplicate(existing));
            }
            Ok((TradeReservation::Conflict, _)) => {
                index.remove_record(&record.request_id);
                return Err(AdmissionError::Conflict(CONFLICT));
            }
            Err(error) => {
                index.remove_record(&record.request_id);
                return Err(AdmissionError::PersistenceUnavailable(error));
            }
        }
        return Ok(AdmissionDecision::Created {
            execution: AdmittedTrade::created(
                &record,
                payload,
                request_hash.expect("created durable reservation hash"),
                mode,
            )
            .map_err(AdmissionError::Internal)?,
            record,
            shadow_error: None,
        });
    }
    let shadow_error = if mode == TradeJournalMode::Shadow {
        async {
            journal.ensure_snapshot_storage(SnapshotWrite::Mirror)?;
            journal.mirror_accepted(&record, payload.snapshot()?).await
        }
        .await
        .err()
    } else {
        None
    };
    Ok(AdmissionDecision::Created {
        execution: AdmittedTrade::created(
            &record,
            payload,
            request_hash.expect("created memory reservation hash"),
            mode,
        )
        .map_err(AdmissionError::Internal)?,
        record,
        shadow_error,
    })
}
