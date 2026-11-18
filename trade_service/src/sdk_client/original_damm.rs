use super::{
    lane_execution::{LaneExecution, PreparedLaneRequest},
    SdkTradeClient,
};
use crate::{
    adapters::{
        rpc::{
            damm_sell_simulation::shared::DammSimulationReader, read_runtime::SharedReadRuntime,
        },
        sdk::{
            damm_probe::{DammBuyCandidate, DammBuyContext, DammBuySignedCandidate},
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
    domain::{sdk_prepared_record::SdkPreparedRecord, submission::SignedSubmission},
};
use anyhow::{ensure, Context, Result};
use solana_sdk::signature::Signer;
use std::sync::Arc;

#[cfg(test)]
mod tests;

enum OriginalCandidate {
    Sell(DammSellCandidate),
    Buy(DammBuyCandidate),
}

enum OriginalSigned {
    Sell(DammSignedCandidate),
    Buy(DammBuySignedCandidate),
}

fn buy_context(context: DammSellContext) -> DammBuyContext {
    DammBuyContext {
        wallet: context.wallet,
        provider: context.provider,
        version: context.version,
        seed_optimized: context.seed_optimized,
    }
}

struct OriginalDammBackend<'a> {
    wallet: &'a SdkTradeClient,
    execution: &'a AdmittedTrade,
    context: DammSellContext,
    sender: OriginalSdkSender,
    wait_confirmed: bool,
    verification_ms: u64,
}

impl OrdinarySdkCandidateBackend for OriginalDammBackend<'_> {
    type Candidate = OriginalCandidate;
    type Signed = OriginalSigned;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.wallet.ensure_broadcast_allowed()
    }
    fn metadata<'a>(&self, candidate: &'a OriginalCandidate) -> &'a SdkPreparedRecord {
        match candidate {
            OriginalCandidate::Sell(c) => c.metadata(),
            OriginalCandidate::Buy(c) => c.metadata(),
        }
    }
    fn unsigned_message(&self, candidate: &OriginalCandidate) -> Result<Vec<u8>> {
        Ok(match candidate {
            OriginalCandidate::Sell(c) => c.message_bytes(),
            OriginalCandidate::Buy(c) => c.message_bytes(),
        })
    }
    fn authorize(&self, execution: &AdmittedTrade, candidate: &OriginalCandidate) -> Result<()> {
        ensure!(
            execution.reservation().request_id() == self.execution.reservation().request_id()
                && execution.reservation().request_hash()
                    == self.execution.reservation().request_hash(),
            "SDK original backend order changed"
        );
        match candidate {
            OriginalCandidate::Sell(c) => c.authorize_live(execution, self.context),
            OriginalCandidate::Buy(c) => c.authorize_live(execution, buy_context(self.context)),
        }
    }
    fn sign(
        &self,
        candidate: OriginalCandidate,
        permit: SdkSigningPermit<'_>,
    ) -> Result<OriginalSigned> {
        let acquire = || {
            self.wallet.ensure_broadcast_allowed()?;
            Ok(self.wallet.wallet_signer())
        };
        match candidate {
            OriginalCandidate::Sell(c) => c
                .sign_original(permit, self.context, acquire)
                .map(OriginalSigned::Sell),
            OriginalCandidate::Buy(c) => c
                .sign_original(permit, buy_context(self.context), acquire)
                .map(OriginalSigned::Buy),
        }
    }
    fn signed_submission<'a>(&self, signed: &'a OriginalSigned) -> &'a SignedSubmission {
        match signed {
            OriginalSigned::Sell(s) => s.submission(),
            OriginalSigned::Buy(s) => s.submission(),
        }
    }
    fn verify_signed(
        &self,
        signed: &OriginalSigned,
        metadata: &SdkPreparedRecord,
        message: &[u8],
    ) -> Result<()> {
        self.authorize_broadcast(signed)?;
        crate::adapters::sdk::prepared_candidate::validate_signed_candidate(
            metadata,
            message,
            self.signed_submission(signed),
        )
    }
    fn authorize_broadcast(&self, signed: &OriginalSigned) -> Result<()> {
        match signed {
            OriginalSigned::Sell(s) => s.recheck(self.execution, self.context),
            OriginalSigned::Buy(s) => s.recheck(self.execution, buy_context(self.context)),
        }
    }
    async fn broadcast(
        &self,
        artifact: &OriginalSigned,
        signed: &SignedSubmission,
    ) -> Result<String> {
        self.wallet.ensure_broadcast_allowed()?;
        match artifact {
            OriginalSigned::Sell(artifact) => {
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
            OriginalSigned::Buy(artifact) => {
                self.sender
                    .broadcast_buy(
                        self.execution,
                        buy_context(self.context),
                        artifact,
                        signed,
                        self.wait_confirmed,
                        self.verification_ms,
                    )
                    .await
            }
        }
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
    ) -> Result<(OriginalDammBackend<'a>, OriginalCandidate)> {
        let mut failures = Vec::new();
        for lane in self.trade_execution_lanes(trade)? {
            let source =
                LaneExecution::new(self, lane, execution.command().amount.side(), &runtime);
            let prepared = match source.prepare(trade).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    if crate::adapters::sdk::ordinary_lanes::preparation_rejection(&error).is_some()
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
            let client = match &prepared {
                PreparedLaneRequest::Sell { client, .. }
                | PreparedLaneRequest::Buy { client, .. } => client,
            };
            let context = DammSellContext {
                wallet: client.payer.pubkey(),
                provider: lane.swqos_type,
                version: client.transaction_version,
                seed_optimized: client.use_seed_optimize,
            };
            let sender = OriginalSdkSender::from_client(&client, lane.swqos_type, runtime.clone())?;
            let tip = sender.freeze_tip(trade.tip_sol)?;
            let candidate = match prepared {
                PreparedLaneRequest::Sell {
                    client,
                    message,
                    preview_message,
                    ..
                } => {
                    let profile = message.context("SDK original live profile missing")?;
                    let preview =
                        preview_message.context("SDK original unsigned preview missing")?;
                    let reader =
                        DammSimulationReader::new(runtime.as_ref(), client.get_rpc().url());
                    profile.validate_unsigned_preview(&preview, &reader).await?;
                    OriginalCandidate::Sell(profile.prepare_live(tip)?)
                }
                PreparedLaneRequest::Buy {
                    client, message, ..
                } => {
                    let profile = message.context("SDK original buy live profile missing")?;
                    let candidate = profile.prepare_live(tip)?;
                    let reader =
                        DammSimulationReader::new(runtime.as_ref(), client.get_rpc().url());
                    candidate.validate_unsigned_live(&reader).await?;
                    OriginalCandidate::Buy(candidate)
                }
            };
            let backend = OriginalDammBackend {
                wallet: self,
                execution,
                context,
                sender,
                wait_confirmed: trade.wait_tx_confirmed,
                verification_ms: trade.mev_submit_verify_ms,
            };
            backend.authorize(execution, &candidate)?;
            return Ok((backend, candidate));
        }
        anyhow::bail!("no original SDK lane prepared; {}", failures.join("; "))
    }
}
