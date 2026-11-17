use super::{SignedWsolTransaction, UnsignedWsolTransaction};
use crate::{
    adapters::rpc::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime},
    application::{wsol_failure::WsolFailure, wsol_submission_types::WsolQuoteSnapshot},
};
use anyhow::{bail, ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use solana_client::rpc_response::RpcSimulateTransactionResult;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::transaction::Transaction;

// Constructed only after the original unsigned authorization or signed snapshot checks.
pub(crate) struct WsolSimulationPacket {
    packet: String,
    commitment: CommitmentConfig,
}
impl WsolSimulationPacket {
    fn encode(transaction: &Transaction, commitment: CommitmentConfig) -> Result<Self> {
        ensure!(
            (1..=2).contains(&transaction.message.header.num_required_signatures)
                && transaction.signatures.len()
                    == usize::from(transaction.message.header.num_required_signatures),
            "WSOL simulation signer count is invalid"
        );
        ensure!(
            bincode::serialized_size(transaction)? <= 1232,
            "WSOL simulation packet exceeds limit"
        );
        Ok(Self {
            packet: base64::engine::general_purpose::STANDARD
                .encode(bincode::serialize(transaction)?),
            commitment,
        })
    }
    fn unsigned(
        prepared: &UnsignedWsolTransaction,
        quote: &WsolQuoteSnapshot,
        commitment: CommitmentConfig,
    ) -> Result<Self> {
        prepared.validate(
            &prepared.owner,
            quote,
            crate::application::wsol_submission::current_time_ms()?,
        )?;
        Self::encode(
            &Transaction::new_unsigned(prepared.message.clone()),
            commitment,
        )
    }
    fn signed(signed: &SignedWsolTransaction, commitment: CommitmentConfig) -> Result<Self> {
        signed.validate_snapshot(&signed.owner)?;
        Self::encode(&signed.transaction, commitment)
    }
    pub(crate) fn body(&self, id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"simulateTransaction","params":[self.packet,{
            "sigVerify":false,"replaceRecentBlockhash":false,"commitment":self.commitment.commitment,
            "encoding":"base64","accounts":null,"minContextSlot":null,"innerInstructions":false
        }]})
    }
}

pub(crate) async fn simulate_unsigned<T: ReadHttpTransport>(
    prepared: &UnsignedWsolTransaction,
    quote: &WsolQuoteSnapshot,
    runtime: &SharedReadRuntime<T>,
    url: &str,
    commitment: CommitmentConfig,
) -> Result<RpcSimulateTransactionResult> {
    let packet = WsolSimulationPacket::unsigned(prepared, quote, commitment)?;
    let simulation = runtime.wsol_simulation(url, packet).await?;
    if let Some(err) = simulation.err.as_ref() {
        bail!(WsolFailure::rejected(format!(
            "WSOL conversion simulation failed: {err}"
        )));
    }
    Ok(simulation)
}
pub(crate) async fn simulate_signed<T: ReadHttpTransport>(
    signed: &SignedWsolTransaction,
    runtime: &SharedReadRuntime<T>,
    url: &str,
    commitment: CommitmentConfig,
) -> Result<RpcSimulateTransactionResult> {
    let packet = WsolSimulationPacket::signed(signed, commitment)?;
    runtime.wsol_simulation(url, packet).await
}

#[cfg(test)]
pub(crate) mod tests;
