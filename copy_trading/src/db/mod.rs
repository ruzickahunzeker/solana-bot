pub mod models;

use crate::execution_params::{ExecutionParams, LimitAction};
use anyhow::{Context, Result};
use models::{
    AlphaObservationEvent, AlphaPaperSignal, AlphaShadowOrder, AppUser, LimitOrderSummary,
    ObservedPoolRoute, OrderDetail, OrderSummary, PositionSummary, PumpApiEvent, SignalSummary,
    StrategyConfig, UserTradeSettings, WalletRecord, WatchMatch, WatchWallet,
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Executor, Row, SqlitePool,
};
use std::{str::FromStr, time::Duration};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self> {
        if let Some(path) = database_url.strip_prefix("sqlite://") {
            if let Some(parent) = std::path::Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
        }

        let options = SqliteConnectOptions::from_str(database_url)
            .with_context(|| format!("invalid database url {database_url}"))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .with_context(|| format!("failed to connect database {database_url}"))?;

        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<()> {
        for migration in [
            include_str!("../../migrations/001_init.sql"),
            include_str!("../../migrations/002_mint_pool_routes.sql"),
            include_str!("../../migrations/003_mint_pool_route_candidates.sql"),
            include_str!("../../migrations/004_alpha_observation_events.sql"),
            include_str!("../../migrations/005_alpha_paper_signals.sql"),
            include_str!("../../migrations/006_alpha_signal_lifecycle.sql"),
            include_str!("../../migrations/007_alpha_shadow_orders.sql"),
        ] {
            for statement in migration.split(';') {
                let statement = statement.trim();
                if !statement.is_empty() {
                    self.pool.execute(statement).await?;
                }
            }
        }
        self.ensure_column("wallets", "external_wallet_id", "TEXT")
            .await?;
        self.pool
            .execute("DROP INDEX IF EXISTS idx_wallets_external_wallet_id")
            .await?;
        self.pool
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_wallets_user_external_wallet_id
                 ON wallets(user_id,external_wallet_id) WHERE external_wallet_id IS NOT NULL",
            )
            .await?;
        self.ensure_column("signals", "user_id", "INTEGER REFERENCES app_users(id)")
            .await?;
        self.ensure_column("signals", "metadata_json", "TEXT")
            .await?;
        self.ensure_column("alpha_paper_signals", "order_id", "TEXT")
            .await?;
        self.ensure_column("alpha_paper_signals", "quote_mint", "TEXT")
            .await?;
        self.ensure_column("alpha_paper_signals", "trade_service_request_id", "TEXT")
            .await?;
        self.ensure_column("alpha_paper_signals", "remote_status", "TEXT")
            .await?;
        self.ensure_column("alpha_paper_signals", "updated_at_ms", "INTEGER")
            .await?;
        self.pool
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_alpha_paper_signals_request
                 ON alpha_paper_signals(trade_service_request_id)",
            )
            .await?;
        self.ensure_column("watch_wallets", "is_deleted", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column(
            "strategies",
            "watch_wallet_id",
            "INTEGER REFERENCES watch_wallets(id)",
        )
        .await?;
        self.pool
            .execute(
                "UPDATE watch_wallets
                 SET enabled = 0
                 WHERE is_deleted = 1 OR id IN (
                   SELECT watch_wallet_id FROM strategies
                   WHERE watch_wallet_id IS NOT NULL AND enabled = 0
                 )",
            )
            .await?;
        self.pool
            .execute(
                "UPDATE strategies
                 SET enabled = 0, updated_at = CURRENT_TIMESTAMP
                 WHERE watch_wallet_id IN (
                   SELECT id FROM watch_wallets WHERE enabled = 0 OR is_deleted = 1
                 )",
            )
            .await?;
        self.ensure_column("strategies", "min_sol_in_pool", "REAL NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("strategies", "buy_mode", "TEXT NOT NULL DEFAULT 'fixed'")
            .await?;
        self.ensure_column(
            "strategies",
            "buy_ratio_percent",
            "REAL NOT NULL DEFAULT 100",
        )
        .await?;
        self.ensure_column("strategies", "min_mcap", "REAL NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("strategies", "max_mcap", "REAL NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column(
            "strategies",
            "min_target_buy_sol",
            "REAL NOT NULL DEFAULT 0",
        )
        .await?;
        self.ensure_column(
            "strategies",
            "max_target_buy_sol",
            "REAL NOT NULL DEFAULT 0",
        )
        .await?;
        self.ensure_column("strategies", "max_signal_delay_ms", "INTEGER")
            .await?;
        self.ensure_column("strategies", "allowed_pools", "TEXT")
            .await?;
        self.ensure_column("strategies", "mev_mode", "TEXT NOT NULL DEFAULT 'none'")
            .await?;
        self.ensure_column(
            "strategies",
            "execution_mode",
            "TEXT NOT NULL DEFAULT 'dry_run'",
        )
        .await?;
        self.ensure_column(
            "strategies",
            "execution_method",
            "TEXT NOT NULL DEFAULT 'sdk'",
        )
        .await?;
        self.pool
            .execute(
                "UPDATE strategies
                 SET execution_mode = CASE WHEN dry_run = 1 THEN 'dry_run' ELSE 'live' END
                 WHERE execution_mode = ''
                    OR (execution_mode = 'dry_run' AND dry_run = 0)
                    OR (execution_mode = 'live' AND dry_run = 1)",
            )
            .await?;
        self.ensure_column("strategies", "live", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.pool
            .execute(
                "CREATE TABLE IF NOT EXISTS dry_run_positions (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  user_id INTEGER NOT NULL REFERENCES app_users(id),
                  strategy_id INTEGER NOT NULL REFERENCES strategies(id),
                  wallet_id INTEGER REFERENCES wallets(id),
                  mint TEXT NOT NULL,
                  amount_sol REAL NOT NULL DEFAULT 0,
                  buy_count INTEGER NOT NULL DEFAULT 0,
                  sell_count INTEGER NOT NULL DEFAULT 0,
                  last_buy_signal_id INTEGER REFERENCES signals(id),
                  last_sell_signal_id INTEGER REFERENCES signals(id),
                  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                  UNIQUE(user_id, strategy_id, mint)
                )",
            )
            .await?;
        self.ensure_column("limit_orders", "triggered_price", "REAL")
            .await?;
        self.ensure_column(
            "limit_orders",
            "trigger_currency",
            "TEXT NOT NULL DEFAULT 'USD'",
        )
        .await?;
        self.ensure_column("limit_orders", "triggered_currency", "TEXT")
            .await?;
        self.ensure_column(
            "limit_orders",
            "triggered_source_event_id",
            "INTEGER REFERENCES pumpapi_events(id)",
        )
        .await?;
        self.ensure_column("limit_orders", "triggered_at", "TEXT")
            .await?;
        self.ensure_column("limit_orders", "dry_run", "INTEGER NOT NULL DEFAULT 1")
            .await?;
        self.ensure_column("limit_orders", "limit_action", "TEXT NOT NULL DEFAULT ''")
            .await?;
        self.ensure_column(
            "limit_orders",
            "trigger_metric",
            "TEXT NOT NULL DEFAULT 'price_usd'",
        )
        .await?;
        self.ensure_column(
            "limit_orders",
            "execution_mode",
            "TEXT NOT NULL DEFAULT 'dry_run'",
        )
        .await?;
        self.ensure_column(
            "limit_orders",
            "execution_method",
            "TEXT NOT NULL DEFAULT 'sdk'",
        )
        .await?;
        self.ensure_column(
            "limit_orders",
            "slippage_bps",
            "INTEGER NOT NULL DEFAULT 1500",
        )
        .await?;
        self.ensure_column("limit_orders", "tip_sol", "REAL NOT NULL DEFAULT 0.0001")
            .await?;
        self.ensure_column("limit_orders", "mev_mode", "TEXT NOT NULL DEFAULT 'none'")
            .await?;
        self.pool
            .execute(
                "UPDATE limit_orders
                 SET limit_action = CASE
                       WHEN side = 'buy' AND trigger_type IN ('above', 'price_gte') THEN 'buy_above'
                       WHEN side = 'sell' AND trigger_type IN ('below', 'price_lte') THEN 'sell_below'
                       WHEN side = 'sell' THEN 'sell_above'
                       ELSE 'buy_below'
                     END
                 WHERE limit_action = ''",
            )
            .await?;
        self.pool
            .execute(
                "UPDATE limit_orders
                 SET execution_mode = CASE WHEN dry_run = 1 THEN 'dry_run' ELSE 'live' END
                 WHERE execution_mode = ''
                    OR (execution_mode = 'dry_run' AND dry_run = 0)
                    OR (execution_mode = 'live' AND dry_run = 1)",
            )
            .await?;
        self.ensure_column("orders", "trade_service_request_id", "TEXT")
            .await?;
        self.ensure_column(
            "user_trade_settings",
            "default_execution_mode",
            "TEXT NOT NULL DEFAULT 'dry_run'",
        )
        .await?;
        self.ensure_column(
            "user_trade_settings",
            "quick_trade_confirmation_enabled",
            "INTEGER NOT NULL DEFAULT 1",
        )
        .await?;
        Ok(())
    }

    async fn ensure_column(&self, table: &str, column: &str, definition: &str) -> Result<()> {
        let pragma = format!("PRAGMA table_info({table})");
        let rows = sqlx::query(&pragma).fetch_all(&self.pool).await?;
        let exists = rows
            .iter()
            .any(|row| row.get::<String, _>("name") == column);
        if !exists {
            let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
            self.pool.execute(sql.as_str()).await?;
        }
        Ok(())
    }

    pub async fn upsert_user(&self, telegram_id: i64, username: Option<&str>) -> Result<AppUser> {
        sqlx::query(
            "INSERT INTO app_users (telegram_id, username) VALUES (?, ?)
             ON CONFLICT(telegram_id) DO UPDATE SET username=excluded.username, updated_at=CURRENT_TIMESTAMP",
        )
        .bind(telegram_id)
        .bind(username)
        .execute(&self.pool)
        .await?;

        self.get_user_by_telegram_id(telegram_id).await
    }

    pub async fn get_user_by_telegram_id(&self, telegram_id: i64) -> Result<AppUser> {
        let row =
            sqlx::query("SELECT id, telegram_id, username FROM app_users WHERE telegram_id = ?")
                .bind(telegram_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(AppUser {
            id: row.get("id"),
            telegram_id: row.get("telegram_id"),
            username: row.get("username"),
        })
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> Result<AppUser> {
        let row = sqlx::query("SELECT id, telegram_id, username FROM app_users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(AppUser {
            id: row.get("id"),
            telegram_id: row.get("telegram_id"),
            username: row.get("username"),
        })
    }

    pub async fn get_or_create_user_trade_settings(
        &self,
        defaults: &UserTradeSettings,
    ) -> Result<UserTradeSettings> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT OR IGNORE INTO user_trade_settings
             (user_id, use_mev, default_mev_provider, default_execution_mode, default_execution_method,
              default_slippage_bps, default_tip_sol, default_buy_preset_slot,
              default_sell_preset_slot, quick_trade_confirmation_enabled)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(defaults.user_id)
        .bind(i64::from(defaults.use_mev))
        .bind(&defaults.default_mev_provider)
        .bind(&defaults.default_execution_mode)
        .bind(&defaults.default_execution_method)
        .bind(defaults.default_slippage_bps as i64)
        .bind(defaults.default_tip_sol)
        .bind((defaults.default_buy_preset_slot + 1) as i64)
        .bind((defaults.default_sell_preset_slot + 1) as i64)
        .bind(i64::from(defaults.quick_trade_confirmation_enabled))
        .execute(&mut *tx)
        .await?;
        for (side, values) in [
            ("buy", defaults.buy_presets_sol),
            ("sell", defaults.sell_presets_percent),
        ] {
            for (index, value) in values.into_iter().enumerate() {
                sqlx::query(
                    "INSERT OR IGNORE INTO user_trade_presets (user_id, side, slot, value)
                     VALUES (?, ?, ?, ?)",
                )
                .bind(defaults.user_id)
                .bind(side)
                .bind((index + 1) as i64)
                .bind(value)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        self.get_user_trade_settings(defaults.user_id).await
    }

    pub async fn get_user_trade_settings(&self, user_id: i64) -> Result<UserTradeSettings> {
        let row = sqlx::query(
            "SELECT user_id, use_mev, default_mev_provider, default_execution_mode, default_execution_method,
                    default_slippage_bps, default_tip_sol, default_buy_preset_slot,
                    default_sell_preset_slot, quick_trade_confirmation_enabled
             FROM user_trade_settings WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        let preset_rows = sqlx::query(
            "SELECT side, slot, value FROM user_trade_presets
             WHERE user_id = ? ORDER BY side, slot",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        let mut buy = [0.0; 5];
        let mut sell = [0.0; 5];
        for preset in preset_rows {
            let slot = preset.get::<i64, _>("slot") as usize;
            if !(1..=5).contains(&slot) {
                continue;
            }
            let target = if preset.get::<String, _>("side") == "buy" {
                &mut buy
            } else {
                &mut sell
            };
            target[slot - 1] = preset.get("value");
        }
        if buy.iter().any(|value| *value <= 0.0) || sell.iter().any(|value| *value <= 0.0) {
            anyhow::bail!("用户交易预设不完整");
        }
        Ok(UserTradeSettings {
            user_id: row.get("user_id"),
            use_mev: row.get::<i64, _>("use_mev") != 0,
            default_mev_provider: row.get("default_mev_provider"),
            default_execution_mode: row.get("default_execution_mode"),
            default_execution_method: row.get("default_execution_method"),
            default_slippage_bps: row.get::<i64, _>("default_slippage_bps") as u64,
            default_tip_sol: row.get("default_tip_sol"),
            default_buy_preset_slot: (row.get::<i64, _>("default_buy_preset_slot") - 1).clamp(0, 4)
                as usize,
            default_sell_preset_slot: (row.get::<i64, _>("default_sell_preset_slot") - 1)
                .clamp(0, 4) as usize,
            quick_trade_confirmation_enabled: row.get::<i64, _>("quick_trade_confirmation_enabled")
                != 0,
            buy_presets_sol: buy,
            sell_presets_percent: sell,
        })
    }

    pub async fn update_user_trade_setting(
        &self,
        user_id: i64,
        field: &str,
        value: &str,
    ) -> Result<()> {
        if ![
            "use_mev",
            "default_mev_provider",
            "default_execution_mode",
            "default_execution_method",
            "default_slippage_bps",
            "default_tip_sol",
            "default_buy_preset_slot",
            "default_sell_preset_slot",
            "quick_trade_confirmation_enabled",
        ]
        .contains(&field)
        {
            anyhow::bail!("unsupported user trade setting: {field}");
        }
        let sql = format!(
            "UPDATE user_trade_settings SET {field} = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ?"
        );
        sqlx::query(&sql)
            .bind(value)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_user_trade_preset(
        &self,
        user_id: i64,
        side: &str,
        slot: usize,
        value: f64,
    ) -> Result<()> {
        if !matches!(side, "buy" | "sell") || !(1..=5).contains(&slot) {
            anyhow::bail!("invalid trade preset");
        }
        sqlx::query(
            "UPDATE user_trade_presets SET value = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND side = ? AND slot = ?",
        )
        .bind(value)
        .bind(user_id)
        .bind(side)
        .bind(slot as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn user_mev_enabled(&self, user_id: i64) -> Result<bool> {
        let value = sqlx::query("SELECT use_mev FROM user_trade_settings WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(value.is_some_and(|row| row.get::<i64, _>("use_mev") != 0))
    }

    pub async fn list_wallets(&self, user_id: i64) -> Result<Vec<WalletRecord>> {
        let rows = sqlx::query(
            "SELECT id, user_id, external_wallet_id, label, public_key, is_active, is_deleted
             FROM wallets
             WHERE user_id = ? AND is_deleted = 0
             ORDER BY is_active DESC, id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| WalletRecord {
                id: row.get("id"),
                user_id: row.get("user_id"),
                external_wallet_id: row.get("external_wallet_id"),
                label: row.get("label"),
                public_key: row.get("public_key"),
                is_active: row.get::<i64, _>("is_active") != 0,
                is_deleted: row.get::<i64, _>("is_deleted") != 0,
            })
            .collect())
    }

    pub async fn get_wallet(&self, user_id: i64, wallet_id: i64) -> Result<WalletRecord> {
        let row = sqlx::query(
            "SELECT id, user_id, external_wallet_id, label, public_key, is_active, is_deleted
             FROM wallets
             WHERE user_id = ? AND id = ? AND is_deleted = 0",
        )
        .bind(user_id)
        .bind(wallet_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(WalletRecord {
            id: row.get("id"),
            user_id: row.get("user_id"),
            external_wallet_id: row.get("external_wallet_id"),
            label: row.get("label"),
            public_key: row.get("public_key"),
            is_active: row.get::<i64, _>("is_active") != 0,
            is_deleted: row.get::<i64, _>("is_deleted") != 0,
        })
    }

    pub async fn get_wallet_by_public_key(
        &self,
        user_id: i64,
        public_key: &str,
        include_deleted: bool,
    ) -> Result<Option<WalletRecord>> {
        let sql = if include_deleted {
            "SELECT id, user_id, external_wallet_id, label, public_key, is_active, is_deleted
             FROM wallets WHERE user_id = ? AND public_key = ?"
        } else {
            "SELECT id, user_id, external_wallet_id, label, public_key, is_active, is_deleted
             FROM wallets WHERE user_id = ? AND public_key = ? AND is_deleted = 0"
        };
        let row = sqlx::query(sql)
            .bind(user_id)
            .bind(public_key)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|row| WalletRecord {
            id: row.get("id"),
            user_id: row.get("user_id"),
            external_wallet_id: row.get("external_wallet_id"),
            label: row.get("label"),
            public_key: row.get("public_key"),
            is_active: row.get::<i64, _>("is_active") != 0,
            is_deleted: row.get::<i64, _>("is_deleted") != 0,
        }))
    }

    #[allow(dead_code)]
    pub async fn insert_wallet(
        &self,
        user_id: i64,
        label: &str,
        public_key: &str,
        encrypted_private_key: &str,
        nonce: &str,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count FROM wallets WHERE user_id = ? AND is_deleted = 0",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?
        .get("count");
        let is_active = if count == 0 { 1 } else { 0 };

        let result = sqlx::query(
            "INSERT INTO wallets (user_id, label, public_key, is_active) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(label)
        .bind(public_key)
        .bind(is_active)
        .execute(&mut *tx)
        .await?;
        let wallet_id = result.last_insert_rowid();

        sqlx::query(
            "INSERT INTO wallet_secrets (wallet_id, encrypted_private_key, nonce) VALUES (?, ?, ?)",
        )
        .bind(wallet_id)
        .bind(encrypted_private_key)
        .bind(nonce)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(wallet_id)
    }

    pub async fn insert_external_wallet(
        &self,
        user_id: i64,
        external_wallet_id: &str,
        label: &str,
        public_key: &str,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count FROM wallets WHERE user_id = ? AND is_deleted = 0",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?
        .get("count");
        let is_active = i64::from(count == 0);
        let result = sqlx::query(
            "INSERT INTO wallets(user_id,external_wallet_id,label,public_key,is_active)
             VALUES(?,?,?,?,?)",
        )
        .bind(user_id)
        .bind(external_wallet_id)
        .bind(label)
        .bind(public_key)
        .bind(is_active)
        .execute(&mut *tx)
        .await?;
        let wallet_id = result.last_insert_rowid();
        tx.commit().await?;
        Ok(wallet_id)
    }

    pub async fn restore_external_wallet(
        &self,
        user_id: i64,
        wallet_id: i64,
        external_wallet_id: &str,
        label: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count FROM wallets WHERE user_id = ? AND is_deleted = 0",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?
        .get("count");
        sqlx::query(
            "UPDATE wallets
             SET external_wallet_id=?,label=?,is_deleted=0,is_active=?,updated_at=CURRENT_TIMESTAMP
             WHERE user_id=? AND id=?",
        )
        .bind(external_wallet_id)
        .bind(label)
        .bind(i64::from(count == 0))
        .bind(user_id)
        .bind(wallet_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn bind_external_wallet(
        &self,
        user_id: i64,
        wallet_id: i64,
        external_wallet_id: &str,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE wallets SET external_wallet_id=?,updated_at=CURRENT_TIMESTAMP
             WHERE user_id=? AND id=? AND is_deleted=0",
        )
        .bind(external_wallet_id)
        .bind(user_id)
        .bind(wallet_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            anyhow::bail!("wallet is unavailable");
        }
        Ok(())
    }

    pub async fn sync_external_wallet_metadata(
        &self,
        user_id: i64,
        wallet_id: i64,
        external_wallet_id: &str,
        label: &str,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE wallets
             SET external_wallet_id=?,label=?,updated_at=CURRENT_TIMESTAMP
             WHERE user_id=? AND id=? AND is_deleted=0",
        )
        .bind(external_wallet_id)
        .bind(label)
        .bind(user_id)
        .bind(wallet_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            anyhow::bail!("wallet is unavailable");
        }
        Ok(())
    }

    pub async fn list_unbound_wallets(&self) -> Result<Vec<WalletRecord>> {
        let rows = sqlx::query(
            "SELECT id,user_id,external_wallet_id,label,public_key,is_active,is_deleted
             FROM wallets WHERE external_wallet_id IS NULL AND is_deleted=0 ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| WalletRecord {
                id: row.get("id"),
                user_id: row.get("user_id"),
                external_wallet_id: row.get("external_wallet_id"),
                label: row.get("label"),
                public_key: row.get("public_key"),
                is_active: row.get::<i64, _>("is_active") != 0,
                is_deleted: row.get::<i64, _>("is_deleted") != 0,
            })
            .collect())
    }

    #[allow(dead_code)]
    pub async fn restore_wallet(
        &self,
        user_id: i64,
        wallet_id: i64,
        label: &str,
        encrypted_private_key: &str,
        nonce: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count FROM wallets WHERE user_id = ? AND is_deleted = 0",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?
        .get("count");
        let is_active = if count == 0 { 1 } else { 0 };

        sqlx::query(
            "UPDATE wallets
             SET label = ?, is_deleted = 0, is_active = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ?",
        )
        .bind(label)
        .bind(is_active)
        .bind(user_id)
        .bind(wallet_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE wallet_secrets
             SET encrypted_private_key = ?, nonce = ?
             WHERE wallet_id = ?",
        )
        .bind(encrypted_private_key)
        .bind(nonce)
        .bind(wallet_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_wallet_secret(&self, wallet_id: i64) -> Result<(String, String)> {
        let row = sqlx::query(
            "SELECT encrypted_private_key, nonce FROM wallet_secrets WHERE wallet_id = ?",
        )
        .bind(wallet_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((row.get("encrypted_private_key"), row.get("nonce")))
    }

    pub async fn set_active_wallet(&self, user_id: i64, wallet_id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE wallets SET is_active = 0, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE wallets SET is_active = 1, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND id = ? AND is_deleted = 0",
        )
        .bind(user_id)
        .bind(wallet_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn soft_delete_wallet(&self, user_id: i64, wallet_id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE wallets SET is_deleted = 1, is_active = 0, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ?",
        )
        .bind(user_id)
        .bind(wallet_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rename_wallet(&self, user_id: i64, wallet_id: i64, label: &str) -> Result<()> {
        sqlx::query(
            "UPDATE wallets SET label = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ? AND is_deleted = 0",
        )
        .bind(label)
        .bind(user_id)
        .bind(wallet_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn active_wallet(&self, user_id: i64) -> Result<Option<WalletRecord>> {
        let row = sqlx::query(
            "SELECT id, user_id, external_wallet_id, label, public_key, is_active, is_deleted
             FROM wallets
             WHERE user_id = ? AND is_active = 1 AND is_deleted = 0
             ORDER BY id ASC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| WalletRecord {
            id: row.get("id"),
            user_id: row.get("user_id"),
            external_wallet_id: row.get("external_wallet_id"),
            label: row.get("label"),
            public_key: row.get("public_key"),
            is_active: row.get::<i64, _>("is_active") != 0,
            is_deleted: row.get::<i64, _>("is_deleted") != 0,
        }))
    }

    pub async fn list_watch_wallets(&self, user_id: i64) -> Result<Vec<WatchWallet>> {
        let rows = sqlx::query(
            "SELECT id, user_id, address, label, enabled FROM watch_wallets
             WHERE user_id = ? AND is_deleted = 0 ORDER BY id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| WatchWallet {
                id: row.get("id"),
                user_id: row.get("user_id"),
                address: row.get("address"),
                label: row.get("label"),
                enabled: row.get::<i64, _>("enabled") != 0,
            })
            .collect())
    }

    pub async fn count_strategies(&self, user_id: i64) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM strategies WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("count"))
    }

    pub async fn count_live_strategies(&self, user_id: i64) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS count FROM strategies WHERE user_id = ? AND dry_run = 0",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("count"))
    }

    pub async fn count_pending_limit_orders(&self, user_id: i64) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS count FROM limit_orders WHERE user_id = ? AND status = 'pending'",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("count"))
    }

    pub async fn count_live_pending_limit_orders(&self, user_id: i64) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS count
             FROM limit_orders
             WHERE user_id = ? AND status = 'pending' AND dry_run = 0",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("count"))
    }

    pub async fn latest_pumpapi_event_at(&self) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT COALESCE(
               (SELECT value FROM system_state WHERE key = 'pumpapi_stream_last_seen'),
               (SELECT MAX(received_at) FROM pumpapi_events)
             ) AS received_at",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("received_at"))
    }

    pub async fn touch_pumpapi_stream(&self) -> Result<()> {
        sqlx::query(
            "INSERT INTO system_state (key, value, updated_at)
             VALUES ('pumpapi_stream_last_seen', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET
               value = CURRENT_TIMESTAMP,
               updated_at = CURRENT_TIMESTAMP",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_watch_wallet(&self, user_id: i64, address: &str, label: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO watch_wallets (user_id, address, label) VALUES (?, ?, ?)
             ON CONFLICT(user_id, address) DO UPDATE SET
               label=excluded.label, enabled=1, is_deleted=0",
        )
        .bind(user_id)
        .bind(address)
        .bind(label)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE strategies SET enabled = 1, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND watch_wallet_id = (
               SELECT id FROM watch_wallets WHERE user_id = ? AND address = ?
             )",
        )
        .bind(user_id)
        .bind(user_id)
        .bind(address)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_watch_wallet(&self, user_id: i64, watch_id: i64) -> Result<WatchWallet> {
        let row = sqlx::query(
            "SELECT id, user_id, address, label, enabled FROM watch_wallets
             WHERE user_id = ? AND id = ? AND is_deleted = 0",
        )
        .bind(user_id)
        .bind(watch_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(WatchWallet {
            id: row.get("id"),
            user_id: row.get("user_id"),
            address: row.get("address"),
            label: row.get("label"),
            enabled: row.get::<i64, _>("enabled") != 0,
        })
    }

    pub async fn set_watch_enabled(
        &self,
        user_id: i64,
        watch_id: i64,
        enabled: bool,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE watch_wallets SET enabled = ?
             WHERE user_id = ? AND id = ? AND is_deleted = 0",
        )
        .bind(if enabled { 1 } else { 0 })
        .bind(user_id)
        .bind(watch_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE strategies SET enabled = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND watch_wallet_id = ?",
        )
        .bind(if enabled { 1 } else { 0 })
        .bind(user_id)
        .bind(watch_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_strategy_enabled_with_watch(
        &self,
        user_id: i64,
        strategy_id: i64,
        enabled: bool,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT watch_wallet_id FROM strategies WHERE user_id = ? AND id = ?")
                .bind(user_id)
                .bind(strategy_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(row) = row else {
            anyhow::bail!("strategy not found");
        };
        let watch_wallet_id: Option<i64> = row.get("watch_wallet_id");
        sqlx::query(
            "UPDATE strategies SET enabled = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ?",
        )
        .bind(if enabled { 1 } else { 0 })
        .bind(user_id)
        .bind(strategy_id)
        .execute(&mut *tx)
        .await?;
        if let Some(watch_wallet_id) = watch_wallet_id {
            sqlx::query(
                "UPDATE watch_wallets SET enabled = ?
                 WHERE user_id = ? AND id = ? AND is_deleted = 0",
            )
            .bind(if enabled { 1 } else { 0 })
            .bind(user_id)
            .bind(watch_wallet_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn rename_watch_wallet(
        &self,
        user_id: i64,
        watch_id: i64,
        label: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE watch_wallets SET label = ?
             WHERE user_id = ? AND id = ? AND is_deleted = 0",
        )
        .bind(label)
        .bind(user_id)
        .bind(watch_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_watch_wallet(&self, user_id: i64, watch_id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE strategies SET enabled = 0, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND watch_wallet_id = ?",
        )
        .bind(user_id)
        .bind(watch_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE watch_wallets SET enabled = 0, is_deleted = 1
             WHERE user_id = ? AND id = ?",
        )
        .bind(user_id)
        .bind(watch_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn find_watch_matches(&self, signer: &str) -> Result<Vec<WatchMatch>> {
        let rows = sqlx::query(
            "SELECT w.id AS watch_id, w.user_id, u.telegram_id, w.label AS watch_label, w.address AS watch_address
             FROM watch_wallets w
             JOIN app_users u ON u.id = w.user_id
             WHERE w.enabled = 1 AND w.is_deleted = 0 AND w.address = ?",
        )
        .bind(signer)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| WatchMatch {
                watch_id: row.get("watch_id"),
                user_id: row.get("user_id"),
                telegram_id: row.get("telegram_id"),
                watch_label: row.get("watch_label"),
                watch_address: row.get("watch_address"),
            })
            .collect())
    }

    pub async fn list_active_watch_matches(&self) -> Result<Vec<WatchMatch>> {
        let rows = sqlx::query(
            "SELECT w.id AS watch_id, w.user_id, u.telegram_id,
                    w.label AS watch_label, w.address AS watch_address
             FROM watch_wallets w
             JOIN app_users u ON u.id = w.user_id
             WHERE w.enabled = 1 AND w.is_deleted = 0
             ORDER BY w.id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| WatchMatch {
                watch_id: row.get("watch_id"),
                user_id: row.get("user_id"),
                telegram_id: row.get("telegram_id"),
                watch_label: row.get("watch_label"),
                watch_address: row.get("watch_address"),
            })
            .collect())
    }

    pub async fn insert_or_get_pumpapi_event(&self, event: &PumpApiEvent) -> Result<i64> {
        let inserted = self.insert_pumpapi_event(event).await?;
        if inserted > 0 {
            return Ok(inserted);
        }
        let Some(signature) = event.signature.as_deref() else {
            anyhow::bail!("duplicate PumpAPI event without signature cannot be resolved");
        };
        let row = sqlx::query(
            "SELECT id FROM pumpapi_events
             WHERE signature = ? AND tx_type IS ?
             ORDER BY id ASC LIMIT 1",
        )
        .bind(signature)
        .bind(&event.tx_type)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn get_or_create_strategy_for_watch(
        &self,
        user_id: i64,
        watch_wallet_id: i64,
        wallet_id: Option<i64>,
        name: &str,
        default_buy_amount_sol: f64,
        default_slippage_bps: u64,
        default_tip_sol: f64,
    ) -> Result<StrategyConfig> {
        if let Some(strategy) = self
            .get_strategy_for_watch(user_id, watch_wallet_id)
            .await?
        {
            return Ok(strategy);
        }

        sqlx::query(
            "INSERT INTO strategies
             (user_id, wallet_id, watch_wallet_id, name, enabled, dry_run, copy_buy, copy_sell,
              buy_amount_sol, buy_mode, buy_ratio_percent, sell_percent, max_slippage_bps, tip_sol,
              max_per_mint_sol, daily_max_sol, cooldown_secs, only_once,
              min_sol_in_pool, min_mcap, max_mcap, min_target_buy_sol, max_target_buy_sol,
              max_signal_delay_ms, allowed_pools, mev_mode, live)
             VALUES (?, ?, ?, ?, 1, 1, 1, 0, ?, 'fixed', 100, NULL, ?, ?, 0.05, 0.2, 60, 0,
                     0, 0, 0, 0, 0, NULL, NULL, 'none', 0)",
        )
        .bind(user_id)
        .bind(wallet_id)
        .bind(watch_wallet_id)
        .bind(name)
        .bind(default_buy_amount_sol)
        .bind(default_slippage_bps as i64)
        .bind(default_tip_sol)
        .execute(&self.pool)
        .await?;

        self.get_strategy_for_watch(user_id, watch_wallet_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("strategy was not created"))
    }

    pub async fn get_strategy_for_watch(
        &self,
        user_id: i64,
        watch_wallet_id: i64,
    ) -> Result<Option<StrategyConfig>> {
        let row = sqlx::query(
            "SELECT id, user_id, wallet_id, watch_wallet_id, name, enabled, dry_run,
                    execution_mode, execution_method,
                    copy_buy, copy_sell, buy_amount_sol, buy_mode, buy_ratio_percent,
                    sell_percent, max_slippage_bps,
                    tip_sol, max_per_mint_sol, daily_max_sol, cooldown_secs, only_once,
                    min_sol_in_pool, min_mcap, max_mcap, min_target_buy_sol, max_target_buy_sol,
                    max_signal_delay_ms, allowed_pools, mev_mode, live
             FROM strategies
             WHERE user_id = ? AND watch_wallet_id = ?
             ORDER BY id ASC
             LIMIT 1",
        )
        .bind(user_id)
        .bind(watch_wallet_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(row_to_strategy))
    }

    pub async fn list_strategies(&self, user_id: i64) -> Result<Vec<StrategyConfig>> {
        let rows = sqlx::query(
            "SELECT id, user_id, wallet_id, watch_wallet_id, name, enabled, dry_run,
                    execution_mode, execution_method,
                    copy_buy, copy_sell, buy_amount_sol, buy_mode, buy_ratio_percent,
                    sell_percent, max_slippage_bps,
                    tip_sol, max_per_mint_sol, daily_max_sol, cooldown_secs, only_once,
                    min_sol_in_pool, min_mcap, max_mcap, min_target_buy_sol, max_target_buy_sol,
                    max_signal_delay_ms, allowed_pools, mev_mode, live
             FROM strategies
             WHERE user_id = ?
             ORDER BY id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_strategy).collect())
    }

    pub async fn get_strategy(
        &self,
        user_id: i64,
        strategy_id: i64,
    ) -> Result<Option<StrategyConfig>> {
        let row = sqlx::query(
            "SELECT id, user_id, wallet_id, watch_wallet_id, name, enabled, dry_run,
                    execution_mode, execution_method,
                    copy_buy, copy_sell, buy_amount_sol, buy_mode, buy_ratio_percent,
                    sell_percent, max_slippage_bps, tip_sol, max_per_mint_sol,
                    daily_max_sol, cooldown_secs, only_once, min_sol_in_pool,
                    min_mcap, max_mcap, min_target_buy_sol, max_target_buy_sol,
                    max_signal_delay_ms, allowed_pools, mev_mode, live
             FROM strategies WHERE user_id = ? AND id = ?",
        )
        .bind(user_id)
        .bind(strategy_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_strategy))
    }

    pub async fn update_strategy_field(
        &self,
        user_id: i64,
        strategy_id: i64,
        field: &str,
        value: &str,
    ) -> Result<()> {
        let allowed = [
            "enabled",
            "dry_run",
            "execution_mode",
            "execution_method",
            "live",
            "copy_buy",
            "copy_sell",
            "buy_amount_sol",
            "buy_mode",
            "buy_ratio_percent",
            "sell_percent",
            "max_slippage_bps",
            "tip_sol",
            "min_sol_in_pool",
            "min_mcap",
            "max_mcap",
            "min_target_buy_sol",
            "max_target_buy_sol",
            "max_signal_delay_ms",
            "allowed_pools",
            "mev_mode",
        ];
        if !allowed.contains(&field) {
            anyhow::bail!("unsupported strategy field: {field}");
        }
        let sql = if field == "dry_run" {
            "UPDATE strategies SET dry_run = ?, execution_mode = CASE WHEN ? = '1' THEN 'dry_run' ELSE 'live' END, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND id = ?".to_string()
        } else if field == "execution_mode" {
            "UPDATE strategies SET execution_mode = ?, dry_run = CASE WHEN ? = 'dry_run' THEN 1 ELSE 0 END, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND id = ?".to_string()
        } else {
            format!("UPDATE strategies SET {field} = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND id = ?")
        };
        let mut query = sqlx::query(&sql).bind(value);
        if field == "dry_run" || field == "execution_mode" {
            query = query.bind(value);
        }
        query
            .bind(user_id)
            .bind(strategy_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_pumpapi_event(&self, event: &PumpApiEvent) -> Result<i64> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO pumpapi_events
             (signature, mint, pool, pool_id, quote_mint, tx_type, signer, raw_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.signature)
        .bind(&event.mint)
        .bind(&event.pool)
        .bind(&event.pool_id)
        .bind(&event.quote_mint)
        .bind(&event.tx_type)
        .bind(&event.signer)
        .bind(&event.raw_json)
        .execute(&self.pool)
        .await?;
        Ok(if result.rows_affected() > 0 {
            result.last_insert_rowid()
        } else {
            0
        })
    }

    pub async fn insert_alpha_observation_event(&self, event: &PumpApiEvent) -> Result<bool> {
        let event_id = alpha_event_id(event);
        let route_status = if event.pool.as_deref().is_some_and(is_known_trade_pool)
            && event.quote_mint.is_some()
        {
            "candidate"
        } else if event.pool.is_some() {
            "observable-only"
        } else {
            "unknown"
        };
        let result = sqlx::query(
            "INSERT OR IGNORE INTO alpha_observation_events
             (event_id, signature, mint, pool, pool_id, quote_mint, tx_type, signer,
              event_timestamp_ms, received_unix_ms, route_status, raw_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(event_id)
        .bind(&event.signature)
        .bind(&event.mint)
        .bind(&event.pool)
        .bind(&event.pool_id)
        .bind(&event.quote_mint)
        .bind(&event.tx_type)
        .bind(&event.signer)
        .bind(event.event_timestamp_ms)
        .bind(event.received_unix_ms)
        .bind(route_status)
        .bind(&event.raw_json)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn list_alpha_observation_events(
        &self,
        after_received_ms: i64,
        limit: usize,
    ) -> Result<Vec<AlphaObservationEvent>> {
        let limit = limit.clamp(1, 100_000) as i64;
        let rows = sqlx::query(
            "SELECT event_id, signature, mint, pool, pool_id, quote_mint, tx_type, signer,
                    event_timestamp_ms, received_unix_ms, route_status, raw_json
             FROM alpha_observation_events
             WHERE received_unix_ms >= ?
             ORDER BY received_unix_ms ASC, event_id ASC
             LIMIT ?",
        )
        .bind(after_received_ms)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AlphaObservationEvent {
                event_id: row.get("event_id"),
                signature: row.get("signature"),
                mint: row.get("mint"),
                pool: row.get("pool"),
                pool_id: row.get("pool_id"),
                quote_mint: row.get("quote_mint"),
                tx_type: row.get("tx_type"),
                signer: row.get("signer"),
                event_timestamp_ms: row.get("event_timestamp_ms"),
                received_unix_ms: row.get("received_unix_ms"),
                route_status: row.get("route_status"),
                raw_json: row.get("raw_json"),
            })
            .collect())
    }

    pub async fn insert_alpha_paper_signal(&self, signal: &AlphaPaperSignal) -> Result<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO alpha_paper_signals
             (signal_id, event_id, mint, pool, pool_id, quote_mint, side, score, status,
             features_json, created_at_ms, updated_at_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&signal.signal_id)
        .bind(&signal.event_id)
        .bind(&signal.mint)
        .bind(&signal.pool)
        .bind(&signal.pool_id)
        .bind(&signal.quote_mint)
        .bind(&signal.side)
        .bind(signal.score)
        .bind(&signal.status)
        .bind(&signal.features_json)
        .bind(signal.created_at_ms)
        .bind(signal.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn list_alpha_paper_signals(&self, limit: usize) -> Result<Vec<AlphaPaperSignal>> {
        let limit = limit.clamp(1, 100_000) as i64;
        let rows = sqlx::query(
            "SELECT signal_id, event_id, mint, pool, pool_id, quote_mint, side, score, status,
                    features_json, created_at_ms, order_id,
                    trade_service_request_id, remote_status, updated_at_ms
             FROM alpha_paper_signals
             ORDER BY created_at_ms DESC, signal_id DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AlphaPaperSignal {
                signal_id: row.get("signal_id"),
                event_id: row.get("event_id"),
                mint: row.get("mint"),
                pool: row.get("pool"),
                pool_id: row.get("pool_id"),
                quote_mint: row.get("quote_mint"),
                side: row.get("side"),
                score: row.get("score"),
                status: row.get("status"),
                features_json: row.get("features_json"),
                created_at_ms: row.get("created_at_ms"),
                order_id: row.get("order_id"),
                trade_service_request_id: row.get("trade_service_request_id"),
                remote_status: row.get("remote_status"),
                updated_at_ms: row.get("updated_at_ms"),
            })
            .collect())
    }

    pub async fn update_alpha_paper_signal_link(
        &self,
        signal_id: &str,
        order_id: Option<&str>,
        trade_service_request_id: Option<&str>,
        remote_status: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE alpha_paper_signals
                SET order_id = COALESCE(?, order_id),
                    trade_service_request_id = COALESCE(?, trade_service_request_id),
                    remote_status = COALESCE(?, remote_status),
                    updated_at_ms = ?
              WHERE signal_id = ?",
        )
        .bind(order_id)
        .bind(trade_service_request_id)
        .bind(remote_status)
        .bind(updated_at_ms)
        .bind(signal_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn reserve_alpha_shadow_order(
        &self,
        signal: &AlphaPaperSignal,
        idempotency_key: &str,
        request_json: &str,
        amount_sol: Option<f64>,
        sell_percent: Option<i64>,
        now_ms: i64,
    ) -> Result<(String, bool)> {
        let id = uuid::Uuid::new_v4().to_string();
        let pool = signal.pool.as_deref().unwrap_or("unknown");
        let quote_mint = signal
            .quote_mint
            .as_deref()
            .context("alpha signal is missing quote_mint")?;
        let result = sqlx::query(
            "INSERT OR IGNORE INTO alpha_shadow_orders
             (id, signal_id, mint, side, pool, pool_id, quote_mint, amount_sol, sell_percent,
              status, idempotency_key, request_json, created_at_ms, updated_at_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'created', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&signal.signal_id)
        .bind(&signal.mint)
        .bind(&signal.side)
        .bind(pool)
        .bind(&signal.pool_id)
        .bind(quote_mint)
        .bind(amount_sol)
        .bind(sell_percent)
        .bind(idempotency_key)
        .bind(request_json)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() > 0 {
            return Ok((id, true));
        }
        let row = sqlx::query(
            "SELECT id FROM alpha_shadow_orders WHERE signal_id = ? OR idempotency_key = ?",
        )
        .bind(&signal.signal_id)
        .bind(idempotency_key)
        .fetch_one(&self.pool)
        .await?;
        Ok((row.get("id"), false))
    }

    pub async fn update_alpha_shadow_order(
        &self,
        order_id: &str,
        status: &str,
        request_id: Option<&str>,
        response_json: Option<&str>,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE alpha_shadow_orders
                SET status = ?,
                    trade_service_request_id = COALESCE(?, trade_service_request_id),
                    response_json = COALESCE(?, response_json),
                    error = ?,
                    updated_at_ms = ?
              WHERE id = ?",
        )
        .bind(status)
        .bind(request_id)
        .bind(response_json)
        .bind(error)
        .bind(now_ms)
        .bind(order_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_alpha_shadow_reconcilable(
        &self,
        limit: usize,
    ) -> Result<Vec<AlphaShadowOrder>> {
        let limit = limit.clamp(1, 100_000) as i64;
        let rows = sqlx::query(
            "SELECT id, signal_id, mint, side, pool, pool_id, quote_mint, amount_sol,
                    sell_percent, status, idempotency_key, request_json, response_json,
                    trade_service_request_id, error, created_at_ms, updated_at_ms
               FROM alpha_shadow_orders
              WHERE trade_service_request_id IS NOT NULL
                AND status IN ('accepted', 'warming_params', 'building_tx', 'submitting', 'unknown')
              ORDER BY updated_at_ms ASC
              LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AlphaShadowOrder {
                id: row.get("id"),
                signal_id: row.get("signal_id"),
                mint: row.get("mint"),
                side: row.get("side"),
                pool: row.get("pool"),
                pool_id: row.get("pool_id"),
                quote_mint: row.get("quote_mint"),
                amount_sol: row.get("amount_sol"),
                sell_percent: row.get("sell_percent"),
                status: row.get("status"),
                idempotency_key: row.get("idempotency_key"),
                request_json: row.get("request_json"),
                response_json: row.get("response_json"),
                trade_service_request_id: row.get("trade_service_request_id"),
                error: row.get("error"),
                created_at_ms: row.get("created_at_ms"),
                updated_at_ms: row.get("updated_at_ms"),
            })
            .collect())
    }

    pub async fn latest_observed_pool_route(
        &self,
        mint: &str,
    ) -> Result<Option<ObservedPoolRoute>> {
        let row = sqlx::query(
            "SELECT pool,pool_id,quote_mint
             FROM mint_pool_routes
             WHERE mint=?",
        )
        .bind(mint)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ObservedPoolRoute {
            pool: row.get("pool"),
            pool_id: row.get("pool_id"),
            quote_mint: row.get("quote_mint"),
        }))
    }

    pub async fn observed_pool_route_candidates(
        &self,
        mint: &str,
    ) -> Result<Vec<ObservedPoolRoute>> {
        let rows = sqlx::query(
            "SELECT pool,pool_id,quote_mint
             FROM mint_pool_route_candidates
             WHERE mint=?
             ORDER BY updated_at DESC",
        )
        .bind(mint)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ObservedPoolRoute {
                pool: row.get("pool"),
                pool_id: match row.get::<String, _>("pool_id").as_str() {
                    "" => None,
                    value => Some(value.to_string()),
                },
                quote_mint: row.get("quote_mint"),
            })
            .collect())
    }

    pub async fn upsert_pool_route(
        &self,
        mint: &str,
        pool: &str,
        pool_id: Option<&str>,
        quote_mint: &str,
        source: &str,
    ) -> Result<()> {
        let route_key = format!("{mint}|{pool}|{}|{quote_mint}", pool_id.unwrap_or_default());
        sqlx::query(
            "INSERT INTO mint_pool_route_candidates
             (route_key,mint,pool,pool_id,quote_mint,source)
             VALUES (?,?,?,?,?,?)
             ON CONFLICT(route_key) DO UPDATE SET
               source=excluded.source,
               updated_at=CURRENT_TIMESTAMP",
        )
        .bind(route_key)
        .bind(mint)
        .bind(pool)
        .bind(pool_id.unwrap_or_default())
        .bind(quote_mint)
        .bind(source)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "INSERT INTO mint_pool_routes (mint,pool,pool_id,quote_mint,source)
             VALUES (?,?,?,?,?)
             ON CONFLICT(mint) DO UPDATE SET
               pool=excluded.pool,
               pool_id=excluded.pool_id,
               quote_mint=excluded.quote_mint,
               source=excluded.source,
               updated_at=CURRENT_TIMESTAMP",
        )
        .bind(mint)
        .bind(pool)
        .bind(pool_id)
        .bind(quote_mint)
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_signal(
        &self,
        user_id: i64,
        strategy_id: Option<i64>,
        source_event_id: i64,
        mint: &str,
        side: &str,
        reason: &str,
        status: &str,
        metadata_json: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO signals (user_id, strategy_id, source_event_id, mint, side, reason, status, metadata_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(strategy_id)
        .bind(source_event_id)
        .bind(mint)
        .bind(side)
        .bind(reason)
        .bind(status)
        .bind(metadata_json)
        .execute(&self.pool)
        .await?;

        self.audit(
            Some(user_id),
            "signal.created",
            Some("signal"),
            Some(&result.last_insert_rowid().to_string()),
            metadata_json.or(Some(reason)),
        )
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_recent_signals(
        &self,
        user_id: i64,
        limit: i64,
    ) -> Result<Vec<SignalSummary>> {
        let rows = sqlx::query(
            "SELECT id, mint, side, status, reason, metadata_json, created_at
             FROM signals
             WHERE user_id = ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| SignalSummary {
                id: row.get("id"),
                mint: row.get("mint"),
                side: row.get("side"),
                status: row.get("status"),
                reason: row.get("reason"),
                metadata_json: row.get("metadata_json"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_order(
        &self,
        user_id: i64,
        wallet_id: Option<i64>,
        strategy_id: Option<i64>,
        signal_id: Option<i64>,
        mint: &str,
        side: &str,
        amount_sol: Option<f64>,
        sell_percent: Option<f64>,
        status: &str,
        idempotency_key: &str,
        request_json: Option<&str>,
        response_json: Option<&str>,
        tx_signature: Option<&str>,
        error: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO orders
             (id, user_id, wallet_id, strategy_id, signal_id, mint, side, amount_sol, sell_percent, status,
              idempotency_key, trade_service_request_json, trade_service_response_json,
              tx_signature, error)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(wallet_id)
        .bind(strategy_id)
        .bind(signal_id)
        .bind(mint)
        .bind(side)
        .bind(amount_sol)
        .bind(sell_percent)
        .bind(status)
        .bind(idempotency_key)
        .bind(request_json)
        .bind(response_json)
        .bind(tx_signature)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn reserve_order(
        &self,
        user_id: i64,
        wallet_id: Option<i64>,
        strategy_id: Option<i64>,
        signal_id: Option<i64>,
        mint: &str,
        side: &str,
        amount_sol: Option<f64>,
        sell_percent: Option<f64>,
        idempotency_key: &str,
        request_json: &str,
    ) -> Result<(String, bool)> {
        let id = uuid::Uuid::new_v4().to_string();
        let result = sqlx::query(
            "INSERT OR IGNORE INTO orders
             (id, user_id, wallet_id, strategy_id, signal_id, mint, side, amount_sol, sell_percent,
              status, idempotency_key, trade_service_request_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'created', ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(wallet_id)
        .bind(strategy_id)
        .bind(signal_id)
        .bind(mint)
        .bind(side)
        .bind(amount_sol)
        .bind(sell_percent)
        .bind(idempotency_key)
        .bind(request_json)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok((id, true));
        }
        let row = sqlx::query("SELECT id FROM orders WHERE idempotency_key = ?")
            .bind(idempotency_key)
            .fetch_one(&self.pool)
            .await?;
        Ok((row.get("id"), false))
    }

    pub async fn update_order_from_trade_service(
        &self,
        order_id: &str,
        status: &str,
        trade_service_request_id: Option<&str>,
        response_json: Option<&str>,
        tx_signature: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE orders
             SET status = ?,
                 trade_service_request_id = COALESCE(?, trade_service_request_id),
                 trade_service_response_json = COALESCE(?, trade_service_response_json),
                 tx_signature = COALESCE(?, tx_signature),
                 error = ?,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
        )
        .bind(status)
        .bind(trade_service_request_id)
        .bind(response_json)
        .bind(tx_signature)
        .bind(error)
        .bind(order_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_reconcilable_orders(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, String, String)>> {
        let rows = sqlx::query(
            "SELECT id, trade_service_request_id, status
             FROM orders
             WHERE trade_service_request_id IS NOT NULL
               AND status IN ('accepted', 'warming_params', 'building_tx', 'submitting')
             ORDER BY updated_at ASC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.get("id"),
                    row.get("trade_service_request_id"),
                    row.get("status"),
                )
            })
            .collect())
    }

    pub async fn list_orphaned_submissions(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT o.id, o.trade_service_request_json,
                    (SELECT COUNT(*) FROM order_attempts a
                     WHERE a.order_id = o.id AND a.status = 'submitting') AS submit_attempts
             FROM orders o
             WHERE o.trade_service_request_id IS NULL
               AND o.status IN ('created', 'submitting')
               AND o.trade_service_request_json IS NOT NULL
               AND o.updated_at <= datetime('now', '-45 seconds')
             ORDER BY o.updated_at ASC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.get("id"),
                    row.get("trade_service_request_json"),
                    row.get("submit_attempts"),
                )
            })
            .collect())
    }

    pub async fn insert_order_attempt(
        &self,
        order_id: &str,
        status: &str,
        request_json: Option<&str>,
        response_json: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO order_attempts (order_id, status, request_json, response_json, error)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(order_id)
        .bind(status)
        .bind(request_json)
        .bind(response_json)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_dry_run_position_amount(
        &self,
        user_id: i64,
        strategy_id: i64,
        mint: &str,
    ) -> Result<Option<f64>> {
        let row = sqlx::query(
            "SELECT amount_sol
             FROM dry_run_positions
             WHERE user_id = ? AND strategy_id = ? AND mint = ?",
        )
        .bind(user_id)
        .bind(strategy_id)
        .bind(mint)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| row.get("amount_sol")))
    }

    pub async fn add_dry_run_position_buy(
        &self,
        user_id: i64,
        strategy_id: i64,
        wallet_id: Option<i64>,
        mint: &str,
        amount_sol: f64,
        signal_id: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO dry_run_positions
             (user_id, strategy_id, wallet_id, mint, amount_sol, buy_count, last_buy_signal_id)
             VALUES (?, ?, ?, ?, ?, 1, ?)
             ON CONFLICT(user_id, strategy_id, mint) DO UPDATE SET
               wallet_id = excluded.wallet_id,
               amount_sol = dry_run_positions.amount_sol + excluded.amount_sol,
               buy_count = dry_run_positions.buy_count + 1,
               last_buy_signal_id = excluded.last_buy_signal_id,
               updated_at = CURRENT_TIMESTAMP",
        )
        .bind(user_id)
        .bind(strategy_id)
        .bind(wallet_id)
        .bind(mint)
        .bind(amount_sol)
        .bind(signal_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn apply_dry_run_position_sell(
        &self,
        user_id: i64,
        strategy_id: i64,
        mint: &str,
        sell_percent: f64,
        signal_id: i64,
    ) -> Result<Option<(f64, f64, f64)>> {
        let Some(before) = self
            .get_dry_run_position_amount(user_id, strategy_id, mint)
            .await?
        else {
            return Ok(None);
        };
        if before <= 0.0 {
            return Ok(None);
        }

        let pct = sell_percent.clamp(0.0, 100.0);
        let sold = before * pct / 100.0;
        let after = (before - sold).max(0.0);
        sqlx::query(
            "UPDATE dry_run_positions
             SET amount_sol = ?,
                 sell_count = sell_count + 1,
                 last_sell_signal_id = ?,
                 updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND strategy_id = ? AND mint = ?",
        )
        .bind(after)
        .bind(signal_id)
        .bind(user_id)
        .bind(strategy_id)
        .bind(mint)
        .execute(&self.pool)
        .await?;

        Ok(Some((before, sold, after)))
    }

    pub async fn list_recent_orders(&self, user_id: i64, limit: i64) -> Result<Vec<OrderSummary>> {
        let rows = sqlx::query(
            "SELECT id, mint, side, status, amount_sol, sell_percent, trade_service_request_json, tx_signature, error, created_at
             FROM orders WHERE user_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| OrderSummary {
                id: row.get("id"),
                mint: row.get("mint"),
                side: row.get("side"),
                status: row.get("status"),
                amount_sol: row.get("amount_sol"),
                sell_percent: row.get("sell_percent"),
                request_json: row.get("trade_service_request_json"),
                tx_signature: row.get("tx_signature"),
                error: row.get("error"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    pub async fn get_order_detail(
        &self,
        user_id: i64,
        order_id: &str,
    ) -> Result<Option<OrderDetail>> {
        let row = sqlx::query(
            "SELECT id, wallet_id, strategy_id, signal_id, mint, side, amount_sol, sell_percent,
                    status, idempotency_key, trade_service_request_id, trade_service_request_json,
                    trade_service_response_json, tx_signature, error, created_at, updated_at
             FROM orders
             WHERE user_id = ? AND id = ?",
        )
        .bind(user_id)
        .bind(order_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| OrderDetail {
            id: row.get("id"),
            wallet_id: row.get("wallet_id"),
            strategy_id: row.get("strategy_id"),
            signal_id: row.get("signal_id"),
            mint: row.get("mint"),
            side: row.get("side"),
            status: row.get("status"),
            amount_sol: row.get("amount_sol"),
            sell_percent: row.get("sell_percent"),
            idempotency_key: row.get("idempotency_key"),
            trade_service_request_id: row.get("trade_service_request_id"),
            request_json: row.get("trade_service_request_json"),
            response_json: row.get("trade_service_response_json"),
            tx_signature: row.get("tx_signature"),
            error: row.get("error"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }))
    }

    pub async fn list_open_dry_run_positions(
        &self,
        user_id: i64,
        limit: i64,
    ) -> Result<Vec<PositionSummary>> {
        let rows = sqlx::query(
            "SELECT mint, amount_sol, buy_count, sell_count, updated_at
             FROM dry_run_positions
             WHERE user_id = ? AND amount_sol > 0
             ORDER BY updated_at DESC
             LIMIT ?",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| PositionSummary {
                mint: row.get("mint"),
                amount_sol: row.get("amount_sol"),
                buy_count: row.get("buy_count"),
                sell_count: row.get("sell_count"),
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    pub async fn insert_limit_order(
        &self,
        user_id: i64,
        wallet_id: Option<i64>,
        mint: &str,
        action: LimitAction,
        trigger_metric: &str,
        trigger_price: f64,
        amount_sol: Option<f64>,
        sell_percent: Option<f64>,
        execution: &ExecutionParams,
        expires_at: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO limit_orders
             (id, user_id, wallet_id, mint, side, trigger_type, limit_action, trigger_metric, trigger_price,
              trigger_currency, amount_sol, sell_percent, dry_run, execution_mode, execution_method,
              slippage_bps, tip_sol, mev_mode, expires_at, status)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'USD', ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending')",
        )
        .bind(&id)
        .bind(user_id)
        .bind(wallet_id)
        .bind(mint)
        .bind(action.side())
        .bind(action.trigger_type())
        .bind(action.as_str())
        .bind(trigger_metric)
        .bind(trigger_price)
        .bind(amount_sol)
        .bind(sell_percent)
        .bind(if execution.mode.is_dry_run() { 1 } else { 0 })
        .bind(execution.mode.as_str())
        .bind(execution.method.as_str())
        .bind(execution.slippage_bps as i64)
        .bind(execution.tip_sol)
        .bind(&execution.mev_mode)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn list_limit_orders(
        &self,
        user_id: i64,
        limit: i64,
    ) -> Result<Vec<LimitOrderSummary>> {
        let rows = sqlx::query(
            "SELECT id, user_id, wallet_id, mint, side, trigger_type, trigger_metric, trigger_price,
                    trigger_currency, amount_sol, sell_percent, dry_run, execution_mode,
                    execution_method, slippage_bps, tip_sol, mev_mode, limit_action, status, triggered_price,
                    triggered_currency, triggered_source_event_id, triggered_at, expires_at, created_at
             FROM limit_orders
             WHERE user_id = ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| LimitOrderSummary {
                id: row.get("id"),
                user_id: row.get("user_id"),
                wallet_id: row.get("wallet_id"),
                mint: row.get("mint"),
                side: row.get("side"),
                trigger_type: row.get("trigger_type"),
                trigger_metric: row.get("trigger_metric"),
                trigger_price: row.get("trigger_price"),
                trigger_currency: row.get("trigger_currency"),
                amount_sol: row.get("amount_sol"),
                sell_percent: row.get("sell_percent"),
                dry_run: row.get::<i64, _>("dry_run") != 0,
                execution_mode: row.get("execution_mode"),
                execution_method: row.get("execution_method"),
                slippage_bps: row.get::<i64, _>("slippage_bps") as u64,
                tip_sol: row.get("tip_sol"),
                mev_mode: row.get("mev_mode"),
                limit_action: row.get("limit_action"),
                status: row.get("status"),
                triggered_price: row.get("triggered_price"),
                triggered_currency: row.get("triggered_currency"),
                triggered_source_event_id: row.get("triggered_source_event_id"),
                triggered_at: row.get("triggered_at"),
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    pub async fn has_pending_limit_orders_for_mint(&self, mint: &str) -> Result<bool> {
        let count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count
             FROM limit_orders
             WHERE mint = ? AND status = 'pending'",
        )
        .bind(mint)
        .fetch_one(&self.pool)
        .await?
        .get("count");
        Ok(count > 0)
    }

    pub async fn list_pending_limit_orders_for_mint(
        &self,
        mint: &str,
    ) -> Result<Vec<LimitOrderSummary>> {
        let rows = sqlx::query(
            "SELECT id, user_id, wallet_id, mint, side, trigger_type, trigger_metric, trigger_price,
                    trigger_currency, amount_sol, sell_percent, dry_run, execution_mode,
                    execution_method, slippage_bps, tip_sol, mev_mode, limit_action, status, triggered_price,
                    triggered_currency, triggered_source_event_id, triggered_at, expires_at, created_at
             FROM limit_orders
             WHERE mint = ? AND status = 'pending'
               AND (expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP)
             ORDER BY created_at ASC",
        )
        .bind(mint)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| LimitOrderSummary {
                id: row.get("id"),
                user_id: row.get("user_id"),
                wallet_id: row.get("wallet_id"),
                mint: row.get("mint"),
                side: row.get("side"),
                trigger_type: row.get("trigger_type"),
                trigger_metric: row.get("trigger_metric"),
                trigger_price: row.get("trigger_price"),
                trigger_currency: row.get("trigger_currency"),
                amount_sol: row.get("amount_sol"),
                sell_percent: row.get("sell_percent"),
                dry_run: row.get::<i64, _>("dry_run") != 0,
                execution_mode: row.get("execution_mode"),
                execution_method: row.get("execution_method"),
                slippage_bps: row.get::<i64, _>("slippage_bps") as u64,
                tip_sol: row.get("tip_sol"),
                mev_mode: row.get("mev_mode"),
                limit_action: row.get("limit_action"),
                status: row.get("status"),
                triggered_price: row.get("triggered_price"),
                triggered_currency: row.get("triggered_currency"),
                triggered_source_event_id: row.get("triggered_source_event_id"),
                triggered_at: row.get("triggered_at"),
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    pub async fn get_limit_order(
        &self,
        user_id: i64,
        order_id: &str,
    ) -> Result<Option<LimitOrderSummary>> {
        let row = sqlx::query(
            "SELECT id, user_id, wallet_id, mint, side, trigger_type, trigger_metric, trigger_price,
                    trigger_currency, amount_sol, sell_percent, dry_run, execution_mode,
                    execution_method, slippage_bps, tip_sol, mev_mode, limit_action, status, triggered_price,
                    triggered_currency, triggered_source_event_id, triggered_at, expires_at, created_at
             FROM limit_orders
             WHERE user_id = ? AND id = ?",
        )
        .bind(user_id)
        .bind(order_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| LimitOrderSummary {
            id: row.get("id"),
            user_id: row.get("user_id"),
            wallet_id: row.get("wallet_id"),
            mint: row.get("mint"),
            side: row.get("side"),
            trigger_type: row.get("trigger_type"),
            trigger_metric: row.get("trigger_metric"),
            trigger_price: row.get("trigger_price"),
            trigger_currency: row.get("trigger_currency"),
            amount_sol: row.get("amount_sol"),
            sell_percent: row.get("sell_percent"),
            dry_run: row.get::<i64, _>("dry_run") != 0,
            execution_mode: row.get("execution_mode"),
            execution_method: row.get("execution_method"),
            slippage_bps: row.get::<i64, _>("slippage_bps") as u64,
            tip_sol: row.get("tip_sol"),
            mev_mode: row.get("mev_mode"),
            limit_action: row.get("limit_action"),
            status: row.get("status"),
            triggered_price: row.get("triggered_price"),
            triggered_currency: row.get("triggered_currency"),
            triggered_source_event_id: row.get("triggered_source_event_id"),
            triggered_at: row.get("triggered_at"),
            expires_at: row.get("expires_at"),
            created_at: row.get("created_at"),
        }))
    }

    pub async fn find_order_for_limit_order(
        &self,
        user_id: i64,
        limit_order_id: &str,
    ) -> Result<Option<OrderSummary>> {
        let needle = format!("%\"limit_order_id\":\"{}\"%", limit_order_id);
        let row = sqlx::query(
            "SELECT id, mint, side, status, amount_sol, sell_percent, trade_service_request_json, tx_signature, error, created_at
             FROM orders
             WHERE user_id = ? AND trade_service_request_json LIKE ?
             ORDER BY created_at DESC
             LIMIT 1",
        )
        .bind(user_id)
        .bind(needle)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| OrderSummary {
            id: row.get("id"),
            mint: row.get("mint"),
            side: row.get("side"),
            status: row.get("status"),
            amount_sol: row.get("amount_sol"),
            sell_percent: row.get("sell_percent"),
            request_json: row.get("trade_service_request_json"),
            tx_signature: row.get("tx_signature"),
            error: row.get("error"),
            created_at: row.get("created_at"),
        }))
    }

    pub async fn mark_limit_order_triggered(
        &self,
        user_id: i64,
        order_id: &str,
        triggered_price: f64,
        triggered_currency: &str,
        source_event_id: i64,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE limit_orders
             SET status = 'triggered',
                 triggered_price = ?,
                 triggered_currency = ?,
                 triggered_source_event_id = ?,
                 triggered_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ? AND status = 'pending'",
        )
        .bind(triggered_price)
        .bind(triggered_currency)
        .bind(source_event_id)
        .bind(user_id)
        .bind(order_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn cancel_limit_order(&self, user_id: i64, order_id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE limit_orders
             SET status = 'cancelled', updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ? AND status = 'pending'",
        )
        .bind(user_id)
        .bind(order_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_limit_order_dry_run(
        &self,
        user_id: i64,
        order_id: &str,
        dry_run: bool,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE limit_orders
             SET dry_run = ?, execution_mode = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ? AND status = 'pending'",
        )
        .bind(if dry_run { 1 } else { 0 })
        .bind(if dry_run { "dry_run" } else { "live" })
        .bind(user_id)
        .bind(order_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_limit_order_execution_field(
        &self,
        user_id: i64,
        order_id: &str,
        field: &str,
        value: &str,
    ) -> Result<bool> {
        if !["slippage_bps", "tip_sol", "mev_mode", "execution_method"].contains(&field) {
            anyhow::bail!("unsupported limit execution field: {field}");
        }
        let sql = format!(
            "UPDATE limit_orders SET {field} = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND id = ? AND status = 'pending'"
        );
        let result = sqlx::query(&sql)
            .bind(value)
            .bind(user_id)
            .bind(order_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn audit(
        &self,
        user_id: Option<i64>,
        action: &str,
        target_type: Option<&str>,
        target_id: Option<&str>,
        metadata_json: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO audit_logs (user_id, action, target_type, target_id, metadata_json)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(metadata_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn is_known_trade_pool(pool: &str) -> bool {
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

fn alpha_event_id(event: &PumpApiEvent) -> String {
    let signature = event.signature.as_deref().unwrap_or("unsigned");
    let action = event.tx_type.as_deref().unwrap_or("unknown");
    let mint = event.mint.as_deref().unwrap_or("unknown");
    let pool_id = event.pool_id.as_deref().unwrap_or("unknown");
    if event.signature.is_some() {
        return format!("{signature}:{action}:{mint}:{pool_id}");
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(event.raw_json.as_bytes());
    format!("unsigned:{}", hex::encode(digest))
}

fn row_to_strategy(row: sqlx::sqlite::SqliteRow) -> StrategyConfig {
    let allowed_pools = row
        .get::<Option<String>, _>("allowed_pools")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect();

    StrategyConfig {
        id: row.get("id"),
        user_id: row.get("user_id"),
        wallet_id: row.get("wallet_id"),
        watch_wallet_id: row.get("watch_wallet_id"),
        name: row.get("name"),
        enabled: row.get::<i64, _>("enabled") != 0,
        dry_run: row.get::<i64, _>("dry_run") != 0,
        execution_mode: row.get("execution_mode"),
        execution_method: row.get("execution_method"),
        copy_buy: row.get::<i64, _>("copy_buy") != 0,
        copy_sell: row.get::<i64, _>("copy_sell") != 0,
        buy_amount_sol: row.get("buy_amount_sol"),
        buy_mode: row.get("buy_mode"),
        buy_ratio_percent: row.get("buy_ratio_percent"),
        sell_percent: row.get("sell_percent"),
        max_slippage_bps: row.get::<i64, _>("max_slippage_bps") as u64,
        tip_sol: row.get("tip_sol"),
        max_per_mint_sol: row.get("max_per_mint_sol"),
        daily_max_sol: row.get("daily_max_sol"),
        cooldown_secs: row.get::<i64, _>("cooldown_secs") as u64,
        only_once: row.get::<i64, _>("only_once") != 0,
        min_sol_in_pool: row.get("min_sol_in_pool"),
        min_mcap: row.get("min_mcap"),
        max_mcap: row.get("max_mcap"),
        min_target_buy_sol: row.get("min_target_buy_sol"),
        max_target_buy_sol: row.get("max_target_buy_sol"),
        max_signal_delay_ms: row
            .get::<Option<i64>, _>("max_signal_delay_ms")
            .filter(|value| *value > 0),
        allowed_pools,
        mev_mode: row.get("mev_mode"),
        live: row.get::<i64, _>("live") != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::Database;
    use crate::db::models::{AlphaPaperSignal, PumpApiEvent, UserTradeSettings};
    use crate::execution_params::{ExecutionMethod, ExecutionMode, ExecutionParams, LimitAction};
    use sqlx::{Executor, Row};

    #[tokio::test]
    async fn migration_adds_external_wallet_id_before_creating_index() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.pool
            .execute(
                "CREATE TABLE app_users(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   telegram_id INTEGER NOT NULL UNIQUE,
                   username TEXT,
                   created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                   updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 )",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "CREATE TABLE wallets(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   user_id INTEGER NOT NULL REFERENCES app_users(id),
                   label TEXT NOT NULL,
                   public_key TEXT NOT NULL,
                   is_active INTEGER NOT NULL DEFAULT 0,
                   is_deleted INTEGER NOT NULL DEFAULT 0,
                   created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                   updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                   UNIQUE(user_id, public_key)
                 )",
            )
            .await
            .unwrap();

        db.migrate().await.unwrap();

        let columns = sqlx::query("PRAGMA table_info(wallets)")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        assert!(columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "external_wallet_id"));
    }

    #[tokio::test]
    async fn external_wallet_sync_updates_remote_id_and_label() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(789, Some("wallet-sync")).await.unwrap();
        let wallet_id = db
            .insert_external_wallet(user.id, "old-id", "Old label", "pubkey-1")
            .await
            .unwrap();

        db.sync_external_wallet_metadata(user.id, wallet_id, "new-id", "New label")
            .await
            .unwrap();

        let wallet = db.get_wallet(user.id, wallet_id).await.unwrap();
        assert_eq!(wallet.external_wallet_id.as_deref(), Some("new-id"));
        assert_eq!(wallet.label, "New label");
    }

    #[tokio::test]
    async fn alpha_observation_events_are_deduplicated_by_stable_event_id() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let event = PumpApiEvent {
            signature: Some("sig-1".to_string()),
            mint: Some("mint-1".to_string()),
            pool: Some("pump-amm".to_string()),
            pool_id: Some("pool-1".to_string()),
            quote_mint: Some("wsol".to_string()),
            tx_type: Some("buy".to_string()),
            signer: Some("trader-1".to_string()),
            sol_in_pool: Some(1.0),
            tokens_in_pool: Some(2.0),
            quote_in_pool: Some(1.0),
            event_timestamp_ms: Some(100),
            received_unix_ms: 200,
            raw_json: r#"{"signature":"sig-1"}"#.to_string(),
        };
        assert!(db.insert_alpha_observation_event(&event).await.unwrap());
        assert!(!db.insert_alpha_observation_event(&event).await.unwrap());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alpha_observation_events")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let events = db.list_alpha_observation_events(0, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "sig-1:buy:mint-1:pool-1");
        assert_eq!(events[0].route_status, "candidate");
        let signal = AlphaPaperSignal {
            signal_id: "sig-1:alpha-flow-v1".to_string(),
            event_id: events[0].event_id.clone(),
            mint: "mint-1".to_string(),
            pool: Some("pump".to_string()),
            pool_id: None,
            quote_mint: Some("So11111111111111111111111111111111111111112".to_string()),
            side: "buy".to_string(),
            score: 0.8,
            status: "paper".to_string(),
            features_json: "{}".to_string(),
            created_at_ms: 300,
            order_id: None,
            trade_service_request_id: None,
            remote_status: None,
            updated_at_ms: None,
        };
        assert!(db.insert_alpha_paper_signal(&signal).await.unwrap());
        assert!(!db.insert_alpha_paper_signal(&signal).await.unwrap());
        assert!(db
            .update_alpha_paper_signal_link(
                &signal.signal_id,
                Some("order-1"),
                Some("tr_sim-1"),
                Some("simulated"),
                400,
            )
            .await
            .unwrap());
        let signals = db.list_alpha_paper_signals(10).await.unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].order_id.as_deref(), Some("order-1"));
        assert_eq!(
            signals[0].trade_service_request_id.as_deref(),
            Some("tr_sim-1")
        );
        assert_eq!(signals[0].remote_status.as_deref(), Some("simulated"));
    }

    #[tokio::test]
    async fn alpha_shadow_reconciler_includes_unknown_orders_with_request_ids() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let signal = AlphaPaperSignal {
            signal_id: "sig-shadow-1".to_string(),
            event_id: "event-shadow-1".to_string(),
            mint: "mint-1".to_string(),
            pool: Some("pump".to_string()),
            pool_id: None,
            quote_mint: Some("So11111111111111111111111111111111111111112".to_string()),
            side: "buy".to_string(),
            score: 0.99,
            status: "paper".to_string(),
            features_json: "{}".to_string(),
            created_at_ms: 1,
            order_id: None,
            trade_service_request_id: None,
            remote_status: None,
            updated_at_ms: None,
        };
        let (order_id, created) = db
            .reserve_alpha_shadow_order(
                &signal,
                "alpha-shadow:sig-shadow-1",
                "{}",
                Some(0.01),
                None,
                1,
            )
            .await
            .unwrap();
        assert!(created);
        db.update_alpha_shadow_order(
            &order_id,
            "unknown",
            Some("tr_unknown-1"),
            Some("{\"status\":\"unknown\"}"),
            Some("temporary remote state"),
            2,
        )
        .await
        .unwrap();
        let orders = db.list_alpha_shadow_reconcilable(10).await.unwrap();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].status, "unknown");
        assert_eq!(
            orders[0].trade_service_request_id.as_deref(),
            Some("tr_unknown-1")
        );
    }

    #[tokio::test]
    async fn latest_observed_pool_route_uses_pumpapi_pool_id() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        db.upsert_pool_route("mint-1", "pump-amm", Some("pool-1"), "quote-1", "pumpapi")
            .await
            .unwrap();

        let route = db
            .latest_observed_pool_route("mint-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(route.pool, "pump-amm");
        assert_eq!(route.pool_id.as_deref(), Some("pool-1"));
        assert_eq!(route.quote_mint, "quote-1");
    }

    #[tokio::test]
    async fn pool_route_candidates_preserve_multiple_pools_for_one_mint() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        db.upsert_pool_route(
            "mint-1",
            "pump-amm",
            Some("custom-pool"),
            "custom-quote",
            "pumpapi",
        )
        .await
        .unwrap();
        db.upsert_pool_route(
            "mint-1",
            "meteora-damm-v2",
            Some("wsol-pool"),
            "So11111111111111111111111111111111111111112",
            "pumpapi",
        )
        .await
        .unwrap();

        let routes = db.observed_pool_route_candidates("mint-1").await.unwrap();
        assert_eq!(routes.len(), 2);
        assert!(routes.iter().any(|route| route.pool == "pump-amm"));
        assert!(routes.iter().any(|route| {
            route.pool == "meteora-damm-v2"
                && route.quote_mint == "So11111111111111111111111111111111111111112"
        }));
    }

    #[tokio::test]
    async fn insert_or_get_pumpapi_event_returns_the_original_id() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let event = PumpApiEvent {
            signature: Some("duplicate-signature".to_string()),
            mint: Some("mint-1".to_string()),
            pool: Some("pump".to_string()),
            pool_id: None,
            quote_mint: None,
            tx_type: Some("buy".to_string()),
            signer: Some("signer-1".to_string()),
            sol_in_pool: None,
            tokens_in_pool: None,
            quote_in_pool: None,
            event_timestamp_ms: None,
            received_unix_ms: 1,
            raw_json: "{}".to_string(),
        };
        let first = db.insert_or_get_pumpapi_event(&event).await.unwrap();
        let second = db.insert_or_get_pumpapi_event(&event).await.unwrap();
        assert!(first > 0);
        assert_eq!(second, first);
    }

    #[tokio::test]
    async fn limit_order_persists_metric_execution_and_expiry() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(123, Some("test")).await.unwrap();
        let execution = ExecutionParams {
            wallet_id: None,
            mode: ExecutionMode::DryRun,
            method: ExecutionMethod::Sdk,
            slippage_bps: 500,
            tip_sol: 0.0002,
            mev_mode: "temporal".to_string(),
        };
        let id = db
            .insert_limit_order(
                user.id,
                None,
                "So11111111111111111111111111111111111111112",
                LimitAction::BuyAbove,
                "market_cap_usd",
                50_000.0,
                Some(0.01),
                None,
                &execution,
                Some("2099-01-01 00:00:00"),
            )
            .await
            .unwrap();
        let order = db.get_limit_order(user.id, &id).await.unwrap().unwrap();
        assert_eq!(order.trigger_metric, "market_cap_usd");
        assert_eq!(order.slippage_bps, 500);
        assert_eq!(order.mev_mode, "temporal");
        assert_eq!(order.expires_at.as_deref(), Some("2099-01-01 00:00:00"));
    }

    #[tokio::test]
    async fn user_trade_settings_are_seeded_and_persisted() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(456, Some("settings")).await.unwrap();
        let defaults = UserTradeSettings {
            user_id: user.id,
            use_mev: false,
            default_mev_provider: "auto".to_string(),
            default_execution_mode: "dry_run".to_string(),
            default_execution_method: "sdk".to_string(),
            default_slippage_bps: 1500,
            default_tip_sol: 0.0001,
            default_buy_preset_slot: 2,
            default_sell_preset_slot: 4,
            quick_trade_confirmation_enabled: true,
            buy_presets_sol: [0.001, 0.005, 0.01, 0.05, 0.1],
            sell_presets_percent: [10.0, 25.0, 50.0, 75.0, 100.0],
        };
        let seeded = db
            .get_or_create_user_trade_settings(&defaults)
            .await
            .unwrap();
        assert_eq!(seeded.default_buy_preset_slot, 2);
        assert_eq!(seeded.buy_presets_sol[2], 0.01);
        assert_eq!(seeded.default_execution_mode, "dry_run");
        assert!(seeded.quick_trade_confirmation_enabled);

        db.update_user_trade_setting(user.id, "use_mev", "1")
            .await
            .unwrap();
        db.update_user_trade_setting(user.id, "default_buy_preset_slot", "4")
            .await
            .unwrap();
        db.update_user_trade_setting(user.id, "default_execution_mode", "live")
            .await
            .unwrap();
        db.update_user_trade_setting(user.id, "quick_trade_confirmation_enabled", "0")
            .await
            .unwrap();
        db.update_user_trade_preset(user.id, "buy", 4, 0.2)
            .await
            .unwrap();
        let updated = db.get_user_trade_settings(user.id).await.unwrap();
        assert!(updated.use_mev);
        assert_eq!(updated.default_execution_mode, "live");
        assert!(!updated.quick_trade_confirmation_enabled);
        assert_eq!(updated.default_buy_preset_slot, 3);
        assert_eq!(updated.buy_presets_sol[3], 0.2);
    }
}
