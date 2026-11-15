use anyhow::{bail, Context};
use serde::Deserialize;
use std::env;
use std::fs;
use std::net::{IpAddr, SocketAddr};

use crate::wallet_crypto;
pub(crate) mod ordinary_observation;
mod rpc;
pub(crate) mod security;
pub(crate) use security::SecurityConfig;

#[derive(Debug, Clone)]
pub struct Config {
    pub(crate) ordinary_observation: ordinary_observation::OrdinaryObservationConfig,
    pub rpc_urls: Vec<String>,
    pub wallets: Vec<WalletConfig>,
    pub default_wallet_id: String,
    pub wallet_storage: WalletStorageConfig,
    pub allow_internal_system_wallet_transfer: bool,
    pub local_trade_api_token: String,
    pub(crate) security: SecurityConfig,
    pub trade_http_addr: SocketAddr,
    pub trade_allowed_client_ips: Vec<IpAddr>,
    pub default_buy_sol: f64,
    pub default_slippage_percent: f64,
    pub default_tip_sol: f64,
    pub enable_simulate: bool,
    pub wait_tx_confirmed: bool,
    pub enable_sdk_log: bool,
    pub trade_response_wait_ms: u64,
    pub mev_submit_verify_ms: u64,
    pub blockhash_refresh_ms: u64,
    pub blockhash_max_age_ms: u64,
    pub mev: MevConfig,
    pub pumpapi: PumpApiConfig,
    pub stonkfun_via_sol: StonkFunViaSolConfig,
    pub trade_persistence: TradePersistenceConfig,
    pub conversion_database_url: String,
    pub conversion_quotes_enabled: bool,
    pub conversion_live_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradePersistenceMode {
    Memory,
    Shadow,
    Durable,
}

#[derive(Debug, Clone)]
pub struct TradePersistenceConfig {
    pub mode: TradePersistenceMode,
    pub database_url: String,
    pub busy_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WalletConfig {
    pub wallet_id: String,
    pub private_key_base58: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletStorageMode {
    File,
    Database,
}

#[derive(Debug, Clone)]
pub struct WalletStorageConfig {
    pub mode: WalletStorageMode,
    pub database_url: String,
    pub busy_timeout_ms: u64,
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WalletsFile {
    wallets: Vec<WalletConfig>,
}

#[derive(Debug, Clone)]
pub struct MevConfig {
    pub enable_mev_protection: bool,
    pub require_mev_submit: bool,
    pub use_backup_rpcs: bool,
    pub jito: ProviderConfig,
    pub temporal: ProviderConfig,
    pub flashblock: ProviderConfig,
    pub blockrazor: BlockRazorConfig,
    pub astralane: AstralaneConfig,
    pub speedlanding: ProviderConfig,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub enabled: bool,
    pub auth_token: Option<String>,
    pub region: String,
    pub custom_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BlockRazorConfig {
    pub provider: ProviderConfig,
    pub transport: String,
}

#[derive(Debug, Clone)]
pub struct AstralaneConfig {
    pub provider: ProviderConfig,
    pub transport: String,
}

#[derive(Debug, Clone)]
pub struct PumpApiConfig {
    pub enabled: bool,
    pub url: String,
    pub timeout_ms: u64,
    pub default_priority_fee_sol: f64,
    pub lightning_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct StonkFunViaSolConfig {
    pub enabled: bool,
    pub max_buy_sol: f64,
}

impl Config {
    pub(crate) fn readiness_settings(&self) -> crate::application::readiness::ReadinessSettings {
        crate::application::readiness::ReadinessSettings {
            blockhash_max_age_ms: self.blockhash_max_age_ms,
            broadcast: self.security.broadcast,
            pumpapi_enabled: self.pumpapi.enabled,
        }
    }

    pub(crate) fn trade_validation_settings(
        &self,
    ) -> crate::application::trade_settings::TradeValidationSettings {
        crate::application::trade_settings::TradeValidationSettings {
            default_buy_sol: self.default_buy_sol,
            default_slippage_percent: self.default_slippage_percent,
            default_tip_sol: self.default_tip_sol,
            via_sol_enabled: self.stonkfun_via_sol.enabled,
            via_sol_max_buy_sol: self.stonkfun_via_sol.max_buy_sol,
        }
    }

    pub(crate) fn trade_command_defaults(
        &self,
    ) -> crate::application::trade_settings::TradeCommandDefaults {
        crate::application::trade_settings::TradeCommandDefaults {
            default_simulate: self.enable_simulate,
            pumpapi_priority_fee_sol: self.pumpapi.default_priority_fee_sol,
        }
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let rpc_urls = rpc::from_env()?;
        let default_wallet_id =
            env::var("DEFAULT_WALLET_ID").unwrap_or_else(|_| "main".to_string());
        let wallet_storage = WalletStorageConfig::from_env()?;
        let wallets =
            if wallet_storage.mode == WalletStorageMode::Database && !wallet_source_configured() {
                Vec::new()
            } else {
                wallets(&default_wallet_id)?
            };
        let local_trade_api_token = required("LOCAL_TRADE_API_TOKEN")?;
        let security = SecurityConfig::from_env(&local_trade_api_token)?;
        let trade_http_addr = env::var("TRADE_HTTP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:18787".to_string())
            .parse::<SocketAddr>()
            .context("TRADE_HTTP_ADDR must be host:port")?;
        let trade_allowed_client_ips = parse_ip_list("TRADE_ALLOWED_CLIENT_IPS")?;

        if !trade_http_addr.ip().is_loopback() && trade_allowed_client_ips.is_empty() {
            bail!(
                "non-loopback TRADE_HTTP_ADDR requires TRADE_ALLOWED_CLIENT_IPS, e.g. 43.166.214.34"
            );
        }
        let stonkfun_via_sol_max_buy_sol: f64 = parse_env("STONKFUN_VIA_SOL_MAX_BUY_SOL", 0.1)?;
        if !stonkfun_via_sol_max_buy_sol.is_finite() || stonkfun_via_sol_max_buy_sol <= 0.0 {
            bail!("STONKFUN_VIA_SOL_MAX_BUY_SOL must be greater than 0");
        }

        Ok(Self {
            ordinary_observation: ordinary_observation::OrdinaryObservationConfig::from_env()?,
            rpc_urls,
            conversion_database_url: env::var("CONVERSION_DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://./data/conversions.sqlite".to_string()),
            conversion_quotes_enabled: parse_bool("CONVERSION_QUOTES_ENABLED", false)?,
            conversion_live_enabled: parse_bool("CONVERSION_LIVE_ENABLED", false)?,
            wallets,
            default_wallet_id,
            wallet_storage,
            allow_internal_system_wallet_transfer: parse_bool(
                "ALLOW_INTERNAL_SYSTEM_WALLET_TRANSFER",
                false,
            )?,
            local_trade_api_token,
            security,
            trade_http_addr,
            trade_allowed_client_ips,
            default_buy_sol: parse_env_with_legacy(
                "DEFAULT_BUY_SOL",
                "DEFAULT_BUY_LAMPORTS",
                0.001,
                |lamports: u64| lamports as f64 / 1_000_000_000.0,
            )?,
            default_slippage_percent: parse_env_with_legacy(
                "DEFAULT_SLIPPAGE_PERCENT",
                "DEFAULT_SLIPPAGE_BPS",
                15.0,
                |bps: u64| bps as f64 / 100.0,
            )?,
            default_tip_sol: parse_env("DEFAULT_TIP_SOL", 0.0)?,
            enable_simulate: parse_bool("ENABLE_SIMULATE", false)?,
            wait_tx_confirmed: parse_bool("WAIT_TX_CONFIRMED", false)?,
            enable_sdk_log: parse_bool("ENABLE_SDK_LOG", false)?,
            trade_response_wait_ms: parse_env("TRADE_RESPONSE_WAIT_MS", 3_000)?,
            mev_submit_verify_ms: parse_env("MEV_SUBMIT_VERIFY_MS", 1_200)?,
            blockhash_refresh_ms: parse_env("BLOCKHASH_REFRESH_MS", 400)?,
            blockhash_max_age_ms: parse_env("BLOCKHASH_MAX_AGE_MS", 2_000)?,
            mev: MevConfig::from_env()?,
            pumpapi: PumpApiConfig::from_env()?,
            stonkfun_via_sol: StonkFunViaSolConfig {
                enabled: parse_bool("STONKFUN_VIA_SOL_ENABLED", false)?,
                max_buy_sol: stonkfun_via_sol_max_buy_sol,
            },
            trade_persistence: TradePersistenceConfig::from_env()?,
        })
    }

    pub(crate) fn api_access_policy(&self) -> security::ApiAccessPolicy<'_> {
        security::ApiAccessPolicy {
            trade_token: &self.local_trade_api_token,
            allowed_client_ips: &self.trade_allowed_client_ips,
            wallet_admin: &self.security.wallet_admin,
        }
    }
}

impl WalletStorageConfig {
    fn from_env() -> anyhow::Result<Self> {
        let raw = env::var("WALLET_STORAGE_MODE").unwrap_or_else(|_| "file".to_string());
        let mode = match raw.trim().to_ascii_lowercase().as_str() {
            "file" => WalletStorageMode::File,
            "database" | "db" => WalletStorageMode::Database,
            _ => bail!("WALLET_STORAGE_MODE must be file or database"),
        };
        let passphrase = if mode == WalletStorageMode::Database {
            Some(wallet_database_passphrase()?)
        } else {
            None
        };
        Ok(Self {
            mode,
            database_url: env::var("WALLET_DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://./data/wallets.sqlite".to_string()),
            busy_timeout_ms: parse_env("WALLET_DB_BUSY_TIMEOUT_MS", 5_000)?,
            passphrase,
        })
    }
}

fn wallet_database_passphrase() -> anyhow::Result<String> {
    if let Some(value) = optional_env("WALLET_DATABASE_PASSPHRASE") {
        return Ok(value);
    }
    if let Some(path) = optional_env("WALLET_DATABASE_PASSPHRASE_FILE")
        .or_else(|| optional_env("WALLETS_CONFIG_PASSPHRASE_FILE"))
    {
        let value = fs::read_to_string(&path)
            .with_context(|| format!("read wallet database passphrase file {path}"))?;
        let value = value.trim_end_matches(['\r', '\n']).to_string();
        if value.is_empty() {
            bail!("wallet database passphrase file is empty");
        }
        return Ok(value);
    }
    if let Some(value) = optional_env("WALLETS_CONFIG_PASSPHRASE") {
        return Ok(value);
    }
    bail!(
        "WALLET_STORAGE_MODE=database requires WALLET_DATABASE_PASSPHRASE or WALLET_DATABASE_PASSPHRASE_FILE"
    )
}

impl TradePersistenceConfig {
    fn from_env() -> anyhow::Result<Self> {
        let raw = env::var("TRADE_PERSISTENCE_MODE").unwrap_or_else(|_| "memory".to_string());
        let mode = match raw.trim().to_ascii_lowercase().as_str() {
            "memory" => TradePersistenceMode::Memory,
            "shadow" => TradePersistenceMode::Shadow,
            "durable" => TradePersistenceMode::Durable,
            _ => bail!("TRADE_PERSISTENCE_MODE must be memory, shadow, or durable"),
        };
        Ok(Self {
            mode,
            database_url: env::var("TRADE_DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://./data/trades.sqlite".to_string()),
            busy_timeout_ms: parse_env("TRADE_DB_BUSY_TIMEOUT_MS", 5_000)?,
        })
    }
}

fn wallets(default_wallet_id: &str) -> anyhow::Result<Vec<WalletConfig>> {
    if let Some(path) =
        optional_env("WALLETS_CONFIG_ENCRYPTED").or_else(|| optional_env("WALLETS_CONFIG_ENC"))
    {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("read WALLETS_CONFIG_ENCRYPTED {path}"))?;
        let passphrase = wallets_passphrase()?;
        let plaintext = wallet_crypto::decrypt_json(&contents, &passphrase)
            .with_context(|| format!("decrypt {path}"))?;
        let file: WalletsFile =
            serde_json::from_slice(&plaintext).with_context(|| format!("parse {path}"))?;
        validate_wallets(file.wallets)
    } else if let Some(path) = optional_env("WALLETS_CONFIG") {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("read WALLETS_CONFIG {path}"))?;
        let file: WalletsFile =
            serde_json::from_str(&contents).with_context(|| format!("parse {path}"))?;
        validate_wallets(file.wallets)
    } else {
        validate_wallets(vec![WalletConfig {
            wallet_id: default_wallet_id.to_string(),
            private_key_base58: required("PRIVATE_KEY_BASE58")?,
        }])
    }
}

fn wallet_source_configured() -> bool {
    optional_env("WALLETS_CONFIG_ENCRYPTED").is_some()
        || optional_env("WALLETS_CONFIG_ENC").is_some()
        || optional_env("WALLETS_CONFIG").is_some()
        || optional_env("PRIVATE_KEY_BASE58").is_some()
}

fn wallets_passphrase() -> anyhow::Result<String> {
    if let Some(value) = optional_env("WALLETS_CONFIG_PASSPHRASE") {
        return Ok(value);
    }
    if let Some(path) = optional_env("WALLETS_CONFIG_PASSPHRASE_FILE") {
        let value = fs::read_to_string(&path)
            .with_context(|| format!("read WALLETS_CONFIG_PASSPHRASE_FILE {path}"))?;
        let value = value.trim_end_matches(['\r', '\n']).to_string();
        if value.is_empty() {
            bail!("WALLETS_CONFIG_PASSPHRASE_FILE is empty");
        }
        return Ok(value);
    }
    bail!(
        "WALLETS_CONFIG_ENCRYPTED requires WALLETS_CONFIG_PASSPHRASE or WALLETS_CONFIG_PASSPHRASE_FILE"
    )
}

fn validate_wallets(wallets: Vec<WalletConfig>) -> anyhow::Result<Vec<WalletConfig>> {
    if wallets.is_empty() {
        bail!("at least one wallet is required");
    }
    let mut seen = std::collections::HashSet::new();
    for wallet in &wallets {
        if wallet.wallet_id.trim().is_empty() {
            bail!("wallet_id is empty");
        }
        if wallet.private_key_base58.trim().is_empty() {
            bail!(
                "private_key_base58 is empty for wallet {}",
                wallet.wallet_id
            );
        }
        if !seen.insert(wallet.wallet_id.clone()) {
            bail!("duplicate wallet_id {}", wallet.wallet_id);
        }
    }
    Ok(wallets)
}

impl MevConfig {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            enable_mev_protection: parse_bool("ENABLE_MEV_PROTECTION", false)?,
            require_mev_submit: parse_bool("REQUIRE_MEV_SUBMIT", false)?,
            use_backup_rpcs: parse_bool("MEV_USE_BACKUP_RPCS", false)?,
            jito: ProviderConfig::from_env("JITO")?,
            temporal: ProviderConfig::from_env("TEMPORAL")?,
            flashblock: ProviderConfig::from_env("FLASHBLOCK")?,
            blockrazor: BlockRazorConfig {
                provider: ProviderConfig::from_env("BLOCKRAZOR")?,
                transport: env::var("BLOCKRAZOR_TRANSPORT").unwrap_or_else(|_| "grpc".to_string()),
            },
            astralane: AstralaneConfig {
                provider: ProviderConfig::from_env("ASTRALANE")?,
                transport: env::var("ASTRALANE_TRANSPORT").unwrap_or_else(|_| "binary".to_string()),
            },
            speedlanding: ProviderConfig::from_env("SPEEDLANDING")?,
        })
    }
}

impl ProviderConfig {
    fn from_env(prefix: &str) -> anyhow::Result<Self> {
        Ok(Self {
            enabled: parse_bool(&format!("ENABLE_{prefix}"), false)?,
            auth_token: optional_env(&format!("{prefix}_AUTH_TOKEN"))
                .or_else(|| optional_env(&format!("{prefix}_API_KEY")))
                .or_else(|| optional_env(&format!("{prefix}_UUID"))),
            region: env::var(format!("{prefix}_REGION")).unwrap_or_else(|_| "default".to_string()),
            custom_url: optional_env(&format!("{prefix}_URL"))
                .or_else(|| optional_env(&format!("{prefix}_CUSTOM_URL"))),
        })
    }
}

impl PumpApiConfig {
    fn from_env() -> anyhow::Result<Self> {
        let default_priority_fee_sol: f64 = parse_env("PUMPAPI_DEFAULT_PRIORITY_FEE_SOL", 0.0001)?;
        if default_priority_fee_sol < 0.0 || !default_priority_fee_sol.is_finite() {
            bail!("PUMPAPI_DEFAULT_PRIORITY_FEE_SOL must be greater than or equal to 0");
        }
        Ok(Self {
            enabled: parse_bool("ENABLE_PUMPAPI_EXECUTOR", false)?,
            url: env::var("PUMPAPI_URL").unwrap_or_else(|_| "https://api.pumpapi.io".to_string()),
            timeout_ms: parse_env("PUMPAPI_TIMEOUT_MS", 1_200)?,
            default_priority_fee_sol,
            lightning_enabled: parse_bool("PUMPAPI_LIGHTNING_ENABLED", false)?,
        })
    }
}

fn required(key: &str) -> anyhow::Result<String> {
    let value = env::var(key).with_context(|| format!("{key} is required"))?;
    if value.trim().is_empty() {
        bail!("{key} is empty");
    }
    Ok(value)
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_ip_list(key: &str) -> anyhow::Result<Vec<IpAddr>> {
    let Some(value) = optional_env(key) else {
        return Ok(Vec::new());
    };
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<IpAddr>()
                .with_context(|| format!("{key} must contain valid IP addresses"))
        })
        .collect()
}

fn parse_env<T>(key: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    Ok(match env::var(key) {
        Ok(value) => value.parse::<T>().with_context(|| format!("parse {key}"))?,
        Err(_) => default,
    })
}

fn parse_env_with_legacy<T, L, F>(
    key: &str,
    legacy_key: &str,
    default: T,
    convert_legacy: F,
) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
    L: std::str::FromStr,
    L::Err: std::error::Error + Send + Sync + 'static,
    F: FnOnce(L) -> T,
{
    if let Ok(value) = env::var(key) {
        return value.parse::<T>().with_context(|| format!("parse {key}"));
    }
    if let Ok(value) = env::var(legacy_key) {
        let parsed = value
            .parse::<L>()
            .with_context(|| format!("parse {legacy_key}"))?;
        return Ok(convert_legacy(parsed));
    }
    Ok(default)
}

fn parse_bool(key: &str, default: bool) -> anyhow::Result<bool> {
    Ok(match env::var(key) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => bail!("{key} must be true/false"),
        },
        Err(_) => default,
    })
}
