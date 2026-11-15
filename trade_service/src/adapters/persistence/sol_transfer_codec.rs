use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteRow, Row};

use crate::{
    application::{
        sol_transfer_policy::{self, TransferAmountInput, TransferAmountMode},
        sol_transfer_record::SolTransferRecordV1,
    },
    domain::{
        sol_transfer::{SolTransferIntent, SolTransferSnapshot},
        trade::Address,
    },
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentV1 {
    schema_version: u8,
    operation: String,
    mode: String,
    idempotency_key: String,
    owner_type: String,
    owner_id: String,
    wallet_id: String,
    source: [u8; 32],
    destination: [u8; 32],
    amount_mode: String,
    amount_lamports: Option<u64>,
    percent: Option<u64>,
}

pub(super) fn encode_intent(intent: &SolTransferIntent) -> Result<(String, String)> {
    sol_transfer_policy::validate_key(&intent.idempotency_key)?;
    let (amount_lamports, percent) = intent.amount.legacy_params();
    ensure!(
        amount_lamports != Some(0),
        "SOL transfer amount must be positive"
    );
    let value = IntentV1 {
        schema_version: 1,
        operation: "sol-transfer".into(),
        mode: "live".into(),
        idempotency_key: intent.idempotency_key.clone(),
        owner_type: intent.owner_type.clone(),
        owner_id: intent.owner_id.clone(),
        wallet_id: intent.wallet_id.clone(),
        source: intent.source.to_bytes(),
        destination: intent.destination.to_bytes(),
        amount_mode: if amount_lamports.is_some() {
            "fixed"
        } else {
            "percent"
        }
        .into(),
        amount_lamports,
        percent,
    };
    let json = serde_json::to_string(&value)?;
    let fingerprint = format!("{:x}", Sha256::digest(json.as_bytes()));
    Ok((json, fingerprint))
}

pub(super) fn decode_intent(json: &str) -> Result<SolTransferIntent> {
    let value: IntentV1 = serde_json::from_str(json)?;
    ensure!(
        value.schema_version == 1 && value.operation == "sol-transfer" && value.mode == "live",
        "unsupported SOL transfer intent schema"
    );
    sol_transfer_policy::validate_key(&value.idempotency_key)?;
    let mode = match value.amount_mode.as_str() {
        "fixed" => TransferAmountMode::Fixed,
        "percent" => TransferAmountMode::Percent,
        _ => anyhow::bail!("invalid SOL transfer amount mode"),
    };
    let amount = sol_transfer_policy::select_amount(TransferAmountInput {
        mode,
        fixed_lamports: value.amount_lamports,
        percent: value.percent,
    })?;
    Ok(SolTransferIntent {
        idempotency_key: value.idempotency_key,
        owner_type: value.owner_type,
        owner_id: value.owner_id,
        wallet_id: value.wallet_id,
        source: Address::from_bytes(value.source),
        destination: Address::from_bytes(value.destination),
        amount,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotV1 {
    schema_version: u8,
    source: [u8; 32],
    destination: [u8; 32],
    amount_lamports: u64,
    fee_lamports: u64,
    balance_before_lamports: u64,
    reserve_lamports: u64,
}

pub(super) fn encode_snapshot(snapshot: &SolTransferSnapshot) -> Result<String> {
    Ok(serde_json::to_string(&SnapshotV1 {
        schema_version: 1,
        source: snapshot.source.to_bytes(),
        destination: snapshot.destination.to_bytes(),
        amount_lamports: snapshot.amount_lamports,
        fee_lamports: snapshot.fee_lamports,
        balance_before_lamports: snapshot.balance_before_lamports,
        reserve_lamports: snapshot.reserve_lamports,
    })?)
}

pub(super) fn decode_snapshot(json: &str) -> Result<SolTransferSnapshot> {
    let value: SnapshotV1 = serde_json::from_str(json)?;
    ensure!(
        value.schema_version == 1,
        "unsupported SOL transfer snapshot schema"
    );
    Ok(SolTransferSnapshot {
        source: Address::from_bytes(value.source),
        destination: Address::from_bytes(value.destination),
        amount_lamports: value.amount_lamports,
        fee_lamports: value.fee_lamports,
        balance_before_lamports: value.balance_before_lamports,
        reserve_lamports: value.reserve_lamports,
    })
}

pub(super) use crate::application::sol_transfer_policy::validate_snapshot;

pub(crate) fn decode_projection(row: &SqliteRow) -> Result<SolTransferRecordV1> {
    // Only the old V1 projection uses its historical signed-INTEGER round trip.
    Ok(SolTransferRecordV1 {
        transfer_id: row.try_get("transfer_id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        owner_type: row.try_get("owner_type")?,
        owner_id: row.try_get("owner_id")?,
        wallet_id: row.try_get("wallet_id")?,
        from_pubkey: row.try_get("from_pubkey")?,
        to_pubkey: row.try_get("to_pubkey")?,
        amount_mode: row.try_get("amount_mode")?,
        requested_amount_lamports: row
            .try_get::<Option<i64>, _>("requested_amount_lamports")?
            .map(|v| v as u64),
        requested_percent: row
            .try_get::<Option<i64>, _>("requested_percent")?
            .map(|v| v as u64),
        actual_amount_lamports: row
            .try_get::<Option<i64>, _>("actual_amount_lamports")?
            .map(|v| v as u64),
        fee_lamports: row
            .try_get::<Option<i64>, _>("fee_lamports")?
            .map(|v| v as u64),
        balance_before_lamports: row
            .try_get::<Option<i64>, _>("balance_before_lamports")?
            .map(|v| v as u64),
        status: row.try_get("status")?,
        signature: row.try_get("signature")?,
        error: row.try_get("error")?,
    })
}
