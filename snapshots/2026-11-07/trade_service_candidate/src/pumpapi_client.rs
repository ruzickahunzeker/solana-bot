use anyhow::{bail, Context};
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::TradingClient;
use solana_client::rpc_config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signer;
use solana_sdk::transaction::VersionedTransaction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::{sleep, Instant};

use crate::config::PumpApiConfig;
use crate::models::{Side, SimulationReport, USD1_MINT, USDC_MINT};
use crate::sanitize::redact_urls;
use crate::sdk_client::ExecuteResult;

pub struct PumpApiTrade {
    pub config: PumpApiConfig,
    pub side: Side,
    pub mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_id: Option<Pubkey>,
    pub amount_units: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_bps: u64,
    pub priority_fee_sol: f64,
    pub simulate: bool,
    pub wait_tx_confirmed: bool,
}

pub async fn execute(client: &TradingClient, trade: PumpApiTrade) -> anyhow::Result<ExecuteResult> {
    if !trade.config.enabled {
        bail!("ENABLE_PUMPAPI_EXECUTOR=true is required for execution_method=pumpapi");
    }
    if trade.config.lightning_enabled {
        bail!("PUMPAPI_LIGHTNING_ENABLED is reserved; this service only supports PumpAPI Local Transaction mode");
    }

    let request = build_request(client, &trade)?;
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_millis(trade.config.timeout_ms))
        .build()
        .context("build PumpAPI http client")?;

    let response = http_client
        .post(&trade.config.url)
        .json(&request)
        .send()
        .await
        .with_context(|| format!("PumpAPI request failed: {}", redact_urls(&trade.config.url)))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("read PumpAPI transaction response")?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        bail!(
            "PumpAPI returned HTTP {status}: {}",
            redact_urls(text.trim())
        );
    }

    let unsigned_transaction = decode_pumpapi_transaction(&body)?;
    let transaction =
        VersionedTransaction::try_new(unsigned_transaction.message, &[client.payer.as_ref()])
            .context("sign PumpAPI transaction locally")?;

    if trade.simulate {
        return simulate(client, transaction).await;
    }

    submit(client, transaction, trade.wait_tx_confirmed).await
}

fn build_request(client: &TradingClient, trade: &PumpApiTrade) -> anyhow::Result<Value> {
    let (action, amount, denominated_in_quote) = match trade.side {
        Side::Buy => (
            "buy",
            pumpapi_buy_amount(
                trade.amount_units.context("buy amount is missing")?,
                &trade.quote_mint,
            )?,
            "true",
        ),
        Side::Sell => {
            let percent = trade.sell_percent.context("sell_percent is missing")?;
            if percent != 100 {
                bail!("PumpAPI first version only supports sell_percent=100");
            }
            ("sell", Value::String("100%".to_string()), "false")
        }
    };

    let mut request = json!({
        "publicKey": client.payer.pubkey().to_string(),
        "action": action,
        "mint": trade.mint.to_string(),
        "quoteMint": trade.quote_mint.to_string(),
        "amount": amount,
        "denominatedInQuote": denominated_in_quote,
        "slippage": trade.slippage_bps as f64 / 100.0,
        "priorityFee": trade.priority_fee_sol
    });
    if let Some(pool_id) = trade.pool_id {
        request["poolId"] = Value::String(pool_id.to_string());
    }
    Ok(request)
}

fn pumpapi_buy_amount(amount_units: u64, quote_mint: &Pubkey) -> anyhow::Result<Value> {
    let decimals = if is_six_decimal_quote(quote_mint) {
        6
    } else {
        9
    };
    let scale = 10_u64.pow(decimals as u32) as f64;
    let amount = amount_units as f64 / scale;
    if amount <= 0.0 || !amount.is_finite() {
        bail!("PumpAPI buy amount must be greater than 0");
    }
    Ok(json!(amount))
}

fn decode_pumpapi_transaction(body: &[u8]) -> anyhow::Result<VersionedTransaction> {
    let trimmed = trim_ascii_whitespace(body);
    if trimmed.is_empty() {
        bail!("PumpAPI returned an empty transaction response");
    }

    let transaction_bytes = match trimmed.first().copied() {
        Some(b'{') => {
            let value: Value =
                serde_json::from_slice(trimmed).context("parse PumpAPI JSON error response")?;
            bail!("PumpAPI returned JSON instead of transaction bytes: {value}");
        }
        Some(b'[') => {
            let transactions: Vec<String> =
                serde_json::from_slice(trimmed).context("parse PumpAPI transaction bundle")?;
            if transactions.len() != 1 {
                bail!(
                    "PumpAPI returned {} transactions; first version supports exactly one",
                    transactions.len()
                );
            }
            decode_base64_transaction(&transactions[0])?
        }
        Some(b'"') => {
            let transaction: String =
                serde_json::from_slice(trimmed).context("parse PumpAPI transaction string")?;
            decode_base64_transaction(&transaction)?
        }
        _ => trimmed.to_vec(),
    };

    bincode::deserialize::<VersionedTransaction>(&transaction_bytes)
        .context("decode PumpAPI unsigned transaction")
}

fn decode_base64_transaction(value: &str) -> anyhow::Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .context("decode PumpAPI base64 transaction")
}

async fn simulate(
    client: &TradingClient,
    transaction: VersionedTransaction,
) -> anyhow::Result<ExecuteResult> {
    let signature = transaction
        .signatures
        .first()
        .map(ToString::to_string)
        .context("PumpAPI transaction has no signature")?;
    let result = client
        .get_rpc()
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
                min_context_slot: None,
                inner_instructions: true,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;

    if let Some(err) = result.value.err {
        return Ok(ExecuteResult {
            success: false,
            signature: Some(signature),
            error: Some(redact_urls(&format!("PumpAPI simulation failed: {err:?}"))),
            simulation: Some(SimulationReport {
                ok: false,
                protocol: "pump".to_string(),
                source: "pumpapi".to_string(),
                units_consumed: result.value.units_consumed,
                logs: result.value.logs.unwrap_or_default(),
                aggregator: None,
            }),
        });
    }

    Ok(ExecuteResult {
        success: true,
        signature: Some(signature),
        error: None,
        simulation: Some(SimulationReport {
            ok: true,
            protocol: "pump".to_string(),
            source: "pumpapi".to_string(),
            units_consumed: result.value.units_consumed,
            logs: result.value.logs.unwrap_or_default(),
            aggregator: None,
        }),
    })
}

async fn submit(
    client: &TradingClient,
    transaction: VersionedTransaction,
    wait_confirmed: bool,
) -> anyhow::Result<ExecuteResult> {
    let signature = client
        .get_rpc()
        .send_transaction_with_config(
            &transaction,
            RpcSendTransactionConfig {
                skip_preflight: true,
                preflight_commitment: Some(CommitmentLevel::Processed),
                encoding: Some(UiTransactionEncoding::Base64),
                max_retries: Some(3),
                min_context_slot: Some(0),
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
                        error: Some(redact_urls(&format!(
                            "PumpAPI submitted transaction landed with error: {err:?}"
                        ))),
                        simulation: None,
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
                    simulation: None,
                });
            }
            sleep(Duration::from_millis(200)).await;
        }
    }

    Ok(ExecuteResult {
        success: true,
        signature: Some(signature.to_string()),
        error: None,
        simulation: None,
    })
}

fn trim_ascii_whitespace(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);
    &value[start..end]
}

fn is_six_decimal_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(USDC_MINT) || *quote_mint == parse_known_pubkey(USD1_MINT)
}

fn parse_known_pubkey(value: &str) -> Pubkey {
    Pubkey::from_str(value).expect("known mint pubkey must be valid")
}
