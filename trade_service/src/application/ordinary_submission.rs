use crate::{
    application::{
        ordinary_claims::ensure_scope,
        ordinary_sol_buy,
        ports::{
            ordinary_claims::ExecutionOwner,
            ordinary_journal::TradeJournalMode,
            ordinary_reconciliation::{
                OrdinaryReconciliationJournal, RecoveredOrdinaryObservations,
            },
            ordinary_sol_buy::{
                JournaledSolBuyOutcome, OrdinarySolBuyPipeline, SolBuyExecutionOutcome,
            },
            ordinary_submission::OrdinarySubmissionJournal,
            quote_execution::OrdinaryQuoteBackend,
            reconciliation::SignatureStatusReader,
        },
        quote_submission_types::{
            QuoteSubmissionCommand, QuoteSubmissionOutcome, QuoteSubmissionResult,
        },
        trade_admission::AdmittedTrade,
        trade_execution::UncertainSolBuySubmission,
        trade_queries::is_pending_status,
    },
    domain::{
        quote::{BoundQuote, QuotePreflight},
        trade::{TradeAmount, WSOL_MINT},
    },
};
use anyhow::{ensure, Result};
#[cfg(test)]
mod tests;

pub(crate) fn validate_bound(execution: &AdmittedTrade, bound: &BoundQuote) -> Result<()> {
    let anchor = execution.reservation();
    let command = execution.command();
    let TradeAmount::BuySol { lamports } = command.amount else {
        anyhow::bail!("ordinary SOL amount required")
    };
    validate_quote_intent(
        bound,
        anchor.wallet_id(),
        anchor.wallet_pubkey(),
        &command.route.mint.to_string(),
        lamports,
        command.execution.slippage_bps,
    )
}

pub(crate) fn validate_quote_intent(
    bound: &BoundQuote,
    wallet_id: &str,
    wallet_pubkey: &str,
    mint: &str,
    lamports: u64,
    slippage_bps: u64,
) -> Result<()> {
    ensure!(
        bound.wallet_id == wallet_id
            && bound.wallet_pubkey == wallet_pubkey
            && bound.quote.input_mint == WSOL_MINT
            && bound.quote.output_mint == mint
            && bound.quote.amount_lamports == lamports
            && lamports > 0
            && u64::from(bound.quote_slippage_bps) == slippage_bps,
        "ordinary bound quote does not match reserved intent"
    );
    ensure!(
        bound.quote_slippage_bps < 10_000
            && bound.quote.expected_output_units > 0
            && bound.quote.minimum_output_units > 0
            && bound.quote.minimum_output_units <= bound.quote.expected_output_units
            && u128::from(bound.quote.minimum_output_units)
                >= u128::from(bound.quote.expected_output_units)
                    * u128::from(10_000 - bound.quote_slippage_bps)
                    / 10_000,
        "ordinary bound quote minimum output invalid"
    );
    ensure!(
        !bound.quote.route.is_empty() && bound.quote.route.len() <= 4,
        "ordinary quote route invalid"
    );
    let mut input = WSOL_MINT;
    for leg in &bound.quote.route {
        ensure!(
            leg.input_mint == input,
            "ordinary quote route discontinuous"
        );
        input = &leg.output_mint;
    }
    ensure!(
        input == bound.quote.output_mint,
        "ordinary quote route target mismatch"
    );
    Ok(())
}

pub(crate) fn validate_preflight(bound: &BoundQuote, report: &QuotePreflight) -> Result<()> {
    ensure!(
        report.quote_id == bound.quote_id
            && report.wallet_id == bound.wallet_id
            && report.quote_still_valid
            && report.unsigned_transaction_built
            && report.aggregator_simulation_ok
            && report.local_simulation_ok
            && report.instruction_audit_passed
            && report.executable,
        "ordinary preflight did not authorize this quote and wallet"
    );
    Ok(())
}

pub(crate) async fn execute<
    J: OrdinarySubmissionJournal + OrdinaryReconciliationJournal + RecoveredOrdinaryObservations,
    P: OrdinarySolBuyPipeline,
    B: OrdinaryQuoteBackend + SignatureStatusReader,
>(
    journal: &J,
    pipeline: &P,
    backend: &B,
    execution: &AdmittedTrade,
    owner: &ExecutionOwner,
) -> SolBuyExecutionOutcome {
    let start = async {
        ensure_scope(execution, TradeJournalMode::Durable)?;
        owner.ensure_binding(execution)?;
        journal.begin(execution, owner).await
    }
    .await;
    let started = start.is_ok();
    let controlled = ControlledPipeline {
        journal,
        pipeline,
        backend,
        execution,
        owner,
    };
    let result = match start {
        Ok(()) => ordinary_sol_buy::execute(&controlled, execution.command()).await,
        Err(error) => Err(error),
    };
    let (mut signature, error) = match &result {
        Ok(result) => (result.signature.clone(), result.error.clone()),
        Err(error) => {
            let uncertain = error.downcast_ref::<UncertainSolBuySubmission>();
            if started && uncertain.is_none() {
                if let Err(storage_error) = journal
                    .fail_before_sign(
                        execution,
                        owner,
                        &crate::sanitize::redact_urls(&error.to_string()),
                    )
                    .await
                {
                    tracing::error!(error = %crate::sanitize::redact_urls(&storage_error.to_string()), "ordinary pre-sign failure could not be saved; do not resubmit");
                }
            }
            (
                uncertain
                    .and_then(|value| value.signature())
                    .map(str::to_owned),
                Some(crate::sanitize::redact_urls(&error.to_string())),
            )
        }
    };
    let record = match journal.observe(execution, owner).await {
        Ok(record)
            if !is_pending_status(record.status)
                && (signature.is_none() || record.signature == signature) =>
        {
            Some(record)
        }
        Ok(record) => {
            if signature.is_none() {
                signature = record.signature;
            }
            None
        }
        Err(storage_error) => {
            tracing::error!(error = %crate::sanitize::redact_urls(&storage_error.to_string()), "ordinary journal observation unavailable; original operation must not be retried");
            None
        }
    };
    SolBuyExecutionOutcome::Journaled(JournaledSolBuyOutcome {
        record,
        signature,
        error: error.or_else(|| {
            Some("ordinary submission journal outcome unavailable; reconcile original order".into())
        }),
    })
}

struct ControlledPipeline<'a, J, P, B> {
    journal: &'a J,
    pipeline: &'a P,
    backend: &'a B,
    execution: &'a AdmittedTrade,
    owner: &'a ExecutionOwner,
}
impl<
        J: OrdinarySubmissionJournal + OrdinaryReconciliationJournal + RecoveredOrdinaryObservations,
        P: OrdinarySolBuyPipeline,
        B: OrdinaryQuoteBackend + SignatureStatusReader,
    > OrdinarySolBuyPipeline for ControlledPipeline<'_, J, P, B>
{
    type Request = P::Request;
    fn map_request(
        &self,
        input: crate::application::ports::ordinary_sol_buy::SolBuyIntent,
    ) -> Self::Request {
        self.pipeline.map_request(input)
    }
    fn validate_request(&self, request: &Self::Request) -> Result<()> {
        self.pipeline.validate_request(request)
    }
    async fn quote(&self, request: &Self::Request) -> Result<crate::domain::quote::QuoteEstimate> {
        self.pipeline.quote(request).await
    }
    fn bind(
        &self,
        request: &Self::Request,
        quote: crate::domain::quote::QuoteEstimate,
    ) -> Result<BoundQuote> {
        self.pipeline.bind(request, quote)
    }
    async fn submit(
        &self,
        bound: &BoundQuote,
        request: &QuoteSubmissionCommand,
    ) -> Result<QuoteSubmissionResult> {
        submit(
            self.journal,
            self.backend,
            self.execution,
            self.owner,
            bound,
            request,
        )
        .await
    }
}

pub(crate) async fn submit<
    J: OrdinarySubmissionJournal + OrdinaryReconciliationJournal + RecoveredOrdinaryObservations,
    B: OrdinaryQuoteBackend + SignatureStatusReader,
>(
    journal: &J,
    backend: &B,
    execution: &AdmittedTrade,
    owner: &ExecutionOwner,
    bound: &BoundQuote,
    request: &QuoteSubmissionCommand,
) -> Result<QuoteSubmissionResult> {
    owner.ensure_binding(execution)?;
    validate_bound(execution, bound)?;
    ensure!(
        !request.simulate
            && request.confirmed
            && request.wallet_id == execution.reservation().wallet_id()
            && request.idempotency_key == execution.reservation().idempotency_key(),
        "ordinary submit command mismatch"
    );
    backend.ensure_live_allowed()?;
    let (prepared, preflight) = backend.prepare(bound).await?;
    validate_preflight(bound, &preflight)?;
    ensure!(
        u128::from(bound.expires_at_ms) > crate::state::now_ms(),
        "ordinary quote expired"
    );
    let message = backend.prepared_message(&prepared)?;
    journal
        .prepare(execution, owner, bound, &preflight, &message)
        .await?;
    journal
        .authorize_sign(execution, owner, bound, &preflight, &message)
        .await?;
    backend.ensure_live_allowed()?;
    let signed = backend.sign(prepared)?;
    let signature = signed.signature.clone();
    let after_sign = async {
        ensure!(
            backend.signed_message(&signed)? == message,
            "ordinary signed message changed after authorization"
        );
        journal.record_signed(execution, owner, &signed).await?;
        let persisted = journal.read_signed(execution, owner).await?;
        ensure!(
            persisted.signature == signed.signature && persisted.bytes == signed.bytes,
            "ordinary persisted signed bytes differ from original"
        );
        let (submitted, error) = match backend.broadcast(&persisted).await {
            Ok(receipt) if receipt == persisted.signature => (true, None),
            Ok(_) => (
                false,
                Some("RPC returned a different signature; reconcile original signature".into()),
            ),
            Err(error) => (
                false,
                Some(crate::sanitize::redact_urls(&error.to_string())),
            ),
        };
        journal
            .record_broadcast(execution, owner, &persisted, submitted, error.as_deref())
            .await?;
        match crate::application::ordinary_reconciliation::reconcile(
            journal,
            backend,
            execution,
            owner,
            &persisted.signature,
            std::time::Duration::from_secs(3),
        )
        .await
        {
            Ok(observed) => {
                if let Some(error) = observed.lookup_error {
                    tracing::warn!(error = %error, status = ?observed.record.status,
                        "ordinary original-signature lookup unavailable; no transaction retry");
                }
            }
            Err(error) => {
                tracing::warn!(error = %crate::sanitize::redact_urls(&error.to_string()),
                    "ordinary final observation unavailable; preserve original operation without retry");
            }
        }
        Ok::<_, anyhow::Error>((submitted, error))
    }
    .await;
    let (submitted, error) = after_sign.map_err(|error| {
        anyhow::Error::new(UncertainSolBuySubmission::new(
            Some(signature.clone()),
            Some(error.to_string()),
        ))
    })?;
    Ok(QuoteSubmissionResult {
        quote_id: bound.quote_id.clone(),
        wallet_id: bound.wallet_id.clone(),
        status: if submitted {
            QuoteSubmissionOutcome::Submitted
        } else {
            QuoteSubmissionOutcome::Unknown
        },
        signature: Some(signature),
        error,
        preflight,
    })
}
