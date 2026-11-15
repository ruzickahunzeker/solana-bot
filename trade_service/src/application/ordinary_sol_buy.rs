use anyhow::Context;

use crate::application::{
    ports::ordinary_sol_buy::{OrdinarySolBuyPipeline, SolBuyIntent},
    quote_submission_types::{QuoteSubmissionCommand, QuoteSubmissionOutcome},
    trade_command::TradeCommand,
    trade_execution::{TradeExecutionResult, UncertainSolBuySubmission},
};

pub(crate) fn intent(command: &TradeCommand) -> anyhow::Result<SolBuyIntent> {
    let amount_lamports = command
        .amount
        .buy_units()
        .context("SOL aggregator buy amount is missing")?;
    Ok(SolBuyIntent {
        wallet_id: command.wallet_id.clone(),
        mint: command.route.mint,
        amount_lamports,
        slippage_bps: u16::try_from(command.execution.slippage_bps)
            .context("aggregator slippage exceeds supported range")?,
    })
}

pub(crate) async fn execute<P: OrdinarySolBuyPipeline>(
    pipeline: &P,
    command: &TradeCommand,
) -> anyhow::Result<TradeExecutionResult> {
    let request = pipeline.map_request(intent(command)?);
    pipeline.validate_request(&request)?;
    let quote = pipeline.quote(&request).await?;
    let bound = pipeline.bind(&request, quote)?;
    let simulate = command.execution.mode.simulate();
    let submission = pipeline
        .submit(
            &bound,
            &QuoteSubmissionCommand {
                wallet_id: command.wallet_id.clone(),
                confirmed: !simulate,
                simulate,
                idempotency_key: command.idempotency_key.clone(),
            },
        )
        .await?;
    if !simulate && submission.status == QuoteSubmissionOutcome::Unknown {
        return Err(UncertainSolBuySubmission::new(submission.signature, submission.error).into());
    }
    Ok(TradeExecutionResult {
        success: matches!(
            submission.status,
            QuoteSubmissionOutcome::Submitted | QuoteSubmissionOutcome::Simulated
        ),
        signature: submission.signature,
        error: submission.error,
    })
}

#[cfg(test)]
mod tests;
