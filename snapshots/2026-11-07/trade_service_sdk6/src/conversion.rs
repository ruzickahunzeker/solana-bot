use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::{
    instruction::Instruction,
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account_interface::{
    address::get_associated_token_address_with_program_id,
    instruction::create_associated_token_account_idempotent,
};
use spl_token_interface::instruction as token_instruction;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct WsolQuoteRequest {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub direction: WsolDirection,
    pub amount_lamports: Option<u64>,
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Serialize)]
pub struct WsolQuotePreview {
    pub quote: WsolQuote,
    pub executable: bool,
}

pub struct PreparedWsolTransaction {
    pub transaction: Transaction,
    pub signed_bytes: Vec<u8>,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct WsolSimulationResult {
    pub status: &'static str,
    pub quote_id: String,
    pub network_fee_lamports: Option<u64>,
    pub units_consumed: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct WsolSubmissionResult {
    pub submission_id: String,
    pub quote_id: String,
    pub wallet_id: String,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
}

impl WsolSubmissionResult {
    fn from_record(record: ConversionRecord, duplicate: bool) -> Self {
        Self {
            submission_id: record.submission_id,
            quote_id: record.quote_id,
            wallet_id: record.wallet_id,
            status: record.status,
            signature: record.signature,
            error: record.error,
            duplicate,
        }
    }
}

pub async fn submit_wsol(
    state: &AppState,
    submission: WsolSubmission,
) -> Result<WsolSubmissionResult> {
    if submission.simulate || !submission.confirmed || !state.config.conversion_live_enabled {
        bail!("live WSOL conversion is disabled or request is simulation");
    }
    let store = state
        .conversions
        .as_ref()
        .context("conversion quotes are disabled")?;
    let quote = store
        .get_quote(&submission.quote_id)
        .await?
        .context("conversion quote not found")?;
    let now_ms = current_time_ms()?;
    let (record, inserted) = store.reserve_live(&submission, &quote, now_ms).await?;
    if !inserted {
        return Ok(WsolSubmissionResult::from_record(record, true));
    }
    let prepared = match prepare_wsol_transaction(state, &quote).await {
        Ok(prepared) => prepared,
        Err(error) => {
            store
                .mark_failed_before_sign(
                    &record.submission_id,
                    &crate::sanitize::redact_urls(&error.to_string()),
                    current_time_ms()?,
                )
                .await?;
            let failed = store
                .get_by_id(&record.submission_id)
                .await?
                .context("failed conversion record missing")?;
            return Ok(WsolSubmissionResult::from_record(failed, false));
        }
    };
    let client = state
        .wallets
        .get(&quote.wallet_id)
        .context("wallet is not loaded")?;
    let rpc = client
        .rpcs()
        .into_iter()
        .next()
        .context("wallet has no RPC")?;
    let simulation = tokio::time::timeout(
        Duration::from_secs(8),
        rpc.simulate_transaction(&prepared.transaction),
    )
    .await;
    match simulation {
        Ok(Ok(result)) if result.value.err.is_none() => {}
        _ => {
            store
                .mark_failed_before_sign(
                    &record.submission_id,
                    "pre-submit simulation failed or timed out",
                    current_time_ms()?,
                )
                .await?;
            let failed = store
                .get_by_id(&record.submission_id)
                .await?
                .context("failed conversion record missing")?;
            return Ok(WsolSubmissionResult::from_record(failed, false));
        }
    }
    store
        .record_signed(
            &record.submission_id,
            &prepared.signature,
            &prepared.signed_bytes,
            current_time_ms()?,
        )
        .await?;
    let sent = tokio::time::timeout(
        Duration::from_secs(12),
        rpc.send_transaction(&prepared.transaction),
    )
    .await;
    match sent {
        Ok(Ok(signature)) if signature.to_string() == prepared.signature => {
            store
                .mark_broadcast_result(&record.submission_id, "submitted", None, current_time_ms()?)
                .await?;
        }
        Ok(Ok(_)) => {
            store
                .mark_broadcast_result(
                    &record.submission_id,
                    "unknown",
                    Some("RPC returned a different signature; reconcile original signature"),
                    current_time_ms()?,
                )
                .await?;
        }
        Ok(Err(error)) => {
            store
                .mark_broadcast_result(
                    &record.submission_id,
                    "unknown",
                    Some(&format!(
                        "broadcast result uncertain: {}",
                        crate::sanitize::redact_urls(&error.to_string())
                    )),
                    current_time_ms()?,
                )
                .await?;
        }
        Err(_) => {
            store
                .mark_broadcast_result(
                    &record.submission_id,
                    "unknown",
                    Some("broadcast timed out; reconcile original signature"),
                    current_time_ms()?,
                )
                .await?;
        }
    }
    let updated = store
        .get_by_key(&submission.idempotency_key)
        .await?
        .context("conversion record missing after submission")?;
    Ok(WsolSubmissionResult::from_record(updated, false))
}

pub async fn get_wsol_status(
    state: &AppState,
    submission_id: &str,
    owner_type: &str,
    owner_id: &str,
) -> Result<WsolSubmissionResult> {
    let store = state
        .conversions
        .as_ref()
        .context("conversion quotes are disabled")?;
    let record = store
        .get_by_id(submission_id)
        .await?
        .context("conversion submission not found")?;
    if record.owner_type != owner_type || record.owner_id != owner_id {
        bail!("conversion submission does not belong to this owner");
    }
    if matches!(record.status.as_str(), "signed" | "submitted" | "unknown") {
        if let Some(signature) = record.signature.as_ref() {
            let signature = signature.parse::<solana_sdk::signature::Signature>()?;
            let rpc = state
                .wallets
                .get(&record.wallet_id)
                .context("wallet is not loaded")?
                .rpcs()
                .into_iter()
                .next()
                .context("trade service has no RPC")?;
            if let Ok(response) = rpc.get_signature_statuses_with_history(&[signature]).await {
                if let Some(Some(status)) = response.value.first() {
                    let next = if status.err.is_some() {
                        "failed"
                    } else if status.confirmations.is_none() {
                        "confirmed"
                    } else {
                        "submitted"
                    };
                    store
                        .mark_broadcast_result(
                            &record.submission_id,
                            next,
                            status.err.as_ref().map(|err| err.to_string()).as_deref(),
                            current_time_ms()?,
                        )
                        .await?;
                }
            }
        }
    }
    let current = store
        .get_by_id(submission_id)
        .await?
        .context("conversion record missing")?;
    Ok(WsolSubmissionResult::from_record(current, false))
}

fn current_time_ms() -> Result<u64> {
    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

pub async fn simulate_wsol(
    state: &AppState,
    submission: WsolSubmission,
) -> Result<WsolSimulationResult> {
    if !submission.simulate {
        bail!("simulation endpoint requires simulate=true");
    }
    let store = state
        .conversions
        .as_ref()
        .context("conversion quotes are disabled")?;
    let quote = store
        .get_quote(&submission.quote_id)
        .await?
        .context("conversion quote not found")?;
    submission.fingerprint(&quote)?;
    let prepared = prepare_wsol_transaction(state, &quote).await?;
    let client = state
        .wallets
        .get(&quote.wallet_id)
        .context("wallet is not loaded")?;
    let rpc = client
        .rpcs()
        .into_iter()
        .next()
        .context("wallet has no RPC")?;
    let simulation = rpc.simulate_transaction(&prepared.transaction).await?;
    if let Some(err) = simulation.value.err {
        bail!("WSOL conversion simulation failed: {err}");
    }
    Ok(WsolSimulationResult {
        status: "simulated",
        quote_id: quote.quote_id,
        network_fee_lamports: Some(quote.estimated_network_fee_lamports),
        units_consumed: simulation.value.units_consumed,
    })
}

pub async fn prepare_wsol_transaction(
    state: &AppState,
    quote: &WsolQuote,
) -> Result<PreparedWsolTransaction> {
    let wallet = state.wallet_repository.get(&quote.wallet_id).await?;
    if !wallet.enabled {
        bail!("wallet is disabled");
    }
    let now_ms = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    quote.validate_binding(
        &wallet.wallet_id,
        &wallet.pubkey,
        &wallet.owner_type,
        &wallet.owner_id,
        now_ms,
    )?;
    let client = state
        .wallets
        .get(&quote.wallet_id)
        .context("wallet is not loaded")?;
    let signer = client.conversion_signer();
    let owner = signer.pubkey();
    if owner.to_string() != wallet.pubkey {
        bail!("loaded wallet signer does not match quote");
    }
    let rpc = client
        .rpcs()
        .into_iter()
        .next()
        .context("wallet has no RPC")?;
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    let ata = get_associated_token_address_with_program_id(&owner, &mint, &token_program);
    let sol_balance = rpc.get_balance(&owner).await?;
    let ata_account = rpc
        .get_account_with_commitment(&ata, CommitmentConfig::processed())
        .await?
        .value;
    if ata_account.is_some() != quote.ata_exists {
        bail!("WSOL account state changed; request a new quote");
    }
    let wsol_balance = match ata_account.as_ref() {
        Some(account) => {
            if account.owner != token_program {
                bail!("WSOL ATA program owner changed");
            }
            let token = spl_token_interface::state::Account::unpack(&account.data)?;
            if token.mint != mint || token.owner != owner || token.is_native.is_none() {
                bail!("WSOL ATA mint or owner changed");
            }
            token.amount
        }
        None => 0,
    };
    if quote.direction == WsolDirection::Unwrap && quote.amount_lamports == wsol_balance {
        let current_refund = ata_account
            .as_ref()
            .map(|account| account.lamports.saturating_sub(wsol_balance))
            .unwrap_or(0);
        if current_refund != quote.rent_refund_lamports {
            bail!("WSOL account rent changed; request a new quote");
        }
    }
    let current_source_balance = match quote.direction {
        WsolDirection::Wrap => sol_balance,
        WsolDirection::Unwrap => wsol_balance,
    };
    quote.validate_current_balance(current_source_balance)?;
    let account_rent = rpc
        .get_minimum_balance_for_rent_exemption(spl_token_interface::state::Account::LEN)
        .await?;
    let built = build_wsol_instructions(
        &owner,
        quote.direction,
        quote.amount_lamports,
        wsol_balance,
        ata_account.is_some(),
        account_rent,
    )?;
    if built.temporary_rent_lamports != quote.temporary_rent_lamports {
        bail!("temporary rent changed; request a new quote");
    }
    let blockhash = rpc.get_latest_blockhash().await?;
    let mut message = Message::new(&built.instructions, Some(&owner));
    message.recent_blockhash = blockhash;
    let fee = rpc.get_fee_for_message(&message).await?;
    if fee > quote.estimated_network_fee_lamports {
        bail!("network fee increased; request a new quote");
    }
    let rent_funding = if quote.direction == WsolDirection::Wrap && !quote.ata_exists {
        account_rent
    } else {
        built.temporary_rent_lamports
    };
    let required_sol = fee
        .checked_add(rent_funding)
        .and_then(|value| value.checked_add(100_000))
        .and_then(|value| {
            if quote.direction == WsolDirection::Wrap {
                value.checked_add(quote.amount_lamports)
            } else {
                Some(value)
            }
        })
        .context("conversion SOL budget overflow")?;
    if sol_balance < required_sol {
        bail!("SOL balance insufficient for conversion fee, rent and reserve");
    }
    let transaction = match built.temporary_account.as_ref() {
        Some(temp) => Transaction::new_signed_with_payer(
            &built.instructions,
            Some(&owner),
            &[signer.as_ref(), temp],
            blockhash,
        ),
        None => Transaction::new_signed_with_payer(
            &built.instructions,
            Some(&owner),
            &[signer.as_ref()],
            blockhash,
        ),
    };
    let signature = transaction.signatures[0].to_string();
    let signed_bytes = bincode::serialize(&transaction)?;
    Ok(PreparedWsolTransaction {
        transaction,
        signed_bytes,
        signature,
    })
}

pub async fn quote_wsol(state: &AppState, request: WsolQuoteRequest) -> Result<WsolQuotePreview> {
    let store = state
        .conversions
        .as_ref()
        .context("conversion quotes are disabled")?;
    let wallet = state.wallet_repository.get(&request.wallet_id).await?;
    if !wallet.enabled
        || wallet.owner_type != request.owner_type
        || wallet.owner_id != request.owner_id
    {
        bail!("wallet is disabled or does not belong to this owner");
    }
    if request.all && request.amount_lamports.is_some() {
        bail!("provide either all or amount_lamports");
    }
    if request.all && request.direction != WsolDirection::Unwrap {
        bail!("all is only supported for WSOL unwrap");
    }
    let owner = wallet.pubkey.parse::<Pubkey>()?;
    let client = state
        .wallets
        .get(&request.wallet_id)
        .context("wallet is not loaded")?;
    let rpc = client
        .rpcs()
        .into_iter()
        .next()
        .context("wallet has no RPC")?;
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    let ata = get_associated_token_address_with_program_id(&owner, &mint, &token_program);
    let sol_balance = rpc.get_balance(&owner).await?;
    let ata_account = rpc
        .get_account_with_commitment(&ata, CommitmentConfig::processed())
        .await?
        .value;
    let (wsol_balance, source_rent) = match ata_account.as_ref() {
        Some(account) => {
            if account.owner != token_program {
                bail!("WSOL ATA has an unexpected program owner");
            }
            let token = spl_token_interface::state::Account::unpack(&account.data)?;
            if token.mint != mint || token.owner != owner || token.is_native.is_none() {
                bail!("WSOL ATA mint or owner does not match wallet");
            }
            (token.amount, account.lamports.saturating_sub(token.amount))
        }
        None => (0, 0),
    };
    let source_balance = match request.direction {
        WsolDirection::Wrap => sol_balance,
        WsolDirection::Unwrap => wsol_balance,
    };
    let amount = if request.all {
        source_balance
    } else {
        request
            .amount_lamports
            .context("amount_lamports is required")?
    };
    let account_rent = rpc
        .get_minimum_balance_for_rent_exemption(spl_token_interface::state::Account::LEN)
        .await?;
    let built = build_wsol_instructions(
        &owner,
        request.direction,
        amount,
        wsol_balance,
        ata_account.is_some(),
        account_rent,
    )?;
    let mut message = Message::new(&built.instructions, Some(&owner));
    message.recent_blockhash = rpc.get_latest_blockhash().await?;
    let fee = rpc.get_fee_for_message(&message).await?;
    let rent_funding = match request.direction {
        WsolDirection::Wrap if ata_account.is_none() => account_rent,
        WsolDirection::Unwrap => built.temporary_rent_lamports,
        _ => 0,
    };
    let sol_spend = fee
        .checked_add(rent_funding)
        .and_then(|value| value.checked_add(100_000))
        .and_then(|value| {
            if request.direction == WsolDirection::Wrap {
                value.checked_add(amount)
            } else {
                Some(value)
            }
        })
        .context("conversion SOL budget overflow")?;
    if sol_balance < sol_spend {
        bail!("SOL balance insufficient for amount, fee, rent and reserve");
    }
    let now_ms = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let quote = WsolQuote {
        quote_id: format!("cq_{}", uuid::Uuid::new_v4().simple()),
        wallet_id: request.wallet_id,
        wallet_pubkey: wallet.pubkey,
        owner_type: request.owner_type,
        owner_id: request.owner_id,
        direction: request.direction,
        amount_lamports: amount,
        source_balance_lamports: source_balance,
        ata_exists: ata_account.is_some(),
        estimated_network_fee_lamports: fee,
        temporary_rent_lamports: built.temporary_rent_lamports,
        rent_refund_lamports: if request.direction == WsolDirection::Unwrap
            && amount == wsol_balance
        {
            source_rent
        } else {
            0
        },
        expires_at_ms: now_ms + 60_000,
    };
    store.save_quote(&quote).await?;
    Ok(WsolQuotePreview {
        quote,
        executable: false,
    })
}

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS conversion_quotes (
  quote_id TEXT PRIMARY KEY,
  quote_json TEXT NOT NULL,
  expires_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS conversion_submissions (
  submission_id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  fingerprint TEXT NOT NULL,
  quote_id TEXT NOT NULL,
  wallet_id TEXT NOT NULL,
  wallet_pubkey TEXT NOT NULL,
  owner_type TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  status TEXT NOT NULL,
  signature TEXT,
  signed_transaction BLOB,
  error TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_conversion_submissions_owner
  ON conversion_submissions(owner_type, owner_id, wallet_id, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_conversion_one_pending_per_wallet
  ON conversion_submissions(wallet_id)
  WHERE status IN ('reserved', 'signed', 'submitted', 'unknown');
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsolDirection {
    Wrap,
    Unwrap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsolQuote {
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub owner_type: String,
    pub owner_id: String,
    pub direction: WsolDirection,
    pub amount_lamports: u64,
    pub source_balance_lamports: u64,
    pub ata_exists: bool,
    pub estimated_network_fee_lamports: u64,
    pub temporary_rent_lamports: u64,
    pub rent_refund_lamports: u64,
    pub expires_at_ms: u64,
}

impl WsolQuote {
    pub fn validate_current_balance(&self, current_balance_lamports: u64) -> Result<()> {
        if current_balance_lamports != self.source_balance_lamports {
            bail!("conversion source balance changed; request a new quote");
        }
        Ok(())
    }

    pub fn validate_binding(
        &self,
        wallet_id: &str,
        wallet_pubkey: &str,
        owner_type: &str,
        owner_id: &str,
        now_ms: u64,
    ) -> Result<()> {
        if now_ms >= self.expires_at_ms {
            bail!("conversion quote expired");
        }
        if self.wallet_id != wallet_id
            || self.wallet_pubkey != wallet_pubkey
            || self.owner_type != owner_type
            || self.owner_id != owner_id
        {
            bail!("conversion quote does not belong to this wallet and owner");
        }
        if self.amount_lamports == 0 || self.amount_lamports > self.source_balance_lamports {
            bail!("conversion quote amount is invalid for source balance");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsolSubmission {
    pub quote_id: String,
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub simulate: bool,
    #[serde(default)]
    pub confirmed: bool,
    pub idempotency_key: String,
}

impl WsolSubmission {
    pub fn fingerprint(&self, quote: &WsolQuote) -> Result<String> {
        if self.idempotency_key.trim().is_empty() || self.idempotency_key.len() > 128 {
            bail!("idempotency_key must be 1..128 characters");
        }
        if self.quote_id != quote.quote_id
            || self.wallet_id != quote.wallet_id
            || self.owner_type != quote.owner_type
            || self.owner_id != quote.owner_id
        {
            bail!("submission does not match conversion quote");
        }
        let encoded = serde_json::to_vec(&(self, quote))?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}

pub struct WsolInstructions {
    pub instructions: Vec<Instruction>,
    pub temporary_account: Option<Keypair>,
    pub temporary_rent_lamports: u64,
}

pub fn build_wsol_instructions(
    owner: &Pubkey,
    direction: WsolDirection,
    amount_lamports: u64,
    wsol_balance_lamports: u64,
    ata_exists: bool,
    token_account_rent_lamports: u64,
) -> Result<WsolInstructions> {
    if amount_lamports == 0 {
        bail!("conversion amount must be positive");
    }
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    let ata = get_associated_token_address_with_program_id(owner, &mint, &token_program);
    match direction {
        WsolDirection::Wrap => {
            let mut instructions = Vec::new();
            if !ata_exists {
                instructions.push(create_associated_token_account_idempotent(
                    owner,
                    owner,
                    &mint,
                    &token_program,
                ));
            }
            instructions.push(system_instruction::transfer(owner, &ata, amount_lamports));
            instructions.push(token_instruction::sync_native(&token_program, &ata)?);
            Ok(WsolInstructions {
                instructions,
                temporary_account: None,
                temporary_rent_lamports: 0,
            })
        }
        WsolDirection::Unwrap => {
            if !ata_exists || amount_lamports > wsol_balance_lamports {
                bail!("WSOL balance insufficient for requested unwrap");
            }
            if amount_lamports == wsol_balance_lamports {
                return Ok(WsolInstructions {
                    instructions: vec![token_instruction::close_account(
                        &token_program,
                        &ata,
                        owner,
                        owner,
                        &[],
                    )?],
                    temporary_account: None,
                    temporary_rent_lamports: 0,
                });
            }
            let temporary_account = Keypair::new();
            let temp = temporary_account.pubkey();
            let instructions = vec![
                system_instruction::create_account(
                    owner,
                    &temp,
                    token_account_rent_lamports,
                    spl_token_interface::state::Account::LEN as u64,
                    &token_program,
                ),
                token_instruction::initialize_account3(&token_program, &temp, &mint, owner)?,
                token_instruction::transfer_checked(
                    &token_program,
                    &ata,
                    &mint,
                    &temp,
                    owner,
                    &[],
                    amount_lamports,
                    9,
                )?,
                token_instruction::close_account(&token_program, &temp, owner, owner, &[])?,
            ];
            Ok(WsolInstructions {
                instructions,
                temporary_account: Some(temporary_account),
                temporary_rent_lamports: token_account_rent_lamports,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionRecord {
    pub submission_id: String,
    pub idempotency_key: String,
    pub fingerprint: String,
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub owner_type: String,
    pub owner_id: String,
    pub status: String,
    pub signature: Option<String>,
    pub signed_transaction: Option<Vec<u8>>,
    pub error: Option<String>,
}

pub struct ConversionStore {
    pool: SqlitePool,
}

impl ConversionStore {
    pub async fn new(database_url: &str) -> Result<Self> {
        if let Some(path) = database_url.strip_prefix("sqlite://") {
            if let Some(parent) = std::path::Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(MIGRATION).execute(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn save_quote(&self, quote: &WsolQuote) -> Result<()> {
        let quote_json = serde_json::to_string(quote)?;
        sqlx::query(
            "INSERT INTO conversion_quotes(quote_id,quote_json,expires_at_ms) VALUES(?,?,?)",
        )
        .bind(&quote.quote_id)
        .bind(quote_json)
        .bind(i64::try_from(quote.expires_at_ms)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_quote(&self, quote_id: &str) -> Result<Option<WsolQuote>> {
        let row = sqlx::query("SELECT quote_json FROM conversion_quotes WHERE quote_id=?")
            .bind(quote_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| serde_json::from_str(row.get::<&str, _>("quote_json")).map_err(Into::into))
            .transpose()
    }

    pub async fn reserve_live(
        &self,
        submission: &WsolSubmission,
        quote: &WsolQuote,
        now_ms: u64,
    ) -> Result<(ConversionRecord, bool)> {
        if submission.simulate {
            bail!("simulation does not reserve a live idempotency key");
        }
        let fingerprint = submission.fingerprint(quote)?;
        if let Some(existing) = self.get_by_key(&submission.idempotency_key).await? {
            if existing.fingerprint != fingerprint {
                bail!("idempotency key was used with different conversion parameters");
            }
            return Ok((existing, false));
        }
        quote.validate_binding(
            &submission.wallet_id,
            &quote.wallet_pubkey,
            &submission.owner_type,
            &submission.owner_id,
            now_ms,
        )?;
        let submission_id = format!("conv_{}", uuid::Uuid::new_v4().simple());
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO conversion_submissions(submission_id,idempotency_key,fingerprint,quote_id,wallet_id,wallet_pubkey,owner_type,owner_id,status,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?,'reserved',?,?)",
        )
        .bind(&submission_id)
        .bind(&submission.idempotency_key)
        .bind(&fingerprint)
        .bind(&quote.quote_id)
        .bind(&quote.wallet_id)
        .bind(&quote.wallet_pubkey)
        .bind(&quote.owner_type)
        .bind(&quote.owner_id)
        .bind(i64::try_from(now_ms)?)
        .bind(i64::try_from(now_ms)?)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1;
        let record = match self.get_by_key(&submission.idempotency_key).await? {
            Some(record) => record,
            None => {
                bail!("wallet has a pending conversion; reconcile it before submitting another")
            }
        };
        if record.fingerprint != fingerprint {
            bail!("idempotency key was used with different conversion parameters");
        }
        Ok((record, inserted))
    }

    pub async fn record_signed(
        &self,
        submission_id: &str,
        signature: &str,
        signed_transaction: &[u8],
        now_ms: u64,
    ) -> Result<()> {
        if signed_transaction.is_empty() || signature.is_empty() {
            bail!("signed transaction and signature are required before broadcast");
        }
        let result = sqlx::query(
            "UPDATE conversion_submissions SET status='signed',signature=?,signed_transaction=?,updated_at_ms=? WHERE submission_id=? AND status='reserved'",
        )
        .bind(signature)
        .bind(signed_transaction)
        .bind(i64::try_from(now_ms)?)
        .bind(submission_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            bail!("conversion must be reserved exactly once before signing");
        }
        Ok(())
    }

    pub async fn mark_failed_before_sign(
        &self,
        submission_id: &str,
        error: &str,
        now_ms: u64,
    ) -> Result<()> {
        let result = sqlx::query("UPDATE conversion_submissions SET status='failed',error=?,updated_at_ms=? WHERE submission_id=? AND status='reserved' AND signature IS NULL")
            .bind(error).bind(i64::try_from(now_ms)?).bind(submission_id).execute(&self.pool).await?;
        if result.rows_affected() != 1 {
            bail!("conversion is not an unsigned reservation");
        }
        Ok(())
    }

    pub async fn mark_broadcast_result(
        &self,
        submission_id: &str,
        status: &str,
        error: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        if !matches!(status, "submitted" | "confirmed" | "unknown" | "failed") {
            bail!("invalid conversion broadcast status");
        }
        let result = sqlx::query(
            "UPDATE conversion_submissions SET status=?,error=?,updated_at_ms=? WHERE submission_id=? AND signature IS NOT NULL AND signed_transaction IS NOT NULL AND status IN ('signed','submitted','unknown')",
        )
        .bind(status)
        .bind(error)
        .bind(i64::try_from(now_ms)?)
        .bind(submission_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            bail!("conversion has no persisted signature or is already terminal");
        }
        Ok(())
    }

    pub async fn get_by_key(&self, idempotency_key: &str) -> Result<Option<ConversionRecord>> {
        let row = sqlx::query("SELECT * FROM conversion_submissions WHERE idempotency_key=?")
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| ConversionRecord {
            submission_id: row.get("submission_id"),
            idempotency_key: row.get("idempotency_key"),
            fingerprint: row.get("fingerprint"),
            quote_id: row.get("quote_id"),
            wallet_id: row.get("wallet_id"),
            wallet_pubkey: row.get("wallet_pubkey"),
            owner_type: row.get("owner_type"),
            owner_id: row.get("owner_id"),
            status: row.get("status"),
            signature: row.get("signature"),
            signed_transaction: row.get("signed_transaction"),
            error: row.get("error"),
        }))
    }

    pub async fn get_by_id(&self, submission_id: &str) -> Result<Option<ConversionRecord>> {
        let row =
            sqlx::query("SELECT idempotency_key FROM conversion_submissions WHERE submission_id=?")
                .bind(submission_id)
                .fetch_optional(&self.pool)
                .await?;
        match row {
            Some(row) => self.get_by_key(row.get::<&str, _>("idempotency_key")).await,
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WsolDirection, WsolQuote, WsolSubmission};
    use solana_sdk::signature::Signer;

    fn quote() -> WsolQuote {
        WsolQuote {
            quote_id: "q1".into(),
            wallet_id: "w1".into(),
            wallet_pubkey: "pubkey".into(),
            owner_type: "telegram".into(),
            owner_id: "123".into(),
            direction: WsolDirection::Unwrap,
            amount_lamports: 10,
            source_balance_lamports: 10,
            ata_exists: true,
            estimated_network_fee_lamports: 1,
            temporary_rent_lamports: 0,
            rent_refund_lamports: 2,
            expires_at_ms: 100,
        }
    }

    #[test]
    fn partial_unwrap_preserves_source_account_and_full_unwrap_closes_it() {
        let owner = solana_sdk::pubkey::Pubkey::new_unique();
        let partial =
            super::build_wsol_instructions(&owner, WsolDirection::Unwrap, 50, 100, true, 2_039_280)
                .unwrap();
        assert_eq!(partial.instructions.len(), 4);
        assert!(partial.temporary_account.is_some());
        let temporary_pubkey = partial.temporary_account.as_ref().unwrap().pubkey();
        assert_eq!(partial.instructions[3].accounts[0].pubkey, temporary_pubkey);
        assert_eq!(partial.temporary_rent_lamports, 2_039_280);
        let full = super::build_wsol_instructions(
            &owner,
            WsolDirection::Unwrap,
            100,
            100,
            true,
            2_039_280,
        )
        .unwrap();
        assert_eq!(full.instructions.len(), 1);
        assert!(full.temporary_account.is_none());
        let ata = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
            &owner,
            &crate::models::WSOL_MINT.parse().unwrap(),
            &spl_token_interface::ID,
        );
        assert_eq!(full.instructions[0].accounts[0].pubkey, ata);
        assert_eq!(full.temporary_rent_lamports, 0);
    }

    #[test]
    fn wrap_creates_ata_only_when_missing_and_unwrap_rejects_overdraft() {
        let owner = solana_sdk::pubkey::Pubkey::new_unique();
        let new_ata = super::build_wsol_instructions(
            &owner,
            WsolDirection::Wrap,
            1_000_000,
            0,
            false,
            2_039_280,
        )
        .unwrap();
        assert_eq!(new_ata.instructions.len(), 3);
        let existing_ata = super::build_wsol_instructions(
            &owner,
            WsolDirection::Wrap,
            1_000_000,
            0,
            true,
            2_039_280,
        )
        .unwrap();
        assert_eq!(existing_ata.instructions.len(), 2);
        assert!(super::build_wsol_instructions(
            &owner,
            WsolDirection::Unwrap,
            101,
            100,
            true,
            2_039_280,
        )
        .is_err());
    }

    #[test]
    fn quote_is_bound_to_owner_wallet_and_expiry() {
        let quote = quote();
        assert!(quote
            .validate_binding("w1", "pubkey", "telegram", "123", 99)
            .is_ok());
        assert!(quote
            .validate_binding("w1", "pubkey", "telegram", "other", 99)
            .is_err());
        assert!(quote
            .validate_binding("w1", "changed", "telegram", "123", 99)
            .is_err());
        assert!(quote
            .validate_binding("w1", "pubkey", "telegram", "123", 100)
            .is_err());
        assert!(quote.validate_current_balance(10).is_ok());
        assert!(quote.validate_current_balance(9).is_err());
        assert!(quote.validate_current_balance(11).is_err());
    }

    #[test]
    fn simulation_and_live_have_distinct_fingerprints() {
        let quote = quote();
        let simulation = WsolSubmission {
            quote_id: "q1".into(),
            wallet_id: "w1".into(),
            owner_type: "telegram".into(),
            owner_id: "123".into(),
            simulate: true,
            confirmed: false,
            idempotency_key: "same-key".into(),
        };
        let mut live = simulation.clone();
        live.simulate = false;
        assert_ne!(
            simulation.fingerprint(&quote).unwrap(),
            live.fingerprint(&quote).unwrap()
        );
        live.wallet_id = "other".into();
        assert!(live.fingerprint(&quote).is_err());
    }

    #[tokio::test]
    async fn durable_reservation_rejects_changed_request_and_persists_signature() {
        let store = super::ConversionStore::new("sqlite::memory:")
            .await
            .unwrap();
        let quote = quote();
        store.save_quote(&quote).await.unwrap();
        assert_eq!(store.get_quote("q1").await.unwrap(), Some(quote.clone()));
        let submission = WsolSubmission {
            quote_id: "q1".into(),
            wallet_id: "w1".into(),
            owner_type: "telegram".into(),
            owner_id: "123".into(),
            simulate: false,
            confirmed: true,
            idempotency_key: "key".into(),
        };
        let (record, inserted) = store.reserve_live(&submission, &quote, 50).await.unwrap();
        assert!(inserted);
        assert_eq!(record.status, "reserved");
        let (again, inserted) = store.reserve_live(&submission, &quote, 101).await.unwrap();
        assert!(!inserted);
        assert_eq!(again.submission_id, record.submission_id);
        let mut changed = submission.clone();
        changed.quote_id = "other".into();
        assert!(store.reserve_live(&changed, &quote, 50).await.is_err());
        let mut changed_amount = quote.clone();
        changed_amount.amount_lamports = 9;
        assert!(store
            .reserve_live(&submission, &changed_amount, 50)
            .await
            .is_err());
        let mut simulation = submission.clone();
        simulation.simulate = true;
        assert!(store.reserve_live(&simulation, &quote, 50).await.is_err());
        let mut concurrent = submission.clone();
        concurrent.idempotency_key = "another-key".into();
        assert!(store.reserve_live(&concurrent, &quote, 50).await.is_err());
        assert!(store.get_by_key("another-key").await.unwrap().is_none());
        store
            .record_signed(&record.submission_id, "sig", &[1, 2, 3], 60)
            .await
            .unwrap();
        let signed = store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(
            store.get_by_id(&record.submission_id).await.unwrap(),
            Some(signed.clone())
        );
        assert_eq!(signed.status, "signed");
        assert_eq!(signed.signature.as_deref(), Some("sig"));
        assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        assert!(store
            .record_signed(&record.submission_id, "new", &[4], 61)
            .await
            .is_err());
        store
            .mark_broadcast_result(&record.submission_id, "unknown", None, 62)
            .await
            .unwrap();
        let unknown = store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(unknown.signature.as_deref(), Some("sig"));
        assert_eq!(unknown.status, "unknown");
        assert!(store.reserve_live(&concurrent, &quote, 63).await.is_err());
        store
            .mark_broadcast_result(&record.submission_id, "confirmed", None, 63)
            .await
            .unwrap();
        assert!(store
            .mark_broadcast_result(&record.submission_id, "submitted", None, 64)
            .await
            .is_err());
        let mut unsigned = submission.clone();
        unsigned.idempotency_key = "unsigned".into();
        let (unsigned_record, _) = store.reserve_live(&unsigned, &quote, 50).await.unwrap();
        store
            .mark_failed_before_sign(&unsigned_record.submission_id, "preflight failed", 51)
            .await
            .unwrap();
        assert_eq!(
            store.get_by_key("unsigned").await.unwrap().unwrap().status,
            "failed"
        );
        assert!(store
            .record_signed(&unsigned_record.submission_id, "sig2", &[1], 52)
            .await
            .is_err());
    }
}
