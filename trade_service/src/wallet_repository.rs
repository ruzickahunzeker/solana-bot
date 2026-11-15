use anyhow::{bail, Context};
use solana_sdk::signature::{Keypair, Signer};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

use crate::{
    adapters::persistence::sol_transfer_journal::SqliteSolTransferJournal,
    config::{WalletConfig, WalletStorageConfig, WalletStorageMode},
    wallet_crypto,
};

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS wallets (
  wallet_id TEXT PRIMARY KEY,
  owner_type TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  label TEXT NOT NULL,
  pubkey TEXT NOT NULL UNIQUE,
  encrypted_private_key TEXT NOT NULL,
  is_default INTEGER NOT NULL DEFAULT 0,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_wallets_owner ON wallets(owner_type, owner_id, enabled);
CREATE TABLE IF NOT EXISTS wallet_audit_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  wallet_id TEXT,
  action TEXT NOT NULL,
  owner_type TEXT,
  owner_id TEXT,
  metadata_json TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS wallet_generation_jobs (
  job_id TEXT PRIMARY KEY,
  owner_type TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  label TEXT NOT NULL,
  match_side TEXT NOT NULL,
  pattern TEXT NOT NULL,
  status TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  max_attempts INTEGER NOT NULL,
  wallet_id TEXT,
  pubkey TEXT,
  error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_wallet_generation_owner
  ON wallet_generation_jobs(owner_type, owner_id, status);
CREATE TABLE IF NOT EXISTS wallet_sol_transfers (
  transfer_id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  owner_type TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  wallet_id TEXT NOT NULL,
  from_pubkey TEXT NOT NULL,
  to_pubkey TEXT NOT NULL,
  amount_mode TEXT NOT NULL,
  requested_amount_lamports INTEGER,
  requested_percent INTEGER,
  actual_amount_lamports INTEGER,
  fee_lamports INTEGER,
  balance_before_lamports INTEGER,
  status TEXT NOT NULL,
  signature TEXT,
  error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_wallet_sol_transfers_owner
  ON wallet_sol_transfers(owner_type, owner_id, created_at);
"#;

pub use crate::domain::wallet::ManagedWallet;

pub use crate::domain::wallet::VanityJob;

#[cfg(test)]
use crate::adapters::persistence::sol_transfer_codec;
#[cfg(test)]
pub use crate::application::sol_transfer_record::SolTransferRecordV1 as SolTransferRecord;

#[derive(Clone)]
pub struct WalletRepository {
    mode: WalletStorageMode,
    pool: Option<SqlitePool>,
    passphrase: Option<String>,
}

impl WalletRepository {
    #[cfg(test)]
    pub(crate) fn pool_for_transfer_tests(&self) -> SqlitePool {
        self.pool.as_ref().expect("test database required").clone()
    }

    #[cfg(test)]
    pub(crate) async fn empty_for_transfer_tests() -> anyhow::Result<(Self, SqlitePool)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::from_str("sqlite::memory:")?
                    .synchronous(SqliteSynchronous::Full)
                    .foreign_keys(true),
            )
            .await?;
        sqlx::raw_sql(MIGRATION).execute(&pool).await?;
        SqliteSolTransferJournal::new(pool.clone())
            .initialize()
            .await?;
        Ok((
            Self {
                mode: WalletStorageMode::Database,
                pool: Some(pool.clone()),
                passphrase: None,
            },
            pool,
        ))
    }

    pub async fn new(
        config: &WalletStorageConfig,
        bootstrap_wallets: &[WalletConfig],
        default_wallet_id: &str,
    ) -> anyhow::Result<Self> {
        if config.mode == WalletStorageMode::File {
            return Ok(Self {
                mode: config.mode,
                pool: None,
                passphrase: None,
            });
        }

        if let Some(path) = config.database_url.strip_prefix("sqlite://") {
            if let Some(parent) = std::path::Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
        }
        let options = SqliteConnectOptions::from_str(&config.database_url)
            .with_context(|| format!("invalid WALLET_DATABASE_URL {}", config.database_url))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(config.busy_timeout_ms));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("connect wallet database")?;
        sqlx::raw_sql(MIGRATION)
            .execute(&pool)
            .await
            .context("migrate wallet database")?;

        let repository = Self {
            mode: config.mode,
            pool: Some(pool),
            passphrase: config.passphrase.clone(),
        };
        repository
            .sol_transfer_journal()
            .context("wallet transfer journal is unavailable")?
            .initialize()
            .await
            .context("initialize SOL transfer journal")?;
        repository
            .bootstrap(bootstrap_wallets, default_wallet_id)
            .await?;
        Ok(repository)
    }

    pub fn database_enabled(&self) -> bool {
        self.mode == WalletStorageMode::Database
    }

    pub(crate) fn sol_transfer_journal(&self) -> Option<SqliteSolTransferJournal> {
        self.pool.clone().map(SqliteSolTransferJournal::new)
    }

    pub async fn load_wallet_configs(
        &self,
        fallback: &[WalletConfig],
    ) -> anyhow::Result<Vec<WalletConfig>> {
        let Some(pool) = &self.pool else {
            return Ok(fallback.to_vec());
        };
        let rows = sqlx::query(
            "SELECT wallet_id, pubkey, encrypted_private_key FROM wallets WHERE enabled=1 ORDER BY is_default DESC, created_at ASC",
        )
        .fetch_all(pool)
        .await?;
        let mut wallets = Vec::with_capacity(rows.len());
        for row in rows {
            let wallet_id: String = row.get("wallet_id");
            let expected_pubkey: String = row.get("pubkey");
            let encrypted: String = row.get("encrypted_private_key");
            let private_key_base58 = self.decrypt_secret(&encrypted)?;
            let actual_pubkey = parse_keypair(&private_key_base58)?.pubkey().to_string();
            if actual_pubkey != expected_pubkey {
                bail!("wallet {wallet_id} encrypted secret does not match stored pubkey");
            }
            wallets.push(WalletConfig {
                wallet_id,
                private_key_base58,
            });
        }
        if wallets.is_empty() {
            bail!("wallet database contains no enabled wallets");
        }
        Ok(wallets)
    }

    pub async fn list(&self) -> anyhow::Result<Vec<ManagedWallet>> {
        let pool = self.database_pool()?;
        let rows = sqlx::query(
            "SELECT wallet_id,owner_type,owner_id,label,pubkey,is_default,enabled FROM wallets ORDER BY is_default DESC,created_at ASC",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(row_to_wallet).collect())
    }

    pub async fn create(
        &self,
        owner_type: &str,
        owner_id: &str,
        label: &str,
    ) -> anyhow::Result<(ManagedWallet, String)> {
        let keypair = Keypair::new();
        let private_key_base58 = bs58::encode(keypair.to_bytes()).into_string();
        let wallet = self
            .insert(
                owner_type,
                owner_id,
                label,
                &private_key_base58,
                None,
                false,
            )
            .await?;
        Ok((wallet, private_key_base58))
    }

    pub async fn import(
        &self,
        owner_type: &str,
        owner_id: &str,
        label: &str,
        private_key_base58: &str,
    ) -> anyhow::Result<(ManagedWallet, bool)> {
        let keypair = parse_keypair(private_key_base58)?;
        let pubkey = keypair.pubkey().to_string();
        if let Some(existing) = self.find_by_pubkey(&pubkey).await? {
            if existing.owner_type != owner_type || existing.owner_id != owner_id {
                bail!("wallet pubkey is already owned by another principal");
            }
            if !existing.enabled {
                let pool = self.database_pool()?;
                sqlx::query(
                    "UPDATE wallets SET enabled=1,label=?,updated_at=CURRENT_TIMESTAMP WHERE wallet_id=?",
                )
                .bind(label)
                .bind(&existing.wallet_id)
                .execute(pool)
                .await?;
                self.audit(
                    Some(&existing.wallet_id),
                    "wallet.restore",
                    Some(owner_type),
                    Some(owner_id),
                    None,
                )
                .await?;
                return Ok((self.get(&existing.wallet_id).await?, true));
            }
            return Ok((existing, false));
        }
        let wallet = self
            .insert(owner_type, owner_id, label, private_key_base58, None, false)
            .await?;
        Ok((wallet, true))
    }

    pub async fn export(&self, wallet_id: &str) -> anyhow::Result<String> {
        let pool = self.database_pool()?;
        let row = sqlx::query(
            "SELECT encrypted_private_key,owner_type,owner_id FROM wallets WHERE wallet_id=? AND enabled=1",
        )
        .bind(wallet_id)
        .fetch_one(pool)
        .await
        .context("wallet is unavailable")?;
        let encrypted: String = row.get("encrypted_private_key");
        self.audit(
            Some(wallet_id),
            "wallet.export",
            Some(row.get("owner_type")),
            Some(row.get("owner_id")),
            None,
        )
        .await?;
        self.decrypt_secret(&encrypted)
    }

    pub async fn rename(&self, wallet_id: &str, label: &str) -> anyhow::Result<ManagedWallet> {
        let pool = self.database_pool()?;
        let result = sqlx::query(
            "UPDATE wallets SET label=?,updated_at=CURRENT_TIMESTAMP WHERE wallet_id=? AND enabled=1",
        )
        .bind(label)
        .bind(wallet_id)
        .execute(pool)
        .await?;
        if result.rows_affected() != 1 {
            bail!("wallet is unavailable");
        }
        self.get(wallet_id).await
    }

    pub async fn disable(&self, wallet_id: &str) -> anyhow::Result<ManagedWallet> {
        let wallet = self.get(wallet_id).await?;
        if wallet.is_default {
            bail!("default wallet cannot be deleted");
        }
        let pool = self.database_pool()?;
        sqlx::query("UPDATE wallets SET enabled=0,updated_at=CURRENT_TIMESTAMP WHERE wallet_id=?")
            .bind(wallet_id)
            .execute(pool)
            .await?;
        self.audit(
            Some(wallet_id),
            "wallet.disable",
            Some(&wallet.owner_type),
            Some(&wallet.owner_id),
            None,
        )
        .await?;
        self.get(wallet_id).await
    }

    pub async fn get(&self, wallet_id: &str) -> anyhow::Result<ManagedWallet> {
        let pool = self.database_pool()?;
        let row = sqlx::query(
            "SELECT wallet_id,owner_type,owner_id,label,pubkey,is_default,enabled FROM wallets WHERE wallet_id=?",
        )
        .bind(wallet_id)
        .fetch_one(pool)
        .await
        .context("wallet not found")?;
        Ok(row_to_wallet(row))
    }

    pub(crate) async fn find(&self, wallet_id: &str) -> anyhow::Result<Option<ManagedWallet>> {
        let row = sqlx::query(
            "SELECT wallet_id,owner_type,owner_id,label,pubkey,is_default,enabled FROM wallets WHERE wallet_id=?",
        )
        .bind(wallet_id)
        .fetch_optional(self.database_pool()?)
        .await?;
        row.as_ref()
            .map(|row| -> anyhow::Result<ManagedWallet> {
                Ok(ManagedWallet {
                    wallet_id: row.try_get("wallet_id")?,
                    owner_type: row.try_get("owner_type")?,
                    owner_id: row.try_get("owner_id")?,
                    label: row.try_get("label")?,
                    pubkey: row.try_get("pubkey")?,
                    is_default: row.try_get::<i64, _>("is_default")? != 0,
                    enabled: row.try_get::<i64, _>("enabled")? != 0,
                })
            })
            .transpose()
    }

    pub async fn create_vanity_job(
        &self,
        owner_type: &str,
        owner_id: &str,
        label: &str,
        match_side: &str,
        pattern: &str,
        max_attempts: u64,
    ) -> anyhow::Result<VanityJob> {
        validate_identity(owner_type, owner_id, label)?;
        let pool = self.database_pool()?;
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM wallet_generation_jobs WHERE owner_type=? AND owner_id=? AND status IN ('queued','running','cancel_requested')",
        )
        .bind(owner_type)
        .bind(owner_id)
        .fetch_one(pool)
        .await?;
        if active > 0 {
            bail!("this owner already has an active wallet generation job");
        }
        let job_id = format!("vanity_{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO wallet_generation_jobs(job_id,owner_type,owner_id,label,match_side,pattern,status,max_attempts) VALUES(?,?,?,?,?,?,'queued',?)",
        )
        .bind(&job_id)
        .bind(owner_type)
        .bind(owner_id)
        .bind(label)
        .bind(match_side)
        .bind(pattern)
        .bind(max_attempts as i64)
        .execute(pool)
        .await?;
        self.get_vanity_job(&job_id).await
    }

    pub async fn get_vanity_job(&self, job_id: &str) -> anyhow::Result<VanityJob> {
        let row = sqlx::query(
            "SELECT job_id,owner_type,owner_id,label,match_side,pattern,status,attempts,max_attempts,wallet_id,pubkey,error FROM wallet_generation_jobs WHERE job_id=?",
        )
        .bind(job_id)
        .fetch_one(self.database_pool()?)
        .await
        .context("wallet generation job not found")?;
        Ok(row_to_vanity_job(row))
    }

    pub async fn update_vanity_job(
        &self,
        job_id: &str,
        status: &str,
        attempts: u64,
        wallet_id: Option<&str>,
        pubkey: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE wallet_generation_jobs SET status=?,attempts=?,wallet_id=?,pubkey=?,error=?,updated_at=CURRENT_TIMESTAMP WHERE job_id=?",
        )
        .bind(status)
        .bind(attempts as i64)
        .bind(wallet_id)
        .bind(pubkey)
        .bind(error)
        .bind(job_id)
        .execute(self.database_pool()?)
        .await?;
        Ok(())
    }

    pub async fn interrupt_active_vanity_jobs(&self) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE wallet_generation_jobs SET status='interrupted',error='service restarted',updated_at=CURRENT_TIMESTAMP WHERE status IN ('queued','running')",
        )
        .execute(self.database_pool()?)
        .await?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn reserve_sol_transfer(&self, transfer: &SolTransferRecord) -> anyhow::Result<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO wallet_sol_transfers(transfer_id,idempotency_key,owner_type,owner_id,wallet_id,from_pubkey,to_pubkey,amount_mode,requested_amount_lamports,requested_percent,status) VALUES(?,?,?,?,?,?,?,?,?,?,'processing')",
        )
        .bind(&transfer.transfer_id)
        .bind(&transfer.idempotency_key)
        .bind(&transfer.owner_type)
        .bind(&transfer.owner_id)
        .bind(&transfer.wallet_id)
        .bind(&transfer.from_pubkey)
        .bind(&transfer.to_pubkey)
        .bind(&transfer.amount_mode)
        .bind(transfer.requested_amount_lamports.map(|v| v as i64))
        .bind(transfer.requested_percent.map(|v| v as i64))
        .execute(self.database_pool()?)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    #[cfg(test)]
    pub async fn finish_sol_transfer(
        &self,
        transfer_id: &str,
        status: &str,
        actual_amount_lamports: Option<u64>,
        fee_lamports: Option<u64>,
        balance_before_lamports: Option<u64>,
        signature: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE wallet_sol_transfers SET status=?,actual_amount_lamports=?,fee_lamports=?,balance_before_lamports=?,signature=?,error=?,updated_at=CURRENT_TIMESTAMP WHERE transfer_id=?",
        )
        .bind(status)
        .bind(actual_amount_lamports.map(|v| v as i64))
        .bind(fee_lamports.map(|v| v as i64))
        .bind(balance_before_lamports.map(|v| v as i64))
        .bind(signature)
        .bind(error)
        .bind(transfer_id)
        .execute(self.database_pool()?)
        .await?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn get_sol_transfer_by_idempotency(
        &self,
        key: &str,
    ) -> anyhow::Result<SolTransferRecord> {
        let row = sqlx::query(
            "SELECT transfer_id,idempotency_key,owner_type,owner_id,wallet_id,from_pubkey,to_pubkey,amount_mode,requested_amount_lamports,requested_percent,actual_amount_lamports,fee_lamports,balance_before_lamports,status,signature,error FROM wallet_sol_transfers WHERE idempotency_key=?",
        )
        .bind(key)
        .fetch_one(self.database_pool()?)
        .await
        .context("SOL transfer not found")?;
        sol_transfer_codec::decode_projection(&row)
    }

    async fn bootstrap(
        &self,
        wallets: &[WalletConfig],
        default_wallet_id: &str,
    ) -> anyhow::Result<()> {
        let pool = self.database_pool()?;
        for wallet in wallets {
            let configured_pubkey = parse_keypair(&wallet.private_key_base58)?
                .pubkey()
                .to_string();
            let existing = sqlx::query("SELECT pubkey FROM wallets WHERE wallet_id=?")
                .bind(&wallet.wallet_id)
                .fetch_optional(pool)
                .await?;
            if let Some(existing) = existing {
                let stored_pubkey: String = existing.get("pubkey");
                if stored_pubkey != configured_pubkey {
                    bail!(
                        "wallet database entry {} does not match configured file wallet",
                        wallet.wallet_id
                    );
                }
                continue;
            }
            self.insert(
                "system",
                "global",
                &wallet.wallet_id,
                &wallet.private_key_base58,
                Some(&wallet.wallet_id),
                wallet.wallet_id == default_wallet_id,
            )
            .await?;
        }
        Ok(())
    }

    async fn insert(
        &self,
        owner_type: &str,
        owner_id: &str,
        label: &str,
        private_key_base58: &str,
        requested_wallet_id: Option<&str>,
        is_default: bool,
    ) -> anyhow::Result<ManagedWallet> {
        validate_identity(owner_type, owner_id, label)?;
        let keypair = parse_keypair(private_key_base58)?;
        let pubkey = keypair.pubkey().to_string();
        let wallet_id = requested_wallet_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("wallet_{}", Uuid::new_v4().simple()));
        let encrypted = self.encrypt_secret(private_key_base58)?;
        let pool = self.database_pool()?;
        sqlx::query(
            "INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,is_default) VALUES(?,?,?,?,?,?,?)",
        )
        .bind(&wallet_id)
        .bind(owner_type)
        .bind(owner_id)
        .bind(label)
        .bind(&pubkey)
        .bind(encrypted)
        .bind(i64::from(is_default))
        .execute(pool)
        .await
        .with_context(|| format!("insert wallet {wallet_id}"))?;
        self.audit(
            Some(&wallet_id),
            "wallet.create",
            Some(owner_type),
            Some(owner_id),
            None,
        )
        .await?;
        self.get(&wallet_id).await
    }

    async fn find_by_pubkey(&self, pubkey: &str) -> anyhow::Result<Option<ManagedWallet>> {
        let pool = self.database_pool()?;
        let row = sqlx::query(
            "SELECT wallet_id,owner_type,owner_id,label,pubkey,is_default,enabled FROM wallets WHERE pubkey=?",
        )
        .bind(pubkey)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(row_to_wallet))
    }

    async fn audit(
        &self,
        wallet_id: Option<&str>,
        action: &str,
        owner_type: Option<&str>,
        owner_id: Option<&str>,
        metadata_json: Option<&str>,
    ) -> anyhow::Result<()> {
        let pool = self.database_pool()?;
        sqlx::query(
            "INSERT INTO wallet_audit_logs(wallet_id,action,owner_type,owner_id,metadata_json) VALUES(?,?,?,?,?)",
        )
        .bind(wallet_id)
        .bind(action)
        .bind(owner_type)
        .bind(owner_id)
        .bind(metadata_json)
        .execute(pool)
        .await?;
        Ok(())
    }

    fn database_pool(&self) -> anyhow::Result<&SqlitePool> {
        self.pool
            .as_ref()
            .context("wallet database mode is not enabled")
    }

    fn encrypt_secret(&self, private_key_base58: &str) -> anyhow::Result<String> {
        let passphrase = self
            .passphrase
            .as_deref()
            .context("wallet database passphrase is unavailable")?;
        wallet_crypto::encrypt_json(private_key_base58.as_bytes(), passphrase)
    }

    fn decrypt_secret(&self, encrypted: &str) -> anyhow::Result<String> {
        let passphrase = self
            .passphrase
            .as_deref()
            .context("wallet database passphrase is unavailable")?;
        let bytes = wallet_crypto::decrypt_json(encrypted, passphrase)?;
        String::from_utf8(bytes).context("wallet secret is not UTF-8")
    }
}

fn parse_keypair(private_key_base58: &str) -> anyhow::Result<Keypair> {
    Keypair::try_from_base58_string(private_key_base58.trim())
        .context("private key must be a valid base58 Solana keypair")
}

fn validate_identity(owner_type: &str, owner_id: &str, label: &str) -> anyhow::Result<()> {
    if owner_type.trim().is_empty() || owner_type.len() > 32 {
        bail!("owner_type must be 1..32 characters");
    }
    if owner_id.trim().is_empty() || owner_id.len() > 128 {
        bail!("owner_id must be 1..128 characters");
    }
    if label.trim().is_empty() || label.len() > 80 {
        bail!("label must be 1..80 characters");
    }
    Ok(())
}

fn row_to_wallet(row: sqlx::sqlite::SqliteRow) -> ManagedWallet {
    ManagedWallet {
        wallet_id: row.get("wallet_id"),
        owner_type: row.get("owner_type"),
        owner_id: row.get("owner_id"),
        label: row.get("label"),
        pubkey: row.get("pubkey"),
        is_default: row.get::<i64, _>("is_default") != 0,
        enabled: row.get::<i64, _>("enabled") != 0,
    }
}

fn row_to_vanity_job(row: sqlx::sqlite::SqliteRow) -> VanityJob {
    VanityJob {
        job_id: row.get("job_id"),
        owner_type: row.get("owner_type"),
        owner_id: row.get("owner_id"),
        label: row.get("label"),
        match_side: row.get("match_side"),
        pattern: row.get("pattern"),
        status: row.get("status"),
        attempts: row.get::<i64, _>("attempts") as u64,
        max_attempts: row.get::<i64, _>("max_attempts") as u64,
        wallet_id: row.get("wallet_id"),
        pubkey: row.get("pubkey"),
        error: row.get("error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn database_bootstraps_file_wallets_and_manages_runtime_wallets() {
        let path = std::env::temp_dir().join(format!("wallet-test-{}.sqlite", Uuid::new_v4()));
        let database_url = format!("sqlite://{}", path.display());
        let main_keypair = Keypair::new();
        let main_secret = bs58::encode(main_keypair.to_bytes()).into_string();
        let config = WalletStorageConfig {
            mode: WalletStorageMode::Database,
            database_url,
            busy_timeout_ms: 1_000,
            passphrase: Some("test-wallet-passphrase".to_string()),
        };
        let bootstrap = vec![WalletConfig {
            wallet_id: "main".to_string(),
            private_key_base58: main_secret.clone(),
        }];

        let repository = WalletRepository::new(&config, &bootstrap, "main")
            .await
            .expect("repository");
        let loaded = repository
            .load_wallet_configs(&[])
            .await
            .expect("load wallets");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].wallet_id, "main");
        assert_eq!(loaded[0].private_key_base58, main_secret);

        let (created, created_secret) = repository
            .create("telegram", "42", "TG Wallet")
            .await
            .expect("create wallet");
        assert_eq!(created.owner_type, "telegram");
        assert_eq!(
            repository.export(&created.wallet_id).await.expect("export"),
            created_secret
        );
        assert!(repository.disable("main").await.is_err());
        assert!(
            !repository
                .disable(&created.wallet_id)
                .await
                .expect("disable")
                .enabled
        );
        let (restored, restored_now) = repository
            .import("telegram", "42", "Restored", &created_secret)
            .await
            .expect("restore");
        assert!(restored_now);
        assert!(restored.enabled);
        assert_eq!(restored.wallet_id, created.wallet_id);

        let job = repository
            .create_vanity_job("telegram", "42", "Vanity", "prefix", "9999", 10_000)
            .await
            .expect("create vanity job");
        repository
            .update_vanity_job(&job.job_id, "cancelled", 123, None, None, None)
            .await
            .expect("cancel vanity job");
        let job = repository
            .get_vanity_job(&job.job_id)
            .await
            .expect("get vanity job");
        assert_eq!(job.status, "cancelled");
        assert_eq!(job.attempts, 123);

        let transfer = SolTransferRecord {
            transfer_id: "transfer_test".to_string(),
            idempotency_key: "test-key".to_string(),
            owner_type: "telegram".to_string(),
            owner_id: "42".to_string(),
            wallet_id: created.wallet_id.clone(),
            from_pubkey: created.pubkey.clone(),
            to_pubkey: Keypair::new().pubkey().to_string(),
            amount_mode: "percent".to_string(),
            requested_amount_lamports: None,
            requested_percent: Some(50),
            actual_amount_lamports: None,
            fee_lamports: None,
            balance_before_lamports: None,
            status: "processing".to_string(),
            signature: None,
            error: None,
        };
        assert!(repository
            .reserve_sol_transfer(&transfer)
            .await
            .expect("reserve transfer"));
        assert!(!repository
            .reserve_sol_transfer(&transfer)
            .await
            .expect("reserve duplicate transfer"));
        repository
            .finish_sol_transfer(
                &transfer.transfer_id,
                "submitted",
                Some(1_000),
                Some(5_000),
                Some(20_000),
                Some("signature"),
                None,
            )
            .await
            .expect("finish transfer");
        let stored = repository
            .get_sol_transfer_by_idempotency(&transfer.idempotency_key)
            .await
            .expect("get transfer");
        assert_eq!(stored.status, "submitted");
        assert_eq!(stored.actual_amount_lamports, Some(1_000));

        drop(repository);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}
