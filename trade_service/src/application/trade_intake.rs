use crate::{
    application::{
        ports::{
            address_parser::TradeAddressParser,
            ordinary_journal::OrdinaryTradeJournal,
            trade_admission::{MemoryTradeReservations, TradeRecordIndex},
            trade_intake::TradeIntakeRequest,
            wallet_identity::{LoadedWalletDirectory, LoadedWalletIdentity},
        },
        trade_admission::{self, AdmissionDecision, AdmissionError, AdmittedTrade},
        trade_command::{CommandContext, TradeCommand},
        trade_execution::ensure_execution_allowed,
        trade_record::TradeRecordV1,
        trade_settings::{TradeCommandDefaults, TradeValidationSettings},
        trade_validation::TradeValidator,
    },
    domain::{execution_policy::BroadcastPolicy, trade::TradeStatus},
};
use tracing::{info, warn};

// Preserve existing RUST_LOG filters while moving the business orchestration.
const LOG_TARGET: &str = concat!(env!("CARGO_CRATE_NAME"), "::api::handlers::trades");

#[derive(Debug, Clone, Copy)]
pub(crate) struct TradeIntakeSettings {
    pub validation: TradeValidationSettings,
    pub command: TradeCommandDefaults,
    pub broadcast: BroadcastPolicy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntakeFailureKind {
    UnknownWallet,
    Forbidden,
    Internal,
    Conflict,
    Unavailable,
}
#[derive(Debug)]
pub(crate) struct IntakeFailure {
    pub kind: IntakeFailureKind,
    pub message: String,
}
impl IntakeFailure {
    fn new(kind: IntakeFailureKind, message: impl ToString) -> Self {
        Self {
            kind,
            message: message.to_string(),
        }
    }
}
pub(crate) enum IntakeDecision {
    Rejected,
    Duplicate,
    Created(AdmittedTrade),
}
pub(crate) struct IntakeOutcome {
    pub wallet_id: String,
    pub record: TradeRecordV1,
    pub decision: IntakeDecision,
    pub wallet_identity: Box<dyn LoadedWalletIdentity>,
}

pub(crate) struct TradeIntakeService<'a, J, I, R> {
    journal: &'a J,
    index: &'a I,
    reservations: &'a R,
    wallets: &'a dyn LoadedWalletDirectory,
    parser: &'a (dyn TradeAddressParser + Sync),
    settings: TradeIntakeSettings,
    new_request_id: fn() -> String,
    now_ms: fn() -> u128,
}
impl<'a, J: OrdinaryTradeJournal, I: TradeRecordIndex, R: MemoryTradeReservations>
    TradeIntakeService<'a, J, I, R>
{
    pub(crate) fn new(
        journal: &'a J,
        index: &'a I,
        reservations: &'a R,
        wallets: &'a dyn LoadedWalletDirectory,
        parser: &'a (dyn TradeAddressParser + Sync),
        settings: TradeIntakeSettings,
        new_request_id: fn() -> String,
        now_ms: fn() -> u128,
    ) -> Self {
        Self {
            journal,
            index,
            reservations,
            wallets,
            parser,
            settings,
            new_request_id,
            now_ms,
        }
    }

    pub(crate) async fn accept<Q: TradeIntakeRequest>(
        &self,
        request: &mut Q,
        forced_wallet_id: Option<String>,
    ) -> Result<IntakeOutcome, IntakeFailure> {
        let wallet_id = forced_wallet_id
            .unwrap_or_else(|| self.wallets.resolve_id(request.requested_wallet_id()));
        request.bind_wallet(wallet_id.clone());
        let wallet = self.wallets.lookup(&wallet_id).ok_or_else(|| {
            IntakeFailure::new(
                IntakeFailureKind::UnknownWallet,
                format!("wallet_id {wallet_id} not found"),
            )
        })?;
        let wallet_pubkey = wallet.pubkey().to_string();
        let input = request.validation_input();
        let validation =
            TradeValidator::new(self.parser).validate(&input, &self.settings.validation);
        let normalized = match validation {
            Ok(normalized) => normalized,
            Err(error) => {
                let payload = request.rejected_payload(&wallet_id, &wallet_pubkey);
                let record = trade_admission::reject(
                    self.index,
                    &payload,
                    (self.new_request_id)(),
                    error.to_string(),
                    self.now_ms,
                );
                warn!(target: LOG_TARGET, request_id = %record.request_id, wallet_id = %record.wallet_id,
                    idempotency_key = %input.idempotency_key, side = ?input.side, pool = ?input.pool,
                    mint = %input.mint, status = ?record.status, error = ?record.error,"trade rejected");
                if let Err(persist_error) =
                    trade_admission::mirror_rejection(self.journal, &payload, &record).await
                {
                    warn!(target: LOG_TARGET, error = %persist_error, request_id = %record.request_id,"failed to persist rejected trade");
                }
                drop(payload);
                return Ok(IntakeOutcome {
                    wallet_id,
                    record,
                    decision: IntakeDecision::Rejected,
                    wallet_identity: wallet,
                });
            }
        };
        ensure_execution_allowed(
            &normalized,
            self.settings.command.default_simulate,
            self.settings.broadcast,
        )
        .map_err(|error| IntakeFailure::new(IntakeFailureKind::Forbidden, error))?;
        let command = TradeCommand::from_validated(
            CommandContext {
                wallet_id: wallet_id.clone(),
                idempotency_key: input.idempotency_key,
                side: input.side,
                pool: input.pool,
                default_simulate: self.settings.command.default_simulate,
                pumpapi_priority_fee_sol: input
                    .tip_sol
                    .unwrap_or(self.settings.command.pumpapi_priority_fee_sol),
            },
            &normalized,
        )
        .map_err(|error| IntakeFailure::new(IntakeFailureKind::Internal, error))?;
        let payload = request.validated_payload(&wallet_id, &wallet_pubkey, &normalized, &command);
        let decision = trade_admission::accept(
            self.journal,
            self.index,
            self.reservations,
            &payload,
            (self.new_request_id)(),
        )
        .await;
        drop(payload);
        let (record, execution) = match decision {
            Ok(AdmissionDecision::Duplicate(record)) => {
                return Ok(IntakeOutcome {
                    wallet_id,
                    record,
                    decision: IntakeDecision::Duplicate,
                    wallet_identity: wallet,
                })
            }
            Ok(AdmissionDecision::Created {
                record,
                shadow_error,
                execution,
            }) => {
                if let Some(error) = shadow_error {
                    warn!(target: LOG_TARGET, error = %error, request_id = %record.request_id,"shadow trade persistence failed");
                }
                (record, execution)
            }
            Err(AdmissionError::Internal(error)) => {
                return Err(IntakeFailure::new(IntakeFailureKind::Internal, error))
            }
            Err(AdmissionError::Conflict(message)) => {
                return Err(IntakeFailure::new(IntakeFailureKind::Conflict, message))
            }
            Err(AdmissionError::PersistenceUnavailable(error)) => {
                warn!(target: LOG_TARGET, error = %error,"durable idempotency reservation failed; trade not started");
                return Err(IntakeFailure::new(
                    IntakeFailureKind::Unavailable,
                    "trade persistence unavailable; trade was not started",
                ));
            }
        };
        info!(target: LOG_TARGET, request_id = %record.request_id, wallet_id = %record.wallet_id,
            idempotency_key = %input.idempotency_key, side = ?input.side, pool = ?input.pool,
            mint = %input.mint, status = ?TradeStatus::Accepted,"trade accepted");
        Ok(IntakeOutcome {
            wallet_id,
            record,
            decision: IntakeDecision::Created(execution),
            wallet_identity: wallet,
        })
    }
}

#[cfg(test)]
mod tests;
