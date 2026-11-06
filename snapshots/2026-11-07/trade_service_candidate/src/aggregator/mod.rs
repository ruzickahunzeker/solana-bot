//! Provider-neutral aggregator boundary.
//!
//! The legacy `aggregator_quote` module remains the compatibility endpoint for
//! the existing Jupiter SOL-buy API. New selection, compare and persistence
//! code should depend on this module instead of Jupiter-specific JSON.

use anyhow::{bail, Result};
use async_trait::async_trait;

pub mod dflow;
pub mod jupiter;
pub mod types;

pub use dflow::DflowBackend;
pub use jupiter::JupiterBackend;
pub use types::{
    AggregatorCapabilities, AggregatorQuote, AggregatorQuoteRequest, AggregatorRouteLeg,
    PreflightReport, ProviderId, UnsignedTransaction, WalletBalanceSnapshot,
};

#[async_trait]
pub trait AggregatorBackend: Send + Sync {
    fn provider(&self) -> ProviderId;
    fn capabilities(&self) -> AggregatorCapabilities;

    async fn quote(&self, request: &AggregatorQuoteRequest) -> Result<AggregatorQuote>;

    async fn build(
        &self,
        _quote: &AggregatorQuote,
        _wallet_pubkey: &str,
    ) -> Result<UnsignedTransaction> {
        bail!(
            "aggregator provider {} does not support transaction build",
            self.provider()
        )
    }

    async fn preflight(&self, _transaction: &UnsignedTransaction) -> Result<PreflightReport> {
        bail!(
            "aggregator provider {} does not support transaction preflight",
            self.provider()
        )
    }

    async fn submit(
        &self,
        _transaction: &UnsignedTransaction,
        _idempotency_key: &str,
    ) -> Result<String> {
        bail!(
            "aggregator provider {} does not support transaction submit",
            self.provider()
        )
    }
}

#[derive(Default)]
pub struct AggregatorRegistry {
    providers: Vec<std::sync::Arc<dyn AggregatorBackend>>,
}

impl AggregatorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, provider: std::sync::Arc<dyn AggregatorBackend>) {
        self.providers
            .retain(|existing| existing.provider() != provider.provider());
        self.providers.push(provider);
    }

    pub fn get(&self, provider: ProviderId) -> Option<std::sync::Arc<dyn AggregatorBackend>> {
        self.providers
            .iter()
            .find(|candidate| candidate.provider() == provider)
            .cloned()
    }

    pub fn capabilities(&self) -> Vec<(ProviderId, AggregatorCapabilities)> {
        self.providers
            .iter()
            .map(|provider| (provider.provider(), provider.capabilities()))
            .collect()
    }

    pub fn providers(&self) -> Vec<std::sync::Arc<dyn AggregatorBackend>> {
        self.providers.clone()
    }

    pub async fn quote(
        &self,
        provider: ProviderId,
        request: &AggregatorQuoteRequest,
    ) -> Result<AggregatorQuote> {
        let backend = self
            .get(provider)
            .ok_or_else(|| anyhow::anyhow!("provider {provider} is not registered"))?;
        if !backend.capabilities().supports_quote {
            bail!("provider {provider} quote capability is disabled");
        }
        backend.quote(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_replaces_provider_by_id() {
        let mut registry = AggregatorRegistry::new();
        registry.register(std::sync::Arc::new(DflowBackend::disabled()));
        registry.register(std::sync::Arc::new(DflowBackend::disabled()));
        assert_eq!(registry.capabilities().len(), 1);
        assert!(registry.get(ProviderId::Dflow).is_some());
    }

    #[tokio::test]
    async fn read_only_provider_cannot_build_or_submit() {
        let provider = DflowBackend::disabled();
        let request = AggregatorQuoteRequest {
            wallet_id: "w".into(),
            wallet_pubkey: "wallet".into(),
            input_mint: "in".into(),
            output_mint: "out".into(),
            input_amount_units: 1,
            slippage_bps: 100,
            balance_snapshot: None,
        };
        assert!(provider.quote(&request).await.is_err());
        let quote = AggregatorQuote {
            provider: ProviderId::Dflow,
            provider_quote_id: "q".into(),
            wallet_id: "w".into(),
            wallet_pubkey: "wallet".into(),
            input_mint: "in".into(),
            output_mint: "out".into(),
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
        assert!(provider.build(&quote, "wallet").await.is_err());
    }

    #[tokio::test]
    async fn registry_rejects_quote_for_disabled_provider_without_calling_endpoint() {
        let mut registry = AggregatorRegistry::new();
        registry.register(std::sync::Arc::new(DflowBackend::disabled()));
        let request = AggregatorQuoteRequest {
            wallet_id: "w".into(),
            wallet_pubkey: "wallet".into(),
            input_mint: "in".into(),
            output_mint: "out".into(),
            input_amount_units: 1,
            slippage_bps: 100,
            balance_snapshot: None,
        };
        let error = registry
            .quote(ProviderId::Dflow, &request)
            .await
            .expect_err("disabled provider must be rejected");
        assert!(error.to_string().contains("quote capability is disabled"));
    }
}
