use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport, MAX_BODY},
    read_runtime::SharedReadRuntime,
};
use anyhow::{ensure, Result};
use solana_client::rpc_response::Response;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::TransactionStatus;
use std::time::Duration;

/// The original single confirmation read; no history, poll, fallback or send capability.
pub(crate) struct SolTransferConfirmationReader<'a, T = ReqwestReadTransport> {
    runtime: &'a SharedReadRuntime<T>,
    url: String,
    signature: Signature,
    commitment: CommitmentConfig,
}

impl<'a, T: ReadHttpTransport> SolTransferConfirmationReader<'a, T> {
    pub(crate) fn new(
        runtime: &'a SharedReadRuntime<T>,
        url: String,
        signature: Signature,
        commitment: CommitmentConfig,
    ) -> Self {
        Self {
            runtime,
            url,
            signature,
            commitment,
        }
    }

    pub(crate) async fn confirm(self, budget: Duration) -> Result<bool> {
        self.runtime
            .sol_transfer_confirmation(&self.url, self.signature, self.commitment, budget)
            .await
    }
}

#[derive(serde::Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Response<Vec<Option<TransactionStatus>>>>,
    error: Option<serde_json::Value>,
}

pub(super) fn decode(id: u64, bytes: &[u8], commitment: CommitmentConfig) -> Result<bool> {
    ensure!(
        bytes.len() <= MAX_BODY,
        "SOL transfer confirmation body exceeds limit"
    );
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("invalid SOL transfer confirmation response"))?;
    ensure!(
        envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
        "SOL transfer confirmation RPC identity or result invalid"
    );
    let response = envelope
        .result
        .ok_or_else(|| anyhow::anyhow!("SOL transfer confirmation RPC omitted result"))?;
    ensure!(
        response.value.len() == 1,
        "invalid SOL transfer confirmation result count"
    );
    // Retain the pinned client's exact commitment/legacy-error decision, not WSOL's status policy.
    Ok(response
        .value
        .into_iter()
        .next()
        .flatten()
        .filter(|status| status.satisfies_commitment(commitment))
        .is_some_and(|status| status.status.is_ok()))
}

#[cfg(test)]
mod tests;
