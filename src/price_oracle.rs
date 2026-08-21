use reqwest::Client;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::{info, warn};

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

#[derive(Clone)]
pub struct PriceOracle {
    sol_usd: Arc<RwLock<Option<f64>>>,
    client: Client,
    api_url: String,
    api_key: Option<String>,
    refresh_secs: u64,
}

impl PriceOracle {
    pub fn new(
        initial_sol_usd: Option<f64>,
        api_url: String,
        api_key: Option<String>,
        refresh_secs: u64,
    ) -> Self {
        Self {
            sol_usd: Arc::new(RwLock::new(initial_sol_usd)),
            client: Client::new(),
            api_url,
            api_key,
            refresh_secs,
        }
    }

    pub async fn sol_usd_price(&self) -> Option<f64> {
        *self.sol_usd.read().await
    }

    pub fn start(self) {
        if self.refresh_secs == 0 || self.api_url.trim().is_empty() {
            return;
        }
        tokio::spawn(async move {
            self.refresh_once().await;
            let mut interval = tokio::time::interval(Duration::from_secs(self.refresh_secs));
            loop {
                interval.tick().await;
                self.refresh_once().await;
            }
        });
    }

    async fn refresh_once(&self) {
        match self.fetch_sol_usd().await {
            Ok(price) if price > 0.0 => {
                *self.sol_usd.write().await = Some(price);
                info!(price, "updated SOL/USD price");
            }
            Ok(price) => warn!(price, "ignored invalid SOL/USD price"),
            Err(err) => warn!(?err, "failed to refresh SOL/USD price"),
        }
    }

    async fn fetch_sol_usd(&self) -> anyhow::Result<f64> {
        let url = price_url(&self.api_url);
        let mut request = self.client.get(url);
        if let Some(api_key) = &self.api_key {
            request = request.header("x-api-key", api_key);
        }
        let response = request.send().await?.error_for_status()?;
        let json = response.json::<Value>().await?;
        json.get(SOL_MINT)
            .and_then(|item| item.get("usdPrice"))
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow::anyhow!("Jupiter response missing SOL usdPrice"))
    }
}

fn price_url(base: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}ids={SOL_MINT}")
}
