use crate::db::Database;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy)]
pub struct AlphaRetentionConfig {
    pub interval_secs: u64,
    pub observation_retention_hours: u64,
    pub paper_signal_retention_days: u64,
    pub shadow_order_retention_days: u64,
}

pub fn start_alpha_retention(db: Database, config: AlphaRetentionConfig) {
    tokio::spawn(async move {
        loop {
            match db.prune_alpha(config).await {
                Ok(result) if result.total() > 0 => info!(
                    observation_events = result.observation_events,
                    paper_signals = result.paper_signals,
                    shadow_orders = result.shadow_orders,
                    "alpha retention completed"
                ),
                Ok(_) => {}
                Err(err) => warn!(?err, "alpha retention failed; will retry"),
            }
            tokio::time::sleep(Duration::from_secs(config.interval_secs)).await;
        }
    });
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_millis() as i64
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlphaRetentionResult {
    pub observation_events: u64,
    pub paper_signals: u64,
    pub shadow_orders: u64,
}

impl AlphaRetentionResult {
    pub fn total(self) -> u64 {
        self.observation_events + self.paper_signals + self.shadow_orders
    }
}

pub(crate) fn cutoff_ms(now_ms: i64, amount: u64, unit_ms: i64) -> i64 {
    let amount = i64::try_from(amount).unwrap_or(i64::MAX);
    now_ms.saturating_sub(amount.saturating_mul(unit_ms))
}
