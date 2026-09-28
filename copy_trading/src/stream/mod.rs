use crate::{
    db::{models::PumpApiEvent, Database},
    limit_order::LimitOrderEngine,
    pool_route::PoolRouteCache,
    strategy::StrategyEngine,
};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

const STONKFUN_STANDARD_PLATFORM_CONFIG: &str = "4E876qZTE9FJMrBzgVtBrSrzz2TLivB5Y5QXPjB4gZL7";
const STONKFUN_REWARD_PLATFORM_CONFIG: &str = "6BwHHDg3u1854jC8PDLXvR4spTcLNaoBxLJNGC4nTESt";

pub fn spawn_zmq_consumer(
    endpoint: String,
    db: Database,
    strategy: StrategyEngine,
    limit_orders: LimitOrderEngine,
    pool_routes: PoolRouteCache,
    limit_queue_capacity: usize,
) {
    let handle = Handle::current();
    let (limit_tx, mut limit_rx) = mpsc::channel::<PumpApiEvent>(limit_queue_capacity.max(1));
    let limit_db = db.clone();
    let limit_strategy = strategy.clone();
    handle.spawn(async move {
        while let Some(event) = limit_rx.recv().await {
            pool_routes.observe_event(&event).await;
            process_limit_or_legacy_event(&limit_db, &limit_strategy, &limit_orders, event).await;
        }
    });
    std::thread::spawn(move || loop {
        if let Err(err) = run_blocking_consumer(
            &endpoint,
            db.clone(),
            strategy.clone(),
            handle.clone(),
            limit_tx.clone(),
        ) {
            error!(?err, "zmq consumer stopped; restarting");
        }
        std::thread::sleep(Duration::from_secs(1));
    });
}

fn run_blocking_consumer(
    endpoint: &str,
    db: Database,
    strategy: StrategyEngine,
    handle: Handle,
    limit_tx: mpsc::Sender<PumpApiEvent>,
) -> Result<()> {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/zmq_subscriber.py");
    let mut child = Command::new("python3")
        .arg(script)
        .arg("--endpoint")
        .arg(endpoint)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to start python zmq subscriber")?;
    let stdout = child
        .stdout
        .take()
        .context("python zmq subscriber stdout unavailable")?;
    let reader = BufReader::new(stdout);
    info!(endpoint, "started pumpapi zmq subscriber");
    let mut last_heartbeat = Instant::now()
        .checked_sub(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);
    let mut last_queue_warning = Instant::now()
        .checked_sub(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);

    for line in reader.lines() {
        let line = line?;
        let Some(event) = parse_payload(line.as_bytes()) else {
            continue;
        };
        let should_heartbeat = last_heartbeat.elapsed() >= Duration::from_secs(10);
        if should_heartbeat {
            last_heartbeat = Instant::now();
        }
        if should_heartbeat {
            let db = db.clone();
            handle.spawn(async move {
                if let Err(err) = db.touch_pumpapi_stream().await {
                    warn!(?err, "failed to update PumpAPI stream heartbeat");
                }
            });
        }
        if !is_followable_event(&event) {
            continue;
        }
        strategy.dispatch_event(&handle, event.clone());
        if let Err(err) = limit_tx.try_send(event) {
            if last_queue_warning.elapsed() >= Duration::from_secs(5) {
                last_queue_warning = Instant::now();
                warn!(
                    ?err,
                    "limit/legacy event queue is full; fast follow remains active"
                );
            }
        }
    }

    let status = child.wait().context("failed to wait for zmq subscriber")?;
    anyhow::bail!("python zmq subscriber exited with {status}");
}

async fn process_limit_or_legacy_event(
    db: &Database,
    strategy: &StrategyEngine,
    limit_orders: &LimitOrderEngine,
    event: PumpApiEvent,
) {
    let limit_matched = if let Some(mint) = event.mint.as_deref() {
        match db.has_pending_limit_orders_for_mint(mint).await {
            Ok(found) => found,
            Err(err) => {
                warn!(?err, mint, "failed to lookup pending limit orders");
                false
            }
        }
    } else {
        false
    };
    let legacy_watch_matched = if strategy.fast_follow_enabled() {
        false
    } else if let Some(signer) = event.signer.as_deref() {
        db.find_watch_matches(signer)
            .await
            .map(|items| !items.is_empty())
            .unwrap_or(false)
    } else {
        false
    };
    if !limit_matched && !legacy_watch_matched {
        return;
    }
    let source_event_id = match db.insert_or_get_pumpapi_event(&event).await {
        Ok(id) => id,
        Err(err) => {
            warn!(?err, "failed to store limit/legacy PumpAPI event");
            return;
        }
    };
    if limit_matched {
        limit_orders.observe_event(source_event_id, &event).await;
    }
    if legacy_watch_matched {
        strategy.observe_event(source_event_id, &event).await;
    }
}

fn is_followable_event(event: &PumpApiEvent) -> bool {
    let Some(tx_type) = event.tx_type.as_deref() else {
        return false;
    };
    let tx_type = tx_type.trim().to_ascii_lowercase();
    matches!(tx_type.as_str(), "buy" | "sell") && event.mint.is_some()
}

fn parse_payload(raw: &[u8]) -> Option<PumpApiEvent> {
    let received_unix_ms = Utc::now().timestamp_millis();
    let value: Value = serde_json::from_slice(raw).ok()?;
    let raw_json = String::from_utf8_lossy(raw).to_string();
    let pool = classify_pool(
        text_field(
            &value,
            &["trade_service_pool", "pool", "event_pool", "dex", "amm"],
        ),
        text_field(&value, &["launchpad_config", "launchpadConfig"]),
        text_field(&value, &["platform"]),
    );
    Some(PumpApiEvent {
        signature: text_field(&value, &["signature"]),
        mint: text_field(&value, &["mint"]),
        pool,
        pool_id: text_field(&value, &["pool_id", "poolId", "ammId", "amm_id"]),
        quote_mint: text_field(&value, &["quote_mint", "quoteMint"]),
        tx_type: text_field(&value, &["tx_type", "txType", "action"]),
        signer: text_field(&value, &["tx_signer", "txSigner", "signer"])
            .or_else(|| first_trader(&value)),
        sol_in_pool: number_field(&value, &["sol_in_pool", "solInPool"]),
        tokens_in_pool: number_field(&value, &["tokens_in_pool", "tokensInPool"]),
        quote_in_pool: number_field(&value, &["quote_in_pool", "quoteInPool"]),
        event_timestamp_ms: integer_field(&value, &["timestamp", "blockTime", "block_time"]),
        received_unix_ms,
        raw_json,
    })
}

fn classify_pool(
    pool: Option<String>,
    launchpad_config: Option<String>,
    platform: Option<String>,
) -> Option<String> {
    let pool = pool?;
    let normalized = pool.trim().to_ascii_lowercase().replace('_', "-");
    if matches!(
        normalized.as_str(),
        "raydium-launchpad" | "raydium-launchlab"
    ) && (platform
        .as_deref()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("stonkfun"))
        || matches!(
            launchpad_config.as_deref(),
            Some(STONKFUN_STANDARD_PLATFORM_CONFIG) | Some(STONKFUN_REWARD_PLATFORM_CONFIG)
        ))
    {
        return Some("stonkfun".to_string());
    }
    Some(normalized)
}

fn text_field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .filter_map(|name| value.get(*name))
        .filter_map(Value::as_str)
        .find(|item| !item.is_empty())
        .map(ToString::to_string)
}

fn number_field(value: &Value, names: &[&str]) -> Option<f64> {
    for name in names {
        let Some(item) = value.get(*name) else {
            continue;
        };
        if let Some(number) = item.as_f64() {
            return Some(number);
        }
        if let Some(text) = item.as_str() {
            if let Ok(number) = text.parse::<f64>() {
                return Some(number);
            }
        }
    }
    None
}

fn integer_field(value: &Value, names: &[&str]) -> Option<i64> {
    for name in names {
        let Some(item) = value.get(*name) else {
            continue;
        };
        if let Some(number) = item.as_i64() {
            return Some(normalize_timestamp_ms(number));
        }
        if let Some(number) = item.as_u64() {
            if let Ok(value) = i64::try_from(number) {
                return Some(normalize_timestamp_ms(value));
            }
        }
        if let Some(text) = item.as_str() {
            if let Ok(number) = text.parse::<i64>() {
                return Some(normalize_timestamp_ms(number));
            }
        }
    }
    None
}

fn normalize_timestamp_ms(value: i64) -> i64 {
    if value > 10_000_000_000 {
        value
    } else {
        value * 1000
    }
}

fn first_trader(value: &Value) -> Option<String> {
    let traders = value.get("tradersInvolved")?;
    if let Some(items) = traders.as_array() {
        return items
            .first()
            .and_then(Value::as_str)
            .map(ToString::to_string);
    }
    traders
        .as_object()
        .and_then(|items| items.keys().next())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_stonkfun_launchpad_configs() {
        for config in [
            STONKFUN_STANDARD_PLATFORM_CONFIG,
            STONKFUN_REWARD_PLATFORM_CONFIG,
        ] {
            assert_eq!(
                classify_pool(
                    Some("raydium-launchpad".to_string()),
                    Some(config.to_string()),
                    None,
                )
                .as_deref(),
                Some("stonkfun")
            );
        }
    }

    #[test]
    fn leaves_other_launchlab_platforms_unclassified() {
        assert_eq!(
            classify_pool(
                Some("raydium-launchpad".to_string()),
                Some("BuM6KDpWiTcxvrpXywWFiw45R2RNH8WURdvqoTDV1BW4".to_string()),
                None,
            )
            .as_deref(),
            Some("raydium-launchpad")
        );
    }

    #[test]
    fn classifies_official_stonkfun_platform_field() {
        assert_eq!(
            classify_pool(
                Some("raydium-launchpad".to_string()),
                Some("unknown-config".to_string()),
                Some("stonkfun".to_string()),
            )
            .as_deref(),
            Some("stonkfun")
        );
    }

    #[test]
    fn create_event_is_not_followable() {
        let event = parse_payload(
            br#"{"action":"create","mint":"mint","pool":"pump","quoteMint":"So11111111111111111111111111111111111111112"}"#,
        )
        .unwrap();
        assert!(!is_followable_event(&event));
    }

    #[test]
    fn reads_first_trader_from_current_object_shape() {
        let event = parse_payload(
            br#"{"action":"buy","mint":"mint","pool":"pump","tradersInvolved":{"wallet":{}}}"#,
        )
        .unwrap();
        assert_eq!(event.signer.as_deref(), Some("wallet"));
    }
}
