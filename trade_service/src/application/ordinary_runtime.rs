use tracing::{error, info};

use crate::{
    application::{
        ports::{
            blockhash::CachedBlockhashRead,
            ordinary_claims::{ExecutionClaim, ExecutionOwner, OrdinaryExecutionClaimer},
            ordinary_execution::{OrdinaryExecutionBackend, SdkExecutionContext},
            ordinary_journal::{OrdinaryTradeJournal, TradeJournalMode},
            ordinary_runtime::{OrdinaryExecutionStorage, TradeExecutionIndex},
            ordinary_sol_buy::{
                JournaledSolBuyOutcome, OrdinarySolBuyBackend, SolBuyExecutionOutcome,
            },
            pool_queries::ParamCacheWriter,
        },
        trade_admission::AdmittedTrade,
        trade_command::TradeCommand,
        trade_execution::{classify_result, describe_trade, TradeExecutionResult as ExecuteResult},
        trade_record::TradeRecordV1 as TradeRecord,
        trade_settings::OrdinaryExecutionSettings,
    },
    domain::{
        route::PoolRouteHint,
        trade::{ExecutionMethod, TradeStatus},
    },
};

const LOG_TARGET: &str = concat!(env!("CARGO_CRATE_NAME"), "::executor");

mod prepared;

#[cfg(test)]
mod tests;

/// Borrows the original resources; this service neither spawns tasks nor owns a signer.
pub(crate) struct OrdinaryExecutionService<'a, J, I> {
    journal: &'a J,
    index: &'a I,
    blockhash: &'a dyn CachedBlockhashRead,
    execution: &'a dyn OrdinaryExecutionBackend,
    sol_buy: &'a dyn OrdinarySolBuyBackend,
    params: &'a dyn ParamCacheWriter,
    settings: OrdinaryExecutionSettings,
}

impl<'a, J: OrdinaryExecutionStorage, I: TradeExecutionIndex> OrdinaryExecutionService<'a, J, I> {
    pub(crate) fn new(
        journal: &'a J,
        index: &'a I,
        blockhash: &'a dyn CachedBlockhashRead,
        execution: &'a dyn OrdinaryExecutionBackend,
        sol_buy: &'a dyn OrdinarySolBuyBackend,
        params: &'a dyn ParamCacheWriter,
        settings: OrdinaryExecutionSettings,
    ) -> Self {
        Self {
            journal,
            index,
            blockhash,
            execution,
            sol_buy,
            params,
            settings,
        }
    }

    pub(crate) async fn execute<C: OrdinaryExecutionClaimer>(
        &self,
        execution: &AdmittedTrade,
        claims: &C,
    ) {
        let reservation = execution.reservation();
        let request_id = reservation.request_id().to_owned();
        let command = execution.command();
        let mut owner = if command.aggregator_sol && !command.execution.mode.simulate() {
            match claims.claim(execution).await {
                Ok(ExecutionClaim::Acquired(owner)) => Some(owner),
                Ok(ExecutionClaim::AlreadyHeld) => return,
                Err(error) => {
                    error!(target: LOG_TARGET, request_id = %request_id, error = %crate::sanitize::redact_urls(&error.to_string()), "ordinary execution claim unavailable or inconsistent; execution stopped without retry");
                    return;
                }
            }
        } else {
            None
        };
        if !self.prepared_parent_allows_execution(execution).await {
            return;
        }
        if let Err(error) = self.settings.broadcast.ensure_trade_allowed(
            command.execution.method == ExecutionMethod::Sdk && !command.aggregator_sol,
            command.execution.mode.simulate(),
        ) {
            let record = self.index.update_status(
                &request_id,
                TradeStatus::Rejected,
                None,
                Some(error.to_string()),
            );
            persist_update(self.journal, record.as_ref()).await;
            return;
        }
        if self.try_prepared_sdk(execution, claims).await {
            return;
        }
        if matches!(
            crate::application::ordinary_claims::operation_for_claim(execution, reservation.mode()),
            Ok(crate::application::ordinary_claims::ClaimOperation::SdkTrade)
        ) {
            match claims.claim(execution).await {
                Ok(ExecutionClaim::Acquired(acquired)) => owner = Some(acquired),
                Ok(ExecutionClaim::AlreadyHeld) => return,
                Err(error) => {
                    error!(target: LOG_TARGET, request_id = %request_id, error = %crate::sanitize::redact_urls(&error.to_string()), "original SDK execution claim unavailable; no execution or retry");
                    return;
                }
            }
        }
        let idempotency_key = reservation.idempotency_key().to_owned();
        let wallet_id = reservation.wallet_id().to_owned();
        if !self.begin_execution(execution).await {
            return;
        }

        if owner.as_ref().is_some_and(|owner| {
            owner.operation() == crate::application::ordinary_claims::ClaimOperation::SdkTrade
        }) {
            self.execute_original_sdk(execution, owner.take().expect("checked original SDK owner"))
                .await;
            return;
        }
        if command.aggregator_sol {
            let Some(wallet) = self.sol_buy.resolve(&wallet_id) else {
                self.fail_missing_wallet(&request_id, &wallet_id).await;
                return;
            };
            let simulate = command.execution.mode.simulate();
            let result = wallet.buy(execution, owner.as_ref()).await;
            match result {
                Ok(SolBuyExecutionOutcome::Legacy(result)) => {
                    self.finish_trade(&request_id, &idempotency_key, command, Ok(result), simulate)
                        .await
                }
                Ok(SolBuyExecutionOutcome::Journaled(result)) => {
                    self.finish_journaled(&request_id, &idempotency_key, command, result)
                }
                Err(error) => {
                    self.finish_trade(&request_id, &idempotency_key, command, Err(error), simulate)
                        .await
                }
            }
            return;
        }
        let Some(wallet) = self.execution.resolve(&wallet_id) else {
            self.fail_missing_wallet(&request_id, &wallet_id).await;
            return;
        };
        if command.execution.method == ExecutionMethod::Pumpapi {
            let record = self
                .index
                .update_status(&request_id, TradeStatus::Submitting, None, None);
            if !persist_before_execution(self.journal, record.as_ref()).await {
                return;
            }
            let simulate = command.execution.mode.simulate();
            let result = wallet
                .pumpapi(command, self.settings.wait_tx_confirmed)
                .await;
            self.finish_trade(&request_id, &idempotency_key, command, result, simulate)
                .await;
            return;
        }
        let Some(blockhash) = self
            .blockhash
            .fresh_hash(self.settings.blockhash_max_age_ms)
        else {
            let record = self.index.update_status(
                &request_id,
                TradeStatus::Rejected,
                None,
                Some("blockhash cache is empty or stale".to_string()),
            );
            persist_update(self.journal, record.as_ref()).await;
            return;
        };
        let submitting = self
            .index
            .update_status(&request_id, TradeStatus::Submitting, None, None);
        if !persist_before_execution(self.journal, submitting.as_ref()).await {
            return;
        }
        let simulate = command.execution.mode.simulate();
        let result = wallet
            .sdk(
                command,
                SdkExecutionContext {
                    blockhash,
                    wait_tx_confirmed: self.settings.wait_tx_confirmed,
                    mev_submit_verify_ms: self.settings.mev_submit_verify_ms,
                },
            )
            .await;
        self.finish_trade(&request_id, &idempotency_key, command, result, simulate)
            .await;
    }

    async fn begin_execution(&self, execution: &AdmittedTrade) -> bool {
        let reservation = execution.reservation();
        let request_id = reservation.request_id().to_owned();
        let command = execution.command();
        let wallet_id = reservation.wallet_id().to_owned();
        let idempotency_key = reservation.idempotency_key().to_owned();
        let detail = describe_trade(
            command.amount.side(),
            command.amount.buy_units(),
            command.amount.sell_percent(),
        );
        info!(
            target: LOG_TARGET,
            request_id = %request_id,
            wallet_id = %wallet_id,
            idempotency_key = %idempotency_key,
            request_hash = %reservation.request_hash(),
            reservation_mode = ?reservation.mode(),
            side = ?command.amount.side(),
            pool = ?command.route.pool,
            mint = %command.route.mint,
            %detail,
            status = "building_tx",
            "trade execution started"
        );
        let building = self
            .index
            .update_status(&request_id, TradeStatus::BuildingTx, None, None);
        persist_before_execution(self.journal, building.as_ref()).await
    }

    async fn execute_original_sdk(&self, execution: &AdmittedTrade, owner: ExecutionOwner) {
        let anchor = execution.reservation();
        let command = execution.command();
        let Some(journal) = self.journal.sdk_journal() else {
            self.finish_journaled(
                anchor.request_id(),
                anchor.idempotency_key(),
                command,
                JournaledSolBuyOutcome {
                    record: None,
                    signature: None,
                    error: Some(
                        "original SDK durable journal is unavailable; do not resubmit".into(),
                    ),
                },
            );
            return;
        };
        let Some(wallet) = self.execution.resolve(anchor.wallet_id()) else {
            let outcome = crate::application::ordinary_sdk_submission::reject_preparation(
                &journal,
                execution,
                owner,
                anyhow::anyhow!("wallet_id {} not found", anchor.wallet_id()),
            )
            .await;
            self.finish_journaled(
                anchor.request_id(),
                anchor.idempotency_key(),
                command,
                outcome,
            );
            return;
        };
        let Some(blockhash) = self
            .blockhash
            .fresh_hash(self.settings.blockhash_max_age_ms)
        else {
            let outcome = crate::application::ordinary_sdk_submission::reject_context(
                &journal,
                execution,
                owner,
                "blockhash cache is empty or stale",
            )
            .await;
            self.finish_journaled(
                anchor.request_id(),
                anchor.idempotency_key(),
                command,
                outcome,
            );
            return;
        };
        let outcome = wallet
            .sdk_journaled(
                execution,
                owner,
                SdkExecutionContext {
                    blockhash,
                    wait_tx_confirmed: self.settings.wait_tx_confirmed,
                    mev_submit_verify_ms: self.settings.mev_submit_verify_ms,
                },
            )
            .await;
        self.finish_journaled(
            anchor.request_id(),
            anchor.idempotency_key(),
            command,
            outcome,
        );
    }

    async fn fail_missing_wallet(&self, request_id: &str, wallet_id: &str) {
        let record = self.index.update_status(
            request_id,
            TradeStatus::Failed,
            None,
            Some(format!("wallet_id {wallet_id} not found")),
        );
        let _ = persist_before_execution(self.journal, record.as_ref()).await;
    }

    async fn finish_trade(
        &self,
        request_id: &str,
        idempotency_key: &str,
        command: &TradeCommand,
        result: anyhow::Result<ExecuteResult>,
        simulate: bool,
    ) {
        let (status, signature, error_text) = map_status(result, simulate);
        let record =
            self.index
                .update_status(request_id, status, signature.clone(), error_text.clone());
        persist_update(self.journal, record.as_ref()).await;
        self.after_trade(idempotency_key, command, record);
    }

    fn finish_journaled(
        &self,
        request_id: &str,
        key: &str,
        command: &TradeCommand,
        outcome: JournaledSolBuyOutcome,
    ) {
        let record = match outcome.record {
            Some(record) => {
                self.index.insert_record(&record);
                Some(record)
            }
            None => self.index.update_status(
                request_id,
                TradeStatus::Unknown,
                outcome.signature,
                outcome.error,
            ),
        };
        // Durable journal owns the parent write. Never mirror a stale executor snapshot over it.
        self.after_trade(key, command, record);
    }

    fn after_trade(
        &self,
        idempotency_key: &str,
        command: &TradeCommand,
        record: Option<TradeRecord>,
    ) {
        self.params.record(&PoolRouteHint {
            pool: command.route.pool,
            mint: command.route.mint,
            quote_mint: command.route.quote_mint,
            pool_id: command.route.pool_id,
        });
        if let Some(record) = record {
            info!(
                target: LOG_TARGET,
                request_id = %record.request_id,
                wallet_id = %record.wallet_id,
                idempotency_key = %idempotency_key,
                side = ?record.side,
                pool = ?record.pool,
                mint = %record.mint,
                execution_method = ?record.execution_method,
                status = ?record.status,
                signature = ?record.signature,
                error = ?record.error,
                "trade execution finished"
            );
        }
    }
}

pub(crate) async fn persist_update(
    journal: &impl OrdinaryTradeJournal,
    record: Option<&TradeRecord>,
) {
    let Some(record) = record else {
        return;
    };
    if let Err(error) = journal.update_record(record).await {
        error!(target: LOG_TARGET, request_id = %record.request_id, error = %error, "failed to persist trade status; transaction will not be retried automatically");
    }
}

pub(crate) async fn persist_before_execution(
    journal: &impl OrdinaryTradeJournal,
    record: Option<&TradeRecord>,
) -> bool {
    let Some(record) = record else {
        return false;
    };
    match journal.update_record(record).await {
        Ok(()) => true,
        Err(error) if journal.mode() == TradeJournalMode::Durable => {
            error!(target: LOG_TARGET, request_id = %record.request_id, error = %error, "durable trade status could not be saved; trade execution stopped before broadcast");
            false
        }
        Err(error) => {
            error!(target: LOG_TARGET, request_id = %record.request_id, error = %error, "shadow trade status could not be saved; original in-memory execution continues");
            true
        }
    }
}

pub(crate) fn map_status(
    result: anyhow::Result<ExecuteResult>,
    simulate: bool,
) -> (TradeStatus, Option<String>, Option<String>) {
    if let Err(error) = &result {
        error!(target: LOG_TARGET, error = %error, "trade execution call failed");
    }
    classify_result(result, simulate)
}
