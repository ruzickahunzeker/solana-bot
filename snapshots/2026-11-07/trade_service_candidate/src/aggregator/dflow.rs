use anyhow::{bail, Result};
use async_trait::async_trait;

use super::{
    AggregatorBackend, AggregatorCapabilities, AggregatorQuote, AggregatorQuoteRequest, ProviderId,
};

/// DFlow is a disabled read-only placeholder. A future adapter must implement
/// quote validation before it can advertise quote capability.
#[derive(Debug, Clone, Default)]
pub struct DflowBackend;

impl DflowBackend {
    pub const fn disabled() -> Self {
        Self
    }
}

#[async_trait]
impl AggregatorBackend for DflowBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Dflow
    }

    fn capabilities(&self) -> AggregatorCapabilities {
        AggregatorCapabilities::disabled_read_only()
    }

    async fn quote(&self, _request: &AggregatorQuoteRequest) -> Result<AggregatorQuote> {
        bail!("DFlow quote adapter is disabled")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_provider_advertises_no_quote_or_execution() {
        let provider = DflowBackend::disabled();
        let capabilities = provider.capabilities();
        assert!(!capabilities.supports_quote);
        assert!(capabilities.read_only);
        assert!(!capabilities.supports_build);
        assert!(!capabilities.supports_preflight);
        assert!(!capabilities.supports_submit);
    }

    #[tokio::test]
    async fn disabled_provider_has_no_quote_network_path() {
        let provider = DflowBackend::disabled();
        let request = AggregatorQuoteRequest {
            wallet_id: "wallet".into(),
            wallet_pubkey: "pubkey".into(),
            input_mint: "in".into(),
            output_mint: "out".into(),
            input_amount_units: 1,
            slippage_bps: 100,
            balance_snapshot: None,
        };
        let error = provider
            .quote(&request)
            .await
            .expect_err("DFlow adapter must remain disabled");
        assert!(error.to_string().contains("quote adapter is disabled"));
    }
}
