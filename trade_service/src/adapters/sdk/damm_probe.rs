use anyhow::{bail, Result};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig},
    rpc_response::{Response, RpcSimulateTransactionResult},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    hash::Hash, instruction::Instruction, message::Message, pubkey::Pubkey,
    transaction::Transaction,
};
use solana_transaction_status_client_types::UiTransactionEncoding;

mod buy_plan;
mod evidence;
mod output;
mod plan;
mod quoted_output;
#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use super::evidence::tests::{change_token, clock, key, Fixture};
}
pub(crate) use buy_plan::binding::{DammBuyContext, DammBuyQuote};
pub(crate) use evidence::inspect_prepared_damm_probe_at_slot;
pub(crate) use output::evidence_damm_probe_output;
pub(crate) use plan::prepare_damm_probe;
#[cfg(test)]
use plan::PreparedDammProbe;
pub(crate) use quoted_output::DammOutputEvidence;

fn unsigned_probe(
    wallet: Pubkey,
    instructions: &[Instruction],
    blockhash: Hash,
) -> Result<Transaction> {
    let message = Message::new_with_blockhash(instructions, Some(&wallet), &blockhash);
    if message.header.num_required_signatures != 1 || message.account_keys.first() != Some(&wallet)
    {
        bail!("DAMM v2 quote probe requires exactly one wallet signer");
    }
    Ok(Transaction::new_unsigned(message))
}

/// Public inputs only; never produces a real signature or exposes a send method.
#[cfg(test)]
pub(crate) async fn simulate_damm_probe(
    rpc: &RpcClient,
    wallet: Pubkey,
    instructions: &[Instruction],
    blockhash: Hash,
    output_ata: Pubkey,
    min_context_slot: u64,
) -> Result<Response<RpcSimulateTransactionResult>> {
    let transaction = unsigned_probe(wallet, instructions, blockhash)?;
    simulate_packet(rpc, &transaction, &[output_ata], min_context_slot, false).await
}

#[cfg(test)]
async fn simulate_prepared_damm_probe(
    rpc: &RpcClient,
    prepared: &PreparedDammProbe,
    min_context_slot: u64,
) -> Result<Response<RpcSimulateTransactionResult>> {
    simulate_packet(
        rpc,
        prepared.transaction(),
        &[prepared.output_ata()],
        min_context_slot,
        false,
    )
    .await
}

async fn simulate_packet(
    rpc: &RpcClient,
    transaction: &Transaction,
    accounts: &[Pubkey],
    min_context_slot: u64,
    inner_instructions: bool,
) -> Result<Response<RpcSimulateTransactionResult>> {
    Ok(rpc
        .simulate_transaction_with_config(
            transaction,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: false,
                commitment: Some(CommitmentConfig::processed()),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: Some(RpcSimulateTransactionAccountsConfig {
                    encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                    addresses: accounts.iter().map(ToString::to_string).collect(),
                }),
                min_context_slot: Some(min_context_slot),
                inner_instructions,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", crate::sanitize::redact_urls(&error.to_string())))?)
}

#[cfg(test)]
mod plan_tests;
#[cfg(test)]
mod sdk_oracle_tests;
#[cfg(test)]
mod tests;
