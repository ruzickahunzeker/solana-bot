#[cfg(test)]
use anyhow::{ensure, Context, Result};
#[cfg(test)]
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSimulateTransactionConfig};
#[cfg(test)]
use solana_commitment_config::CommitmentConfig;
#[cfg(test)]
use solana_sdk::{
    hash::Hash, message::VersionedMessage, signature::Signature, transaction::VersionedTransaction,
};
#[cfg(test)]
use solana_transaction_status_client_types::UiTransactionEncoding;
#[cfg(test)]
use std::time::Duration;

pub(crate) mod shared;

pub(crate) struct DammSimulationResult {
    pub(crate) error: Option<String>,
}

/// Fixed read-only RPC method. No signed transaction or broadcast method is accepted.
#[cfg(test)]
pub(crate) async fn simulate_unsigned_at_slot(
    rpc: &RpcClient,
    transaction: &VersionedTransaction,
    min_context_slot: u64,
) -> Result<DammSimulationResult> {
    ensure!(
        min_context_slot > 0,
        "DAMM simulation requires its output quote slot"
    );
    simulate(rpc, transaction, Some(min_context_slot)).await
}

#[cfg(test)]
async fn simulate_unsigned(
    rpc: &RpcClient,
    transaction: &VersionedTransaction,
) -> Result<DammSimulationResult> {
    simulate(rpc, transaction, None).await
}

#[cfg(test)]
async fn simulate(
    rpc: &RpcClient,
    transaction: &VersionedTransaction,
    min_context_slot: Option<u64>,
) -> Result<DammSimulationResult> {
    ensure!(
        transaction.signatures == [Signature::default()]
            && matches!(transaction.message, VersionedMessage::Legacy(_))
            && transaction.message.header().num_required_signatures == 1
            && transaction
                .message
                .static_account_keys()
                .first()
                .is_some_and(|key| *key != solana_sdk::pubkey::Pubkey::default())
            && *transaction.message.recent_blockhash() != Hash::default()
            && bincode::serialized_size(transaction)? <= 1232,
        "DAMM simulation requires a bounded single-payer zero-signature candidate"
    );
    let result = tokio::time::timeout(
        Duration::from_secs(4),
        rpc.simulate_transaction_with_config(
            transaction,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: false,
                commitment: Some(CommitmentConfig::processed()),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: None,
                min_context_slot,
                inner_instructions: true,
            },
        ),
    )
    .await
    .context("DAMM unsigned simulation timed out")?
    .map_err(|error| {
        anyhow::anyhow!(
            "DAMM unsigned simulation RPC unavailable: {}",
            crate::sanitize::redact_urls(&error.to_string())
        )
    })?;
    ensure!(
        result.context.slot > 0
            && min_context_slot.is_none_or(|minimum| result.context.slot >= minimum),
        "DAMM unsigned simulation returned a zero slot or precedes its output quote"
    );
    ensure!(
        result.value.replacement_blockhash.is_none()
            && result
                .value
                .loaded_addresses
                .as_ref()
                .is_none_or(|loaded| { loaded.writable.is_empty() && loaded.readonly.is_empty() }),
        "DAMM unsigned simulation changed the fixed candidate context"
    );
    Ok(DammSimulationResult {
        error: result.value.err.map(|error| format!("{error:?}")),
    })
}

#[cfg(test)]
pub(crate) mod tests;
