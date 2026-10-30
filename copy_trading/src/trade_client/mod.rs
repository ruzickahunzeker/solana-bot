#![allow(dead_code)]

use anyhow::{Context, Result};
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct TradeServiceClient {
    base_url: String,
    token: Option<String>,
    client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRequest {
    pub wallet_id: Option<String>,
    pub execution_method: String,
    pub side: String,
    pub pool: String,
    pub mint: String,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
    pub amount_quote_units: Option<u64>,
    pub settlement_asset: Option<String>,
    pub sol_hop_pool: Option<String>,
    pub sol_hop_pool_id: Option<String>,
    pub amount_sol: Option<f64>,
    pub sell_percent: Option<u64>,
    pub slippage_percent: f64,
    pub tip_sol: f64,
    pub use_mev: bool,
    pub mev_provider: Option<String>,
    pub simulate: bool,
    pub source: Option<String>,
    pub source_signature: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResponse {
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub wallet_id: Option<String>,
    #[serde(default)]
    pub wallet_pubkey: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub duplicate: bool,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedPoolRoute {
    pub mint: String,
    pub pool: String,
    pub pool_id: Option<String>,
    pub quote_mint: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SolHopRoute {
    pub available: bool,
    pub pool: Option<String>,
    pub pool_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Serialize)]
struct SolHopRequest<'a> {
    quote_mint: &'a str,
}

#[derive(Debug, Serialize)]
struct ResolvePoolRequest<'a> {
    mint: &'a str,
}

#[derive(Debug, Serialize)]
struct WarmupRouteRequest<'a> {
    pool: &'a str,
    mint: &'a str,
    pool_id: Option<&'a str>,
    quote_mint: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeServiceWallet {
    pub wallet_id: String,
    pub pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletsResponse {
    pub default_wallet_id: String,
    pub wallets: Vec<TradeServiceWallet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub default_wallet_id: String,
    pub wallets: Vec<TradeServiceWallet>,
    #[serde(default)]
    pub rpc_ok: bool,
    #[serde(default)]
    pub has_blockhash: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedWalletResponse {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub pubkey: String,
    pub is_default: bool,
    pub enabled: bool,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedWalletsResponse {
    pub wallets: Vec<ManagedWalletResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletBalanceItem {
    pub wallet_id: String,
    pub pubkey: String,
    pub balance_lamports: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletBalancesResponse {
    pub fetched_at_ms: u128,
    pub wallets: Vec<WalletBalanceItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuoteBalanceResponse {
    pub quote_mint: String,
    pub decimals: u8,
    pub balance_units: u64,
    pub balance_lamports: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WsolConversionQuote {
    pub quote_id: String,
    pub direction: String,
    pub amount_lamports: u64,
    pub source_balance_lamports: u64,
    pub estimated_network_fee_lamports: u64,
    pub temporary_rent_lamports: u64,
    pub rent_refund_lamports: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WsolConversionPreview {
    pub quote: WsolConversionQuote,
    pub executable: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WsolConversionSimulation {
    pub status: String,
    pub quote_id: String,
    pub network_fee_lamports: Option<u64>,
    pub units_consumed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolBuyQuoteResponse {
    pub quote_id: String,
    pub wallet_id: String,
    pub amount_lamports: u64,
    pub expected_output_units: u64,
    pub minimum_output_units: u64,
    pub expires_at_ms: u64,
    pub route: Vec<SolBuyQuoteLeg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolBuyQuoteLeg {
    pub label: String,
    pub input_mint: String,
    pub output_mint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolBuyPreflightResponse {
    pub quote_id: String,
    pub executable: bool,
    pub local_simulation_ok: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolBuySubmitResponse {
    pub quote_id: String,
    pub wallet_id: String,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub preflight: SolBuyPreflightResponse,
}

#[derive(Debug, Serialize)]
struct SolBuyQuoteRequest<'a> {
    wallet_id: &'a str,
    mint: &'a str,
    amount_lamports: u64,
    slippage_bps: u16,
}

#[derive(Debug, Serialize)]
struct SolBuySubmitRequest<'a> {
    wallet_id: &'a str,
    confirmed: bool,
    simulate: bool,
    idempotency_key: &'a str,
}

#[derive(Serialize)]
struct WsolConversionQuoteRequest<'a> {
    wallet_id: &'a str,
    owner_type: &'a str,
    owner_id: &'a str,
    direction: &'a str,
    amount_lamports: Option<u64>,
    all: bool,
}

#[derive(Serialize)]
struct WsolConversionSimulationRequest<'a> {
    quote_id: &'a str,
    wallet_id: &'a str,
    owner_type: &'a str,
    owner_id: &'a str,
    simulate: bool,
    idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VanityJobResponse {
    pub job_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub match_side: String,
    pub pattern: String,
    pub status: String,
    pub attempts: u64,
    pub max_attempts: u64,
    pub wallet_id: Option<String>,
    pub pubkey: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateVanityJobRequest<'a> {
    owner_type: &'a str,
    owner_id: String,
    label: &'a str,
    match_side: &'a str,
    pattern: &'a str,
}

#[derive(Debug, Serialize)]
pub struct SolTransferRequest<'a> {
    pub owner_type: &'a str,
    pub owner_id: String,
    pub to_wallet_id: Option<&'a str>,
    pub to_address: Option<&'a str>,
    pub amount_mode: &'a str,
    pub amount_lamports: Option<u64>,
    pub percent: Option<u64>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolTransferResponse {
    pub transfer_id: String,
    pub wallet_id: String,
    pub from_pubkey: String,
    pub to_pubkey: String,
    pub status: String,
    pub amount_lamports: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub balance_before_lamports: Option<u64>,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
}

#[derive(Debug, Serialize)]
struct CreateWalletRequest<'a> {
    owner_type: &'a str,
    owner_id: String,
    label: &'a str,
    private_key_base58: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct UpdateWalletRequest<'a> {
    label: &'a str,
}

#[derive(Debug, Deserialize)]
struct ExportWalletResponse {
    private_key_base58: String,
}

impl TradeServiceClient {
    pub fn new(base_url: String, token: Option<String>, timeout_secs: u64) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .build()
            .context("failed to build trade_service HTTP client")?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            client,
        })
    }

    pub async fn health(&self) -> Result<String> {
        let response = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .context("failed to call trade_service health")?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("trade_service health returned {status}: {text}");
        }
        Ok(text)
    }

    pub async fn submit_trade(&self, request: &TradeRequest) -> Result<TradeResponse> {
        let response = self
            .authorized(self.client.post(format!("{}/v1/trades", self.base_url)))
            .json(request)
            .send()
            .await
            .context("failed to submit trade")?;
        self.decode_response(response, "submit trade").await
    }

    pub async fn sol_buy_quote(
        &self,
        wallet_id: &str,
        mint: &str,
        amount_lamports: u64,
        slippage_bps: u16,
    ) -> Result<SolBuyQuoteResponse> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/quotes/sol-buy", self.base_url)),
            )
            .json(&SolBuyQuoteRequest {
                wallet_id,
                mint,
                amount_lamports,
                slippage_bps,
            })
            .send()
            .await
            .context("failed to request SOL buy quote")?;
        self.decode_response(response, "request SOL buy quote")
            .await
    }

    pub async fn sol_buy_preflight(
        &self,
        quote_id: &str,
        wallet_id: &str,
    ) -> Result<SolBuyPreflightResponse> {
        let response = self
            .authorized(self.client.post(format!(
                "{}/v1/quotes/sol-buy/{quote_id}/preflight",
                self.base_url
            )))
            .query(&[("wallet_id", wallet_id)])
            .send()
            .await
            .context("failed to preflight SOL buy quote")?;
        self.decode_response(response, "preflight SOL buy quote")
            .await
    }

    pub async fn submit_sol_buy(
        &self,
        quote_id: &str,
        wallet_id: &str,
        confirmed: bool,
        simulate: bool,
        idempotency_key: &str,
    ) -> Result<SolBuySubmitResponse> {
        let response = self
            .authorized(self.client.post(format!(
                "{}/v1/quotes/sol-buy/{quote_id}/submit",
                self.base_url
            )))
            .json(&SolBuySubmitRequest {
                wallet_id,
                confirmed,
                simulate,
                idempotency_key,
            })
            .send()
            .await
            .context("failed to submit SOL buy")?;
        self.decode_response(response, "submit SOL buy").await
    }

    pub async fn resolve_pool(&self, mint: &str) -> Result<ResolvedPoolRoute> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/pools/resolve", self.base_url)),
            )
            .json(&ResolvePoolRequest { mint })
            .send()
            .await
            .context("failed to resolve mint pool")?;
        self.decode_response(response, "resolve mint pool").await
    }

    pub async fn warmup_route(&self, route: &ResolvedPoolRoute) -> Result<()> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/cache/warmup", self.base_url)),
            )
            .json(&WarmupRouteRequest {
                pool: &route.pool,
                mint: &route.mint,
                pool_id: route.pool_id.as_deref(),
                quote_mint: Some(&route.quote_mint),
            })
            .send()
            .await
            .context("failed to verify pool route")?;
        let _: serde_json::Value = self.decode_response(response, "verify pool route").await?;
        Ok(())
    }

    pub async fn resolve_sol_hop(&self, quote_mint: &str) -> Result<SolHopRoute> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/pools/sol-hop", self.base_url)),
            )
            .json(&SolHopRequest { quote_mint })
            .send()
            .await
            .context("failed to resolve SOL hop")?;
        self.decode_response(response, "resolve SOL hop").await
    }

    pub async fn get_trade(&self, request_id: &str) -> Result<TradeResponse> {
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/v1/trades/{request_id}", self.base_url)),
            )
            .send()
            .await
            .context("failed to query trade")?;
        self.decode_response(response, "query trade").await
    }

    pub async fn wallets(&self) -> Result<WalletsResponse> {
        let response = self
            .authorized(self.client.get(format!("{}/v1/wallets", self.base_url)))
            .send()
            .await
            .context("failed to query trade_service wallets")?;
        self.decode_response(response, "query wallets").await
    }

    pub async fn ready(&self) -> Result<ReadyResponse> {
        let response = self
            .authorized(self.client.get(format!("{}/ready", self.base_url)))
            .send()
            .await
            .context("failed to query trade_service readiness")?;
        self.decode_response(response, "query readiness").await
    }

    pub async fn wallet_id_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self
            .wallets()
            .await?
            .wallets
            .into_iter()
            .find(|wallet| wallet.pubkey == pubkey)
            .map(|wallet| wallet.wallet_id))
    }

    pub async fn create_wallet(&self, user_id: i64, label: &str) -> Result<ManagedWalletResponse> {
        self.create_or_import_wallet(user_id, label, None).await
    }

    pub async fn managed_wallets(&self) -> Result<Vec<ManagedWalletResponse>> {
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/v1/wallets/manage", self.base_url)),
            )
            .send()
            .await
            .context("failed to query managed trade_service wallets")?;
        let response: ManagedWalletsResponse = self
            .decode_response(response, "query managed wallets")
            .await?;
        Ok(response.wallets)
    }

    pub async fn wsol_conversion_quote(
        &self,
        wallet_id: &str,
        owner_type: &str,
        owner_id: &str,
        direction: &str,
        amount_lamports: Option<u64>,
        all: bool,
    ) -> Result<WsolConversionPreview> {
        let request = WsolConversionQuoteRequest {
            wallet_id,
            owner_type,
            owner_id,
            direction,
            amount_lamports,
            all,
        };
        let response = self
            .authorized(self.client.post(format!(
                "{}/internal/conversions/wsol/quotes",
                self.base_url
            )))
            .json(&request)
            .send()
            .await
            .context("failed to request WSOL conversion quote")?;
        self.decode_response(response, "request WSOL conversion quote")
            .await
    }

    pub async fn simulate_wsol_conversion(
        &self,
        quote_id: &str,
        wallet_id: &str,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<WsolConversionSimulation> {
        let request = WsolConversionSimulationRequest {
            quote_id,
            wallet_id,
            owner_type,
            owner_id,
            simulate: true,
            idempotency_key: format!("tg-wsol-sim-{}", uuid::Uuid::new_v4()),
        };
        let response = self
            .authorized(self.client.post(format!(
                "{}/internal/conversions/wsol/simulate",
                self.base_url
            )))
            .json(&request)
            .send()
            .await
            .context("failed to simulate WSOL conversion")?;
        self.decode_response(response, "simulate WSOL conversion")
            .await
    }

    pub async fn wallet_balances(&self, user_id: i64) -> Result<WalletBalancesResponse> {
        let response = self
            .authorized(
                self.client
                    .get(format!("{}/internal/wallet-balances", self.base_url))
                    .query(&[
                        ("owner_type", "copy_trading"),
                        ("owner_id", &user_id.to_string()),
                    ]),
            )
            .send()
            .await
            .context("failed to query wallet balances")?;
        self.decode_response(response, "query wallet balances")
            .await
    }

    pub async fn quote_balance(
        &self,
        user_id: i64,
        wallet_id: &str,
        mint: &str,
    ) -> Result<QuoteBalanceResponse> {
        let response = self
            .authorized(self.client.get(format!(
                "{}/internal/wallets/{}/quote-balance/{}",
                self.base_url, wallet_id, mint
            )))
            .query(&[
                ("owner_type", "copy_trading"),
                ("owner_id", &user_id.to_string()),
            ])
            .send()
            .await
            .context("failed to query quote balance")?;
        self.decode_response(response, "query quote balance").await
    }

    pub async fn create_vanity_job(
        &self,
        user_id: i64,
        side: &str,
        pattern: &str,
    ) -> Result<VanityJobResponse> {
        let request = CreateVanityJobRequest {
            owner_type: "copy_trading",
            owner_id: user_id.to_string(),
            label: "Vanity",
            match_side: side,
            pattern,
        };
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/internal/wallets/vanity-jobs", self.base_url)),
            )
            .json(&request)
            .send()
            .await
            .context("failed to create vanity wallet job")?;
        self.decode_response(response, "create vanity wallet job")
            .await
    }

    pub async fn vanity_job(&self, user_id: i64, job_id: &str) -> Result<VanityJobResponse> {
        let owner_id = user_id.to_string();
        let response = self
            .authorized(
                self.client
                    .get(format!(
                        "{}/internal/wallets/vanity-jobs/{job_id}",
                        self.base_url
                    ))
                    .query(&[
                        ("owner_type", "copy_trading"),
                        ("owner_id", owner_id.as_str()),
                    ]),
            )
            .send()
            .await
            .context("failed to query vanity wallet job")?;
        self.decode_response(response, "query vanity wallet job")
            .await
    }

    pub async fn cancel_vanity_job(&self, user_id: i64, job_id: &str) -> Result<VanityJobResponse> {
        let owner_id = user_id.to_string();
        let response = self
            .authorized(
                self.client
                    .delete(format!(
                        "{}/internal/wallets/vanity-jobs/{job_id}",
                        self.base_url
                    ))
                    .query(&[
                        ("owner_type", "copy_trading"),
                        ("owner_id", owner_id.as_str()),
                    ]),
            )
            .send()
            .await
            .context("failed to cancel vanity wallet job")?;
        self.decode_response(response, "cancel vanity wallet job")
            .await
    }

    pub async fn transfer_sol(
        &self,
        user_id: i64,
        wallet_id: &str,
        to_wallet_id: Option<&str>,
        to_address: Option<&str>,
        amount_lamports: Option<u64>,
        percent: Option<u64>,
        idempotency_key: String,
    ) -> Result<SolTransferResponse> {
        let request = SolTransferRequest {
            owner_type: "copy_trading",
            owner_id: user_id.to_string(),
            to_wallet_id,
            to_address,
            amount_mode: if amount_lamports.is_some() {
                "fixed"
            } else {
                "percent"
            },
            amount_lamports,
            percent,
            idempotency_key,
        };
        let response = self
            .authorized(self.client.post(format!(
                "{}/internal/wallets/{wallet_id}/transfer-sol",
                self.base_url
            )))
            .json(&request)
            .send()
            .await
            .context("failed to transfer SOL")?;
        self.decode_response(response, "transfer SOL").await
    }

    pub async fn import_wallet(
        &self,
        user_id: i64,
        label: &str,
        private_key_base58: &str,
    ) -> Result<ManagedWalletResponse> {
        self.create_or_import_wallet(user_id, label, Some(private_key_base58))
            .await
    }

    pub async fn rename_wallet(&self, wallet_id: &str, label: &str) -> Result<()> {
        let response = self
            .authorized(
                self.client
                    .patch(format!("{}/v1/wallets/{wallet_id}", self.base_url)),
            )
            .json(&UpdateWalletRequest { label })
            .send()
            .await
            .context("failed to rename trade_service wallet")?;
        let _: ManagedWalletResponse = self.decode_response(response, "rename wallet").await?;
        Ok(())
    }

    pub async fn delete_wallet(&self, wallet_id: &str) -> Result<()> {
        let response = self
            .authorized(
                self.client
                    .delete(format!("{}/v1/wallets/{wallet_id}", self.base_url)),
            )
            .send()
            .await
            .context("failed to delete trade_service wallet")?;
        let _: ManagedWalletResponse = self.decode_response(response, "delete wallet").await?;
        Ok(())
    }

    pub async fn export_wallet(&self, wallet_id: &str) -> Result<String> {
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/wallets/{wallet_id}/export", self.base_url)),
            )
            .send()
            .await
            .context("failed to export trade_service wallet")?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("trade_service export wallet returned {status}");
        }
        let response: ExportWalletResponse =
            serde_json::from_str(&text).context("invalid trade_service export wallet response")?;
        Ok(response.private_key_base58)
    }

    async fn create_or_import_wallet(
        &self,
        user_id: i64,
        label: &str,
        private_key_base58: Option<&str>,
    ) -> Result<ManagedWalletResponse> {
        let request = CreateWalletRequest {
            owner_type: "copy_trading",
            owner_id: user_id.to_string(),
            label,
            private_key_base58,
        };
        let response = self
            .authorized(self.client.post(format!("{}/v1/wallets", self.base_url)))
            .json(&request)
            .send()
            .await
            .context("failed to create trade_service wallet")?;
        self.decode_response(response, "create wallet").await
    }

    fn authorized(&self, builder: RequestBuilder) -> RequestBuilder {
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    async fn decode_response<T: for<'de> Deserialize<'de>>(
        &self,
        response: reqwest::Response,
        action: &str,
    ) -> Result<T> {
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("trade_service {action} returned {status}: {text}");
        }
        serde_json::from_str(&text)
            .with_context(|| format!("invalid trade_service {action} response: {text}"))
    }
}
