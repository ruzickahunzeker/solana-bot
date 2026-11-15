use anyhow::{ensure, Context, Result};
use serde_json::json;
use sqlx::{sqlite::SqliteRow, Row, Sqlite, Transaction};

use super::{final_observation, read_signature, sha, validate_metadata_bound};
use crate::{
    adapters::persistence::trade_snapshot::verify_stored_identity,
    adapters::sdk::address_parser::SdkAddressParser,
    application::{
        ordinary_observation_context::{CapturedOrdinaryObservation, OrdinaryObservationContext},
        ports::address_parser::TradeAddressParser,
        trade_record::TradeRecordV1,
    },
    domain::trade::{ExecutionMethod, Side, TradeStatus},
    models::TradeRequest,
    persistence::row_to_record,
};

pub(super) async fn capture_checked(
    tx: &mut Transaction<'_, Sqlite>,
    request_id: &str,
) -> Result<(SqliteRow, CapturedOrdinaryObservation)> {
    let operation: String =
        sqlx::query_scalar("SELECT operation FROM ordinary_execution_claims WHERE request_id=?")
            .bind(request_id)
            .fetch_one(&mut **tx)
            .await?;
    match operation.as_str() {
        "ordinary-sdk-trade" => {
            crate::adapters::persistence::ordinary_sdk_preparation::observation::capture_checked(
                tx, request_id,
            )
            .await
        }
        "ordinary-sol-buy" => capture_sol_checked(tx, request_id).await,
        _ => anyhow::bail!("unsupported ordinary observation operation"),
    }
}

async fn capture_sol_checked(
    tx: &mut Transaction<'_, Sqlite>,
    request_id: &str,
) -> Result<(SqliteRow, CapturedOrdinaryObservation)> {
    let raw = sqlx::query("SELECT * FROM trades WHERE request_id=?")
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("ordinary observation parent missing")?;
    let key: String = raw.try_get("idempotency_key")?;
    let hash: String = raw.try_get("request_hash")?;
    let request_json: String = raw.try_get("request_json")?;
    let parent = row_to_record(raw)?;
    let mint = SdkAddressParser.parse(&parent.mint)?;
    SdkAddressParser.parse(&parent.wallet_pubkey)?;
    if let Some(pool_id) = &parent.pool_id {
        SdkAddressParser.parse(pool_id)?;
    }
    let request: TradeRequest = serde_json::from_str(&request_json)?;
    ensure!(
        parent.request_id == request_id
            && parent.side == Side::Buy
            && request.pool_id == parent.pool_id
            && request.execution_method.unwrap_or(ExecutionMethod::Sdk) == parent.execution_method
            && verify_stored_identity(&parent, &request_json, &key, true)?,
        "ordinary observation stored request mismatch"
    );
    let (amount, explicit_slippage) = crate::validation::replay_sol_observation_amount(&request)?;
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("ordinary observation claim missing")?;
    let nonce: Option<String> = row.try_get("owner_nonce")?;
    ensure!(
        row.try_get::<i64, _>("schema_version")? == 1
            && row.try_get::<&str, _>("operation")? == "ordinary-sol-buy"
            && row.try_get::<&str, _>("request_hash")? == hash
            && row.try_get::<i64, _>("created_at_ms")? >= 0
            && nonce.as_ref().is_some_and(
                |value| value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
            ),
        "ordinary observation claim identity invalid"
    );
    validate_metadata_bound(&row, |bound| {
        crate::application::ordinary_submission::validate_quote_intent(
            bound,
            &parent.wallet_id,
            &parent.wallet_pubkey,
            &mint.to_string(),
            amount,
            explicit_slippage.unwrap_or(u64::from(bound.quote_slippage_bps)),
        )
    })?;
    final_observation::validate(&row, &parent)?;
    let stage: &str = row.try_get("submission_stage")?;
    ensure!(
        matches!(stage, "signed" | "submitted" | "unknown")
            && matches!(
                parent.status,
                TradeStatus::Submitting
                    | TradeStatus::Submitted
                    | TradeStatus::Unknown
                    | TradeStatus::Confirmed
                    | TradeStatus::Failed
            )
            && (parent.status != TradeStatus::Submitting || stage == "signed"),
        "ordinary operation has no recoverable signed observation"
    );
    let signed = read_signature(&row, &parent)?;
    // State/error/finality may advance; every original identity and signed artifact stays bound.
    let fingerprint = sha(&serde_json::to_vec(&json!({
        "v":1,"request_id":parent.request_id,"wallet_id":parent.wallet_id,"wallet_pubkey":parent.wallet_pubkey,
        "side":parent.side,"pool":parent.pool,"mint":parent.mint,"pool_id":parent.pool_id,
        "execution_method":parent.execution_method,"created_at_ms":parent.created_at_ms,
        "key":key,"request_hash":hash,"request_json":request_json,
        "claim_version":row.try_get::<i64, _>("schema_version")?,
        "claim_operation":row.try_get::<&str, _>("operation")?,
        "claim_created_at_ms":row.try_get::<i64, _>("created_at_ms")?,
        "owner_nonce":nonce,
        "bound_quote_json":row.try_get::<Option<String>, _>("bound_quote_json")?,
        "preflight_json":row.try_get::<Option<String>, _>("preflight_json")?,
        "unsigned_sha256":row.try_get::<Option<String>, _>("unsigned_sha256")?,
        "signature":signed.signature,"signed_sha256":row.try_get::<Option<String>, _>("signed_sha256")?
    }))?);
    let context = OrdinaryObservationContext::captured(&parent, key, hash, fingerprint)?;
    Ok((
        row,
        CapturedOrdinaryObservation {
            context,
            record: parent,
        },
    ))
}

pub(super) async fn checked_context(
    tx: &mut Transaction<'_, Sqlite>,
    context: &OrdinaryObservationContext,
) -> Result<(SqliteRow, TradeRecordV1)> {
    let (row, captured) = capture_checked(tx, context.request_id()).await?;
    context.ensure_same(&captured.context)?;
    Ok((row, captured.record))
}
