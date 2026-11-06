use anyhow::{Context, Result};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    AggregatorBackend, AggregatorCapabilities, AggregatorQuote, AggregatorQuoteRequest,
    AggregatorRouteLeg, ProviderId,
};
use crate::aggregator_quote::{self, SolBuyQuoteRequest};
use crate::models::WSOL_MINT;

#[derive(Debug, Clone, Copy, Default)]
pub struct JupiterBackend;

impl JupiterBackend {
    pub const fn new() -> Self {
        Self
    }

    fn request_hash(request: &AggregatorQuoteRequest) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(request)?)
        ))
    }
}

#[async_trait]
impl AggregatorBackend for JupiterBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Jupiter
    }

    fn capabilities(&self) -> AggregatorCapabilities {
        // The compatibility implementation has quote and local audit code,
        // but Gate 2 is incomplete, so it is intentionally read-only here.
        AggregatorCapabilities::read_only_quote()
    }

    async fn quote(&self, request: &AggregatorQuoteRequest) -> Result<AggregatorQuote> {
        if request.wallet_id.trim().is_empty() {
            anyhow::bail!("wallet_id is required");
        }
        if request
            .wallet_pubkey
            .parse::<solana_sdk::pubkey::Pubkey>()
            .is_err()
        {
            anyhow::bail!("wallet_pubkey must be a valid Solana public key");
        }
        if request
            .output_mint
            .parse::<solana_sdk::pubkey::Pubkey>()
            .is_err()
        {
            anyhow::bail!("output_mint must be a valid Solana public key");
        }
        if request.input_amount_units == 0 {
            anyhow::bail!("input_amount_units must be greater than zero");
        }
        if request.slippage_bps == 0 || request.slippage_bps > 3_000 {
            anyhow::bail!("slippage_bps must be between 1 and 3000");
        }
        if request.input_mint != WSOL_MINT {
            anyhow::bail!("Jupiter compatibility adapter currently requires WSOL input");
        }
        let balance_snapshot = request
            .balance_snapshot
            .as_ref()
            .context("wallet balance snapshot is required for Jupiter quote")?;
        let snapshot_age_ms = crate::aggregator_quote::current_time_ms()
            .checked_sub(balance_snapshot.fetched_at_ms)
            .context("wallet balance snapshot is from the future")?;
        if snapshot_age_ms > 5_000 {
            anyhow::bail!("wallet balance snapshot is stale");
        }
        let quote = aggregator_quote::quote(&SolBuyQuoteRequest {
            wallet_id: request.wallet_id.clone(),
            mint: request.output_mint.clone(),
            amount_lamports: request.input_amount_units,
            slippage_bps: request.slippage_bps,
        })
        .await?;
        let route: Vec<AggregatorRouteLeg> = quote
            .route
            .into_iter()
            .map(|leg| AggregatorRouteLeg {
                label: leg.label,
                input_mint: leg.input_mint,
                output_mint: leg.output_mint,
            })
            .collect();
        let route_fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&route)?));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before unix epoch")?
            .as_millis() as u64;
        Ok(AggregatorQuote {
            provider: ProviderId::Jupiter,
            provider_quote_id: format!("jupiter:{route_fingerprint}"),
            wallet_id: request.wallet_id.clone(),
            wallet_pubkey: request.wallet_pubkey.clone(),
            input_mint: quote.input_mint,
            output_mint: quote.output_mint,
            input_amount_units: quote.amount_lamports,
            expected_output_units: quote.expected_output_units,
            minimum_output_units: quote.minimum_output_units,
            slippage_bps: request.slippage_bps,
            route_fingerprint,
            request_hash: Self::request_hash(request)?,
            created_at_ms: now,
            expires_at_ms: now.saturating_add(15_000),
            price_impact_percent: Some(quote.price_impact_percent),
            route,
            balance_snapshot: request.balance_snapshot.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregator::{AggregatorBackend, UnsignedTransaction};

    #[tokio::test]
    async fn gate_two_keeps_jupiter_quote_only() {
        let provider = JupiterBackend::new();
        let capabilities = provider.capabilities();
        assert!(capabilities.supports_quote);
        assert!(capabilities.read_only);
        assert!(!capabilities.supports_build);
        assert!(!capabilities.supports_preflight);
        assert!(!capabilities.supports_submit);

        let quote = AggregatorQuote {
            provider: ProviderId::Jupiter,
            provider_quote_id: "jupiter:test".into(),
            wallet_id: "wallet".into(),
            wallet_pubkey: "pubkey".into(),
            input_mint: WSOL_MINT.into(),
            output_mint: "mint".into(),
            input_amount_units: 1,
            expected_output_units: 1,
            minimum_output_units: 1,
            slippage_bps: 100,
            route_fingerprint: "route".into(),
            request_hash: "request".into(),
            created_at_ms: 1,
            expires_at_ms: 2,
            price_impact_percent: None,
            route: Vec::new(),
            balance_snapshot: None,
        };
        let transaction = UnsignedTransaction {
            bytes: Vec::new(),
            message_hash: "message".into(),
        };
        assert!(provider.build(&quote, "pubkey").await.is_err());
        assert!(provider.preflight(&transaction).await.is_err());
        assert!(provider.submit(&transaction, "idempotency").await.is_err());
    }

    #[tokio::test]
    async fn quote_rejects_unbound_or_invalid_requests_before_network() {
        let provider = JupiterBackend::new();
        let request = AggregatorQuoteRequest {
            wallet_id: String::new(),
            wallet_pubkey: "invalid".into(),
            input_mint: WSOL_MINT.into(),
            output_mint: "invalid".into(),
            input_amount_units: 0,
            slippage_bps: 0,
            balance_snapshot: None,
        };
        let error = provider
            .quote(&request)
            .await
            .expect_err("request must fail");
        assert!(error.to_string().contains("wallet_id is required"));
    }

    #[tokio::test]
    async fn quote_rejects_missing_and_stale_balance_before_network() {
        let provider = JupiterBackend::new();
        let mut request = AggregatorQuoteRequest {
            wallet_id: "wallet".into(),
            wallet_pubkey: solana_sdk::pubkey::Pubkey::new_unique().to_string(),
            input_mint: WSOL_MINT.into(),
            output_mint: crate::models::USDC_MINT.into(),
            input_amount_units: 1_000_000,
            slippage_bps: 100,
            balance_snapshot: None,
        };
        assert!(provider
            .quote(&request)
            .await
            .unwrap_err()
            .to_string()
            .contains("balance snapshot is required"));
        request.balance_snapshot = Some(crate::aggregator::WalletBalanceSnapshot {
            balance_lamports: 2_000_000,
            fetched_at_ms: crate::aggregator_quote::current_time_ms() - 10_000,
        });
        assert!(provider
            .quote(&request)
            .await
            .unwrap_err()
            .to_string()
            .contains("balance snapshot is stale"));
    }
}
