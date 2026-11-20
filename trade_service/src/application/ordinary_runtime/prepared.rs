use super::*;

impl<J: OrdinaryExecutionStorage, I: TradeExecutionIndex> OrdinaryExecutionService<'_, J, I> {
    /// False is only the pre-owner, explicitly unsupported route back to the legacy path.
    pub(super) async fn try_prepared_sdk<C: OrdinaryExecutionClaimer>(
        &self,
        execution: &AdmittedTrade,
        claims: &C,
    ) -> bool {
        if !self.execution.supports_prepared_sdk()
            || !crate::application::ordinary_claims::pump_prepared_scope(
                execution,
                self.journal.mode(),
            )
        {
            return false;
        }
        let a = execution.reservation();
        // Keep the existing missing-wallet/stale-hash error order in the legacy path.
        let Some(wallet) = self.execution.resolve(a.wallet_id()) else {
            return false;
        };
        let Some(blockhash) = self
            .blockhash
            .fresh_hash(self.settings.blockhash_max_age_ms)
        else {
            return false;
        };
        let prepared = match wallet
            .prepare_original_sdk(
                execution,
                SdkExecutionContext {
                    blockhash,
                    wait_tx_confirmed: self.settings.wait_tx_confirmed,
                    mev_submit_verify_ms: self.settings.mev_submit_verify_ms,
                },
            )
            .await
        {
            Ok(Some(prepared)) => prepared,
            Ok(None) => return false,
            Err(error) => {
                self.fail_prepared_without_owner(execution, error).await;
                return true;
            }
        };
        if let Err(error) = prepared.validate_current(execution) {
            self.fail_prepared_without_owner(execution, error).await;
            return true;
        }
        let owner = match claims
            .claim_prepared_sdk(execution, prepared.as_ref())
            .await
        {
            Ok(ExecutionClaim::Acquired(owner)) => owner,
            Ok(ExecutionClaim::AlreadyHeld) => return true,
            Err(error) => {
                // Commit acknowledgement may be lost. Do not fail the parent or try legacy.
                error!(target: LOG_TARGET, request_id = %a.request_id(), error = %crate::sanitize::redact_urls(&error.to_string()), "prepared SDK claim unavailable; no execution, fallback or retry");
                return true;
            }
        };
        if !self.begin_execution(execution).await {
            return true;
        }
        let outcome = prepared.execute(execution, owner).await;
        self.finish_journaled(
            a.request_id(),
            a.idempotency_key(),
            execution.command(),
            outcome,
        );
        true
    }

    async fn fail_prepared_without_owner(&self, execution: &AdmittedTrade, error: anyhow::Error) {
        let a = execution.reservation();
        let error = crate::sanitize::redact_urls(&error.to_string());
        match self
            .journal
            .fail_unclaimed_preparation(execution, &error)
            .await
        {
            Ok(Some(record)) => {
                self.index.insert_record(&record);
                self.after_trade(a.idempotency_key(), execution.command(), Some(record));
            }
            Ok(None) => {
                // Another owner or terminal parent wins; never insert Unknown over its result.
                info!(target: LOG_TARGET, request_id = %a.request_id(), "prepared SDK failure did not own the parent transition; stopped");
            }
            Err(storage_error) => {
                error!(target: LOG_TARGET, request_id = %a.request_id(), error = %crate::sanitize::redact_urls(&storage_error.to_string()), "prepared SDK failure could not be recorded; stopped without index overwrite or retry");
            }
        }
    }
}

#[cfg(test)]
mod tests;
