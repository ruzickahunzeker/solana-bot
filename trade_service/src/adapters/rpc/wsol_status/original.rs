// Test-only oracle: the original pinned RpcClient WSOL status path.
use anyhow::{bail, Context};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::TransactionConfirmationStatus;
use std::sync::Arc;

use crate::{
    application::ports::wsol_queries::WsolStatusReader,
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        wsol_progress::WsolSignatureObservation,
    },
};

pub(crate) struct WsolSignatureReader {
    pub(super) rpc: Arc<RpcClient>,
    signature: Signature,
}
impl WsolSignatureReader {
    pub(crate) fn new(rpc: Arc<RpcClient>, signature: Signature) -> Self {
        Self { rpc, signature }
    }
}
impl WsolStatusReader for WsolSignatureReader {
    async fn lookup(&self) -> anyhow::Result<Option<WsolSignatureObservation>> {
        let response = self
            .rpc
            .get_signature_statuses_with_history(&[self.signature])
            .await?;
        if response.value.len() != 1 {
            bail!("WSOL signature status RPC returned an invalid result count");
        }
        if response.context.slot == 0 {
            bail!("WSOL signature status RPC returned a zero context slot");
        }
        let Some(status) = response.value.into_iter().next().flatten() else {
            return Ok(None);
        };
        if status.status.as_ref().err() != status.err.as_ref() {
            bail!("WSOL signature status RPC returned inconsistent transaction errors");
        }
        let commitment = match status
            .confirmation_status
            .context("WSOL signature status RPC omitted explicit confirmation status")?
        {
            TransactionConfirmationStatus::Processed => ChainCommitment::Processed,
            TransactionConfirmationStatus::Confirmed => ChainCommitment::Confirmed,
            TransactionConfirmationStatus::Finalized => ChainCommitment::Finalized,
        };
        if (commitment == ChainCommitment::Finalized) != status.confirmations.is_none() {
            bail!("WSOL signature status RPC returned inconsistent rooted confirmation fields");
        }
        let signature = self.signature.to_string();
        let observation = WsolSignatureObservation {
            chain: ChainObservation {
                signature: signature.clone(),
                commitment,
                slot: status.slot,
                error: status.err.as_ref().map(ToString::to_string),
            },
            context_slot: response.context.slot,
        };
        observation.outcome_for(&signature)?;
        Ok(Some(observation))
    }
}
