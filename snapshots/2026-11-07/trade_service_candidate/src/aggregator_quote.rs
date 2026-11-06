use anyhow::{bail, Context, Result};
use base64::Engine;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use spl_associated_token_account_interface::address::get_associated_token_address_with_program_id;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    ConnectOptions, Row, SqlitePool,
};
use std::collections::HashSet;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::aggregator::WalletBalanceSnapshot;
use crate::config::{TradePersistenceConfig, TradePersistenceMode};
use crate::models::WSOL_MINT;
use crate::sdk_client::{check_target_token_instruction, SdkTradeClient};

const JUPITER_QUOTE_URL: &str = "https://lite-api.jup.ag/swap/v1/quote";
const JUPITER_SWAP_URL: &str = "https://lite-api.jup.ag/swap/v1/swap";
const QUOTE_TTL_MS: u64 = 15_000;
const MAX_COMPUTE_UNIT_LIMIT: u64 = 1_400_000;
const MAX_PRIORITY_FEE_LAMPORTS: u64 = 100_000;
const MAX_PRIORITY_FEE_METADATA_ROUNDING_LAMPORTS: u64 = 1;
const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
const MAX_EXTRA_SYSTEM_SPEND_LAMPORTS: u64 = 20_000_000;
const MAX_WRITABLE_ACCOUNTS: usize = 64;
const MAX_BASE_FEE_LAMPORTS: u64 = 50_000;
const PREFLIGHT_BALANCE_RESERVE_LAMPORTS: u64 = 10_000;
const AGGREGATOR_MIGRATION: &str = concat!(
    include_str!("../migrations/003_aggregator_quotes.sql"),
    include_str!("../migrations/004_aggregator_preflight.sql"),
    include_str!("../migrations/005_aggregator_balance_snapshot.sql"),
    include_str!("../migrations/006_aggregator_shadow_route_length.sql")
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolBuyQuoteRequest {
    pub wallet_id: String,
    pub mint: String,
    pub amount_lamports: u64,
    pub slippage_bps: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct SolBuyQuoteResponse {
    pub input_mint: String,
    pub output_mint: String,
    pub amount_lamports: u64,
    pub expected_output_units: u64,
    pub minimum_output_units: u64,
    pub price_impact_percent: String,
    pub route: Vec<QuoteLeg>,
    pub executable: bool,
    pub balance_snapshot: Option<WalletBalanceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteLeg {
    pub label: String,
    pub input_mint: String,
    pub output_mint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolBuyPreflightResponse {
    pub quote_id: String,
    pub wallet_id: String,
    pub expected_output_units: u64,
    pub minimum_output_units: u64,
    pub quote_still_valid: bool,
    pub unsigned_transaction_built: bool,
    pub aggregator_simulation_ok: bool,
    pub local_simulation_ok: bool,
    pub instruction_count: usize,
    pub address_lookup_table_count: usize,
    pub resolved_account_count: usize,
    pub instruction_audit_passed: bool,
    pub executable: bool,
    pub reason: String,
    pub prioritization_fee_lamports: u64,
    pub compute_unit_limit: u64,
    pub calculated_priority_fee_lamports: u64,
    pub explicit_system_spend_lamports: u64,
    pub writable_account_count: usize,
    pub target_ata: String,
    pub top_level_programs: Vec<String>,
    pub simulated_fee_lamports: Option<u64>,
    pub simulated_units_consumed: Option<u64>,
    pub simulated_payer_debit_lamports: Option<u64>,
    pub simulated_inner_instruction_groups: Option<usize>,
    pub simulated_target_token_delta_units: Option<u64>,
    pub simulated_target_token_post_units: Option<u64>,
    pub baseline_slot: u64,
    pub simulation_slot: u64,
    pub observed_same_slot_payer_debit_lamports: Option<u64>,
    pub observed_same_slot_target_token_delta_units: Option<u64>,
    pub simulated_inner_instruction_count: Option<usize>,
    pub simulated_inner_programs: Option<Vec<String>>,
    pub simulated_inner_payer_system_spend_lamports: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SolBuySubmitRequest {
    pub wallet_id: String,
    pub confirmed: bool,
    pub simulate: bool,
    pub idempotency_key: String,
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

fn validate_unsigned_envelope<'a>(
    transaction: &'a VersionedTransaction,
    wallet_pubkey: &str,
) -> Result<(&'a Pubkey, usize)> {
    transaction
        .sanitize()
        .map_err(|_| anyhow::anyhow!("aggregator transaction failed Solana sanitization"))?;
    let payer = transaction
        .message
        .static_account_keys()
        .first()
        .context("transaction has no payer")?;
    if payer.to_string() != wallet_pubkey
        || transaction.message.header().num_required_signatures != 1
        || transaction.signatures.len() != 1
        || transaction.signatures[0] != Signature::default()
    {
        bail!("aggregator transaction payer or signature count does not match quote wallet");
    }
    let instruction_count = transaction.message.instructions().len();
    if instruction_count == 0 || instruction_count > 32 {
        bail!("aggregator transaction has unsupported instruction count");
    }
    if matches!(&transaction.message, VersionedMessage::V1(_)) {
        bail!("aggregator message version is not supported for account audit");
    }
    Ok((payer, instruction_count))
}

fn require_writable_target_ata(
    message: &VersionedMessage,
    resolved_keys: &[Pubkey],
    target_ata: &Pubkey,
) -> Result<u8> {
    let target_index = resolved_keys
        .iter()
        .position(|key| key == target_ata)
        .context("aggregator transaction does not reference wallet target ATA")?;
    if !message.is_maybe_writable_with_reserved_addresses(target_index, None::<&HashSet<Pubkey>>) {
        bail!("aggregator target ATA is not writable");
    }
    u8::try_from(target_index).context("aggregator target ATA index is out of range")
}

fn read_explicit_system_spend(
    instructions: &[solana_sdk::message::compiled_instruction::CompiledInstruction],
    keys: &[Pubkey],
    payer: &Pubkey,
    input_lamports: u64,
) -> Result<u64> {
    let system_program = Pubkey::default();
    let wsol_mint = Pubkey::from_str(WSOL_MINT).context("invalid WSOL mint")?;
    let wsol_ata =
        get_associated_token_address_with_program_id(payer, &wsol_mint, &spl_token_interface::ID);
    let mut total = 0u64;
    for instruction in instructions {
        let program = keys
            .get(usize::from(instruction.program_id_index))
            .context("invalid system instruction program index")?;
        if *program != system_program {
            continue;
        }
        if instruction.accounts.len() != 2
            || instruction.data.len() != 12
            || instruction.data[..4] != [2, 0, 0, 0]
        {
            bail!("unsupported system instruction in aggregator transaction");
        }
        let sender = keys
            .get(usize::from(instruction.accounts[0]))
            .context("invalid system transfer sender index")?;
        if sender != payer {
            bail!("aggregator system transfer sender does not match payer");
        }
        let recipient = keys
            .get(usize::from(instruction.accounts[1]))
            .context("invalid system transfer recipient index")?;
        if recipient != &wsol_ata {
            bail!("aggregator system transfer recipient is not wallet WSOL ATA");
        }
        let lamports = u64::from_le_bytes(instruction.data[4..12].try_into()?);
        total = total
            .checked_add(lamports)
            .context("system transfer total overflow")?;
    }
    if total > input_lamports {
        bail!("aggregator explicit WSOL funding exceeds quote input");
    }
    Ok(total)
}

fn read_compute_budget_fee(
    instructions: &[solana_sdk::message::compiled_instruction::CompiledInstruction],
    keys: &[Pubkey],
) -> Result<(u64, u64)> {
    let program = Pubkey::from_str(COMPUTE_BUDGET_PROGRAM)?;
    let mut limit = None;
    let mut price = None;
    for instruction in instructions {
        if keys[usize::from(instruction.program_id_index)] != program {
            continue;
        }
        if !instruction.accounts.is_empty() {
            bail!("compute budget instruction has unexpected accounts");
        }
        match instruction.data.as_slice() {
            [2, raw @ ..] if raw.len() == 4 && limit.is_none() => {
                limit = Some(u32::from_le_bytes(raw.try_into()?) as u64);
            }
            [3, raw @ ..] if raw.len() == 8 && price.is_none() => {
                price = Some(u64::from_le_bytes(raw.try_into()?));
            }
            _ => bail!("unsupported or duplicate compute budget instruction"),
        }
    }
    let limit = limit.context("aggregator transaction lacks explicit compute unit limit")?;
    if limit == 0 || limit > MAX_COMPUTE_UNIT_LIMIT {
        bail!("aggregator compute unit limit exceeds preflight cap");
    }
    let price = price.unwrap_or(0);
    let priority_fee = limit
        .checked_mul(price)
        .context("priority fee overflow")?
        .div_ceil(1_000_000);
    if priority_fee > MAX_PRIORITY_FEE_LAMPORTS {
        bail!("aggregator priority fee exceeds preflight cap");
    }
    Ok((limit, priority_fee))
}

fn validate_fee_metadata(
    reported_limit: Option<u64>,
    reported_fee: Option<u64>,
    decoded_limit: u64,
    decoded_fee: u64,
) -> Result<()> {
    if let Some(limit) = reported_limit {
        if limit != decoded_limit {
            bail!("aggregator compute limit mismatch: reported {limit}, decoded {decoded_limit}");
        }
    }
    if let Some(fee) = reported_fee {
        if fee.abs_diff(decoded_fee) > MAX_PRIORITY_FEE_METADATA_ROUNDING_LAMPORTS {
            bail!("aggregator priority fee mismatch: reported {fee}, decoded {decoded_fee}");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct BoundSolBuyQuote {
    pub quote_id: String,
    pub provider: String,
    pub provider_quote_id: String,
    pub route_fingerprint: String,
    pub request_hash: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub quote_slippage_bps: u16,
    pub expires_at_ms: u64,
    pub status: String,
    #[serde(flatten)]
    pub quote: SolBuyQuoteResponse,
    #[serde(skip)]
    pub message_hash: Option<String>,
    pub preflight: Option<SolBuyPreflightResponse>,
    pub audit: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AggregatorShadowMetric {
    pub provider: String,
    pub wallet_id: String,
    pub input_mint: String,
    pub output_mint: String,
    pub amount_lamports: u64,
    pub slippage_bps: u16,
    pub status: String,
    pub latency_ms: u64,
    pub expected_output_units: Option<u64>,
    pub minimum_output_units: Option<u64>,
    pub route_fingerprint: Option<String>,
    pub route_length: Option<u8>,
    pub price_impact_percent: Option<String>,
    pub error_class: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorShadowSummary {
    pub sample_count: u64,
    pub quoted_count: u64,
    pub failed_count: u64,
    pub unavailable_count: u64,
    pub p50_latency_ms: Option<u64>,
    pub p95_latency_ms: Option<u64>,
    pub p99_latency_ms: Option<u64>,
    pub success_rate: Option<f64>,
    pub expiry_rate: Option<f64>,
    pub min_output_checked_count: u64,
    pub min_output_missing_count: u64,
    pub min_output_coverage_rate: Option<f64>,
    pub min_output_pass_count: u64,
    pub min_output_pass_rate: Option<f64>,
}

pub struct QuoteStore {
    quotes: DashMap<String, BoundSolBuyQuote>,
    reservations: DashMap<String, String>,
    submissions: DashMap<String, SolBuySubmitResponse>,
    pool: Option<SqlitePool>,
}

#[derive(Debug, Clone)]
pub enum SubmissionReservation {
    Reserved,
    Existing(SolBuySubmitResponse),
    InFlight,
    Unknown,
    Conflict,
}

impl QuoteStore {
    pub async fn new(config: &TradePersistenceConfig) -> Result<Self> {
        if config.mode == TradePersistenceMode::Memory {
            return Ok(Self::default());
        }
        if let Some(path) = config.database_url.strip_prefix("sqlite://") {
            if let Some(parent) = std::path::Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let options = SqliteConnectOptions::from_str(&config.database_url)
            .context("parse aggregator trade database URL")?
            .create_if_missing(true)
            .busy_timeout(Duration::from_millis(config.busy_timeout_ms))
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(AGGREGATOR_MIGRATION).execute(&pool).await?;
        ensure_preflight_columns(&pool).await?;
        ensure_balance_snapshot_column(&pool).await?;
        ensure_shadow_route_length_column(&pool).await?;
        sqlx::query(
            "UPDATE aggregator_submissions SET status='unknown', updated_at_ms=CAST(strftime('%s','now') AS INTEGER) * 1000 WHERE status IN ('reserved','submitting')",
        )
        .execute(&pool)
        .await?;
        Ok(Self {
            quotes: DashMap::new(),
            reservations: DashMap::new(),
            submissions: DashMap::new(),
            pool: Some(pool),
        })
    }

    fn build_bound(
        &self,
        request: &SolBuyQuoteRequest,
        wallet_pubkey: String,
        quote: SolBuyQuoteResponse,
    ) -> Result<BoundSolBuyQuote> {
        let now = now_ms();
        self.quotes
            .retain(|_, existing| existing.expires_at_ms > now);
        if self.quotes.len() >= 512 {
            bail!("quote capacity reached; retry after active quotes expire");
        }
        let request_hash = format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(request)?));
        let route_fingerprint = format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&quote.route)?)
        );
        let bound = BoundSolBuyQuote {
            quote_id: Uuid::new_v4().to_string(),
            provider: "jupiter".to_string(),
            provider_quote_id: format!("jup:{}", route_fingerprint),
            route_fingerprint,
            request_hash,
            wallet_id: request.wallet_id.clone(),
            wallet_pubkey,
            quote_slippage_bps: request.slippage_bps,
            expires_at_ms: now + QUOTE_TTL_MS,
            status: "quoted".to_string(),
            quote,
            message_hash: None,
            preflight: None,
            audit: None,
        };
        Ok(bound)
    }

    pub fn insert_local(
        &self,
        request: &SolBuyQuoteRequest,
        wallet_pubkey: String,
        quote: SolBuyQuoteResponse,
    ) -> Result<BoundSolBuyQuote> {
        let bound = self.build_bound(request, wallet_pubkey, quote)?;
        self.quotes.insert(bound.quote_id.clone(), bound.clone());
        Ok(bound)
    }

    pub async fn insert(
        &self,
        request: &SolBuyQuoteRequest,
        wallet_pubkey: String,
        quote: SolBuyQuoteResponse,
    ) -> Result<BoundSolBuyQuote> {
        let bound = self.build_bound(request, wallet_pubkey, quote)?;
        if let Some(pool) = &self.pool {
            let route_json = serde_json::to_string(&bound.quote.route)?;
            let now = now_ms();
            sqlx::query(
                "INSERT INTO aggregator_quotes(quote_id,provider,provider_quote_id,wallet_id,wallet_pubkey,input_mint,output_mint,amount_lamports,expected_output_units,minimum_output_units,slippage_bps,price_impact_percent,route_json,route_fingerprint,request_hash,message_hash,expires_at_ms,status,created_at_ms,updated_at_ms,balance_snapshot_json) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&bound.quote_id)
            .bind(&bound.provider)
            .bind(&bound.provider_quote_id)
            .bind(&bound.wallet_id)
            .bind(&bound.wallet_pubkey)
            .bind(&bound.quote.input_mint)
            .bind(&bound.quote.output_mint)
            .bind(i64::try_from(bound.quote.amount_lamports)?)
            .bind(i64::try_from(bound.quote.expected_output_units)?)
            .bind(i64::try_from(bound.quote.minimum_output_units)?)
            .bind(i64::from(bound.quote_slippage_bps))
            .bind(&bound.quote.price_impact_percent)
            .bind(route_json)
            .bind(&bound.route_fingerprint)
            .bind(&bound.request_hash)
            .bind(&bound.message_hash)
            .bind(i64::try_from(bound.expires_at_ms)?)
            .bind("quoted")
            .bind(i64::try_from(now)?)
            .bind(i64::try_from(now)?)
            .bind(
                bound
                    .quote
                    .balance_snapshot
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
            )
            .execute(pool)
            .await?;
        }
        self.quotes.insert(bound.quote_id.clone(), bound.clone());
        Ok(bound)
    }

    /// Persist a redacted provider Shadow observation. Raw provider responses,
    /// URLs and transaction bytes are intentionally excluded.
    pub async fn record_shadow_metric(
        &self,
        provider: &str,
        wallet_id: &str,
        input_mint: &str,
        output_mint: &str,
        amount_lamports: u64,
        slippage_bps: u16,
        status: &str,
        latency_ms: u64,
        quote: Option<&crate::aggregator::AggregatorQuote>,
        error_class: Option<&str>,
    ) -> Result<()> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };
        let now = now_ms();
        let error_class = error_class.map(crate::sanitize::redact_urls);
        sqlx::query(
            "INSERT INTO aggregator_shadow_metrics(metric_id,provider,wallet_id,input_mint,output_mint,amount_lamports,slippage_bps,status,latency_ms,expected_output_units,minimum_output_units,route_fingerprint,price_impact_percent,error_class,created_at_ms,route_length) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(format!("agsm_{}", Uuid::new_v4().simple()))
        .bind(provider)
        .bind(wallet_id)
        .bind(input_mint)
        .bind(output_mint)
        .bind(i64::try_from(amount_lamports)?)
        .bind(i64::from(slippage_bps))
        .bind(status)
        .bind(i64::try_from(latency_ms)?)
        .bind(quote.map(|value| i64::try_from(value.expected_output_units)).transpose()?)
        .bind(quote.map(|value| i64::try_from(value.minimum_output_units)).transpose()?)
        .bind(quote.map(|value| value.route_fingerprint.as_str()))
        .bind(quote.and_then(|value| value.price_impact_percent.as_deref()))
        .bind(error_class)
        .bind(i64::try_from(now)?)
        .bind(quote.map(|value| i64::try_from(value.route.len())).transpose()?)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn shadow_metrics(
        &self,
        wallet_id: Option<&str>,
        provider: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AggregatorShadowMetric>> {
        let Some(pool) = &self.pool else {
            return Ok(Vec::new());
        };
        let limit = i64::from(limit.clamp(1, 100));
        let rows = sqlx::query(
            "SELECT provider,wallet_id,input_mint,output_mint,amount_lamports,slippage_bps,status,latency_ms,expected_output_units,minimum_output_units,route_fingerprint,price_impact_percent,error_class,created_at_ms,route_length FROM aggregator_shadow_metrics WHERE (? IS NULL OR wallet_id=?) AND (? IS NULL OR provider=?) ORDER BY created_at_ms DESC LIMIT ?",
        )
        .bind(wallet_id)
        .bind(wallet_id)
        .bind(provider)
        .bind(provider)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AggregatorShadowMetric {
                    provider: row.get("provider"),
                    wallet_id: row.get("wallet_id"),
                    input_mint: row.get("input_mint"),
                    output_mint: row.get("output_mint"),
                    amount_lamports: u64::try_from(row.get::<i64, _>("amount_lamports"))?,
                    slippage_bps: u16::try_from(row.get::<i64, _>("slippage_bps"))?,
                    status: row.get("status"),
                    latency_ms: u64::try_from(row.get::<i64, _>("latency_ms"))?,
                    expected_output_units: row
                        .try_get::<Option<i64>, _>("expected_output_units")?
                        .map(u64::try_from)
                        .transpose()?,
                    minimum_output_units: row
                        .try_get::<Option<i64>, _>("minimum_output_units")?
                        .map(u64::try_from)
                        .transpose()?,
                    route_fingerprint: row.try_get("route_fingerprint")?,
                    route_length: row
                        .try_get::<Option<i64>, _>("route_length")?
                        .map(u8::try_from)
                        .transpose()?,
                    price_impact_percent: row.try_get("price_impact_percent")?,
                    error_class: row.try_get("error_class")?,
                    created_at_ms: u64::try_from(row.get::<i64, _>("created_at_ms"))?,
                })
            })
            .collect()
    }

    pub async fn shadow_summary(
        &self,
        wallet_id: Option<&str>,
        provider: Option<&str>,
    ) -> Result<AggregatorShadowSummary> {
        let Some(pool) = &self.pool else {
            return Ok(AggregatorShadowSummary {
                sample_count: 0,
                quoted_count: 0,
                failed_count: 0,
                unavailable_count: 0,
                p50_latency_ms: None,
                p95_latency_ms: None,
                p99_latency_ms: None,
                success_rate: None,
                expiry_rate: None,
                min_output_checked_count: 0,
                min_output_missing_count: 0,
                min_output_coverage_rate: None,
                min_output_pass_count: 0,
                min_output_pass_rate: None,
            });
        };
        let rows = sqlx::query(
            "SELECT status,latency_ms,expected_output_units,minimum_output_units,slippage_bps FROM aggregator_shadow_metrics WHERE (? IS NULL OR wallet_id=?) AND (? IS NULL OR provider=?)",
        )
        .bind(wallet_id)
        .bind(wallet_id)
        .bind(provider)
        .bind(provider)
        .fetch_all(pool)
        .await?;
        let sample_count = rows.len() as u64;
        let quoted_count = rows
            .iter()
            .filter(|r| r.get::<String, _>("status") == "quoted")
            .count() as u64;
        let failed_count = rows
            .iter()
            .filter(|r| r.get::<String, _>("status") == "failed")
            .count() as u64;
        let unavailable_count = rows
            .iter()
            .filter(|r| r.get::<String, _>("status") == "unavailable")
            .count() as u64;
        let expired_count = rows
            .iter()
            .filter(|r| r.get::<String, _>("status") == "expired")
            .count() as u64;
        let mut min_output_checked_count = 0_u64;
        let mut min_output_pass_count = 0_u64;
        for row in &rows {
            if row.get::<String, _>("status") != "quoted" {
                continue;
            }
            let expected: Option<i64> = row.try_get("expected_output_units")?;
            let minimum: Option<i64> = row.try_get("minimum_output_units")?;
            let (Some(expected), Some(minimum)) = (expected, minimum) else {
                continue;
            };
            min_output_checked_count += 1;
            let slippage: i64 = row.get("slippage_bps");
            if expected > 0
                && minimum > 0
                && minimum <= expected
                && (1..=3000).contains(&slippage)
                && (minimum as u128) >= (expected as u128) * (10_000 - slippage as u128) / 10_000
            {
                min_output_pass_count += 1;
            }
        }
        let mut latencies: Vec<u64> = rows
            .iter()
            .map(|r| u64::try_from(r.get::<i64, _>("latency_ms")))
            .collect::<std::result::Result<_, _>>()?;
        latencies.sort_unstable();
        let percentile = |p: usize| {
            latencies
                .get(((latencies.len() * p).saturating_add(99) / 100).saturating_sub(1))
                .copied()
        };
        Ok(AggregatorShadowSummary {
            sample_count,
            quoted_count,
            failed_count,
            unavailable_count,
            p50_latency_ms: percentile(50),
            p95_latency_ms: percentile(95),
            p99_latency_ms: percentile(99),
            success_rate: (sample_count > 0).then(|| quoted_count as f64 / sample_count as f64),
            expiry_rate: (sample_count > 0).then(|| expired_count as f64 / sample_count as f64),
            min_output_checked_count,
            min_output_missing_count: quoted_count.saturating_sub(min_output_checked_count),
            min_output_coverage_rate: (quoted_count > 0)
                .then(|| min_output_checked_count as f64 / quoted_count as f64),
            min_output_pass_count,
            min_output_pass_rate: (quoted_count > 0)
                .then(|| min_output_pass_count as f64 / quoted_count as f64),
        })
    }

    pub async fn get(
        &self,
        quote_id: &str,
        wallet_id: &str,
        wallet_pubkey: &str,
    ) -> Result<BoundSolBuyQuote> {
        let bound = if let Some(bound) = self.quotes.get(quote_id) {
            bound.value().clone()
        } else if let Some(pool) = &self.pool {
            let row = sqlx::query("SELECT * FROM aggregator_quotes WHERE quote_id=?")
                .bind(quote_id)
                .fetch_optional(pool)
                .await?
                .context("quote not found or service restarted")?;
            let expires_at_ms = u64::try_from(row.get::<i64, _>("expires_at_ms"))?;
            let route: Vec<QuoteLeg> = serde_json::from_str(row.get("route_json"))?;
            let bound = BoundSolBuyQuote {
                quote_id: row.get("quote_id"),
                provider: row.get("provider"),
                provider_quote_id: row.get("provider_quote_id"),
                route_fingerprint: row.get("route_fingerprint"),
                request_hash: row.get("request_hash"),
                wallet_id: row.get("wallet_id"),
                wallet_pubkey: row.get("wallet_pubkey"),
                quote_slippage_bps: u16::try_from(row.get::<i64, _>("slippage_bps"))?,
                expires_at_ms,
                status: row.get("status"),
                quote: SolBuyQuoteResponse {
                    input_mint: row.get("input_mint"),
                    output_mint: row.get("output_mint"),
                    amount_lamports: u64::try_from(row.get::<i64, _>("amount_lamports"))?,
                    expected_output_units: u64::try_from(
                        row.get::<i64, _>("expected_output_units"),
                    )?,
                    minimum_output_units: u64::try_from(row.get::<i64, _>("minimum_output_units"))?,
                    price_impact_percent: row.get("price_impact_percent"),
                    route,
                    executable: false,
                    balance_snapshot: row
                        .try_get::<Option<String>, _>("balance_snapshot_json")?
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                },
                message_hash: row.try_get::<Option<String>, _>("message_hash")?,
                preflight: row
                    .try_get::<Option<String>, _>("preflight_json")?
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                audit: row
                    .try_get::<Option<String>, _>("audit_json")?
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            };
            self.quotes.insert(quote_id.to_string(), bound.clone());
            bound
        } else {
            bail!("quote not found or service restarted");
        };
        if bound.expires_at_ms <= now_ms() {
            self.quotes.remove(quote_id);
            if let Some(pool) = &self.pool {
                sqlx::query(
                    "UPDATE aggregator_quotes SET status='expired',updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE quote_id=? AND status IN ('quoted','preflighted')",
                )
                    .bind(quote_id)
                    .execute(pool)
                    .await?;
            }
            bail!("quote expired");
        }
        if bound.wallet_id != wallet_id || bound.wallet_pubkey != wallet_pubkey {
            bail!("quote wallet mismatch");
        }
        Ok(bound)
    }

    pub async fn mark_quote_status(&self, quote_id: &str, status: &str) -> Result<()> {
        if !matches!(status, "quoted" | "preflighted" | "failed" | "expired") {
            bail!("invalid aggregator quote status");
        }
        let cached_bound = self.quotes.get(quote_id).map(|bound| bound.clone());
        if let Some(bound) = cached_bound.as_ref() {
            let allowed = match (bound.status.as_str(), status) {
                ("quoted", "quoted" | "preflighted" | "failed" | "expired")
                | ("preflighted", "preflighted" | "failed")
                | ("failed", "failed")
                | ("expired", "expired") => true,
                _ => false,
            };
            if !allowed {
                bail!("invalid aggregator quote status transition");
            }
        } else if self.pool.is_none() {
            bail!("aggregator quote not found");
        }
        if let Some(pool) = &self.pool {
            let current = sqlx::query_scalar::<_, String>(
                "SELECT status FROM aggregator_quotes WHERE quote_id=?",
            )
            .bind(quote_id)
            .fetch_optional(pool)
            .await?
            .context("aggregator quote not found")?;
            let allowed = match (current.as_str(), status) {
                ("quoted", "quoted" | "preflighted" | "failed" | "expired")
                | ("preflighted", "preflighted" | "failed")
                | ("failed", "failed")
                | ("expired", "expired") => true,
                _ => false,
            };
            if !allowed {
                bail!("invalid aggregator quote status transition");
            }
            sqlx::query(
                "UPDATE aggregator_quotes SET status=?,updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE quote_id=?",
            )
            .bind(status)
            .bind(quote_id)
            .execute(pool)
            .await?;
        }
        if let Some(mut bound) = self.quotes.get_mut(quote_id) {
            bound.status = status.to_string();
        }
        Ok(())
    }

    pub async fn record_preflight(
        &self,
        quote_id: &str,
        preflight: &SolBuyPreflightResponse,
    ) -> Result<()> {
        let preflight_json = serde_json::to_string(preflight)?;
        let audit_json = serde_json::json!({
            "instruction_audit_passed": preflight.instruction_audit_passed,
            "executable": preflight.executable,
            "instruction_count": preflight.instruction_count,
            "address_lookup_table_count": preflight.address_lookup_table_count,
            "resolved_account_count": preflight.resolved_account_count,
            "writable_account_count": preflight.writable_account_count,
            "explicit_system_spend_lamports": preflight.explicit_system_spend_lamports,
            "top_level_programs": preflight.top_level_programs,
            "reason": preflight.reason,
        });
        if self.pool.is_none() {
            if let Some(mut bound) = self.quotes.get_mut(quote_id) {
                bound.preflight = Some(preflight.clone());
                bound.audit = Some(audit_json);
                return Ok(());
            }
            bail!("aggregator quote not found");
        }
        let pool = self.pool.as_ref().expect("checked above");
        let result = sqlx::query(
            "UPDATE aggregator_quotes SET preflight_json=?,audit_json=?,updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE quote_id=?",
        )
        .bind(preflight_json)
        .bind(audit_json.to_string())
        .bind(quote_id)
        .execute(pool)
        .await?;
        if result.rows_affected() != 1 {
            bail!("aggregator quote not found");
        }
        if let Some(mut bound) = self.quotes.get_mut(quote_id) {
            bound.preflight = Some(preflight.clone());
            bound.audit = Some(audit_json);
        }
        Ok(())
    }

    pub async fn get_submission(&self, key: &str) -> Result<Option<SolBuySubmitResponse>> {
        if let Some(entry) = self.submissions.get(key) {
            return Ok(Some(entry.clone()));
        }
        let Some(pool) = &self.pool else {
            return Ok(None);
        };
        let (wallet_id, idempotency_key) = key
            .split_once(':')
            .context("invalid aggregator idempotency key")?;
        let row = sqlx::query(
            "SELECT response_json,status FROM aggregator_submissions WHERE wallet_id=? AND idempotency_key=?",
        )
        .bind(wallet_id)
        .bind(idempotency_key)
        .fetch_optional(pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let status: String = row.get("status");
        if matches!(status.as_str(), "reserved" | "submitting" | "unknown") {
            return Ok(None);
        }
        let response_json: String = row.get("response_json");
        if response_json == "{}" {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&response_json)?))
    }

    /// Look up a submission reservation before requesting a new provider quote.
    /// This is required for the `/v1/trades` compatibility path, where the
    /// quote ID is created inside the service after the idempotency key arrives.
    pub async fn lookup_submission(&self, key: &str) -> Result<Option<SubmissionReservation>> {
        if let Some(existing) = self.submissions.get(key) {
            return Ok(Some(SubmissionReservation::Existing(existing.clone())));
        }
        if let Some(reservation) = self.reservations.get(key) {
            if reservation.starts_with("unknown:") {
                return Ok(Some(SubmissionReservation::Unknown));
            }
            return Ok(Some(SubmissionReservation::InFlight));
        }
        let Some(pool) = &self.pool else {
            return Ok(None);
        };
        let (wallet_id, idempotency_key) = key
            .split_once(':')
            .context("invalid aggregator idempotency key")?;
        let row = sqlx::query(
            "SELECT status,response_json FROM aggregator_submissions WHERE wallet_id=? AND idempotency_key=?",
        )
        .bind(wallet_id)
        .bind(idempotency_key)
        .fetch_optional(pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let status: String = row.get("status");
        let reservation = match status.as_str() {
            "reserved" | "submitting" => SubmissionReservation::InFlight,
            "unknown" => SubmissionReservation::Unknown,
            _ => {
                let response_json: String = row.get("response_json");
                if response_json == "{}" {
                    SubmissionReservation::InFlight
                } else {
                    SubmissionReservation::Existing(serde_json::from_str(&response_json)?)
                }
            }
        };
        Ok(Some(reservation))
    }

    /// Verify the durable reservation is still bound to the original quote.
    /// This deliberately does not load the quote itself, because submission
    /// recovery must remain possible after the short quote TTL expires.
    pub async fn submission_matches_quote(&self, key: &str, quote_id: &str) -> Result<bool> {
        if let Some(existing) = self.submissions.get(key) {
            return Ok(existing.quote_id == quote_id);
        }
        if let Some(existing) = self.reservations.get(key) {
            return Ok(
                existing.as_str() == quote_id || existing.as_str() == format!("unknown:{quote_id}")
            );
        }
        let Some(pool) = &self.pool else {
            return Ok(false);
        };
        let (wallet_id, idempotency_key) = key
            .split_once(':')
            .context("invalid aggregator idempotency key")?;
        let row = sqlx::query(
            "SELECT quote_id FROM aggregator_submissions WHERE wallet_id=? AND idempotency_key=?",
        )
        .bind(wallet_id)
        .bind(idempotency_key)
        .fetch_optional(pool)
        .await?;
        Ok(row
            .map(|value| value.get::<String, _>("quote_id") == quote_id)
            .unwrap_or(false))
    }

    /// Atomically reserve a wallet/idempotency key before asking the provider
    /// to build or submit a transaction. A second caller never reaches the
    /// provider: it sees the existing terminal response, an in-flight marker,
    /// an unknown marker, or a conflicting quote binding.
    pub async fn reserve_submission(
        &self,
        key: &str,
        quote_id: &str,
    ) -> Result<SubmissionReservation> {
        let (wallet_id, idempotency_key) = key
            .split_once(':')
            .context("invalid aggregator idempotency key")?;
        if let Some(pool) = &self.pool {
            let now = now_ms();
            let inserted = sqlx::query(
                "INSERT INTO aggregator_submissions(submission_id,quote_id,wallet_id,idempotency_key,fingerprint,message_hash,status,response_json,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(wallet_id,idempotency_key) DO NOTHING",
            )
            .bind(format!("ags_{}", Uuid::new_v4().simple()))
            .bind(quote_id)
            .bind(wallet_id)
            .bind(idempotency_key)
            .bind("")
            .bind(None::<String>)
            .bind("reserved")
            .bind("{}")
            .bind(i64::try_from(now)?)
            .bind(i64::try_from(now)?)
            .execute(pool)
            .await?
            .rows_affected();
            if inserted == 1 {
                return Ok(SubmissionReservation::Reserved);
            }
            let row = sqlx::query(
                "SELECT quote_id,status,response_json FROM aggregator_submissions WHERE wallet_id=? AND idempotency_key=?",
            )
            .bind(wallet_id)
            .bind(idempotency_key)
            .fetch_optional(pool)
            .await?
            .context("idempotency reservation disappeared after conflict")?;
            let existing_quote: String = row.get("quote_id");
            if existing_quote != quote_id {
                return Ok(SubmissionReservation::Conflict);
            }
            let status: String = row.get("status");
            return match status.as_str() {
                "reserved" | "submitting" => Ok(SubmissionReservation::InFlight),
                "unknown" => Ok(SubmissionReservation::Unknown),
                _ => {
                    let response_json: String = row.get("response_json");
                    if response_json == "{}" {
                        Ok(SubmissionReservation::InFlight)
                    } else {
                        Ok(SubmissionReservation::Existing(serde_json::from_str(
                            &response_json,
                        )?))
                    }
                }
            };
        }
        if let Some(existing) = self.submissions.get(key) {
            if existing.quote_id != quote_id {
                return Ok(SubmissionReservation::Conflict);
            }
            return Ok(SubmissionReservation::Existing(existing.clone()));
        }
        match self.reservations.entry(key.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(existing) => {
                let value = existing.get().as_str();
                Ok(if value == quote_id {
                    SubmissionReservation::InFlight
                } else if value == format!("unknown:{quote_id}") {
                    SubmissionReservation::Unknown
                } else if let Some(unknown_quote) = value.strip_prefix("unknown:") {
                    if unknown_quote == quote_id {
                        SubmissionReservation::Unknown
                    } else {
                        SubmissionReservation::Conflict
                    }
                } else {
                    SubmissionReservation::Conflict
                })
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                vacant.insert(quote_id.to_string());
                Ok(SubmissionReservation::Reserved)
            }
        }
    }

    pub async fn mark_submitting(&self, key: &str, quote_id: &str) -> Result<()> {
        let (wallet_id, idempotency_key) = key
            .split_once(':')
            .context("invalid aggregator idempotency key")?;
        if let Some(pool) = &self.pool {
            let result = sqlx::query(
                "UPDATE aggregator_submissions SET status='submitting',updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE wallet_id=? AND idempotency_key=? AND quote_id=? AND status='reserved'",
            )
            .bind(wallet_id)
            .bind(idempotency_key)
            .bind(quote_id)
            .execute(pool)
            .await?;
            if result.rows_affected() != 1 {
                bail!("aggregator submission reservation is missing or already consumed");
            }
            return Ok(());
        }
        match self.reservations.get(key) {
            Some(entry) if entry.as_str() == quote_id => Ok(()),
            Some(_) => bail!("aggregator submission reservation quote mismatch"),
            None => bail!("aggregator submission reservation is missing"),
        }
    }

    pub async fn record_submission(
        &self,
        key: String,
        response: SolBuySubmitResponse,
    ) -> Result<()> {
        let (wallet_id, idempotency_key) = key
            .split_once(':')
            .context("invalid aggregator idempotency key")?;
        if response.wallet_id != wallet_id {
            bail!("aggregator submission wallet mismatch");
        }
        let Some(pool) = &self.pool else {
            let expected_quote = self
                .reservations
                .get(&key)
                .map(|entry| entry.clone())
                .context("aggregator submission reservation is missing")?;
            if expected_quote != response.quote_id {
                bail!("aggregator submission quote mismatch");
            }
            match self.submissions.entry(key.clone()) {
                dashmap::mapref::entry::Entry::Occupied(_) => {
                    bail!("aggregator submission reservation is already terminal")
                }
                dashmap::mapref::entry::Entry::Vacant(entry) => {
                    entry.insert(response);
                }
            }
            self.reservations.remove(&key);
            return Ok(());
        };
        let now = now_ms();
        let response_json = serde_json::to_string(&response)?;
        let quote_metadata = sqlx::query(
            "SELECT route_fingerprint,message_hash FROM aggregator_quotes WHERE quote_id=?",
        )
        .bind(&response.quote_id)
        .fetch_optional(pool)
        .await?
        .context("aggregator submission quote is missing")?;
        let fingerprint: String = quote_metadata.get("route_fingerprint");
        let message_hash: Option<String> = quote_metadata.try_get("message_hash")?;
        let updated = sqlx::query(
            "UPDATE aggregator_submissions SET fingerprint=?,message_hash=?,status=?,response_json=?,updated_at_ms=? WHERE wallet_id=? AND idempotency_key=? AND quote_id=? AND status IN ('reserved','submitting')",
        )
        .bind(fingerprint)
        .bind(message_hash)
        .bind(&response.status)
        .bind(response_json)
        .bind(i64::try_from(now)?)
        .bind(wallet_id)
        .bind(idempotency_key)
        .bind(&response.quote_id)
        .execute(pool)
        .await?
        .rows_affected();
        if updated != 1 {
            bail!("aggregator submission reservation is missing or already terminal");
        }
        // Publish the in-memory terminal response only after the durable write
        // succeeds. This prevents a failed SQLite write from making a caller
        // observe a result that would disappear on restart.
        self.reservations.remove(&key);
        self.submissions.insert(key, response);
        Ok(())
    }

    /// Persist the provider response and the associated preflight evidence as
    /// one guarded state transition. Any persistence failure closes the
    /// reservation as unknown so callers cannot silently retry with a new key.
    pub async fn finalize_submission(
        &self,
        key: &str,
        response: SolBuySubmitResponse,
    ) -> Result<()> {
        let quote_id = response.quote_id.clone();
        match self.lookup_submission(key).await? {
            Some(SubmissionReservation::Reserved | SubmissionReservation::InFlight) => {}
            Some(SubmissionReservation::Unknown) => {
                bail!("aggregator submission is already unknown")
            }
            Some(SubmissionReservation::Existing(_)) => {
                bail!("aggregator submission reservation is already terminal")
            }
            Some(SubmissionReservation::Conflict) | None => {
                bail!("aggregator submission reservation is missing")
            }
        }
        if !self.submission_matches_quote(key, &quote_id).await? {
            bail!("aggregator submission is not bound to quote");
        }
        if let Err(error) = self.record_preflight(&quote_id, &response.preflight).await {
            let _ = self.mark_unknown(key, &quote_id, &error.to_string()).await;
            return Err(error);
        }
        let quote_status = if response.preflight.local_simulation_ok {
            "preflighted"
        } else {
            "failed"
        };
        if let Err(error) = self.mark_quote_status(&quote_id, quote_status).await {
            let _ = self.mark_unknown(key, &quote_id, &error.to_string()).await;
            return Err(error);
        }
        if let Err(error) = self.record_submission(key.to_string(), response).await {
            let _ = self.mark_unknown(key, &quote_id, &error.to_string()).await;
            return Err(error);
        }
        Ok(())
    }

    /// Mark a provider call as unknown after the reservation has reached the
    /// provider boundary but no structured response was returned. This closes
    /// the reservation so a retry cannot silently create a second transaction.
    pub async fn mark_unknown(&self, key: &str, quote_id: &str, error: &str) -> Result<()> {
        if let Some(pool) = &self.pool {
            let error = crate::sanitize::redact_urls(error);
            let (wallet_id, idempotency_key) = key
                .split_once(':')
                .context("invalid aggregator idempotency key")?;
            let response_json = serde_json::json!({
                "quote_id": quote_id,
                "status": "unknown",
                "error": error,
            })
            .to_string();
            let result = sqlx::query(
                "UPDATE aggregator_submissions SET status='unknown',response_json=?,updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE wallet_id=? AND idempotency_key=? AND quote_id=? AND status IN ('reserved','submitting')",
            )
            .bind(response_json)
            .bind(wallet_id)
            .bind(idempotency_key)
            .bind(quote_id)
            .execute(pool)
            .await?;
            if result.rows_affected() != 1 {
                bail!("aggregator unknown reservation is missing or already terminal");
            }
            return Ok(());
        }
        self.reservations
            .insert(key.to_string(), format!("unknown:{quote_id}"));
        Ok(())
    }
}

async fn ensure_preflight_columns(pool: &SqlitePool) -> Result<()> {
    for (name, definition) in [("preflight_json", "TEXT"), ("audit_json", "TEXT")] {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pragma_table_info('aggregator_quotes') WHERE name=?",
        )
        .bind(name)
        .fetch_one(pool)
        .await?;
        if exists == 0 {
            let sql = format!("ALTER TABLE aggregator_quotes ADD COLUMN {name} {definition}");
            sqlx::query(&sql).execute(pool).await?;
        }
    }
    sqlx::query(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (4, CAST(strftime('%s','now') AS INTEGER) * 1000)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_balance_snapshot_column(pool: &SqlitePool) -> Result<()> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info('aggregator_quotes') WHERE name='balance_snapshot_json'",
    )
    .fetch_one(pool)
    .await?;
    if exists == 0 {
        sqlx::query("ALTER TABLE aggregator_quotes ADD COLUMN balance_snapshot_json TEXT")
            .execute(pool)
            .await?;
    }
    sqlx::query(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (5, CAST(strftime('%s','now') AS INTEGER) * 1000)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_shadow_route_length_column(pool: &SqlitePool) -> Result<()> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info('aggregator_shadow_metrics') WHERE name='route_length'",
    )
    .fetch_one(pool)
    .await?;
    if exists == 0 {
        sqlx::query("ALTER TABLE aggregator_shadow_metrics ADD COLUMN route_length INTEGER")
            .execute(pool)
            .await?;
    }
    sqlx::query(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (6, CAST(strftime('%s','now') AS INTEGER) * 1000)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

impl Default for QuoteStore {
    fn default() -> Self {
        Self {
            quotes: DashMap::new(),
            reservations: DashMap::new(),
            submissions: DashMap::new(),
            pool: None,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn current_time_ms() -> u64 {
    now_ms()
}

#[derive(Deserialize)]
struct JupiterQuote {
    #[serde(rename = "inputMint")]
    input_mint: String,
    #[serde(rename = "outputMint")]
    output_mint: String,
    #[serde(rename = "inAmount")]
    in_amount: String,
    #[serde(rename = "outAmount")]
    out_amount: String,
    #[serde(rename = "otherAmountThreshold")]
    other_amount_threshold: String,
    #[serde(rename = "swapMode")]
    swap_mode: String,
    #[serde(rename = "slippageBps")]
    slippage_bps: u16,
    #[serde(rename = "priceImpactPct")]
    price_impact_pct: String,
    #[serde(rename = "routePlan")]
    route_plan: Vec<JupiterLeg>,
}

#[derive(Deserialize)]
struct JupiterLeg {
    #[serde(rename = "swapInfo")]
    swap_info: JupiterSwapInfo,
}

#[derive(Deserialize)]
struct JupiterSwapInfo {
    label: String,
    #[serde(rename = "inputMint")]
    input_mint: String,
    #[serde(rename = "outputMint")]
    output_mint: String,
}

pub fn validate_request(request: &SolBuyQuoteRequest) -> Result<()> {
    if request.wallet_id.trim().is_empty() || request.wallet_id.len() > 128 {
        bail!("wallet_id is required and must be at most 128 characters");
    }
    Pubkey::from_str(&request.mint).context("mint must be a valid pubkey")?;
    if request.mint == WSOL_MINT {
        bail!("output mint must differ from WSOL");
    }
    if request.amount_lamports < 100_000 || request.amount_lamports > 100_000_000 {
        bail!("amount_lamports must be between 0.0001 and 0.1 SOL");
    }
    if request.slippage_bps == 0 || request.slippage_bps > 3000 {
        bail!("slippage_bps must be between 1 and 3000");
    }
    Ok(())
}

pub async fn quote(request: &SolBuyQuoteRequest) -> Result<SolBuyQuoteResponse> {
    validate_request(request)?;
    let quote: JupiterQuote = serde_json::from_value(fetch_quote(request).await?)?;
    validate_quote(request, quote)
}

async fn fetch_quote(request: &SolBuyQuoteRequest) -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let response = client
        .get(JUPITER_QUOTE_URL)
        .query(&[
            ("inputMint", WSOL_MINT.to_string()),
            ("outputMint", request.mint.clone()),
            ("amount", request.amount_lamports.to_string()),
            ("slippageBps", request.slippage_bps.to_string()),
        ])
        .send()
        .await
        .context("aggregator quote unavailable")?;
    if !response.status().is_success() {
        bail!("aggregator quote returned {}", response.status());
    }
    response.json().await.context("invalid aggregator quote")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JupiterSwapRequest<'a> {
    quote_response: &'a serde_json::Value,
    user_public_key: &'a str,
    wrap_and_unwrap_sol: bool,
    dynamic_compute_unit_limit: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterSwapResponse {
    swap_transaction: Option<String>,
    simulation_error: Option<serde_json::Value>,
    prioritization_fee_lamports: Option<u64>,
    compute_unit_limit: Option<u64>,
}

fn require_minimum_preflight_balance(balance_lamports: u64, amount_lamports: u64) -> Result<()> {
    let minimum = amount_lamports
        .checked_add(MAX_PRIORITY_FEE_LAMPORTS)
        .and_then(|amount| amount.checked_add(MAX_BASE_FEE_LAMPORTS))
        .and_then(|amount| amount.checked_add(PREFLIGHT_BALANCE_RESERVE_LAMPORTS))
        .context("aggregator preflight minimum balance overflow")?;
    if balance_lamports < minimum {
        bail!("wallet SOL balance is below aggregator amount, fee cap and reserve");
    }
    Ok(())
}

fn require_simulated_preflight_balance(
    baseline_lamports: u64,
    simulated_pre_lamports: Option<u64>,
    simulated_debit_lamports: Option<u64>,
    simulation_ok: bool,
) -> Result<()> {
    if !simulation_ok {
        return Ok(());
    }
    let simulated_pre_lamports = simulated_pre_lamports
        .context("successful aggregator simulation lacks payer pre-balance")?;
    let simulated_debit_lamports =
        simulated_debit_lamports.context("successful aggregator simulation lacks payer debit")?;
    let minimum = simulated_debit_lamports
        .checked_add(PREFLIGHT_BALANCE_RESERVE_LAMPORTS)
        .context("aggregator simulated debit and reserve overflow")?;
    if baseline_lamports < minimum || simulated_pre_lamports < minimum {
        bail!("wallet SOL balance is below simulated aggregator debit and reserve");
    }
    Ok(())
}

fn require_simulated_target_output(
    simulation_ok: bool,
    token_balance_delta: Option<u64>,
    minimum_output_units: u64,
) -> Result<()> {
    if !simulation_ok {
        return Ok(());
    }
    // An observed same-slot ATA difference can include unrelated state
    // changes. Only the simulation's own before/after token balances prove
    // output from the simulated transaction.
    let delta = token_balance_delta
        .context("successful aggregator simulation lacks verified target token output")?;
    if delta < minimum_output_units {
        bail!("local simulation target token output is below quote minimum");
    }
    Ok(())
}

fn validate_preflight_bound(bound: &BoundSolBuyQuote, wallet_pubkey: &str) -> Result<()> {
    if bound.provider != "jupiter" {
        bail!("quote provider is not Jupiter");
    }
    if bound.wallet_pubkey != wallet_pubkey {
        bail!("quote wallet mismatch");
    }
    if bound.expires_at_ms <= now_ms() {
        bail!("quote expired");
    }
    if !matches!(bound.status.as_str(), "quoted" | "preflighted") {
        bail!("quote is terminal; request a new quote");
    }
    let route_fingerprint = format!(
        "{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&bound.quote.route)?)
    );
    if bound.route_fingerprint != route_fingerprint
        || bound.provider_quote_id != format!("jup:{route_fingerprint}")
    {
        bail!("quote route binding mismatch");
    }
    let request = SolBuyQuoteRequest {
        wallet_id: bound.wallet_id.clone(),
        mint: bound.quote.output_mint.clone(),
        amount_lamports: bound.quote.amount_lamports,
        slippage_bps: bound.quote_slippage_bps,
    };
    validate_request(&request)?;
    if bound.quote.input_mint != WSOL_MINT {
        bail!("quote input mint is not WSOL");
    }
    if bound.quote.expected_output_units == 0
        || bound.quote.minimum_output_units == 0
        || bound.quote.minimum_output_units > bound.quote.expected_output_units
    {
        bail!("quote has invalid output amount");
    }
    let minimum_allowed = (bound.quote.expected_output_units as u128)
        .saturating_mul(10_000 - bound.quote_slippage_bps as u128)
        / 10_000;
    if (bound.quote.minimum_output_units as u128) < minimum_allowed {
        bail!("quote exceeds requested slippage");
    }
    if bound.quote.route.is_empty() || bound.quote.route.len() > 4 {
        bail!("quote has unsupported route length");
    }
    let mut current_mint = WSOL_MINT;
    for leg in &bound.quote.route {
        if leg.label.is_empty() || leg.input_mint != current_mint {
            bail!("quote route is discontinuous");
        }
        Pubkey::from_str(&leg.output_mint).context("invalid quote route mint")?;
        current_mint = &leg.output_mint;
    }
    if current_mint != request.mint {
        bail!("quote route does not end at requested mint");
    }
    let price_impact = bound.quote.price_impact_percent.parse::<f64>()?;
    if !price_impact.is_finite() || !(0.0..=10.0).contains(&price_impact) {
        bail!("quote exceeds price impact limit");
    }
    let request_hash = format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(&request)?));
    if bound.request_hash != request_hash {
        bail!("quote request binding mismatch");
    }
    Ok(())
}

async fn prepare_transaction(
    bound: &BoundSolBuyQuote,
    wallet: &SdkTradeClient,
) -> Result<(VersionedTransaction, SolBuyPreflightResponse)> {
    validate_preflight_bound(bound, &wallet.wallet().to_string())?;
    // A quote balance snapshot is only evidence from quote time. Re-read the
    // wallet before contacting Jupiter for a new transaction, and leave room
    // for the capped transaction fee and a small remaining SOL reserve.
    let current_balance = wallet.wallet_balance().await?;
    require_minimum_preflight_balance(current_balance, bound.quote.amount_lamports)?;
    let request = SolBuyQuoteRequest {
        wallet_id: bound.wallet_id.clone(),
        mint: bound.quote.output_mint.clone(),
        amount_lamports: bound.quote.amount_lamports,
        slippage_bps: bound.quote_slippage_bps,
    };
    let fresh_raw = fetch_quote(&request).await?;
    let fresh = validate_quote(&request, serde_json::from_value(fresh_raw.clone())?)?;
    if fresh.route != bound.quote.route
        || fresh.minimum_output_units < bound.quote.minimum_output_units
    {
        bail!("aggregator route or minimum output changed; request a new quote");
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let response = client
        .post(JUPITER_SWAP_URL)
        .json(&JupiterSwapRequest {
            quote_response: &fresh_raw,
            user_public_key: &bound.wallet_pubkey,
            wrap_and_unwrap_sol: true,
            dynamic_compute_unit_limit: true,
        })
        .send()
        .await
        .context("aggregator unsigned transaction unavailable")?;
    if !response.status().is_success() {
        bail!(
            "aggregator unsigned transaction returned {}",
            response.status()
        );
    }
    let swap: JupiterSwapResponse = response
        .json()
        .await
        .context("invalid aggregator swap response")?;
    if swap.simulation_error.is_some() {
        bail!("aggregator reported simulation error");
    }
    let encoded = swap
        .swap_transaction
        .context("aggregator did not return a transaction")?;
    if encoded.len() > 2048 {
        bail!("aggregator transaction response is too large");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("invalid transaction base64")?;
    if bytes.len() > 1232 {
        bail!("aggregator transaction exceeds Solana packet size");
    }
    let transaction: VersionedTransaction =
        bincode::deserialize(&bytes).context("invalid unsigned transaction")?;
    let (payer, instruction_count) =
        validate_unsigned_envelope(&transaction, &bound.wallet_pubkey)?;
    let static_keys = transaction.message.static_account_keys();
    let lookups = transaction.message.address_table_lookups().unwrap_or(&[]);
    let lookup_count = lookups.len();
    if lookup_count > 4 {
        bail!("aggregator transaction has too many address lookup tables");
    }
    let mut writable = Vec::new();
    let mut readonly = Vec::new();
    for lookup in lookups {
        let table = wallet
            .resolve_aggregator_lookup_table(
                &lookup.account_key,
                &lookup.writable_indexes,
                &lookup.readonly_indexes,
            )
            .await?;
        for index in &lookup.writable_indexes {
            writable.push(
                *table
                    .get(usize::from(*index))
                    .context("invalid writable lookup index")?,
            );
        }
        for index in &lookup.readonly_indexes {
            readonly.push(
                *table
                    .get(usize::from(*index))
                    .context("invalid readonly lookup index")?,
            );
        }
    }
    let mut resolved_keys = Vec::with_capacity(static_keys.len() + writable.len() + readonly.len());
    resolved_keys.extend_from_slice(static_keys);
    resolved_keys.extend(writable);
    resolved_keys.extend(readonly);
    if resolved_keys.len() > 256 {
        bail!("aggregator transaction has too many resolved accounts");
    }
    if resolved_keys.iter().copied().collect::<HashSet<_>>().len() != resolved_keys.len() {
        bail!("aggregator transaction has duplicate resolved accounts");
    }
    let writable_account_count = (0..resolved_keys.len())
        .filter(|index| {
            transaction
                .message
                .is_maybe_writable_with_reserved_addresses(*index, None::<&HashSet<Pubkey>>)
        })
        .count();
    if writable_account_count > MAX_WRITABLE_ACCOUNTS {
        bail!("aggregator transaction has too many writable accounts");
    }
    let mint = Pubkey::from_str(&bound.quote.output_mint).context("invalid bound target mint")?;
    let (target_ata, token_program) = wallet.aggregator_target_ata(&mint).await?;
    let target_index =
        require_writable_target_ata(&transaction.message, &resolved_keys, &target_ata)?;
    let mut top_level_programs = Vec::new();
    for instruction in transaction.message.instructions() {
        if instruction.data.len() > 1024 {
            bail!("aggregator instruction data exceeds audit limit");
        }
        let program_index = usize::from(instruction.program_id_index);
        if program_index >= resolved_keys.len()
            || instruction
                .accounts
                .iter()
                .any(|index| usize::from(*index) >= resolved_keys.len())
        {
            bail!("aggregator instruction references an invalid account index");
        }
        let accounts = instruction
            .accounts
            .iter()
            .map(|index| resolved_keys[usize::from(*index)])
            .collect::<Vec<_>>();
        check_target_token_instruction(
            &resolved_keys[program_index],
            &instruction.data,
            &accounts,
            &target_ata,
        )?;
        let program = resolved_keys[program_index].to_string();
        if !top_level_programs.contains(&program) {
            top_level_programs.push(program);
        }
    }
    let (compute_unit_limit, calculated_priority_fee_lamports) =
        read_compute_budget_fee(transaction.message.instructions(), &resolved_keys)?;
    let explicit_system_spend_lamports = read_explicit_system_spend(
        transaction.message.instructions(),
        &resolved_keys,
        payer,
        bound.quote.amount_lamports,
    )?;
    validate_fee_metadata(
        swap.compute_unit_limit,
        swap.prioritization_fee_lamports,
        compute_unit_limit,
        calculated_priority_fee_lamports,
    )?;
    let simulation = wallet
        .simulate_unsigned_aggregator_transaction(
            &transaction,
            target_index,
            &mint,
            &target_ata,
            &token_program,
            &resolved_keys,
        )
        .await?;
    require_simulated_preflight_balance(
        simulation.baseline_payer_lamports,
        simulation.simulated_pre_payer_lamports,
        simulation.payer_debit_lamports,
        simulation.ok,
    )?;
    require_simulated_target_output(
        simulation.ok,
        simulation.target_token_delta_units,
        bound.quote.minimum_output_units,
    )?;
    if let Some(fee) = simulation.fee_lamports {
        if fee > calculated_priority_fee_lamports.saturating_add(MAX_BASE_FEE_LAMPORTS) {
            bail!("local simulation fee exceeds preflight cap");
        }
    }
    if let Some(debit) = simulation.payer_debit_lamports {
        let maximum_debit = bound
            .quote
            .amount_lamports
            .saturating_add(MAX_EXTRA_SYSTEM_SPEND_LAMPORTS)
            .saturating_add(MAX_PRIORITY_FEE_LAMPORTS)
            .saturating_add(MAX_BASE_FEE_LAMPORTS);
        if debit > maximum_debit {
            bail!("local simulation payer debit exceeds preflight cap");
        }
    }
    if let Some(inner_spend) = simulation.inner_payer_system_spend_lamports {
        let combined_spend = explicit_system_spend_lamports
            .checked_add(inner_spend)
            .context("combined System transfer total overflow")?;
        if combined_spend
            > bound
                .quote
                .amount_lamports
                .saturating_add(MAX_EXTRA_SYSTEM_SPEND_LAMPORTS)
        {
            bail!("simulated inner payer System transfers exceed preflight cap");
        }
    }
    let preflight = SolBuyPreflightResponse {
        quote_id: bound.quote_id.clone(),
        wallet_id: bound.wallet_id.clone(),
        expected_output_units: bound.quote.expected_output_units,
        minimum_output_units: bound.quote.minimum_output_units,
        quote_still_valid: true,
        unsigned_transaction_built: true,
        aggregator_simulation_ok: true,
        local_simulation_ok: simulation.ok,
        instruction_count,
        address_lookup_table_count: lookup_count,
        resolved_account_count: resolved_keys.len(),
        // Gate 1 only proves that the unsigned message can be structurally
        // inspected and simulated.  It does not prove that every top-level
        // and inner program is allowlisted or that all balance movements are
        // bounded.  Keep both flags closed until the independent Gate 2 audit
        // is implemented and recorded.
        instruction_audit_passed: false,
        executable: false,
        reason: if simulation.ok {
            "Gate 1 route, target account, fee bounds and local unsigned simulation passed; Gate 2 instruction audit is pending"
        } else {
            "local unsigned simulation failed; transaction is not executable"
        }
        .to_string(),
        prioritization_fee_lamports: calculated_priority_fee_lamports,
        compute_unit_limit,
        calculated_priority_fee_lamports,
        explicit_system_spend_lamports,
        writable_account_count,
        target_ata: target_ata.to_string(),
        top_level_programs,
        simulated_fee_lamports: simulation.fee_lamports,
        simulated_units_consumed: simulation.units_consumed,
        simulated_payer_debit_lamports: simulation.payer_debit_lamports,
        simulated_inner_instruction_groups: simulation.inner_instruction_groups,
        simulated_target_token_delta_units: simulation.target_token_delta_units,
        simulated_target_token_post_units: simulation.target_token_post_units,
        baseline_slot: simulation.baseline_slot,
        simulation_slot: simulation.simulation_slot,
        observed_same_slot_payer_debit_lamports: simulation.observed_same_slot_payer_debit_lamports,
        observed_same_slot_target_token_delta_units: simulation
            .observed_same_slot_target_token_delta_units,
        simulated_inner_instruction_count: simulation.inner_instruction_count,
        simulated_inner_programs: simulation.inner_programs,
        simulated_inner_payer_system_spend_lamports: simulation.inner_payer_system_spend_lamports,
    };
    Ok((transaction, preflight))
}

pub async fn preflight(
    bound: &BoundSolBuyQuote,
    wallet: &SdkTradeClient,
) -> Result<SolBuyPreflightResponse> {
    Ok(prepare_transaction(bound, wallet).await?.1)
}

pub(crate) fn require_gate_two_for_submission(simulate: bool) -> Result<()> {
    if !simulate {
        bail!("aggregator Gate 2 instruction audit is incomplete");
    }
    Ok(())
}

pub async fn submit(
    bound: &BoundSolBuyQuote,
    wallet: &SdkTradeClient,
    request: &SolBuySubmitRequest,
) -> Result<SolBuySubmitResponse> {
    if request.wallet_id != bound.wallet_id {
        bail!("quote wallet mismatch");
    }
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        bail!("idempotency_key is required and must be at most 128 characters");
    }
    if !request.simulate && !request.confirmed {
        bail!("live SOL buy requires confirmed=true");
    }
    // Gate 2 must fail before requesting a fresh quote or swap transaction.
    require_gate_two_for_submission(request.simulate)?;
    let (unsigned, preflight) = prepare_transaction(bound, wallet).await?;
    if request.simulate {
        if !preflight.local_simulation_ok {
            return Ok(SolBuySubmitResponse {
                quote_id: bound.quote_id.clone(),
                wallet_id: bound.wallet_id.clone(),
                status: "failed".to_string(),
                signature: None,
                error: Some(preflight.reason.clone()),
                preflight,
            });
        }
        return Ok(SolBuySubmitResponse {
            quote_id: bound.quote_id.clone(),
            wallet_id: bound.wallet_id.clone(),
            status: "simulated".to_string(),
            signature: None,
            error: None,
            preflight,
        });
    }
    // Defense in depth for any future change to the entry guard.
    if !preflight.executable {
        bail!("aggregator Gate 2 instruction audit is incomplete");
    }
    let signer = wallet.conversion_signer();
    let transaction = VersionedTransaction::try_new(unsigned.message, &[signer.as_ref()])
        .context("failed to sign aggregator transaction")?;
    match wallet.send_aggregator_transaction(&transaction).await {
        Ok(signature) => Ok(SolBuySubmitResponse {
            quote_id: bound.quote_id.clone(),
            wallet_id: bound.wallet_id.clone(),
            status: "submitted".to_string(),
            signature: Some(signature.to_string()),
            error: None,
            preflight,
        }),
        Err(error) => Ok(SolBuySubmitResponse {
            quote_id: bound.quote_id.clone(),
            wallet_id: bound.wallet_id.clone(),
            status: "unknown".to_string(),
            signature: None,
            error: Some(crate::sanitize::redact_urls(&error.to_string())),
            preflight,
        }),
    }
}

fn validate_quote(
    request: &SolBuyQuoteRequest,
    quote: JupiterQuote,
) -> Result<SolBuyQuoteResponse> {
    if quote.input_mint != WSOL_MINT
        || quote.output_mint != request.mint
        || quote.swap_mode != "ExactIn"
        || quote.slippage_bps != request.slippage_bps
    {
        bail!("aggregator quote does not match requested trade");
    }
    if quote.in_amount.parse::<u64>()? != request.amount_lamports {
        bail!("aggregator quote changed input amount");
    }
    let expected_output_units = quote.out_amount.parse::<u64>()?;
    let minimum_output_units = quote.other_amount_threshold.parse::<u64>()?;
    if expected_output_units == 0
        || minimum_output_units == 0
        || minimum_output_units > expected_output_units
    {
        bail!("aggregator quote has invalid output amount");
    }
    let minimum_allowed = (expected_output_units as u128)
        .saturating_mul(10_000 - request.slippage_bps as u128)
        / 10_000;
    if (minimum_output_units as u128) < minimum_allowed {
        bail!("aggregator quote exceeds requested slippage");
    }
    if quote.route_plan.is_empty() || quote.route_plan.len() > 4 {
        bail!("aggregator quote has unsupported route length");
    }
    let mut current_mint = WSOL_MINT.to_string();
    let mut route = Vec::with_capacity(quote.route_plan.len());
    for leg in quote.route_plan {
        if leg.swap_info.input_mint != current_mint || leg.swap_info.label.is_empty() {
            bail!("aggregator quote has discontinuous route");
        }
        Pubkey::from_str(&leg.swap_info.output_mint).context("invalid route mint")?;
        current_mint = leg.swap_info.output_mint.clone();
        route.push(QuoteLeg {
            label: leg.swap_info.label,
            input_mint: leg.swap_info.input_mint,
            output_mint: leg.swap_info.output_mint,
        });
    }
    if current_mint != request.mint {
        bail!("aggregator route does not end at requested mint");
    }
    let price_impact = quote.price_impact_pct.parse::<f64>()?;
    if !price_impact.is_finite() || price_impact < 0.0 || price_impact > 10.0 {
        bail!("aggregator quote exceeds price impact limit");
    }
    Ok(SolBuyQuoteResponse {
        input_mint: WSOL_MINT.to_string(),
        output_mint: request.mint.clone(),
        amount_lamports: request.amount_lamports,
        expected_output_units,
        minimum_output_units,
        price_impact_percent: quote.price_impact_pct,
        route,
        executable: false,
        balance_snapshot: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_two_rejects_live_submission_before_transaction_preparation() {
        assert!(require_gate_two_for_submission(true).is_ok());
        assert!(require_gate_two_for_submission(false)
            .unwrap_err()
            .to_string()
            .contains("Gate 2 instruction audit is incomplete"));
    }

    #[test]
    fn preflight_rechecks_current_balance_and_simulated_debit() {
        let minimum = 1_000_000
            + MAX_PRIORITY_FEE_LAMPORTS
            + MAX_BASE_FEE_LAMPORTS
            + PREFLIGHT_BALANCE_RESERVE_LAMPORTS;
        assert!(require_minimum_preflight_balance(minimum, 1_000_000).is_ok());
        assert!(require_minimum_preflight_balance(minimum - 1, 1_000_000).is_err());
        assert!(require_minimum_preflight_balance(u64::MAX, u64::MAX).is_err());

        assert!(require_simulated_preflight_balance(
            1_500_000,
            Some(1_500_000),
            Some(1_490_000),
            true,
        )
        .is_ok());
        assert!(require_simulated_preflight_balance(
            1_500_000,
            Some(1_499_999),
            Some(1_490_000),
            true,
        )
        .is_err());
        assert!(require_simulated_preflight_balance(
            1_499_999,
            Some(1_500_000),
            Some(1_490_000),
            true,
        )
        .is_err());
        assert!(require_simulated_preflight_balance(1_500_000, None, Some(1), true).is_err());
        assert!(
            require_simulated_preflight_balance(1_500_000, Some(1_500_000), None, true).is_err()
        );
        assert!(require_simulated_preflight_balance(0, None, None, false).is_ok());
    }

    #[test]
    fn successful_preflight_requires_simulation_native_minimum_output() {
        assert!(require_simulated_target_output(true, None, 100).is_err());
        assert!(require_simulated_target_output(true, Some(99), 100).is_err());
        assert!(require_simulated_target_output(true, Some(100), 100).is_ok());
        assert!(require_simulated_target_output(false, None, 100).is_ok());
    }

    #[test]
    fn fee_metadata_allows_one_lamport_rounding_only() {
        assert!(validate_fee_metadata(Some(234_553), Some(99_999), 234_553, 100_000).is_ok());
        assert!(validate_fee_metadata(Some(234_554), Some(99_999), 234_553, 100_000).is_err());
        assert!(validate_fee_metadata(Some(234_553), Some(99_998), 234_553, 100_000).is_err());
        assert!(validate_fee_metadata(None, None, 234_553, 100_000).is_ok());
    }

    #[test]
    fn target_ata_must_be_present_and_writable() {
        let payer = Pubkey::new_unique();
        let ata = Pubkey::new_unique();
        let message = VersionedMessage::Legacy(solana_sdk::message::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![payer, ata],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![],
        });
        assert!(require_writable_target_ata(&message, &[payer, ata], &ata).is_ok());
        assert!(
            require_writable_target_ata(&message, &[payer, ata], &Pubkey::new_unique()).is_err()
        );
        let readonly_message = VersionedMessage::Legacy(solana_sdk::message::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![payer, ata],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![],
        });
        assert!(require_writable_target_ata(&readonly_message, &[payer, ata], &ata).is_err());
    }

    #[test]
    fn unsigned_envelope_rejects_extra_signer_wrong_payer_and_malformed_message() {
        let payer = Pubkey::new_unique();
        let program = Pubkey::default();
        let instruction = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 1,
            accounts: vec![0],
            data: vec![1],
        };
        let transaction = VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::Legacy(solana_sdk::message::Message {
                header: solana_sdk::message::MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 1,
                },
                account_keys: vec![payer, program],
                recent_blockhash: solana_sdk::hash::Hash::new_unique(),
                instructions: vec![instruction.clone()],
            }),
        };
        assert_eq!(
            validate_unsigned_envelope(&transaction, &payer.to_string())
                .unwrap()
                .1,
            1
        );
        assert!(
            validate_unsigned_envelope(&transaction, &Pubkey::new_unique().to_string())
                .unwrap_err()
                .to_string()
                .contains("payer or signature count")
        );

        let mut changed = transaction.clone();
        changed.signatures[0] = Signature::from([1u8; 64]);
        assert!(validate_unsigned_envelope(&changed, &payer.to_string())
            .unwrap_err()
            .to_string()
            .contains("payer or signature count"));

        let mut extra_signer = transaction.clone();
        extra_signer.signatures.push(Signature::default());
        if let VersionedMessage::Legacy(message) = &mut extra_signer.message {
            message.header.num_required_signatures = 2;
            message.account_keys.insert(1, Pubkey::new_unique());
            message.instructions[0].program_id_index = 2;
        }
        assert!(
            validate_unsigned_envelope(&extra_signer, &payer.to_string())
                .unwrap_err()
                .to_string()
                .contains("payer or signature count")
        );

        let mut no_instructions = transaction.clone();
        if let VersionedMessage::Legacy(message) = &mut no_instructions.message {
            message.instructions.clear();
        }
        assert!(
            validate_unsigned_envelope(&no_instructions, &payer.to_string())
                .unwrap_err()
                .to_string()
                .contains("unsupported instruction count")
        );

        let mut too_many = transaction.clone();
        if let VersionedMessage::Legacy(message) = &mut too_many.message {
            message.instructions = vec![instruction; 33];
        }
        assert!(validate_unsigned_envelope(&too_many, &payer.to_string())
            .unwrap_err()
            .to_string()
            .contains("unsupported instruction count"));

        let mut bad_index = transaction;
        if let VersionedMessage::Legacy(message) = &mut bad_index.message {
            message.instructions[0].program_id_index = 3;
        }
        assert!(validate_unsigned_envelope(&bad_index, &payer.to_string())
            .unwrap_err()
            .to_string()
            .contains("failed Solana sanitization"));
    }

    fn system_transfer(
        sender_index: u8,
        lamports: u64,
    ) -> solana_sdk::message::compiled_instruction::CompiledInstruction {
        solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![sender_index, 2],
            data: [vec![2, 0, 0, 0], lamports.to_le_bytes().to_vec()].concat(),
        }
    }

    #[test]
    fn system_spend_accepts_only_bounded_wallet_wsol_funding() {
        let payer = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let wsol_mint = Pubkey::from_str(WSOL_MINT).unwrap();
        let wsol_ata = get_associated_token_address_with_program_id(
            &payer,
            &wsol_mint,
            &spl_token_interface::ID,
        );
        let keys = [Pubkey::default(), payer, wsol_ata, other];
        assert_eq!(
            read_explicit_system_spend(&[system_transfer(1, 1_000_000)], &keys, &payer, 1_000_000)
                .unwrap(),
            1_000_000
        );
        let mut hidden_tip = system_transfer(1, 1);
        hidden_tip.accounts[1] = 3;
        assert!(
            read_explicit_system_spend(&[hidden_tip], &keys, &payer, 1_000_000)
                .unwrap_err()
                .to_string()
                .contains("recipient is not wallet WSOL ATA")
        );
        assert!(
            read_explicit_system_spend(&[system_transfer(2, 1)], &keys, &payer, 1_000_000).is_err()
        );
        assert!(read_explicit_system_spend(
            &[system_transfer(1, 1_000_001)],
            &keys,
            &payer,
            1_000_000
        )
        .is_err());
        let mut unsupported = system_transfer(1, 1);
        unsupported.data[0] = 0;
        assert!(read_explicit_system_spend(&[unsupported], &keys, &payer, 1_000_000).is_err());
        assert!(read_explicit_system_spend(
            &[system_transfer(1, u64::MAX), system_transfer(1, 1)],
            &keys,
            &payer,
            u64::MAX
        )
        .is_err());
        let mut invalid_recipient = system_transfer(1, 1);
        invalid_recipient.accounts[1] = 4;
        assert!(
            read_explicit_system_spend(&[invalid_recipient], &keys, &payer, 1_000_000).is_err()
        );
    }

    #[test]
    fn compute_budget_fee_is_bounded_and_duplicates_rejected() {
        let program = Pubkey::from_str(COMPUTE_BUDGET_PROGRAM).unwrap();
        let keys = [program];
        let limit = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![2], 1_000_000u32.to_le_bytes().to_vec()].concat(),
        };
        let price = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![3], 10_000u64.to_le_bytes().to_vec()].concat(),
        };
        assert_eq!(
            read_compute_budget_fee(&[limit.clone(), price], &keys).unwrap(),
            (1_000_000, 10_000)
        );
        assert!(read_compute_budget_fee(&[limit.clone(), limit], &keys).is_err());
        let excessive = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![3], 200_000u64.to_le_bytes().to_vec()].concat(),
        };
        let normal = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![2], 1_000_000u32.to_le_bytes().to_vec()].concat(),
        };
        assert!(read_compute_budget_fee(&[normal, excessive], &keys).is_err());
    }

    fn sample_quote() -> JupiterQuote {
        serde_json::from_value(serde_json::json!({
            "inputMint": WSOL_MINT,
            "outputMint": "2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL",
            "inAmount": "1000000",
            "outAmount": "19441607746",
            "otherAmountThreshold": "16525366585",
            "swapMode": "ExactIn",
            "slippageBps": 1500,
            "priceImpactPct": "0.0373",
            "routePlan": [
                {"swapInfo": {"label":"Byreal","inputMint":WSOL_MINT,"outputMint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"}},
                {"swapInfo": {"label":"Raydium CLMM","inputMint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","outputMint":"Xsf9mBktVB9BSU5kf4nHxPq5hCBJ2j2ui3ecFGxPRGc"}},
                {"swapInfo": {"label":"Raydium CP","inputMint":"Xsf9mBktVB9BSU5kf4nHxPq5hCBJ2j2ui3ecFGxPRGc","outputMint":"2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL"}}
            ]
        })).unwrap()
    }

    fn sample_request() -> SolBuyQuoteRequest {
        SolBuyQuoteRequest {
            wallet_id: "2TE".into(),
            mint: "2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL".into(),
            amount_lamports: 1_000_000,
            slippage_bps: 1500,
        }
    }

    fn sample_preflight(quote_id: &str) -> SolBuyPreflightResponse {
        SolBuyPreflightResponse {
            quote_id: quote_id.into(),
            wallet_id: "2TE".into(),
            expected_output_units: 100,
            minimum_output_units: 90,
            quote_still_valid: true,
            unsigned_transaction_built: true,
            aggregator_simulation_ok: true,
            local_simulation_ok: true,
            instruction_count: 1,
            address_lookup_table_count: 0,
            resolved_account_count: 1,
            instruction_audit_passed: false,
            executable: false,
            reason: "Gate 2 remains closed".into(),
            prioritization_fee_lamports: 0,
            compute_unit_limit: 100_000,
            calculated_priority_fee_lamports: 0,
            explicit_system_spend_lamports: 0,
            writable_account_count: 1,
            target_ata: "ata".into(),
            top_level_programs: Vec::new(),
            simulated_fee_lamports: None,
            simulated_units_consumed: None,
            simulated_payer_debit_lamports: None,
            simulated_inner_instruction_groups: None,
            simulated_target_token_delta_units: None,
            simulated_target_token_post_units: None,
            baseline_slot: 1,
            simulation_slot: 1,
            observed_same_slot_payer_debit_lamports: None,
            observed_same_slot_target_token_delta_units: None,
            simulated_inner_instruction_count: None,
            simulated_inner_programs: None,
            simulated_inner_payer_system_spend_lamports: None,
        }
    }

    #[test]
    fn rejects_invalid_amount_and_slippage() {
        let mut request = sample_request();
        assert!(validate_request(&request).is_ok());
        request.amount_lamports = 0;
        assert!(validate_request(&request).is_err());
        request.amount_lamports = 1_000_000;
        request.slippage_bps = 5000;
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn accepts_contiguous_three_hop_quote() {
        let quote = validate_quote(&sample_request(), sample_quote()).unwrap();
        assert_eq!(quote.route.len(), 3);
        assert!(!quote.executable);
        assert_eq!(quote.amount_lamports, 1_000_000);
    }

    #[test]
    fn replayed_jupiter_quote_fixture_fails_closed_on_adversarial_fields() {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/jupiter_two_hop_quote.json"))
                .unwrap();
        let request = SolBuyQuoteRequest {
            wallet_id: "fixture-wallet".into(),
            mint: "2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL".into(),
            amount_lamports: 1_000_000,
            slippage_bps: 100,
        };
        let accepted = validate_quote(&request, serde_json::from_value(raw.clone()).unwrap())
            .expect("valid fixture must pass");
        assert_eq!(accepted.route.len(), 2);
        assert_eq!(accepted.minimum_output_units, 99_000);
        assert!(!accepted.executable);

        for (name, pointer, replacement, error) in [
            (
                "input mint substitution",
                "/inputMint",
                serde_json::json!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
                "does not match requested trade",
            ),
            (
                "output mint substitution",
                "/outputMint",
                serde_json::json!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
                "does not match requested trade",
            ),
            (
                "changed input amount",
                "/inAmount",
                serde_json::json!("2000000"),
                "changed input amount",
            ),
            (
                "minimum output below bound",
                "/otherAmountThreshold",
                serde_json::json!("98999"),
                "exceeds requested slippage",
            ),
            (
                "route discontinuity",
                "/routePlan/1/swapInfo/inputMint",
                serde_json::json!("So11111111111111111111111111111111111111112"),
                "discontinuous route",
            ),
            (
                "malformed route mint",
                "/routePlan/1/swapInfo/outputMint",
                serde_json::json!("invalid-mint"),
                "invalid route mint",
            ),
            (
                "excessive price impact",
                "/priceImpactPct",
                serde_json::json!("10.01"),
                "exceeds price impact limit",
            ),
        ] {
            let mut changed = raw.clone();
            *changed
                .pointer_mut(pointer)
                .expect("fixture pointer must exist") = replacement;
            let quote = serde_json::from_value(changed).unwrap();
            let message = validate_quote(&request, quote).expect_err(name).to_string();
            assert!(message.contains(error), "{name}: {message}");
        }
    }

    #[test]
    fn rejects_changed_input_and_broken_route() {
        let mut changed = sample_quote();
        changed.in_amount = "2000000".into();
        assert!(validate_quote(&sample_request(), changed).is_err());
        let mut broken = sample_quote();
        broken.route_plan[1].swap_info.input_mint = WSOL_MINT.into();
        assert!(validate_quote(&sample_request(), broken).is_err());
    }

    #[test]
    fn preflight_rejects_tampered_or_terminal_quote_before_provider_call() {
        let store = QuoteStore::default();
        let request = sample_request();
        let preview = validate_quote(&request, sample_quote()).unwrap();
        let bound = store
            .insert_local(&request, "wallet-pubkey".into(), preview)
            .unwrap();
        assert!(validate_preflight_bound(&bound, "wallet-pubkey").is_ok());
        assert!(validate_preflight_bound(&bound, "other-wallet").is_err());

        let mut changed = bound.clone();
        changed.provider = "dflow".into();
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.status = "failed".into();
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.expires_at_ms = 0;
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.provider_quote_id = "jup:wrong".into();
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.route.clear();
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.amount_lamports += 1;
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.input_mint = request.mint.clone();
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.minimum_output_units = 0;
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.minimum_output_units = changed.quote.expected_output_units + 1;
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.minimum_output_units = 1;
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.price_impact_percent = "NaN".into();
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
        changed = bound.clone();
        changed.quote.route[1].input_mint = WSOL_MINT.into();
        changed.route_fingerprint = format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&changed.quote.route).unwrap())
        );
        changed.provider_quote_id = format!("jup:{}", changed.route_fingerprint);
        assert!(validate_preflight_bound(&changed, "wallet-pubkey").is_err());
    }

    #[tokio::test]
    async fn preflight_rejects_corrupt_durable_quote_after_restart() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_corrupt_preflight_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let config = durable_test_config(&path);
        let store = QuoteStore::new(&config).await.unwrap();
        let request = sample_request();
        let quote = store
            .insert(
                &request,
                "wallet-pubkey".into(),
                validate_quote(&request, sample_quote()).unwrap(),
            )
            .await
            .unwrap();
        sqlx::query("UPDATE aggregator_quotes SET minimum_output_units=0 WHERE quote_id=?")
            .bind(&quote.quote_id)
            .execute(store.pool.as_ref().unwrap())
            .await
            .unwrap();
        drop(store);

        let reopened = QuoteStore::new(&config).await.unwrap();
        let restored = reopened
            .get(&quote.quote_id, &request.wallet_id, "wallet-pubkey")
            .await
            .unwrap();
        assert!(validate_preflight_bound(&restored, "wallet-pubkey").is_err());
        drop(reopened);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn bound_quote_rejects_other_wallet_and_expiry() {
        let store = QuoteStore::default();
        let request = sample_request();
        let preview = validate_quote(&request, sample_quote()).unwrap();
        let bound = store
            .insert_local(&request, "wallet-pubkey".into(), preview)
            .unwrap();
        assert!(store
            .get(&bound.quote_id, "2TE", "wallet-pubkey")
            .await
            .is_ok());
        assert!(store
            .get(&bound.quote_id, "other", "wallet-pubkey")
            .await
            .is_err());
        assert!(store
            .get(&bound.quote_id, "2TE", "other-pubkey")
            .await
            .is_err());
        store
            .record_preflight(&bound.quote_id, &sample_preflight(&bound.quote_id))
            .await
            .unwrap();
        assert!(store
            .get(&bound.quote_id, "2TE", "wallet-pubkey")
            .await
            .unwrap()
            .preflight
            .is_some());
        store.quotes.get_mut(&bound.quote_id).unwrap().expires_at_ms = 0;
        assert!(store
            .get(&bound.quote_id, "2TE", "wallet-pubkey")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn memory_submission_reservation_is_single_and_quote_bound() {
        let store = QuoteStore::default();
        assert!(matches!(
            store.reserve_submission("wallet:key", "q1").await.unwrap(),
            SubmissionReservation::Reserved
        ));
        assert!(matches!(
            store.reserve_submission("wallet:key", "q1").await.unwrap(),
            SubmissionReservation::InFlight
        ));
        assert!(matches!(
            store.reserve_submission("wallet:key", "q2").await.unwrap(),
            SubmissionReservation::Conflict
        ));
        store.mark_submitting("wallet:key", "q1").await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_memory_reservation_has_one_owner() {
        let store = std::sync::Arc::new(QuoteStore::default());
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(64));
        let mut jobs = Vec::new();
        for _ in 0..64 {
            let store = store.clone();
            let barrier = barrier.clone();
            jobs.push(tokio::spawn(async move {
                barrier.wait().await;
                store
                    .reserve_submission("wallet:parallel", "q1")
                    .await
                    .unwrap()
            }));
        }
        let mut reserved = 0;
        let mut in_flight = 0;
        for job in jobs {
            match job.await.unwrap() {
                SubmissionReservation::Reserved => reserved += 1,
                SubmissionReservation::InFlight => in_flight += 1,
                other => panic!("unexpected reservation state: {other:?}"),
            }
        }
        assert_eq!(reserved, 1);
        assert_eq!(in_flight, 63);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_memory_terminal_write_has_one_winner() {
        let store = std::sync::Arc::new(QuoteStore::default());
        assert!(matches!(
            store
                .reserve_submission("2TE:parallel", "q1")
                .await
                .unwrap(),
            SubmissionReservation::Reserved
        ));
        let response = SolBuySubmitResponse {
            quote_id: "q1".into(),
            wallet_id: "2TE".into(),
            status: "simulated".into(),
            signature: None,
            error: None,
            preflight: sample_preflight("q1"),
        };
        let first = {
            let store = store.clone();
            let response = response.clone();
            tokio::spawn(async move {
                store
                    .record_submission("2TE:parallel".into(), response)
                    .await
            })
        };
        let second = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .record_submission("2TE:parallel".into(), response)
                    .await
            })
        };
        let outcomes = [first.await.unwrap(), second.await.unwrap()];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);
    }

    fn durable_test_config(path: &std::path::Path) -> TradePersistenceConfig {
        TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: format!("sqlite://{}", path.display()),
            busy_timeout_ms: 5_000,
        }
    }

    fn remove_sqlite_files(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn legacy_shadow_metrics_gain_route_length_and_survive_reopen() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_shadow_route_length_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let config = durable_test_config(&path);
        let options = SqliteConnectOptions::from_str(&config.database_url)
            .unwrap()
            .create_if_missing(true);
        let legacy = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../migrations/003_aggregator_quotes.sql"))
            .execute(&legacy)
            .await
            .unwrap();
        legacy.close().await;

        let request = sample_request();
        let preview = validate_quote(&request, sample_quote()).unwrap();
        let quote = crate::aggregator::AggregatorQuote {
            provider: crate::aggregator::ProviderId::Jupiter,
            provider_quote_id: "jupiter:test".into(),
            wallet_id: request.wallet_id.clone(),
            wallet_pubkey: "wallet-pubkey".into(),
            input_mint: WSOL_MINT.into(),
            output_mint: request.mint.clone(),
            input_amount_units: request.amount_lamports,
            expected_output_units: preview.expected_output_units,
            minimum_output_units: preview.minimum_output_units,
            slippage_bps: request.slippage_bps,
            route_fingerprint: "route".into(),
            request_hash: "request".into(),
            created_at_ms: now_ms(),
            expires_at_ms: now_ms() + 15_000,
            price_impact_percent: Some(preview.price_impact_percent),
            route: preview
                .route
                .into_iter()
                .map(|leg| crate::aggregator::AggregatorRouteLeg {
                    label: leg.label,
                    input_mint: leg.input_mint,
                    output_mint: leg.output_mint,
                })
                .collect(),
            balance_snapshot: None,
        };
        assert_eq!(quote.route.len(), 3);
        let store = QuoteStore::new(&config).await.unwrap();
        let version =
            sqlx::query_scalar::<_, i64>("SELECT version FROM schema_migrations WHERE version=6")
                .fetch_optional(store.pool.as_ref().unwrap())
                .await
                .unwrap();
        assert_eq!(version, Some(6));
        store
            .record_shadow_metric(
                "jupiter",
                "2TE",
                WSOL_MINT,
                &request.mint,
                request.amount_lamports,
                request.slippage_bps,
                "quoted",
                42,
                Some(&quote),
                None,
            )
            .await
            .unwrap();
        drop(store);
        let reopened = QuoteStore::new(&config).await.unwrap();
        let metrics = reopened
            .shadow_metrics(Some("2TE"), Some("jupiter"), 10)
            .await
            .unwrap();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].route_length, Some(3));
        let summary = reopened
            .shadow_summary(Some("2TE"), Some("jupiter"))
            .await
            .unwrap();
        assert_eq!(summary.min_output_checked_count, 1);
        assert_eq!(summary.min_output_missing_count, 0);
        assert_eq!(summary.min_output_coverage_rate, Some(1.0));
        assert_eq!(summary.min_output_pass_rate, Some(1.0));
        reopened
            .record_shadow_metric(
                "jupiter",
                "2TE",
                WSOL_MINT,
                &request.mint,
                request.amount_lamports,
                request.slippage_bps,
                "quoted",
                42,
                None,
                None,
            )
            .await
            .unwrap();
        let partial_summary = reopened
            .shadow_summary(Some("2TE"), Some("jupiter"))
            .await
            .unwrap();
        assert_eq!(partial_summary.quoted_count, 2);
        assert_eq!(partial_summary.min_output_checked_count, 1);
        assert_eq!(partial_summary.min_output_missing_count, 1);
        assert_eq!(partial_summary.min_output_coverage_rate, Some(0.5));
        assert_eq!(partial_summary.min_output_pass_rate, Some(0.5));
        sqlx::query("UPDATE aggregator_shadow_metrics SET minimum_output_units=1")
            .execute(reopened.pool.as_ref().unwrap())
            .await
            .unwrap();
        let invalid_summary = reopened
            .shadow_summary(Some("2TE"), Some("jupiter"))
            .await
            .unwrap();
        assert_eq!(invalid_summary.min_output_checked_count, 1);
        assert_eq!(invalid_summary.min_output_coverage_rate, Some(0.5));
        assert_eq!(invalid_summary.min_output_pass_rate, Some(0.0));
        drop(reopened);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn failed_durable_quote_insert_does_not_publish_memory_quote() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_failed_quote_insert_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        let store = QuoteStore::new(&durable_test_config(&path)).await.unwrap();
        store.pool.as_ref().unwrap().close().await;
        let request = sample_request();
        let preview = validate_quote(&request, sample_quote()).unwrap();
        assert!(store
            .insert(&request, "wallet-pubkey".into(), preview)
            .await
            .is_err());
        assert!(store.quotes.is_empty());
        drop(store);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn durable_quote_and_balance_snapshot_survive_store_reopen() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_quote_reopen_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let config = durable_test_config(&path);
        let request = sample_request();
        let mut preview = validate_quote(&request, sample_quote()).unwrap();
        let balance_snapshot = WalletBalanceSnapshot {
            balance_lamports: 2_000_000,
            fetched_at_ms: now_ms(),
        };
        preview.balance_snapshot = Some(balance_snapshot.clone());
        let quote_id;
        {
            let store = QuoteStore::new(&config).await.unwrap();
            let bound = store
                .insert(&request, "wallet-pubkey".into(), preview)
                .await
                .unwrap();
            store
                .mark_quote_status(&bound.quote_id, "preflighted")
                .await
                .unwrap();
            assert!(store
                .mark_quote_status(&bound.quote_id, "quoted")
                .await
                .is_err());
            store
                .mark_quote_status(&bound.quote_id, "failed")
                .await
                .unwrap();
            quote_id = bound.quote_id.clone();
            let row = sqlx::query_scalar::<_, i64>(
                "SELECT version FROM schema_migrations WHERE version=5",
            )
            .fetch_optional(store.pool.as_ref().unwrap())
            .await
            .unwrap();
            assert_eq!(row, Some(5));
            for column in ["preflight_json", "audit_json", "balance_snapshot_json"] {
                let present = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM pragma_table_info('aggregator_quotes') WHERE name=?",
                )
                .bind(column)
                .fetch_one(store.pool.as_ref().unwrap())
                .await
                .unwrap();
                assert_eq!(present, 1, "missing persisted preflight column {column}");
            }
            let shadow_table = sqlx::query_scalar::<_, String>(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='aggregator_shadow_metrics'",
            )
            .fetch_optional(store.pool.as_ref().unwrap())
            .await
            .unwrap();
            assert_eq!(shadow_table.as_deref(), Some("aggregator_shadow_metrics"));
            store
                .record_shadow_metric(
                    "jupiter",
                    "2TE",
                    WSOL_MINT,
                    &request.mint,
                    request.amount_lamports,
                    request.slippage_bps,
                    "quoted",
                    42,
                    None,
                    None,
                )
                .await
                .unwrap();
            let metrics = store
                .shadow_metrics(Some("2TE"), Some("jupiter"), 10)
                .await
                .unwrap();
            assert_eq!(metrics.len(), 1);
            assert_eq!(metrics[0].latency_ms, 42);
            assert_eq!(metrics[0].output_mint, request.mint);
            let summary = store
                .shadow_summary(Some("2TE"), Some("jupiter"))
                .await
                .unwrap();
            assert_eq!(summary.sample_count, 1);
            assert_eq!(summary.p50_latency_ms, Some(42));
            assert_eq!(summary.p95_latency_ms, Some(42));
            assert_eq!(summary.success_rate, Some(1.0));
        }
        {
            let store = QuoteStore::new(&config).await.unwrap();
            let restored = store.get(&quote_id, "2TE", "wallet-pubkey").await.unwrap();
            assert_eq!(restored.provider, "jupiter");
            assert_eq!(restored.status, "failed");
            assert_eq!(restored.quote.output_mint, request.mint);
            assert_eq!(restored.quote.balance_snapshot, Some(balance_snapshot));
        }
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn durable_preflight_and_audit_survive_store_reopen() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_preflight_reopen_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let config = durable_test_config(&path);
        let request = sample_request();
        let quote_id;
        let preflight: SolBuyPreflightResponse = serde_json::from_value(serde_json::json!({
            "quote_id": "placeholder",
            "wallet_id": "2TE",
            "expected_output_units": 99,
            "minimum_output_units": 90,
            "quote_still_valid": true,
            "unsigned_transaction_built": true,
            "aggregator_simulation_ok": true,
            "local_simulation_ok": true,
            "instruction_count": 2,
            "address_lookup_table_count": 0,
            "resolved_account_count": 3,
            "instruction_audit_passed": false,
            "executable": false,
            "reason": "Gate 2 remains closed",
            "prioritization_fee_lamports": 0,
            "compute_unit_limit": 100000,
            "calculated_priority_fee_lamports": 0,
            "explicit_system_spend_lamports": 0,
            "writable_account_count": 1,
            "target_ata": "ata",
            "top_level_programs": [],
            "simulated_fee_lamports": null,
            "simulated_units_consumed": null,
            "simulated_payer_debit_lamports": null,
            "simulated_inner_instruction_groups": null,
            "simulated_target_token_delta_units": null,
            "simulated_target_token_post_units": null,
            "baseline_slot": 1,
            "simulation_slot": 1,
            "observed_same_slot_payer_debit_lamports": null,
            "observed_same_slot_target_token_delta_units": null,
            "simulated_inner_instruction_count": null,
            "simulated_inner_programs": null,
            "simulated_inner_payer_system_spend_lamports": null
        }))
        .unwrap();
        {
            let store = QuoteStore::new(&config).await.unwrap();
            let bound = store
                .insert(
                    &request,
                    "wallet-pubkey".into(),
                    validate_quote(&request, sample_quote()).unwrap(),
                )
                .await
                .unwrap();
            quote_id = bound.quote_id.clone();
            store.record_preflight(&quote_id, &preflight).await.unwrap();
            let current = store.get(&quote_id, "2TE", "wallet-pubkey").await.unwrap();
            assert_eq!(
                current.preflight.as_ref().unwrap().reason,
                "Gate 2 remains closed"
            );
            assert_eq!(current.audit.as_ref().unwrap()["executable"], false);
        }
        let reopened = QuoteStore::new(&config).await.unwrap();
        let restored = reopened
            .get(&quote_id, "2TE", "wallet-pubkey")
            .await
            .unwrap();
        assert_eq!(
            restored.preflight.as_ref().unwrap().minimum_output_units,
            90
        );
        assert_eq!(
            restored.audit.as_ref().unwrap()["instruction_audit_passed"],
            false
        );
        drop(reopened);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn shadow_summary_calculates_percentiles_and_status_rates() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_shadow_summary_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let store = QuoteStore::new(&durable_test_config(&path)).await.unwrap();
        for (status, latency) in [
            ("quoted", 10),
            ("quoted", 20),
            ("failed", 30),
            ("unavailable", 40),
            ("expired", 50),
        ] {
            store
                .record_shadow_metric(
                    "jupiter",
                    "2TE",
                    WSOL_MINT,
                    "mint",
                    1_000_000,
                    100,
                    status,
                    latency,
                    None,
                    (status != "quoted").then_some(status),
                )
                .await
                .unwrap();
        }
        let summary = store
            .shadow_summary(Some("2TE"), Some("jupiter"))
            .await
            .unwrap();
        assert_eq!(summary.sample_count, 5);
        assert_eq!(summary.quoted_count, 2);
        assert_eq!(summary.failed_count, 1);
        assert_eq!(summary.unavailable_count, 1);
        assert_eq!(summary.p50_latency_ms, Some(30));
        assert_eq!(summary.p95_latency_ms, Some(50));
        assert_eq!(summary.p99_latency_ms, Some(50));
        assert_eq!(summary.success_rate, Some(0.4));
        assert_eq!(summary.expiry_rate, Some(0.2));
        assert_eq!(summary.min_output_checked_count, 0);
        assert_eq!(summary.min_output_missing_count, 2);
        assert_eq!(summary.min_output_coverage_rate, Some(0.0));
        assert_eq!(summary.min_output_pass_rate, Some(0.0));

        // DFlow is represented in Shadow as an unavailable read-only provider;
        // it must remain visible in its own provider slice without looking like
        // a quote or affecting Jupiter's success rate.
        store
            .record_shadow_metric(
                "dflow",
                "2TE",
                WSOL_MINT,
                "mint",
                1_000_000,
                100,
                "unavailable",
                0,
                None,
                Some("unavailable"),
            )
            .await
            .unwrap();
        let dflow_summary = store
            .shadow_summary(Some("2TE"), Some("dflow"))
            .await
            .unwrap();
        assert_eq!(dflow_summary.sample_count, 1);
        assert_eq!(dflow_summary.quoted_count, 0);
        assert_eq!(dflow_summary.unavailable_count, 1);
        assert_eq!(dflow_summary.success_rate, Some(0.0));

        store
            .record_shadow_metric(
                "jupiter",
                "2TE",
                WSOL_MINT,
                "mint",
                1_000_000,
                100,
                "failed",
                5,
                None,
                Some("provider failed https://rpc.example/?api-key=secret"),
            )
            .await
            .unwrap();
        let stored_error = sqlx::query_scalar::<_, String>(
            "SELECT error_class FROM aggregator_shadow_metrics WHERE provider='jupiter' ORDER BY created_at_ms DESC LIMIT 1",
        )
        .fetch_one(store.pool.as_ref().unwrap())
        .await
        .unwrap();
        assert_eq!(stored_error, "provider failed <redacted-url>");
        assert!(!stored_error.contains("secret"));
        drop(store);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn durable_reservation_is_unique_and_restart_marks_submitting_unknown() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_reservation_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let config = durable_test_config(&path);
        let store = QuoteStore::new(&config).await.unwrap();
        let request = sample_request();
        let preview = validate_quote(&request, sample_quote()).unwrap();
        let bound = store
            .insert(&request, "wallet-pubkey".into(), preview)
            .await
            .unwrap();
        assert!(matches!(
            store
                .reserve_submission("wallet:key", &bound.quote_id)
                .await
                .unwrap(),
            SubmissionReservation::Reserved
        ));
        store
            .mark_submitting("wallet:key", &bound.quote_id)
            .await
            .unwrap();
        assert!(store
            .submission_matches_quote("wallet:key", &bound.quote_id)
            .await
            .unwrap());
        assert!(!store
            .submission_matches_quote("wallet:key", "other-quote")
            .await
            .unwrap());
        drop(store);

        let reopened = QuoteStore::new(&config).await.unwrap();
        assert!(matches!(
            reopened
                .reserve_submission("wallet:key", &bound.quote_id)
                .await
                .unwrap(),
            SubmissionReservation::Unknown
        ));
        assert!(matches!(
            reopened
                .reserve_submission("wallet:key", "other-quote")
                .await
                .unwrap(),
            SubmissionReservation::Conflict
        ));
        assert!(reopened
            .submission_matches_quote("wallet:key", &bound.quote_id)
            .await
            .unwrap());
        drop(reopened);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn durable_unknown_submission_redacts_provider_url() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_unknown_redaction_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let store = QuoteStore::new(&durable_test_config(&path)).await.unwrap();
        let request = sample_request();
        let quote = store
            .insert(
                &request,
                "wallet-pubkey".into(),
                validate_quote(&request, sample_quote()).unwrap(),
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .reserve_submission("2TE:key", &quote.quote_id)
                .await
                .unwrap(),
            SubmissionReservation::Reserved
        ));
        store
            .mark_unknown(
                "2TE:key",
                &quote.quote_id,
                "provider https://rpc.example/?api-key=secret failed",
            )
            .await
            .unwrap();
        let late_response = SolBuySubmitResponse {
            quote_id: quote.quote_id.clone(),
            wallet_id: "2TE".into(),
            status: "simulated".into(),
            signature: None,
            error: None,
            preflight: sample_preflight(&quote.quote_id),
        };
        assert!(store
            .record_submission("2TE:key".into(), late_response)
            .await
            .is_err());
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM aggregator_submissions WHERE wallet_id='2TE' AND idempotency_key='key'",
        )
        .fetch_one(store.pool.as_ref().unwrap())
        .await
        .unwrap();
        assert_eq!(status, "unknown");
        let response_json = sqlx::query_scalar::<_, String>(
            "SELECT response_json FROM aggregator_submissions WHERE wallet_id='2TE' AND idempotency_key='key'",
        )
        .fetch_one(store.pool.as_ref().unwrap())
        .await
        .unwrap();
        assert!(response_json.contains("provider <redacted-url> failed"));
        assert!(!response_json.contains("secret"));
        drop(store);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn finalize_submission_persists_legacy_executor_response() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_finalize_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let store = QuoteStore::new(&durable_test_config(&path)).await.unwrap();
        let request = sample_request();
        let bound = store
            .insert(
                &request,
                "wallet-pubkey".into(),
                validate_quote(&request, sample_quote()).unwrap(),
            )
            .await
            .unwrap();
        store
            .reserve_submission("2TE:key", &bound.quote_id)
            .await
            .unwrap();
        store
            .mark_submitting("2TE:key", &bound.quote_id)
            .await
            .unwrap();
        let response = SolBuySubmitResponse {
            quote_id: bound.quote_id.clone(),
            wallet_id: "2TE".into(),
            status: "simulated".into(),
            signature: None,
            error: None,
            preflight: sample_preflight(&bound.quote_id),
        };
        assert!(store
            .finalize_submission("2TE:wrong", response.clone())
            .await
            .is_err());
        assert!(store
            .get(&bound.quote_id, "2TE", "wallet-pubkey")
            .await
            .unwrap()
            .preflight
            .is_none());
        store
            .finalize_submission("2TE:key", response)
            .await
            .unwrap();
        let restored = QuoteStore::new(&durable_test_config(&path)).await.unwrap();
        assert!(matches!(
            restored.lookup_submission("2TE:key").await.unwrap(),
            Some(SubmissionReservation::Existing(_))
        ));
        assert_eq!(
            restored
                .get(&bound.quote_id, "2TE", "wallet-pubkey")
                .await
                .unwrap()
                .status,
            "preflighted"
        );
        drop(restored);
        remove_sqlite_files(&path);
    }

    #[tokio::test]
    async fn durable_expired_quote_is_retained_as_expired() {
        let path = std::env::temp_dir().join(format!(
            "trade_service_aggregator_expired_quote_{}.sqlite",
            Uuid::new_v4().simple()
        ));
        remove_sqlite_files(&path);
        let config = durable_test_config(&path);
        let store = QuoteStore::new(&config).await.unwrap();
        let request = sample_request();
        let bound = store
            .insert(
                &request,
                "wallet-pubkey".into(),
                validate_quote(&request, sample_quote()).unwrap(),
            )
            .await
            .unwrap();
        sqlx::query("UPDATE aggregator_quotes SET expires_at_ms=0 WHERE quote_id=?")
            .bind(&bound.quote_id)
            .execute(store.pool.as_ref().unwrap())
            .await
            .unwrap();
        drop(store);
        let reopened = QuoteStore::new(&config).await.unwrap();
        assert!(reopened
            .get(&bound.quote_id, "2TE", "wallet-pubkey")
            .await
            .is_err());
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM aggregator_quotes WHERE quote_id=?",
        )
        .bind(&bound.quote_id)
        .fetch_one(reopened.pool.as_ref().unwrap())
        .await
        .unwrap();
        assert_eq!(status, "expired");
        remove_sqlite_files(&path);
    }
}
