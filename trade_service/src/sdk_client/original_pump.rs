use super::{lane_execution::LaneExecution, SdkTradeClient};
use crate::{
    adapters::{
        persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
        rpc::read_runtime::SharedReadRuntime,
        sdk::{
            original_sender::OriginalSdkSender,
            pump_curve_evidence::{
                prepare_native_evidence, NativeEvidenceContext, NativeProfileUnsupported,
            },
            pump_prepared::{PumpPreparedCandidate, PumpPreparedContext, PumpSignedCandidate},
            trade_command::sdk_trade,
        },
    },
    application::{
        ordinary_sdk_submission::{self, SdkSigningPermit},
        ports::{
            ordinary_claims::ExecutionOwner,
            ordinary_execution::{JournaledSdkFuture, PreparedSdkExecution, SdkExecutionContext},
            ordinary_sdk_submission::OrdinarySdkCandidateBackend,
        },
        trade_admission::AdmittedTrade,
    },
    domain::{sdk_prepared_record::SdkPreparedRecord, submission::SignedSubmission},
};
use anyhow::Result;
use sol_trade_sdk::{
    common::TradeTransactionVersion, instruction::utils::pumpfun::global_constants,
};
use solana_sdk::{hash::Hash, signature::Signer};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct OriginalPumpBackend<'a> {
    wallet: &'a SdkTradeClient,
    execution: &'a AdmittedTrade,
    context: PumpPreparedContext,
    sender: OriginalSdkSender,
    wait_confirmed: bool,
    verification_ms: u64,
}

impl<'a> OrdinarySdkCandidateBackend for OriginalPumpBackend<'a> {
    type Candidate = PumpPreparedCandidate<'a>;
    type Signed = PumpSignedCandidate<'a>;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.wallet.ensure_broadcast_allowed()
    }
    fn metadata<'b>(&self, c: &'b Self::Candidate) -> &'b SdkPreparedRecord {
        c.metadata()
    }
    fn unsigned_message(&self, c: &Self::Candidate) -> Result<Vec<u8>> {
        Ok(c.message().to_vec())
    }
    fn authorize(&self, execution: &AdmittedTrade, c: &Self::Candidate) -> Result<()> {
        anyhow::ensure!(
            std::ptr::eq(execution, self.execution),
            "native Pump backend admission changed"
        );
        c.authorize_live(execution, self.context)
    }
    fn sign(&self, c: Self::Candidate, permit: SdkSigningPermit<'_>) -> Result<Self::Signed> {
        c.sign_original(permit, self.context, || {
            self.wallet.ensure_broadcast_allowed()?;
            Ok(self.wallet.wallet_signer())
        })
    }
    fn signed_submission<'b>(&self, s: &'b Self::Signed) -> &'b SignedSubmission {
        s.submission()
    }
    fn verify_signed(
        &self,
        s: &Self::Signed,
        metadata: &SdkPreparedRecord,
        message: &[u8],
    ) -> Result<()> {
        self.authorize_broadcast(s)?;
        crate::adapters::sdk::prepared_candidate::validate_signed_candidate(
            metadata,
            message,
            s.submission(),
        )
    }
    fn authorize_broadcast(&self, s: &Self::Signed) -> Result<()> {
        s.recheck(self.execution, self.context)
    }
    async fn broadcast(
        &self,
        artifact: &Self::Signed,
        signed: &SignedSubmission,
    ) -> Result<String> {
        self.wallet.ensure_broadcast_allowed()?;
        self.sender
            .broadcast_pump(
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

struct PreparedOriginalPump<'a> {
    backend: OriginalPumpBackend<'a>,
    candidate: PumpPreparedCandidate<'a>,
    journal: SqliteOrdinaryExecutionClaims,
}

impl PreparedSdkExecution for PreparedOriginalPump<'_> {
    fn metadata(&self) -> &SdkPreparedRecord {
        self.candidate.metadata()
    }
    fn message(&self) -> &[u8] {
        self.candidate.message()
    }
    fn validate_current(&self, execution: &AdmittedTrade) -> Result<()> {
        self.backend.ensure_live_allowed()?;
        self.backend.authorize(execution, &self.candidate)
    }
    fn execute<'a>(
        self: Box<Self>,
        execution: &'a AdmittedTrade,
        owner: ExecutionOwner,
    ) -> JournaledSdkFuture<'a>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let Self {
                backend,
                candidate,
                journal,
            } = *self;
            ordinary_sdk_submission::execute(&journal, &backend, execution, owner, candidate).await
        })
    }
}

impl SdkTradeClient {
    pub(crate) async fn prepare_original_pump<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        context: SdkExecutionContext,
        runtime: Arc<SharedReadRuntime>,
        journal: SqliteOrdinaryExecutionClaims,
    ) -> Result<Option<Box<dyn PreparedSdkExecution + 'a>>> {
        self.ensure_broadcast_allowed()?;
        anyhow::ensure!(
            self.wallet().to_string() == execution.reservation().wallet_pubkey(),
            "native Pump resolved wallet differs from original admission"
        );
        let trade = sdk_trade(
            execution.command(),
            context.blockhash,
            context.wait_tx_confirmed,
            context.mev_submit_verify_ms,
        );
        let lanes = self.trade_execution_lanes(&trade)?;
        // Preserve the existing pre-sign Auto/fallback policy until all selected lanes share
        // this prepared profile. Do not silently pin a formerly multi-lane request to first().
        let [lane] = lanes.as_slice() else {
            return Ok(None);
        };
        if lane.client.transaction_version != TradeTransactionVersion::V0
            || lane.client.use_seed_optimize
            || lane.client.middleware_manager.is_some()
            || lane.client.risk_gate.is_some()
        {
            return Ok(None);
        }
        let client = LaneExecution::new(self, lane, execution.command().amount.side(), &runtime)
            .execution_client()?;
        let sender = OriginalSdkSender::from_client(&client, lane.swqos_type, runtime.clone())?;
        let tip_destination = sender.freeze_tip(execution.command().execution.tip_sol)?;
        let native = PumpPreparedContext {
            wallet: client.payer.pubkey(),
            provider: lane.swqos_type,
            version: client.transaction_version,
            seed_optimized: client.use_seed_optimize,
            blockhash: Hash::new_from_array(context.blockhash.to_bytes()),
            tip_destination,
        };
        let evidence = match prepare_native_evidence(
            execution,
            runtime.as_ref(),
            &client.get_rpc().url(),
            NativeEvidenceContext {
                minimum_slot: None,
                blockhash: native.blockhash,
                provider: native.provider,
                buyback: global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS[0],
                tip_destination,
                deadline: Instant::now() + Duration::from_secs(3),
            },
        )
        .await
        {
            Ok(evidence) => evidence,
            Err(error) if error.is::<NativeProfileUnsupported>() => return Ok(None),
            Err(error) => return Err(error),
        };
        let candidate = PumpPreparedCandidate::new(evidence, native, execution)?;
        let backend = OriginalPumpBackend {
            wallet: self,
            execution,
            context: native,
            sender,
            wait_confirmed: context.wait_tx_confirmed,
            verification_ms: context.mev_submit_verify_ms,
        };
        backend.authorize(execution, &candidate)?;
        Ok(Some(Box::new(PreparedOriginalPump {
            backend,
            candidate,
            journal,
        })))
    }
}

#[cfg(test)]
mod tests;
