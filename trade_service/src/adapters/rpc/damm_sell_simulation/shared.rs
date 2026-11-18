use super::DammSimulationResult;
use crate::adapters::rpc::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use anyhow::{ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use solana_sdk::{
    hash::Hash, message::VersionedMessage, pubkey::Pubkey, signature::Signature,
    transaction::VersionedTransaction,
};
use std::future::Future;

mod sealed {
    pub trait Sealed {}
}
pub(crate) trait DammSimulator: sealed::Sealed + Send + Sync {
    fn simulate(
        &self,
        transaction: &VersionedTransaction,
        minimum: u64,
    ) -> impl Future<Output = Result<DammSimulationResult>> + Send;
}

/// Borrowed original shared runtime and captured RPC metadata, not an execution client.
pub(crate) struct DammSimulationReader<'a, T = ReqwestReadTransport> {
    runtime: &'a SharedReadRuntime<T>,
    url: String,
}
impl<'a, T: ReadHttpTransport> DammSimulationReader<'a, T> {
    pub(crate) fn new(runtime: &'a SharedReadRuntime<T>, url: String) -> Self {
        Self { runtime, url }
    }
}
impl<T: ReadHttpTransport> sealed::Sealed for DammSimulationReader<'_, T> {}
impl<T: ReadHttpTransport> DammSimulator for DammSimulationReader<'_, T> {
    async fn simulate(
        &self,
        transaction: &VersionedTransaction,
        minimum: u64,
    ) -> Result<DammSimulationResult> {
        let packet = DammSimulationPacket::checked(transaction, minimum)?;
        self.runtime
            .damm_candidate_simulation(&self.url, packet)
            .await
    }
}
pub(crate) async fn simulate_unsigned_at_slot(
    reader: &impl DammSimulator,
    transaction: &VersionedTransaction,
    minimum: u64,
) -> Result<DammSimulationResult> {
    reader.simulate(transaction, minimum).await
}

/// No Clone or arbitrary encoded-packet constructor; the runtime consumes this once.
pub(crate) struct DammSimulationPacket {
    encoded: String,
    minimum: u64,
}
impl DammSimulationPacket {
    fn checked(transaction: &VersionedTransaction, minimum: u64) -> Result<Self> {
        ensure!(
            minimum > 0,
            "DAMM simulation requires its output quote slot"
        );
        ensure!(
            transaction.signatures == [Signature::default()]
                && matches!(transaction.message, VersionedMessage::Legacy(_))
                && transaction.message.header().num_required_signatures == 1
                && transaction
                    .message
                    .static_account_keys()
                    .first()
                    .is_some_and(|key| *key != Pubkey::default())
                && *transaction.message.recent_blockhash() != Hash::default()
                && bincode::serialized_size(transaction)? <= 1232,
            "DAMM simulation requires a bounded single-payer zero-signature candidate"
        );
        Ok(Self {
            encoded: base64::engine::general_purpose::STANDARD
                .encode(bincode::serialize(transaction)?),
            minimum,
        })
    }
    pub(crate) fn minimum(&self) -> u64 {
        self.minimum
    }
    pub(crate) fn body(&self, id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"simulateTransaction","params":[self.encoded,
            {"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
            "accounts":null,"minContextSlot":self.minimum,"innerInstructions":true}]})
    }
}

#[cfg(test)]
impl sealed::Sealed for solana_client::nonblocking::rpc_client::RpcClient {}
#[cfg(test)]
impl DammSimulator for solana_client::nonblocking::rpc_client::RpcClient {
    async fn simulate(
        &self,
        transaction: &VersionedTransaction,
        minimum: u64,
    ) -> Result<DammSimulationResult> {
        super::simulate_unsigned_at_slot(self, transaction, minimum).await
    }
}

#[cfg(test)]
pub(crate) mod tests;
