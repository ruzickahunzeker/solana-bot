use crate::{
    config::AppConfig,
    db::{
        models::{WalletRecord, WatchWallet},
        Database,
    },
    price_oracle::PriceOracle,
    trade_client::{TradeServiceClient, TradeServiceWallet},
};
use anyhow::Result;
use chrono::{NaiveDateTime, Utc};

const PUMPAPI_STREAM_MAX_AGE_SECS: i64 = 60;

#[derive(Debug, Clone)]
pub struct PreflightReport {
    pub active_wallet: Option<WalletRecord>,
    pub wallet_count: usize,
    pub watch_wallet_count: usize,
    pub enabled_watch_wallet_count: usize,
    pub strategy_count: i64,
    pub live_strategy_count: i64,
    pub live_limit_order_count: i64,
    pub pending_limit_order_count: i64,
    pub latest_pumpapi_event_at: Option<String>,
    pub pumpapi_stream_fresh: bool,
    pub sol_usd_price: Option<f64>,
    pub trade_service_ok: bool,
    pub trade_service_message: String,
    pub trade_service_ready: bool,
    pub trade_service_ready_message: String,
    pub trade_service_wallet: Option<TradeServiceWallet>,
    pub default_slippage_bps: u64,
    pub default_tip_sol: f64,
}

impl PreflightReport {
    pub fn blocking_reasons(&self) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self.active_wallet.is_none() {
            reasons.push("没有默认钱包");
        }
        if !self.trade_service_ok {
            reasons.push("trade_service health 不通过");
        }
        if !self.trade_service_ready {
            reasons.push("trade_service 尚未 ready");
        }
        if self.active_wallet.is_some() && self.trade_service_wallet.is_none() {
            reasons.push("默认钱包未绑定到 trade_service 同公钥钱包");
        }
        if self.latest_pumpapi_event_at.is_none() {
            reasons.push("PumpAPI 尚未接收到数据");
        } else if !self.pumpapi_stream_fresh {
            reasons.push("PumpAPI 数据流超过 60 秒未更新");
        }
        if self.sol_usd_price.is_none() {
            reasons.push("SOL/USD 价格源无可用价格");
        }
        reasons
    }

    pub fn is_live_ready(&self) -> bool {
        self.blocking_reasons().is_empty()
    }
}

pub async fn build_report(
    config: &AppConfig,
    db: &Database,
    trade_client: &TradeServiceClient,
    price_oracle: &PriceOracle,
    user_id: i64,
) -> Result<PreflightReport> {
    let wallets = db.list_wallets(user_id).await?;
    let watch_wallets = db.list_watch_wallets(user_id).await?;
    let active_wallet = wallets.iter().find(|wallet| wallet.is_active).cloned();
    let enabled_watch_wallet_count = watch_wallets
        .iter()
        .filter(|watch_wallet: &&WatchWallet| watch_wallet.enabled)
        .count();

    let (trade_service_result, ready_result) =
        tokio::join!(trade_client.health(), trade_client.ready());
    let (trade_service_ok, trade_service_message) = match trade_service_result {
        Ok(body) => (true, trim_status(&body)),
        Err(err) => (false, trim_status(&err.to_string())),
    };
    let (trade_service_ready, trade_service_ready_message, trade_service_wallet) =
        match ready_result {
            Ok(ready) => {
                let binding = active_wallet.as_ref().and_then(|wallet| {
                    ready
                        .wallets
                        .iter()
                        .find(|candidate| candidate.pubkey == wallet.public_key)
                        .cloned()
                });
                (
                    ready.ready,
                    format!(
                        "ready={} rpc={} blockhash={}",
                        ready.ready, ready.rpc_ok, ready.has_blockhash
                    ),
                    binding,
                )
            }
            Err(err) => (false, trim_status(&err.to_string()), None),
        };

    let latest_pumpapi_event_at = db.latest_pumpapi_event_at().await?;
    let pumpapi_stream_fresh = pumpapi_stream_is_fresh(latest_pumpapi_event_at.as_deref());

    Ok(PreflightReport {
        active_wallet,
        wallet_count: wallets.len(),
        watch_wallet_count: watch_wallets.len(),
        enabled_watch_wallet_count,
        strategy_count: db.count_strategies(user_id).await?,
        live_strategy_count: db.count_live_strategies(user_id).await?,
        live_limit_order_count: db.count_live_pending_limit_orders(user_id).await?,
        pending_limit_order_count: db.count_pending_limit_orders(user_id).await?,
        latest_pumpapi_event_at,
        pumpapi_stream_fresh,
        sol_usd_price: price_oracle.sol_usd_price().await.or(config.sol_usd_price),
        trade_service_ok,
        trade_service_message,
        trade_service_ready,
        trade_service_ready_message,
        trade_service_wallet,
        default_slippage_bps: config.default_slippage_bps,
        default_tip_sol: config.default_tip_sol,
    })
}

fn pumpapi_stream_is_fresh(value: Option<&str>) -> bool {
    pumpapi_stream_is_fresh_at(value, Utc::now().naive_utc())
}

fn pumpapi_stream_is_fresh_at(value: Option<&str>, now: NaiveDateTime) -> bool {
    let Some(value) = value else {
        return false;
    };
    let Ok(last_seen) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") else {
        return false;
    };
    let age_secs = now.signed_duration_since(last_seen).num_seconds();
    (-5..=PUMPAPI_STREAM_MAX_AGE_SECS).contains(&age_secs)
}

fn trim_status(value: &str) -> String {
    let compact = value.trim().replace('\n', " ");
    if compact.chars().count() > 120 {
        compact.chars().take(120).collect()
    } else if compact.is_empty() {
        "空响应".to_string()
    } else {
        compact
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn pumpapi_stream_freshness_has_a_bounded_window() {
        let now = NaiveDateTime::parse_from_str("2026-07-20 18:30:00", "%Y-%m-%d %H:%M:%S")
            .expect("valid timestamp");
        let fresh = (now - Duration::seconds(30))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let stale = (now - Duration::seconds(61))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        assert!(pumpapi_stream_is_fresh_at(Some(&fresh), now));
        assert!(!pumpapi_stream_is_fresh_at(Some(&stale), now));
        assert!(!pumpapi_stream_is_fresh_at(None, now));
        assert!(!pumpapi_stream_is_fresh_at(Some("invalid"), now));
    }
}
