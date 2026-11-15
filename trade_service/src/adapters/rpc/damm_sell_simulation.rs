use anyhow::{ensure, Context, Result};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSimulateTransactionConfig};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    hash::Hash, message::VersionedMessage, signature::Signature, transaction::VersionedTransaction,
};
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::time::Duration;

pub(crate) struct DammSimulationResult {
    pub(crate) error: Option<String>,
}

/// Fixed read-only RPC method. No signed transaction or broadcast method is accepted.
pub(crate) async fn simulate_unsigned(
    rpc: &RpcClient,
    transaction: &VersionedTransaction,
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
                min_context_slot: None,
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
        result.context.slot > 0,
        "DAMM unsigned simulation returned a zero slot"
    );
    Ok(DammSimulationResult {
        error: result.value.err.map(|error| format!("{error:?}")),
    })
}

#[cfg(test)]
pub(crate) mod tests;
