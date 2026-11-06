use crate::execution_params::{
    ExecutionMethod, ExecutionMode, ExecutionParams, LimitAction, OrderSize,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppUser {
    pub id: i64,
    pub telegram_id: i64,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTradeSettings {
    pub user_id: i64,
    pub use_mev: bool,
    pub default_mev_provider: String,
    pub default_execution_mode: String,
    pub default_execution_method: String,
    pub default_slippage_bps: u64,
    pub default_tip_sol: f64,
    pub default_buy_preset_slot: usize,
    pub default_sell_preset_slot: usize,
    pub quick_trade_confirmation_enabled: bool,
    pub buy_presets_sol: [f64; 5],
    pub sell_presets_percent: [f64; 5],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRecord {
    pub id: i64,
    pub user_id: i64,
    pub external_wallet_id: Option<String>,
    pub label: String,
    pub public_key: String,
    pub is_active: bool,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchWallet {
    pub id: i64,
    pub user_id: i64,
    pub address: String,
    pub label: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchMatch {
    pub watch_id: i64,
    pub user_id: i64,
    pub telegram_id: i64,
    pub watch_label: String,
    pub watch_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    pub id: i64,
    pub user_id: i64,
    pub wallet_id: Option<i64>,
    pub watch_wallet_id: Option<i64>,
    pub name: String,
    pub enabled: bool,
    pub dry_run: bool,
    pub execution_mode: String,
    pub execution_method: String,
    pub copy_buy: bool,
    pub copy_sell: bool,
    pub buy_amount_sol: f64,
    pub buy_mode: String,
    pub buy_ratio_percent: f64,
    pub sell_percent: Option<f64>,
    pub max_slippage_bps: u64,
    pub tip_sol: f64,
    pub max_per_mint_sol: f64,
    pub daily_max_sol: f64,
    pub cooldown_secs: u64,
    pub only_once: bool,
    pub min_sol_in_pool: f64,
    pub min_mcap: f64,
    pub max_mcap: f64,
    pub min_target_buy_sol: f64,
    pub max_target_buy_sol: f64,
    pub max_signal_delay_ms: Option<i64>,
    pub allowed_pools: Vec<String>,
    pub mev_mode: String,
    pub live: bool,
}

impl StrategyConfig {
    pub fn execution_params(&self) -> ExecutionParams {
        ExecutionParams {
            wallet_id: self.wallet_id,
            mode: ExecutionMode::from_db(&self.execution_mode, self.dry_run),
            method: ExecutionMethod::from_db(&self.execution_method),
            slippage_bps: self.max_slippage_bps,
            tip_sol: self.tip_sol,
            mev_mode: self.mev_mode.clone(),
        }
    }

    pub fn buy_size(&self) -> OrderSize {
        if self.buy_mode == "percentage" {
            OrderSize::FollowBuyRatio {
                percent: self.buy_ratio_percent,
                max_sol: (self.max_per_mint_sol > 0.0).then_some(self.max_per_mint_sol),
            }
        } else {
            OrderSize::FixedBuySol {
                amount_sol: self.buy_amount_sol,
            }
        }
    }

    pub fn sell_size(&self) -> OrderSize {
        OrderSize::SellPercent {
            percent: self.sell_percent.unwrap_or(100.0).round() as u64,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TradeWalletRef {
    CopyTradingWalletId(i64),
    TradeServiceWalletId(String),
    TradeServiceDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSummary {
    pub id: String,
    pub mint: String,
    pub side: String,
    pub status: String,
    pub amount_sol: Option<f64>,
    pub sell_percent: Option<f64>,
    pub request_json: Option<String>,
    pub tx_signature: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDetail {
    pub id: String,
    pub wallet_id: Option<i64>,
    pub strategy_id: Option<i64>,
    pub signal_id: Option<i64>,
    pub mint: String,
    pub side: String,
    pub status: String,
    pub amount_sol: Option<f64>,
    pub sell_percent: Option<f64>,
    pub idempotency_key: String,
    pub trade_service_request_id: Option<String>,
    pub request_json: Option<String>,
    pub response_json: Option<String>,
    pub tx_signature: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalSummary {
    pub id: i64,
    pub mint: String,
    pub side: String,
    pub status: String,
    pub reason: String,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionSummary {
    pub mint: String,
    pub amount_sol: f64,
    pub buy_count: i64,
    pub sell_count: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitOrderSummary {
    pub id: String,
    pub user_id: i64,
    pub wallet_id: Option<i64>,
    pub mint: String,
    pub side: String,
    pub trigger_type: String,
    pub trigger_price: f64,
    pub trigger_currency: String,
    pub amount_sol: Option<f64>,
    pub sell_percent: Option<f64>,
    pub dry_run: bool,
    pub execution_mode: String,
    pub execution_method: String,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub mev_mode: String,
    pub limit_action: String,
    pub trigger_metric: String,
    pub status: String,
    pub triggered_price: Option<f64>,
    pub triggered_currency: Option<String>,
    pub triggered_source_event_id: Option<i64>,
    pub triggered_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

impl LimitOrderSummary {
    pub fn action(&self) -> LimitAction {
        LimitAction::from_db(&self.limit_action, &self.side, &self.trigger_type)
    }

    pub fn execution_params(&self) -> ExecutionParams {
        ExecutionParams {
            wallet_id: self.wallet_id,
            mode: ExecutionMode::from_db(&self.execution_mode, self.dry_run),
            method: ExecutionMethod::from_db(&self.execution_method),
            slippage_bps: self.slippage_bps,
            tip_sol: self.tip_sol,
            mev_mode: self.mev_mode.clone(),
        }
    }

    pub fn order_size(&self) -> Option<OrderSize> {
        if self.action().side() == "buy" {
            self.amount_sol
                .map(|amount_sol| OrderSize::FixedBuySol { amount_sol })
        } else {
            self.sell_percent.map(|percent| OrderSize::SellPercent {
                percent: percent.round() as u64,
            })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintSignalDetails {
    pub strategy_id: Option<i64>,
    pub watch_label: String,
    pub watch_wallet: String,
    pub signer: Option<String>,
    pub signature: Option<String>,
    pub mint: String,
    pub side: String,
    pub pool: Option<String>,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
    pub sol_in_pool: Option<f64>,
    pub tokens_in_pool: Option<f64>,
    pub quote_in_pool: Option<f64>,
    pub event_timestamp_ms: Option<i64>,
    pub received_unix_ms: Option<i64>,
    pub ingest_delay_ms: Option<i64>,
    pub notification_unix_ms: Option<i64>,
    pub notification_delay_ms: Option<i64>,
    pub default_buy_amount_sol: f64,
    pub filter_reason: Option<String>,
    pub target_buy_sol: Option<f64>,
    pub market_cap_quote: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PumpApiEvent {
    pub signature: Option<String>,
    pub mint: Option<String>,
    pub pool: Option<String>,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
    pub tx_type: Option<String>,
    pub signer: Option<String>,
    pub sol_in_pool: Option<f64>,
    pub tokens_in_pool: Option<f64>,
    pub quote_in_pool: Option<f64>,
    pub event_timestamp_ms: Option<i64>,
    pub received_unix_ms: i64,
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlphaPaperSignal {
    pub signal_id: String,
    pub event_id: String,
    pub mint: String,
    pub pool: Option<String>,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
    pub side: String,
    pub score: f64,
    pub status: String,
    pub features_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ObservedPoolRoute {
    pub pool: String,
    pub pool_id: Option<String>,
    pub quote_mint: String,
}
