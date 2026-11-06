use crate::db::models::{AlphaPaperSignal, PumpApiEvent};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};

const WINDOWS_MS: [i64; 3] = [30_000, 120_000, 600_000];

#[derive(Debug, Clone)]
pub struct AlphaConfig {
    pub enabled: bool,
    pub min_score: f64,
    pub min_traders: usize,
    pub cooldown_ms: i64,
}

impl AlphaConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: env_bool("ALPHA_PAPER_ENABLED", false),
            min_score: env_f64("ALPHA_PAPER_MIN_SCORE", 0.80).clamp(0.0, 1.0),
            min_traders: env_usize("ALPHA_PAPER_MIN_TRADERS", 5),
            cooldown_ms: env_u64("ALPHA_PAPER_SIGNAL_COOLDOWN_SECS", 600)
                .min(i64::MAX as u64 / 1_000) as i64
                * 1_000,
        }
    }
}

#[derive(Debug, Clone)]
struct Tick {
    timestamp_ms: i64,
    side: String,
    signer: Option<String>,
    quote_amount: f64,
}

#[derive(Debug, Clone)]
pub struct AlphaEngine {
    config: AlphaConfig,
    windows: HashMap<String, VecDeque<Tick>>,
    seen_events: HashSet<String>,
    last_signal_ms: HashMap<String, i64>,
}

impl AlphaEngine {
    pub fn new(config: AlphaConfig) -> Self {
        Self {
            config,
            windows: HashMap::new(),
            seen_events: HashSet::new(),
            last_signal_ms: HashMap::new(),
        }
    }

    pub fn observe(&mut self, event: &PumpApiEvent) -> Option<AlphaPaperSignal> {
        if !self.config.enabled || !is_candidate(event) {
            return None;
        }
        let event_id = event_id(event);
        if self.seen_events.len() > 50_000 {
            self.seen_events.clear();
        }
        if !self.seen_events.insert(event_id.clone()) {
            return None;
        }
        let side = event.tx_type.as_deref()?.trim().to_ascii_lowercase();
        if !matches!(side.as_str(), "buy" | "sell") {
            return None;
        }
        let timestamp_ms = event.event_timestamp_ms.unwrap_or(event.received_unix_ms);
        let quote_amount = quote_amount(event)?;
        if quote_amount <= 0.0 {
            return None;
        }
        let key = format!(
            "{}:{}",
            event.mint.as_deref()?,
            event
                .pool_id
                .as_deref()
                .unwrap_or(event.pool.as_deref().unwrap_or("unknown"))
        );
        if self.windows.len() > 5_000 {
            self.windows.clear();
            self.last_signal_ms.clear();
        }
        let ticks = self.windows.entry(key.clone()).or_default();
        ticks.push_back(Tick {
            timestamp_ms,
            side: side.clone(),
            signer: event.signer.clone(),
            quote_amount,
        });
        while ticks
            .front()
            .is_some_and(|tick| tick.timestamp_ms < timestamp_ms - WINDOWS_MS[2])
        {
            ticks.pop_front();
        }
        while ticks.len() > 128 {
            ticks.pop_front();
        }
        if side != "buy"
            || self
                .last_signal_ms
                .get(&key)
                .is_some_and(|last| timestamp_ms.saturating_sub(*last) < self.config.cooldown_ms)
        {
            return None;
        }
        let short = features(ticks, timestamp_ms, WINDOWS_MS[0]);
        let medium = features(ticks, timestamp_ms, WINDOWS_MS[1]);
        let long = features(ticks, timestamp_ms, WINDOWS_MS[2]);
        if medium.unique_traders < self.config.min_traders
            || short.net_flow <= 0.0
            || medium.concentration > 0.35
        {
            return None;
        }
        let score = score(&short, &medium, &long);
        if score < self.config.min_score {
            return None;
        }
        self.last_signal_ms.insert(key, timestamp_ms);
        Some(AlphaPaperSignal {
            signal_id: format!("{}:alpha-flow-v1", event_id),
            event_id,
            mint: event.mint.clone()?,
            pool: event.pool.clone(),
            pool_id: event.pool_id.clone(),
            quote_mint: event.quote_mint.clone(),
            side,
            score,
            status: "paper".to_string(),
            features_json: serde_json::to_string(&json!({
                "window_30s": short,
                "window_2m": medium,
                "window_10m": long,
                "feature_version": "flow-v1"
            }))
            .ok()?,
            created_at_ms: event.received_unix_ms,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct Features {
    window_ms: i64,
    buy_count: usize,
    sell_count: usize,
    buy_flow: f64,
    sell_flow: f64,
    net_flow: f64,
    buy_sell_ratio: f64,
    unique_traders: usize,
    concentration: f64,
}

fn features(ticks: &VecDeque<Tick>, as_of_ms: i64, window_ms: i64) -> Features {
    let start = as_of_ms - window_ms;
    let mut buy_count = 0;
    let mut sell_count = 0;
    let mut buy_flow = 0.0;
    let mut sell_flow = 0.0;
    let mut traders = HashSet::new();
    let mut trader_buys = HashMap::<String, f64>::new();
    for tick in ticks.iter().filter(|tick| tick.timestamp_ms >= start) {
        if let Some(signer) = &tick.signer {
            traders.insert(signer.clone());
        }
        if tick.side == "buy" {
            buy_count += 1;
            buy_flow += tick.quote_amount;
            if let Some(signer) = &tick.signer {
                *trader_buys.entry(signer.clone()).or_default() += tick.quote_amount;
            }
        } else if tick.side == "sell" {
            sell_count += 1;
            sell_flow += tick.quote_amount;
        }
    }
    let max_trader_buy = trader_buys.values().copied().fold(0.0, f64::max);
    Features {
        window_ms,
        buy_count,
        sell_count,
        buy_flow,
        sell_flow,
        net_flow: buy_flow - sell_flow,
        buy_sell_ratio: (buy_count as f64 + 1.0) / (sell_count as f64 + 1.0),
        unique_traders: traders.len(),
        concentration: if buy_flow > 0.0 {
            max_trader_buy / buy_flow
        } else {
            0.0
        },
    }
}

fn score(short: &Features, medium: &Features, long: &Features) -> f64 {
    let flow_base = (medium.buy_flow / 4.0).max(1e-12);
    let acceleration = (short.net_flow / flow_base).clamp(-3.0, 3.0);
    let ratio = medium.buy_sell_ratio.clamp(0.0, 3.0) / 3.0;
    let traders = (medium.unique_traders as f64).min(10.0) / 10.0;
    let persistence = f64::from(long.net_flow > 0.0);
    (0.45 * (acceleration + 3.0) / 6.0 + 0.20 * ratio + 0.20 * traders + 0.15 * persistence
        - 0.20 * medium.concentration.clamp(0.0, 1.0))
    .clamp(0.0, 1.0)
}

fn is_candidate(event: &PumpApiEvent) -> bool {
    event.mint.is_some()
        && event.quote_mint.is_some()
        && event.pool.as_deref().is_some_and(is_known_pool)
}

fn is_known_pool(pool: &str) -> bool {
    matches!(
        pool,
        "pump"
            | "pump-amm"
            | "bonk"
            | "stonkfun"
            | "stonkfun-swap"
            | "meteora-damm-v2"
            | "meteora-dbc"
            | "raydium-amm-v4"
            | "raydium-cpmm"
    )
}

fn quote_amount(event: &PumpApiEvent) -> Option<f64> {
    let value: Value = serde_json::from_str(&event.raw_json).ok()?;
    ["quote_amount", "quoteAmount", "quote_amount_raw"]
        .iter()
        .find_map(|field| {
            value.get(*field).and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_u64().map(|n| n as f64))
                    .or_else(|| v.as_str()?.parse().ok())
            })
        })
}

fn event_id(event: &PumpApiEvent) -> String {
    if let Some(signature) = &event.signature {
        return format!(
            "{}:{}:{}:{}",
            signature,
            event.tx_type.as_deref().unwrap_or("unknown"),
            event.mint.as_deref().unwrap_or("unknown"),
            event.pool_id.as_deref().unwrap_or("unknown")
        );
    }
    format!(
        "unsigned:{}",
        hex::encode(Sha256::digest(event.raw_json.as_bytes()))
    )
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}
fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn env_usize(name: &str, default: usize) -> usize {
    env_u64(name, default as u64) as usize
}
