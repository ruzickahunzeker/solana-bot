use std::{
    path::Path,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{bail, Context};
use serde_json::json;
use sha2::{Digest, Sha256};
use solana_sdk::signature::Signature;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    ConnectOptions, Row, SqlitePool,
};

use crate::{
    config::{TradePersistenceConfig, TradePersistenceMode},
    models::{
        ExecutionMethod, MevProvider, Pool, Side, SimulationReport, TradeRecord, TradeRequest,
        TradeStatus,
    },
    sanitize::redact_urls,
    validation::NormalizedTrade,
};

const MIGRATION: &str = include_str!("../migrations/001_trade_persistence.sql");
const SIMULATION_MIGRATION: &str = include_str!("../migrations/002_trade_simulation_report.sql");

pub struct TradePersistence {
    mode: TradePersistenceMode,
    pool: Option<SqlitePool>,
    shadow_tx: Option<tokio::sync::mpsc::Sender<ShadowWrite>>,
    shadow_healthy: Arc<AtomicBool>,
}

enum ShadowWrite {
    Insert {
        record: TradeRecord,
        key: String,
        hash: String,
        request_json: String,
    },
    Update(TradeRecord),
}

pub enum DurableReserve {
    Created,
    Duplicate(TradeRecord),
    Conflict,
}

impl TradePersistence {
    pub async fn new(config: &TradePersistenceConfig) -> anyhow::Result<Self> {
        if config.mode == TradePersistenceMode::Memory {
            return Ok(Self {
                mode: config.mode,
                pool: None,
                shadow_tx: None,
                shadow_healthy: Arc::new(AtomicBool::new(true)),
            });
        }
        ensure_parent_dir(&config.database_url)?;
        let options = SqliteConnectOptions::from_str(&config.database_url)
            .context("parse TRADE_DATABASE_URL")?
            .create_if_missing(true)
            .busy_timeout(Duration::from_millis(config.busy_timeout_ms))
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(MIGRATION).execute(&pool).await?;
        apply_simulation_migration(&pool).await?;
        recover_interrupted(&pool).await?;
        let shadow_healthy = Arc::new(AtomicBool::new(true));
        let shadow_tx = if config.mode == TradePersistenceMode::Shadow {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<ShadowWrite>(1_024);
            let writer_pool = pool.clone();
            let writer_healthy = shadow_healthy.clone();
            tokio::spawn(async move {
                while let Some(write) = rx.recv().await {
                    let result = match write {
                        ShadowWrite::Insert {
                            record,
                            key,
                            hash,
                            request_json,
                        } => insert_trade(&writer_pool, &record, &key, &hash, &request_json)
                            .await
                            .map(|_| ()),
                        ShadowWrite::Update(record) => update_trade(&writer_pool, &record).await,
                    };
                    match result {
                        Ok(()) => writer_healthy.store(true, Ordering::Relaxed),
                        Err(error) => {
                            writer_healthy.store(false, Ordering::Relaxed);
                            let error = redact_urls(&error.to_string());
                            tracing::warn!(%error, "shadow trade persistence write failed");
                        }
                    }
                }
            });
            Some(tx)
        } else {
            None
        };
        Ok(Self {
            mode: config.mode,
            pool: Some(pool),
            shadow_tx,
            shadow_healthy,
        })
    }

    pub fn mode(&self) -> TradePersistenceMode {
        self.mode
    }
    pub fn is_durable(&self) -> bool {
        self.mode == TradePersistenceMode::Durable
    }

    pub async fn healthy(&self) -> bool {
        if !self.shadow_healthy.load(Ordering::Relaxed) {
            return false;
        }
        let Some(pool) = &self.pool else {
            return true;
        };
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(pool)
            .await
            .is_ok()
    }

    pub async fn reserve(
        &self,
        record: &TradeRecord,
        request: &TradeRequest,
        normalized: &NormalizedTrade,
    ) -> anyhow::Result<DurableReserve> {
        if !self.is_durable() {
            return Ok(DurableReserve::Created);
        }
        let pool = self
            .pool
            .as_ref()
            .context("durable database is unavailable")?;
        let hash = request_hash(&record.wallet_id, request, normalized)?;
        let request_json = serde_json::to_string(request)?;
        let inserted =
            insert_trade(pool, record, &request.idempotency_key, &hash, &request_json).await?;
        if inserted {
            return Ok(DurableReserve::Created);
        }
        let existing = get_by_idempotency(pool, &record.wallet_id, &request.idempotency_key)
            .await?
            .context("idempotency row disappeared after conflict")?;
        if existing.1 == hash {
            Ok(DurableReserve::Duplicate(existing.0))
        } else {
            Ok(DurableReserve::Conflict)
        }
    }

    /// Read an existing durable reservation before checking a short-lived
    /// preview credential. An exact retry may return its prior result after
    /// expiry, but a changed request with the same key remains a conflict.
    pub async fn lookup_idempotency(
        &self,
        wallet_id: &str,
        request: &TradeRequest,
        normalized: &NormalizedTrade,
    ) -> anyhow::Result<Option<DurableReserve>> {
        if !self.is_durable() {
            return Ok(None);
        }
        let pool = self
            .pool
            .as_ref()
            .context("durable database is unavailable")?;
        let Some((record, stored_hash)) =
            get_by_idempotency(pool, wallet_id, &request.idempotency_key).await?
        else {
            return Ok(None);
        };
        let incoming_hash = request_hash(wallet_id, request, normalized)?;
        Ok(Some(if incoming_hash == stored_hash {
            DurableReserve::Duplicate(record)
        } else {
            DurableReserve::Conflict
        }))
    }

    pub async fn mirror(
        &self,
        record: &TradeRecord,
        request: &TradeRequest,
        normalized: Option<&NormalizedTrade>,
    ) -> anyhow::Result<()> {
        if self.mode == TradePersistenceMode::Memory {
            return Ok(());
        }
        let pool = self
            .pool
            .as_ref()
            .context("trade database is unavailable")?;
        let hash = match normalized {
            Some(value) => request_hash(&record.wallet_id, request, value)?,
            None => raw_request_hash(&record.wallet_id, request)?,
        };
        let request_json = serde_json::to_string(request)?;
        let storage_key = if normalized.is_some() {
            request.idempotency_key.clone()
        } else {
            format!("__rejected__:{}", record.request_id)
        };
        if self.mode == TradePersistenceMode::Shadow {
            let sender = self
                .shadow_tx
                .as_ref()
                .context("shadow writer is unavailable")?;
            sender
                .try_send(ShadowWrite::Insert {
                    record: record.clone(),
                    key: storage_key,
                    hash,
                    request_json,
                })
                .map_err(|error| {
                    self.shadow_healthy.store(false, Ordering::Relaxed);
                    anyhow::anyhow!("shadow persistence queue rejected write: {error}")
                })?;
            return Ok(());
        }
        insert_trade(pool, record, &storage_key, &hash, &request_json).await?;
        Ok(())
    }

    pub async fn update(&self, record: &TradeRecord) -> anyhow::Result<()> {
        if self.mode == TradePersistenceMode::Memory {
            return Ok(());
        }
        if self.mode == TradePersistenceMode::Shadow {
            let sender = self
                .shadow_tx
                .as_ref()
                .context("shadow writer is unavailable")?;
            sender
                .try_send(ShadowWrite::Update(record.clone()))
                .map_err(|error| {
                    self.shadow_healthy.store(false, Ordering::Relaxed);
                    anyhow::anyhow!("shadow persistence queue rejected update: {error}")
                })?;
            return Ok(());
        }
        let pool = self
            .pool
            .as_ref()
            .context("trade database is unavailable")?;
        update_trade(pool, record).await
    }

    /// Claim the only send attempt for an SDK order after the exact prepared
    /// transaction has been signed. A crash after this write recovers to
    /// `unknown` with the signature intact, so callers can reconcile instead
    /// of rebuilding the transaction on another lane.
    pub async fn reserve_sdk_signature_for_send(
        &self,
        request_id: &str,
        signature: &Signature,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            self.is_durable(),
            "SDK live submission requires durable persistence"
        );
        anyhow::ensure!(
            *signature != Signature::default(),
            "SDK submission signature is empty"
        );
        let pool = self
            .pool
            .as_ref()
            .context("durable database is unavailable")?;
        let result = sqlx::query(
            "UPDATE trades SET status='submitting', signature=?, updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE request_id=? AND execution_method='sdk' AND status='building_tx' AND signature IS NULL AND json_extract(request_json, '$.simulate') = 0",
        )
        .bind(signature.to_string())
        .bind(request_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn get(&self, request_id: &str) -> anyhow::Result<Option<TradeRecord>> {
        let pool = self
            .pool
            .as_ref()
            .context("trade database is unavailable")?;
        sqlx::query("SELECT * FROM trades WHERE request_id=?")
            .bind(request_id)
            .fetch_optional(pool)
            .await?
            .map(row_to_record)
            .transpose()
    }

    pub async fn recent(&self, limit: usize) -> anyhow::Result<Vec<TradeRecord>> {
        let pool = self
            .pool
            .as_ref()
            .context("trade database is unavailable")?;
        sqlx::query("SELECT * FROM trades ORDER BY created_at_ms DESC LIMIT ?")
            .bind(limit as i64)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(row_to_record)
            .collect()
    }
}

async fn insert_trade(
    pool: &SqlitePool,
    record: &TradeRecord,
    key: &str,
    hash: &str,
    request_json: &str,
) -> anyhow::Result<bool> {
    let sql = "INSERT OR IGNORE INTO trades(request_id,wallet_id,wallet_pubkey,idempotency_key,request_hash,request_json,side,pool,mint,pool_id,execution_method,status,signature,error,simulation_json,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)";
    let result = sqlx::query(sql)
        .bind(&record.request_id)
        .bind(&record.wallet_id)
        .bind(&record.wallet_pubkey)
        .bind(key)
        .bind(hash)
        .bind(request_json)
        .bind(side_name(record.side))
        .bind(pool_name(record.pool))
        .bind(&record.mint)
        .bind(&record.pool_id)
        .bind(execution_name(record.execution_method))
        .bind(status_name(record.status))
        .bind(&record.signature)
        .bind(&record.error)
        .bind(
            record
                .simulation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        )
        .bind(to_i64(record.created_at_ms)?)
        .bind(to_i64(record.updated_at_ms)?)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

async fn update_trade(pool: &SqlitePool, record: &TradeRecord) -> anyhow::Result<()> {
    let result = sqlx::query("UPDATE trades SET status=?, signature=COALESCE(?, signature), error=?, simulation_json=COALESCE(?, simulation_json), updated_at_ms=? WHERE request_id=?")
        .bind(status_name(record.status))
        .bind(&record.signature)
        .bind(&record.error)
        .bind(record.simulation.as_ref().map(serde_json::to_string).transpose()?)
        .bind(to_i64(record.updated_at_ms)?)
        .bind(&record.request_id)
        .execute(pool)
        .await?;
    if result.rows_affected() != 1 {
        bail!("trade {} is missing from persistence", record.request_id);
    }
    Ok(())
}

async fn get_by_idempotency(
    pool: &SqlitePool,
    wallet_id: &str,
    key: &str,
) -> anyhow::Result<Option<(TradeRecord, String)>> {
    let row = sqlx::query("SELECT * FROM trades WHERE wallet_id=? AND idempotency_key=?")
        .bind(wallet_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;
    row.map(|row| {
        let hash: String = row.try_get("request_hash")?;
        Ok((row_to_record(row)?, hash))
    })
    .transpose()
}

async fn recover_interrupted(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query("UPDATE trades SET status='unknown', error=CASE WHEN signature IS NULL THEN 'service restarted during non-terminal trade; manual recovery required' ELSE 'service restarted after signature was recorded; confirmation unknown' END, updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE status IN ('accepted','warming_params','building_tx','submitting')")
        .execute(pool).await?;
    Ok(())
}

pub fn request_hash(
    wallet_id: &str,
    request: &TradeRequest,
    value: &NormalizedTrade,
) -> anyhow::Result<String> {
    let mut fields = json!({
        "wallet_id": wallet_id, "side": side_name(request.side), "pool": pool_name(request.pool),
        "mint": value.mint.to_string(), "quote_mint": value.quote_mint.to_string(),
        "pool_id": value.pool_id.map(|v| v.to_string()), "amount_units": value.amount_lamports,
        "settlement_asset": format!("{:?}", value.settlement_asset),
        "sol_hop_pool": value.sol_hop_pool.map(pool_name),
        "sol_hop_pool_id": value.sol_hop_pool_id.map(|v| v.to_string()),
        "sell_percent": value.sell_percent, "slippage_bps": value.slippage_bps,
        "tip_sol_bits": value.tip_sol.to_bits(), "execution_method": execution_name(value.execution_method),
        "simulate": value.simulate, "use_mev": value.use_mev,
        "mev_provider": value.mev_provider.map(mev_name),
    });
    if let Some(preview_id) = request.sdk_preview_id.as_deref() {
        fields["sdk_preview_id"] = json!(preview_id);
        fields["sdk_preview_request_hash"] = json!(raw_request_hash(wallet_id, request)?);
    }
    if crate::validation::has_bridge_sdk_decision_id(request) {
        fields["sdk_bridge_decision_request_hash"] = json!(raw_request_hash(wallet_id, request)?);
    }
    hash_json(fields)
}

fn raw_request_hash(wallet_id: &str, request: &TradeRequest) -> anyhow::Result<String> {
    hash_json(json!({"wallet_id": wallet_id, "request": request}))
}
fn hash_json(value: serde_json::Value) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(&value)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn ensure_parent_dir(url: &str) -> anyhow::Result<()> {
    if let Some(path) = url.strip_prefix("sqlite://") {
        if path != ":memory:" {
            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
    }
    Ok(())
}
fn to_i64(value: u128) -> anyhow::Result<i64> {
    i64::try_from(value).context("timestamp exceeds SQLite INTEGER")
}
fn row_to_record(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<TradeRecord> {
    Ok(TradeRecord {
        request_id: row.try_get("request_id")?,
        wallet_id: row.try_get("wallet_id")?,
        wallet_pubkey: row.try_get("wallet_pubkey")?,
        side: parse_side(row.try_get("side")?)?,
        pool: parse_pool(row.try_get("pool")?)?,
        mint: row.try_get("mint")?,
        pool_id: row.try_get("pool_id")?,
        execution_method: parse_execution(row.try_get("execution_method")?)?,
        status: parse_status(row.try_get("status")?)?,
        signature: row.try_get("signature")?,
        error: row.try_get("error")?,
        simulation: row
            .try_get::<Option<String>, _>("simulation_json")?
            .map(|value| serde_json::from_str::<SimulationReport>(&value))
            .transpose()?,
        duplicate: false,
        created_at_ms: u128::try_from(row.try_get::<i64, _>("created_at_ms")?)?,
        updated_at_ms: u128::try_from(row.try_get::<i64, _>("updated_at_ms")?)?,
    })
}

async fn apply_simulation_migration(pool: &SqlitePool) -> anyhow::Result<()> {
    let applied = sqlx::query("SELECT 1 FROM schema_migrations WHERE version = 2")
        .fetch_optional(pool)
        .await?
        .is_some();
    if applied {
        return Ok(());
    }
    let columns = sqlx::query("PRAGMA table_info(trades)")
        .fetch_all(pool)
        .await?;
    let exists = columns.iter().any(|row| {
        row.try_get::<String, _>("name")
            .map(|name| name == "simulation_json")
            .unwrap_or(false)
    });
    if !exists {
        sqlx::raw_sql(SIMULATION_MIGRATION).execute(pool).await?;
    }
    sqlx::query(
        "INSERT INTO schema_migrations(version, applied_at_ms) VALUES (2, CAST(strftime('%s','now') AS INTEGER) * 1000)",
    )
    .execute(pool)
    .await?;
    Ok(())
}
fn side_name(v: Side) -> &'static str {
    match v {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}
fn pool_name(v: Pool) -> &'static str {
    match v {
        Pool::Pump => "pump",
        Pool::PumpAmm => "pump-amm",
        Pool::Bonk => "bonk",
        Pool::StonkFun => "stonkfun",
        Pool::StonkFunSwap => "stonkfun-swap",
        Pool::MeteoraDammV2 => "meteora-damm-v2",
        Pool::MeteoraDbc => "meteora-dbc",
        Pool::RaydiumAmmV4 => "raydium-amm-v4",
        Pool::RaydiumCpmm => "raydium-cpmm",
    }
}
fn execution_name(v: ExecutionMethod) -> &'static str {
    match v {
        ExecutionMethod::Sdk => "sdk",
        ExecutionMethod::Aggregator => "aggregator",
        ExecutionMethod::Pumpapi => "pumpapi",
    }
}
fn mev_name(v: MevProvider) -> &'static str {
    match v {
        MevProvider::Auto => "auto",
        MevProvider::Default => "default",
        MevProvider::Jito => "jito",
        MevProvider::Temporal => "temporal",
        MevProvider::FlashBlock => "flashblock",
        MevProvider::BlockRazor => "blockrazor",
        MevProvider::Astralane => "astralane",
        MevProvider::Speedlanding => "speedlanding",
    }
}
fn status_name(v: TradeStatus) -> &'static str {
    match v {
        TradeStatus::Accepted => "accepted",
        TradeStatus::WarmingParams => "warming_params",
        TradeStatus::BuildingTx => "building_tx",
        TradeStatus::Submitting => "submitting",
        TradeStatus::Simulated => "simulated",
        TradeStatus::Submitted => "submitted",
        TradeStatus::Confirmed => "confirmed",
        TradeStatus::Failed => "failed",
        TradeStatus::Rejected => "rejected",
        TradeStatus::Unknown => "unknown",
    }
}
fn parse_side(v: String) -> anyhow::Result<Side> {
    match v.as_str() {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        _ => bail!("invalid side {v}"),
    }
}
fn parse_pool(v: String) -> anyhow::Result<Pool> {
    match v.as_str() {
        "pump" => Ok(Pool::Pump),
        "pump-amm" => Ok(Pool::PumpAmm),
        "bonk" => Ok(Pool::Bonk),
        "stonkfun" | "stonk-fun" => Ok(Pool::StonkFun),
        "stonkfun-swap" | "stonk-fun-swap" => Ok(Pool::StonkFunSwap),
        "meteora-damm-v2" => Ok(Pool::MeteoraDammV2),
        "meteora-dbc" => Ok(Pool::MeteoraDbc),
        "raydium-amm-v4" => Ok(Pool::RaydiumAmmV4),
        "raydium-cpmm" => Ok(Pool::RaydiumCpmm),
        _ => bail!("invalid pool {v}"),
    }
}
fn parse_execution(v: String) -> anyhow::Result<ExecutionMethod> {
    match v.as_str() {
        "sdk" => Ok(ExecutionMethod::Sdk),
        "aggregator" => Ok(ExecutionMethod::Aggregator),
        "pumpapi" => Ok(ExecutionMethod::Pumpapi),
        _ => bail!("invalid execution method {v}"),
    }
}
fn parse_status(v: String) -> anyhow::Result<TradeStatus> {
    match v.as_str() {
        "accepted" => Ok(TradeStatus::Accepted),
        "warming_params" => Ok(TradeStatus::WarmingParams),
        "building_tx" => Ok(TradeStatus::BuildingTx),
        "submitting" => Ok(TradeStatus::Submitting),
        "simulated" => Ok(TradeStatus::Simulated),
        "submitted" => Ok(TradeStatus::Submitted),
        "confirmed" => Ok(TradeStatus::Confirmed),
        "failed" => Ok(TradeStatus::Failed),
        "rejected" => Ok(TradeStatus::Rejected),
        "unknown" => Ok(TradeStatus::Unknown),
        _ => bail!("invalid trade status {v}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::{Keypair, Signer};

    #[test]
    fn meteora_dbc_pool_name_round_trips() {
        assert_eq!(pool_name(Pool::MeteoraDbc), "meteora-dbc");
        assert_eq!(
            parse_pool("meteora-dbc".to_string()).unwrap(),
            Pool::MeteoraDbc
        );
    }

    #[test]
    fn stonkfun_pool_names_round_trip() {
        assert_eq!(pool_name(Pool::StonkFun), "stonkfun");
        assert_eq!(pool_name(Pool::StonkFunSwap), "stonkfun-swap");
        assert_eq!(parse_pool("stonkfun".to_string()).unwrap(), Pool::StonkFun);
        assert_eq!(
            parse_pool("stonkfun-swap".to_string()).unwrap(),
            Pool::StonkFunSwap
        );
    }

    #[test]
    fn explicit_aggregator_execution_method_round_trips() {
        assert_eq!(execution_name(ExecutionMethod::Aggregator), "aggregator");
        assert_eq!(
            parse_execution("aggregator".to_string()).unwrap(),
            ExecutionMethod::Aggregator
        );
    }

    fn request(idempotency_key: &str, amount_lamports: u64) -> TradeRequest {
        TradeRequest {
            wallet_id: Some("main".to_string()),
            side: Side::Buy,
            pool: Pool::Pump,
            mint: Pubkey::new_unique().to_string(),
            pool_id: None,
            settlement_asset: None,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            sdk_route_id: None,
            sdk_decision_id: None,
            sdk_preview_id: None,
            quote_mint: None,
            amount_sol: None,
            amount_lamports: Some(amount_lamports),
            amount_quote: None,
            amount_usdc: None,
            amount_quote_units: None,
            sell_percent: None,
            slippage_percent: None,
            slippage_bps: Some(500),
            tip_sol: Some(0.0),
            execution_method: Some(ExecutionMethod::Sdk),
            use_mev: Some(false),
            mev_provider: None,
            simulate: Some(true),
            source: None,
            source_signature: None,
            idempotency_key: idempotency_key.to_string(),
        }
    }

    fn normalized(request: &TradeRequest) -> NormalizedTrade {
        NormalizedTrade {
            mint: request.mint.parse().unwrap(),
            quote_mint: crate::models::WSOL_MINT.parse().unwrap(),
            aggregator_sol: false,
            pool_id: None,
            settlement_asset: crate::models::SettlementAsset::Quote,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: request.amount_lamports,
            sell_percent: None,
            slippage_bps: 500,
            tip_sol: 0.0,
            execution_method: ExecutionMethod::Sdk,
            use_mev: Some(false),
            mev_provider: None,
            simulate: Some(true),
        }
    }

    fn record(request_id: &str, request: &TradeRequest) -> TradeRecord {
        TradeRecord {
            request_id: request_id.to_string(),
            wallet_id: "main".to_string(),
            wallet_pubkey: Pubkey::new_unique().to_string(),
            side: request.side,
            pool: request.pool,
            mint: request.mint.clone(),
            pool_id: request.pool_id.clone(),
            execution_method: ExecutionMethod::Sdk,
            status: TradeStatus::Accepted,
            signature: None,
            error: None,
            simulation: None,
            duplicate: false,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    async fn durable() -> TradePersistence {
        TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: "sqlite::memory:".to_string(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn sdk_signature_reservation_allows_one_send_and_recovers_with_signature() {
        let persistence = durable().await;
        let mut request = request("signed-once", 1_000_000);
        request.simulate = Some(false);
        let mut normalized = normalized(&request);
        normalized.simulate = Some(false);
        let mut trade = record("tr_signed_once", &request);
        assert!(matches!(
            persistence
                .reserve(&trade, &request, &normalized)
                .await
                .unwrap(),
            DurableReserve::Created
        ));
        let signature = Keypair::new().sign_message(b"prepared-sdk-two-hop-message");
        let signature_text = signature.to_string();
        assert!(!persistence
            .reserve_sdk_signature_for_send(&trade.request_id, &signature)
            .await
            .unwrap());
        trade.status = TradeStatus::BuildingTx;
        persistence.update(&trade).await.unwrap();
        let (first, second) = tokio::join!(
            persistence.reserve_sdk_signature_for_send(&trade.request_id, &signature),
            persistence.reserve_sdk_signature_for_send(&trade.request_id, &signature),
        );
        assert_eq!(u8::from(first.unwrap()) + u8::from(second.unwrap()), 1);
        assert!(!persistence
            .reserve_sdk_signature_for_send(&trade.request_id, &signature)
            .await
            .unwrap());
        let saved = persistence.get(&trade.request_id).await.unwrap().unwrap();
        assert_eq!(saved.status, TradeStatus::Submitting);
        assert_eq!(saved.signature.as_deref(), Some(signature_text.as_str()));
        recover_interrupted(persistence.pool.as_ref().unwrap())
            .await
            .unwrap();
        let recovered = persistence.get(&trade.request_id).await.unwrap().unwrap();
        assert_eq!(recovered.status, TradeStatus::Unknown);
        assert_eq!(
            recovered.signature.as_deref(),
            Some(signature_text.as_str())
        );
        assert!(!persistence
            .reserve_sdk_signature_for_send(&trade.request_id, &signature)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn sdk_signature_reservation_rejects_simulation_and_empty_signature() {
        let persistence = durable().await;
        let request = request("simulation-only", 1_000_000);
        let normalized = normalized(&request);
        let mut trade = record("tr_simulation_only", &request);
        persistence
            .reserve(&trade, &request, &normalized)
            .await
            .unwrap();
        trade.status = TradeStatus::BuildingTx;
        persistence.update(&trade).await.unwrap();
        assert!(!persistence
            .reserve_sdk_signature_for_send(
                &trade.request_id,
                &Keypair::new().sign_message(b"should-not-send")
            )
            .await
            .unwrap());
        assert!(persistence
            .reserve_sdk_signature_for_send(&trade.request_id, &Signature::default())
            .await
            .is_err());
    }

    #[test]
    fn status_round_trips() {
        for status in [
            TradeStatus::Accepted,
            TradeStatus::BuildingTx,
            TradeStatus::Submitting,
            TradeStatus::Simulated,
            TradeStatus::Submitted,
            TradeStatus::Confirmed,
            TradeStatus::Failed,
            TradeStatus::Rejected,
            TradeStatus::Unknown,
        ] {
            assert_eq!(
                parse_status(status_name(status).to_string()).unwrap(),
                status
            );
        }
    }

    #[tokio::test]
    async fn durable_reserve_deduplicates_equal_request() {
        let persistence = durable().await;
        let request = request("same", 1_000_000);
        let normalized = normalized(&request);
        assert!(matches!(
            persistence
                .reserve(&record("tr_one", &request), &request, &normalized)
                .await
                .unwrap(),
            DurableReserve::Created
        ));
        match persistence
            .reserve(&record("tr_two", &request), &request, &normalized)
            .await
            .unwrap()
        {
            DurableReserve::Duplicate(existing) => assert_eq!(existing.request_id, "tr_one"),
            _ => panic!("expected duplicate"),
        }
    }

    #[tokio::test]
    async fn preview_retry_lookup_requires_exact_credential_and_request() {
        let persistence = durable().await;
        let mut original = request("preview-key", 1_000_000);
        original.sdk_preview_id = Some("observe:sdk-preview:first".to_string());
        let original_normalized = normalized(&original);
        assert!(matches!(
            persistence
                .reserve(
                    &record("tr_preview", &original),
                    &original,
                    &original_normalized
                )
                .await
                .unwrap(),
            DurableReserve::Created
        ));
        assert!(matches!(
            persistence
                .lookup_idempotency("main", &original, &original_normalized)
                .await
                .unwrap(),
            Some(DurableReserve::Duplicate(_))
        ));
        let mut another_preview = original.clone();
        another_preview.sdk_preview_id = Some("observe:sdk-preview:second".to_string());
        assert!(matches!(
            persistence
                .lookup_idempotency("main", &another_preview, &normalized(&another_preview))
                .await
                .unwrap(),
            Some(DurableReserve::Conflict)
        ));
        let mut changed_amount = original.clone();
        changed_amount.amount_lamports = Some(2_000_000);
        assert!(matches!(
            persistence
                .lookup_idempotency("main", &changed_amount, &normalized(&changed_amount))
                .await
                .unwrap(),
            Some(DurableReserve::Conflict)
        ));
        let mut changed_source = original.clone();
        changed_source.source = Some("different-copy-source".to_string());
        assert!(matches!(
            persistence
                .lookup_idempotency("main", &changed_source, &normalized(&changed_source))
                .await
                .unwrap(),
            Some(DurableReserve::Conflict)
        ));
        assert!(persistence
            .lookup_idempotency("other", &original, &original_normalized)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn bridge_decision_retry_requires_exact_credential_and_request() {
        let persistence = durable().await;
        let mut original = request("bridge-decision-key", 1_000_000);
        original.sdk_decision_id = Some("sdk-decision:bridge:first".into());
        let original_normalized = normalized(&original);
        assert!(matches!(
            persistence
                .reserve(
                    &record("tr_bridge", &original),
                    &original,
                    &original_normalized
                )
                .await
                .unwrap(),
            DurableReserve::Created
        ));
        assert!(matches!(
            persistence
                .lookup_idempotency("main", &original, &original_normalized)
                .await
                .unwrap(),
            Some(DurableReserve::Duplicate(_))
        ));
        let mut changed_credential = original.clone();
        changed_credential.sdk_decision_id = Some("sdk-decision:bridge:second".into());
        assert!(matches!(
            persistence
                .lookup_idempotency(
                    "main",
                    &changed_credential,
                    &normalized(&changed_credential)
                )
                .await
                .unwrap(),
            Some(DurableReserve::Conflict)
        ));
        let mut changed_source = original.clone();
        changed_source.source = Some("different-source".into());
        assert!(matches!(
            persistence
                .lookup_idempotency("main", &changed_source, &normalized(&changed_source))
                .await
                .unwrap(),
            Some(DurableReserve::Conflict)
        ));
    }

    #[tokio::test]
    async fn durable_reserve_rejects_same_key_with_different_trade() {
        let persistence = durable().await;
        let first = request("same", 1_000_000);
        persistence
            .reserve(&record("tr_one", &first), &first, &normalized(&first))
            .await
            .unwrap();
        let mut second = request("same", 2_000_000);
        second.mint = first.mint.clone();
        assert!(matches!(
            persistence
                .reserve(&record("tr_two", &second), &second, &normalized(&second))
                .await
                .unwrap(),
            DurableReserve::Conflict
        ));
    }

    #[tokio::test]
    async fn recovery_marks_non_terminal_trade_unknown_without_replay() {
        let persistence = durable().await;
        let request = request("recover", 1_000_000);
        let record = record("tr_recover", &request);
        persistence
            .reserve(&record, &request, &normalized(&request))
            .await
            .unwrap();
        recover_interrupted(persistence.pool.as_ref().unwrap())
            .await
            .unwrap();
        let recovered = persistence.get("tr_recover").await.unwrap().unwrap();
        assert_eq!(recovered.status, TradeStatus::Unknown);
        assert!(recovered
            .error
            .unwrap()
            .contains("manual recovery required"));
    }

    #[tokio::test]
    async fn one_hundred_concurrent_reservations_create_only_one_trade() {
        let persistence = std::sync::Arc::new(durable().await);
        let original = request("concurrent", 1_000_000);
        let mint = original.mint.clone();
        let mut tasks = Vec::new();
        for index in 0..100 {
            let persistence = persistence.clone();
            let mut request = request("concurrent", 1_000_000);
            request.mint = mint.clone();
            tasks.push(tokio::spawn(async move {
                let normalized = normalized(&request);
                persistence
                    .reserve(
                        &record(&format!("tr_{index}"), &request),
                        &request,
                        &normalized,
                    )
                    .await
                    .unwrap()
            }));
        }
        let mut created = 0;
        let mut duplicates = 0;
        for task in tasks {
            match task.await.unwrap() {
                DurableReserve::Created => created += 1,
                DurableReserve::Duplicate(_) => duplicates += 1,
                DurableReserve::Conflict => panic!("equal requests must not conflict"),
            }
        }
        assert_eq!(created, 1);
        assert_eq!(duplicates, 99);
    }

    #[tokio::test]
    async fn rejected_snapshot_does_not_consume_executable_idempotency_key() {
        let persistence = durable().await;
        let request = request("reusable-after-rejection", 1_000_000);
        let mut rejected = record("tr_rejected", &request);
        rejected.status = TradeStatus::Rejected;
        persistence.mirror(&rejected, &request, None).await.unwrap();
        assert!(matches!(
            persistence
                .reserve(
                    &record("tr_valid", &request),
                    &request,
                    &normalized(&request)
                )
                .await
                .unwrap(),
            DurableReserve::Created
        ));
        assert!(persistence.get("tr_rejected").await.unwrap().is_some());
        assert!(persistence.get("tr_valid").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn memory_mode_does_not_open_or_validate_database() {
        let persistence = TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Memory,
            database_url: "not-a-sqlite-url".to_string(),
            busy_timeout_ms: 1,
        })
        .await
        .unwrap();
        assert_eq!(persistence.mode(), TradePersistenceMode::Memory);
        assert!(persistence.healthy().await);
    }

    #[tokio::test]
    async fn closed_durable_database_rejects_reserve_and_status_update() {
        let persistence = durable().await;
        persistence.pool.as_ref().unwrap().close().await;
        let request = request("closed", 1_000_000);
        let record = record("tr_closed", &request);
        assert!(persistence
            .reserve(&record, &request, &normalized(&request))
            .await
            .is_err());
        assert!(persistence.update(&record).await.is_err());
        assert!(!persistence.healthy().await);
    }

    #[tokio::test]
    async fn shadow_writer_preserves_insert_then_update_order() {
        let persistence = TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Shadow,
            database_url: "sqlite::memory:".to_string(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap();
        let request = request("shadow", 1_000_000);
        let mut record = record("tr_shadow", &request);
        persistence
            .mirror(&record, &request, Some(&normalized(&request)))
            .await
            .unwrap();
        record.status = TradeStatus::Simulated;
        record.updated_at_ms = 2;
        persistence.update(&record).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(saved) = persistence.get("tr_shadow").await.unwrap() {
                if saved.status == TradeStatus::Simulated {
                    break;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "shadow writer timed out"
            );
            tokio::task::yield_now().await;
        }
        assert!(persistence.healthy().await);
    }
}
