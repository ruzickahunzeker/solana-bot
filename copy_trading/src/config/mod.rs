use anyhow::{Context, Result};
use std::{collections::HashSet, env};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub telegram_bot_token: String,
    pub allowed_telegram_user_ids: HashSet<i64>,
    pub database_url: String,
    pub master_key_hex: String,
    pub pumpapi_zmq_endpoint: String,
    pub enable_stream_consumer: bool,
    pub enable_telegram_bot: bool,
    pub trade_service_url: String,
    pub trade_service_token: Option<String>,
    pub trade_service_timeout_secs: u64,
    pub pool_resolve_timeout_secs: u64,
    pub trade_submit_max_attempts: u32,
    pub trade_submit_retry_delay_ms: u64,
    pub trade_reconcile_interval_secs: u64,
    pub fast_follow_enabled: bool,
    pub fast_follow_max_inflight: usize,
    pub fast_follow_cache_refresh_ms: u64,
    pub fast_follow_limit_queue_capacity: usize,
    pub migrate_local_wallets_to_trade_service: bool,
    pub dry_run_default: bool,
    pub default_buy_amount_sol: f64,
    pub quick_buy_presets_sol: [f64; 5],
    pub quick_sell_presets_percent: [f64; 5],
    pub default_slippage_bps: u64,
    pub default_tip_sol: f64,
    pub use_mev: bool,
    pub default_mev_provider: String,
    pub sol_usd_price: Option<f64>,
    pub sol_usd_price_api_url: String,
    pub sol_usd_price_api_key: Option<String>,
    pub sol_usd_refresh_secs: u64,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let telegram_bot_token = env_required("TELEGRAM_BOT_TOKEN")?;
        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://./data/copy_trading.sqlite".to_string());
        let master_key_hex = env_required("MASTER_KEY_HEX")?;
        let allowed_telegram_user_ids = parse_i64_set(&env_required("ALLOWED_TELEGRAM_USER_IDS")?)?;

        let config = Self {
            telegram_bot_token,
            allowed_telegram_user_ids,
            database_url,
            master_key_hex,
            pumpapi_zmq_endpoint: env::var("PUMPAPI_ZMQ_ENDPOINT")
                .unwrap_or_else(|_| "tcp://127.0.0.1:8939".to_string()),
            enable_stream_consumer: env_bool("ENABLE_STREAM_CONSUMER", true),
            enable_telegram_bot: env_bool("ENABLE_TELEGRAM_BOT", true),
            trade_service_url: env::var("TRADE_SERVICE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:18787".to_string()),
            trade_service_token: env::var("TRADE_SERVICE_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            trade_service_timeout_secs: env_u64("TRADE_SERVICE_TIMEOUT_SECS", 15)?,
            pool_resolve_timeout_secs: env_u64("POOL_RESOLVE_TIMEOUT_SECS", 6)?,
            trade_submit_max_attempts: env_u32("TRADE_SUBMIT_MAX_ATTEMPTS", 2)?,
            trade_submit_retry_delay_ms: env_u64("TRADE_SUBMIT_RETRY_DELAY_MS", 250)?,
            trade_reconcile_interval_secs: env_u64("TRADE_RECONCILE_INTERVAL_SECS", 2)?,
            fast_follow_enabled: env_bool("FAST_FOLLOW_ENABLED", true),
            fast_follow_max_inflight: env_u64("FAST_FOLLOW_MAX_INFLIGHT", 16)? as usize,
            fast_follow_cache_refresh_ms: env_u64("FAST_FOLLOW_CACHE_REFRESH_MS", 5000)?,
            fast_follow_limit_queue_capacity: env_u64("FAST_FOLLOW_LIMIT_QUEUE_CAPACITY", 4096)?
                as usize,
            migrate_local_wallets_to_trade_service: env_bool(
                "MIGRATE_LOCAL_WALLETS_TO_TRADE_SERVICE",
                false,
            ),
            dry_run_default: env_bool("DRY_RUN_DEFAULT", true),
            default_buy_amount_sol: env_f64("DEFAULT_BUY_AMOUNT_SOL", 0.01)?,
            quick_buy_presets_sol: [
                env_f64("QUICK_BUY_PRESET_1_SOL", 0.001)?,
                env_f64("QUICK_BUY_PRESET_2_SOL", 0.005)?,
                env_f64("QUICK_BUY_PRESET_3_SOL", 0.01)?,
                env_f64("QUICK_BUY_PRESET_4_SOL", 0.05)?,
                env_f64("QUICK_BUY_PRESET_5_SOL", 0.1)?,
            ],
            quick_sell_presets_percent: [
                env_f64("QUICK_SELL_PRESET_1_PERCENT", 10.0)?,
                env_f64("QUICK_SELL_PRESET_2_PERCENT", 25.0)?,
                env_f64("QUICK_SELL_PRESET_3_PERCENT", 50.0)?,
                env_f64("QUICK_SELL_PRESET_4_PERCENT", 75.0)?,
                env_f64("QUICK_SELL_PRESET_5_PERCENT", 100.0)?,
            ],
            default_slippage_bps: env_u64("DEFAULT_SLIPPAGE_BPS", 1500)?,
            default_tip_sol: env_f64("DEFAULT_TIP_SOL", 0.0001)?,
            use_mev: env_bool("USE_MEV", false),
            default_mev_provider: env::var("DEFAULT_MEV_PROVIDER")
                .unwrap_or_else(|_| "auto".to_string())
                .trim()
                .to_ascii_lowercase(),
            sol_usd_price: env_optional_f64("SOL_USD_PRICE")?,
            sol_usd_price_api_url: env::var("SOL_USD_PRICE_API_URL")
                .unwrap_or_else(|_| "https://api.jup.ag/price/v3".to_string()),
            sol_usd_price_api_key: env::var("JUP_PRICE_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            sol_usd_refresh_secs: env_u64("SOL_USD_REFRESH_SECS", 60)?,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn is_allowed_user(&self, telegram_id: i64) -> bool {
        self.allowed_telegram_user_ids.contains(&telegram_id)
    }

    fn validate(&self) -> Result<()> {
        if self.fast_follow_max_inflight == 0 {
            anyhow::bail!("FAST_FOLLOW_MAX_INFLIGHT must be greater than 0");
        }
        if self.pool_resolve_timeout_secs == 0 {
            anyhow::bail!("POOL_RESOLVE_TIMEOUT_SECS must be greater than 0");
        }
        if self.fast_follow_cache_refresh_ms == 0 {
            anyhow::bail!("FAST_FOLLOW_CACHE_REFRESH_MS must be greater than 0");
        }
        if self.fast_follow_limit_queue_capacity == 0 {
            anyhow::bail!("FAST_FOLLOW_LIMIT_QUEUE_CAPACITY must be greater than 0");
        }
        if self
            .quick_buy_presets_sol
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            anyhow::bail!("QUICK_BUY_PRESET_*_SOL must be greater than 0");
        }
        if self.quick_sell_presets_percent.iter().any(|value| {
            !value.is_finite() || !(1.0..=100.0).contains(value) || value.fract() != 0.0
        }) {
            anyhow::bail!("QUICK_SELL_PRESET_*_PERCENT must be whole numbers from 1 to 100");
        }
        if !matches!(
            self.default_mev_provider.as_str(),
            "auto"
                | "default"
                | "rpc"
                | "jito"
                | "temporal"
                | "flashblock"
                | "blockrazor"
                | "astralane"
                | "speedlanding"
        ) {
            anyhow::bail!("DEFAULT_MEV_PROVIDER is not supported");
        }
        Ok(())
    }
}

fn env_required(name: &str) -> Result<String> {
    env::var(name)
        .with_context(|| format!("missing required environment variable {name}"))
        .map(|value| value.trim().to_string())
        .and_then(|value| {
            if value.is_empty() {
                anyhow::bail!("environment variable {name} is empty");
            }
            Ok(value)
        })
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> Result<f64> {
    match env::var(name) {
        Ok(value) => value
            .parse::<f64>()
            .with_context(|| format!("invalid float value for {name}")),
        Err(_) => Ok(default),
    }
}

fn env_optional_f64(name: &str) -> Result<Option<f64>> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => value
            .parse::<f64>()
            .map(Some)
            .with_context(|| format!("invalid float value for {name}")),
        Err(_) => Ok(None),
    }
}

fn env_u64(name: &str, default: u64) -> Result<u64> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .with_context(|| format!("invalid integer value for {name}")),
        Err(_) => Ok(default),
    }
}

fn env_u32(name: &str, default: u32) -> Result<u32> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u32>()
            .with_context(|| format!("invalid integer value for {name}")),
        Err(_) => Ok(default),
    }
}

fn parse_i64_set(value: &str) -> Result<HashSet<i64>> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.parse::<i64>()
                .with_context(|| format!("invalid Telegram user id {item}"))
        })
        .collect()
}
