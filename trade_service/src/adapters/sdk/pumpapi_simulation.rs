use anyhow::{bail, Context};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSimulateTransactionConfig};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};
use solana_transaction_status_client_types::UiTransactionEncoding;

use super::{
    pumpapi_accounts::resolve_unsigned_accounts, pumpapi_pool_accounts::check_pool_accounts,
    pumpapi_transaction::validate_packet_structure,
};
use crate::{
    application::{trade_command::TradeCommand, trade_execution::TradeExecutionResult},
    domain::trade::Address,
    sanitize::redact_urls,
};

/// A wallet-bound preview, not a signing authorization or a broadcastable artifact.
pub(crate) async fn simulate_unsigned(
    rpc: &RpcClient,
    wallet: Address,
    transaction: VersionedTransaction,
    command: &TradeCommand,
) -> anyhow::Result<TradeExecutionResult> {
    validate_packet_structure(&transaction)?;
    if transaction.message.header().num_required_signatures != 1
        || transaction.signatures.len() != 1
        || transaction.signatures[0] != Signature::default()
    {
        bail!("PumpAPI simulation requires exactly one unsigned wallet signature slot");
    }
    let payer = transaction
        .message
        .static_account_keys()
        .first()
        .context("PumpAPI simulation transaction has no payer")?;
    if payer.to_bytes() != wallet.to_bytes() {
        bail!("PumpAPI simulation payer does not match request wallet");
    }
    let account_view = resolve_unsigned_accounts(rpc, wallet, &transaction).await?;
    let pool_slot = check_pool_accounts(rpc, wallet, command, &transaction, &account_view).await?;
    account_view.ensure_bound(wallet, &transaction)?;
    let result = rpc
        .simulate_transaction_with_config(
            &transaction,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: false,
                commitment: Some(CommitmentConfig {
                    commitment: CommitmentLevel::Processed,
                }),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: None,
                min_context_slot: pool_slot.or(account_view.lookup_slot()),
                inner_instructions: true,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;

    Ok(TradeExecutionResult {
        success: result.value.err.is_none(),
        signature: None,
        error: result
            .value
            .err
            .map(|error| format!("PumpAPI simulation failed: {error:?}")),
    })
}
