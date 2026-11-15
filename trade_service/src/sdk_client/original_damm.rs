use super::{
    lane_execution::{LaneExecution, PreparedLaneRequest},
    SdkTradeClient,
};
use crate::{
    adapters::{
        rpc::read_runtime::SharedReadRuntime,
        sdk::{
            damm_sell_message::{DammSellCandidate, DammSellContext, DammSignedCandidate},
            ordinary_lanes::OrdinarySdkLane,
            original_sender::OriginalSdkSender,
            trade_command::{sdk_trade, ExecuteTrade},
        },
    },
    application::{
        ordinary_claims::ClaimOperation,
        ordinary_sdk_submission::{self, SdkSigningPermit},
        ports::{
            ordinary_claims::ExecutionOwner,
            ordinary_execution::SdkExecutionContext,
            ordinary_sdk_submission::{OrdinarySdkCandidateBackend, OrdinarySdkSubmissionJournal},
        },
        trade_admission::AdmittedTrade,
        trade_execution::JournaledTradeOutcome,
    },
    domain::{
        preparation_balance::PreparationBalanceFailure, sdk_candidate::SdkCandidateMetadata,
        submission::SignedSubmission, trade::Side,
    },
};
use anyhow::{ensure, Context, Result};
use solana_sdk::signature::Signer;
use std::sync::Arc;

#[cfg(test)]
mod tests;

struct OriginalDammBackend<'a> {
    wallet: &'a SdkTradeClient,
    execution: &'a AdmittedTrade,
    context: DammSellContext,
    sender: OriginalSdkSender,
    wait_confirmed: bool,
    verification_ms: u64,
}

impl OrdinarySdkCandidateBackend for OriginalDammBackend<'_> {
    type Candidate = DammSellCandidate;
    type Signed = DammSignedCandidate;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.wallet.ensure_broadcast_allowed()
    }
    fn metadata<'a>(&self, candidate: &'a DammSellCandidate) -> &'a SdkCandidateMetadata {
        candidate.metadata()
    }
    fn unsigned_message(&self, candidate: &DammSellCandidate) -> Result<Vec<u8>> {
        Ok(candidate.message_bytes())
    }
    fn authorize(&self, execution: &AdmittedTrade, candidate: &DammSellCandidate) -> Result<()> {
        ensure!(
            execution.reservation().request_id() == self.execution.reservation().request_id()
                && execution.reservation().request_hash()
                    == self.execution.reservation().request_hash(),
            "SDK original backend order changed"
        );
        candidate.authorize_live(execution, self.context)
    }
    fn sign(
        &self,
        candidate: DammSellCandidate,
        permit: SdkSigningPermit<'_>,
    ) -> Result<DammSignedCandidate> {
        candidate.sign_original(permit, self.context, || {
            self.wallet.ensure_broadcast_allowed()?;
            Ok(self.wallet.wallet_signer())
        })
    }
    fn signed_submission<'a>(&self, signed: &'a DammSignedCandidate) -> &'a SignedSubmission {
        signed.submission()
    }
    fn verify_signed(
        &self,
        signed: &DammSignedCandidate,
        metadata: &SdkCandidateMetadata,
        message: &[u8],
    ) -> Result<()> {
        signed.check_submission(self.execution, self.context, signed.submission())?;
        crate::adapters::sdk::damm_sell_message::validate_signed_candidate(
            metadata,
            message,
            signed.submission(),
        )
    }
    fn authorize_broadcast(&self, signed: &DammSignedCandidate) -> Result<()> {
        signed.recheck(self.execution, self.context)
    }
    async fn broadcast(
        &self,
        artifact: &DammSignedCandidate,
        signed: &SignedSubmission,
    ) -> Result<String> {
        self.wallet.ensure_broadcast_allowed()?;
        self.sender
            .broadcast(
                self.execution,
                self.context,
                artifact,
                signed,
                self.wait_confirmed,
                self.verification_ms,
            )
            .await
    }
}

impl SdkTradeClient {
    pub(crate) async fn execute_original_damm<J: OrdinarySdkSubmissionJournal>(
        &self,
        journal: &J,
        execution: &AdmittedTrade,
        owner: ExecutionOwner,
        context: SdkExecutionContext,
        runtime: Arc<SharedReadRuntime>,
    ) -> JournaledTradeOutcome {
        let prepared = async {
            owner.ensure_binding(execution)?;
            ensure!(
                owner.operation() == ClaimOperation::SdkTrade,
                "SDK original owner operation mismatch"
            );
            self.ensure_broadcast_allowed()?;
            ensure!(
                self.wallet().to_string() == execution.reservation().wallet_pubkey(),
                "SDK original resolved wallet differs from admission"
            );
            let trade = sdk_trade(
                execution.command(),
                context.blockhash,
                context.wait_tx_confirmed,
                context.mev_submit_verify_ms,
            );
            self.prepare_original_damm(execution, &trade, runtime).await
        }
        .await;
        match prepared {
            Ok((backend, candidate)) => {
                ordinary_sdk_submission::execute(journal, &backend, execution, owner, candidate)
                    .await
            }
            Err(error) => {
                ordinary_sdk_submission::reject_preparation(journal, execution, owner, error).await
            }
        }
    }

    async fn prepare_original_damm<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        trade: &ExecuteTrade,
        runtime: Arc<SharedReadRuntime>,
    ) -> Result<(OriginalDammBackend<'a>, DammSellCandidate)> {
        let mut failures = Vec::new();
        for lane in self.trade_execution_lanes(trade)? {
            let source = LaneExecution::new(self, lane, Side::Sell, &runtime);
            let prepared = match source.prepare(trade).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    if error
                        .downcast_ref::<PreparationBalanceFailure>()
                        .is_some_and(|failure| !failure.allows_lane_fallback())
                    {
                        return Err(error);
                    }
                    let error = crate::sanitize::redact_urls(&error.to_string());
                    tracing::warn!(provider = lane.swqos_type.as_str(), error = %error, "original SDK lane unavailable before signing; no transaction was built or sent by a worker");
                    failures.push(format!("{}: {error}", lane.swqos_type.as_str()));
                    continue;
                }
            };
            source.validate_prepared(&prepared, trade)?;
            let PreparedLaneRequest::Sell {
                client,
                message,
                preview_message,
                ..
            } = prepared
            else {
                anyhow::bail!("SDK original preparation returned another direction");
            };
            let profile = message.context("SDK original live profile missing")?;
            let preview = preview_message.context("SDK original unsigned preview missing")?;
            let context = DammSellContext {
                wallet: client.payer.pubkey(),
                provider: lane.swqos_type,
                version: client.transaction_version,
                seed_optimized: client.use_seed_optimize,
            };
            let sender = OriginalSdkSender::from_client(&client, lane.swqos_type, runtime.clone())?;
            let tip = sender.freeze_tip(trade.tip_sol)?;
            profile
                .validate_unsigned_preview(&preview, client.get_rpc())
                .await?;
            let candidate = profile.prepare_live(tip)?;
            candidate.authorize_live(execution, context)?;
            return Ok((
                OriginalDammBackend {
                    wallet: self,
                    execution,
                    context,
                    sender,
                    wait_confirmed: trade.wait_tx_confirmed,
                    verification_ms: trade.mev_submit_verify_ms,
                },
                candidate,
            ));
        }
        anyhow::bail!("no original SDK lane prepared; {}", failures.join("; "))
    }
}
