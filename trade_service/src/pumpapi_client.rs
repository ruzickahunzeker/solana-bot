use anyhow::{bail, Context};
use sol_trade_sdk::TradingClient;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig};
use solana_commitment_config::CommitmentLevel;
use solana_sdk::signature::Signer;
use solana_sdk::transaction::VersionedTransaction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::time::Duration;
use tokio::time::{sleep, Instant};

use crate::adapters::{
    pumpapi::{pumpapi_request, request::build_request, transport::fetch_transaction},
    sdk::{
        pumpapi_accounts::resolve_unsigned_accounts, pumpapi_pool_accounts::check_pool_accounts,
        pumpapi_simulation::simulate_unsigned, pumpapi_transaction::decode_pumpapi_transaction,
    },
};
use crate::application::trade_execution::TradeExecutionResult as ExecuteResult;
use crate::domain::execution_policy::BroadcastPolicy;
use crate::domain::trade::Address;
use crate::sanitize::redact_urls;

pub use crate::adapters::pumpapi::execution::PumpApiTrade;

async fn prepare_local_transaction(
    wallet: Address,
    trade: &PumpApiTrade,
) -> anyhow::Result<VersionedTransaction> {
    if !trade.config().enabled {
        bail!("ENABLE_PUMPAPI_EXECUTOR=true is required for execution_method=pumpapi");
    }
    if trade.config().lightning_enabled {
        bail!("PUMPAPI_LIGHTNING_ENABLED is reserved; this service only supports PumpAPI Local Transaction mode");
    }
    let request = build_request(&pumpapi_request(wallet, trade))?;
    let body = fetch_transaction(trade.config(), &request).await?;
    decode_pumpapi_transaction(&body)
}

pub(crate) async fn simulate_trade(
    rpc: &RpcClient,
    wallet: Address,
    trade: PumpApiTrade,
) -> anyhow::Result<ExecuteResult> {
    if !trade.simulate() {
        bail!("PumpAPI unsigned simulation requires simulate=true");
    }
    let transaction = prepare_local_transaction(wallet, &trade).await?;
    simulate_unsigned(rpc, wallet, transaction, trade.command()).await
}

pub(crate) async fn execute_live(
    client: &TradingClient,
    trade: PumpApiTrade,
    broadcast_policy: BroadcastPolicy,
) -> anyhow::Result<ExecuteResult> {
    broadcast_policy.ensure_allowed()?;
    if trade.simulate() {
        bail!("PumpAPI Live execution requires simulate=false");
    }
    let wallet = Address::from_bytes(client.payer.pubkey().to_bytes());
    let unsigned_transaction = prepare_local_transaction(wallet, &trade).await?;
    let account_view =
        resolve_unsigned_accounts(client.get_rpc(), wallet, &unsigned_transaction).await?;
    let pool_slot = check_pool_accounts(
        client.get_rpc(),
        wallet,
        trade.command(),
        &unsigned_transaction,
        &account_view,
    )
    .await?;
    account_view.ensure_bound(wallet, &unsigned_transaction)?;
    let transaction =
        VersionedTransaction::try_new(unsigned_transaction.message, &[client.payer.as_ref()])
            .context("sign PumpAPI transaction locally")?;

    submit(
        client,
        transaction,
        trade.wait_tx_confirmed(),
        broadcast_policy,
        pool_slot.or(account_view.lookup_slot()),
    )
    .await
}

async fn submit(
    client: &TradingClient,
    transaction: VersionedTransaction,
    wait_confirmed: bool,
    broadcast_policy: BroadcastPolicy,
    lookup_slot: Option<u64>,
) -> anyhow::Result<ExecuteResult> {
    broadcast_policy.ensure_allowed()?;
    let signature = client
        .get_rpc()
        .send_transaction_with_config(
            &transaction,
            RpcSendTransactionConfig {
                skip_preflight: true,
                preflight_commitment: Some(CommitmentLevel::Processed),
                encoding: Some(UiTransactionEncoding::Base64),
                max_retries: Some(3),
                min_context_slot: Some(lookup_slot.unwrap_or(0)),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;

    if wait_confirmed {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let statuses = client
                .get_rpc()
                .get_signature_statuses_with_history(&[signature])
                .await
                .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
            if let Some(status) = statuses.value.into_iter().next().flatten() {
                if let Some(err) = status.err {
                    return Ok(ExecuteResult {
                        success: false,
                        signature: Some(signature.to_string()),
                        error: Some(format!(
                            "PumpAPI submitted transaction landed with error: {err:?}"
                        )),
                    });
                }
                break;
            }
            if Instant::now() >= deadline {
                return Ok(ExecuteResult {
                    success: false,
                    signature: Some(signature.to_string()),
                    error: Some(
                        "PumpAPI submitted transaction was not visible after 30s".to_string(),
                    ),
                });
            }
            sleep(Duration::from_millis(200)).await;
        }
    }

    Ok(ExecuteResult {
        success: true,
        signature: Some(signature.to_string()),
        error: None,
    })
}

#[cfg(test)]
mod tests;
