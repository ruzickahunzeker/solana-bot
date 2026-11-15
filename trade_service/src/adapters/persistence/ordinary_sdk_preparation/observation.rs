use super::*;
use crate::{
    adapters::{
        persistence::{
            ordinary_submission::final_observation::FinalObservationV1,
            trade_snapshot::request_hash,
        },
        sdk::address_parser::SdkAddressParser,
    },
    application::{
        ordinary_observation_context::{CapturedOrdinaryObservation, OrdinaryObservationContext},
        ports::{
            address_parser::TradeAddressParser,
            ordinary_reconciliation::RecoveredOrdinaryObservations,
        },
        trade_validation::TradeValidator,
    },
    domain::{reconciliation::ChainObservation, trade::Address},
    models::TradeRequest,
};
use serde_json::json;

pub(super) fn validate_finality_fields(row: &SqliteRow) -> Result<Option<FinalObservationV1>> {
    let Some(json) = row.try_get::<Option<&str>, _>("final_observation_json")? else {
        return Ok(None);
    };
    ensure!(
        ["signed", "broadcasting", "submitted", "unknown"]
            .contains(&row.try_get::<&str, _>("submission_stage")?),
        "SDK finality requires the original signed stage"
    );
    let signature = row
        .try_get::<Option<&str>, _>("signed_signature")?
        .context("SDK finality has no original signature")?;
    let evidence = FinalObservationV1::decode(json, signature)?;
    ensure!(
        serde_json::to_string(&evidence)? == json,
        "SDK finality is not the original canonical record"
    );
    Ok(Some(evidence))
}

pub(in crate::adapters::persistence) async fn capture_checked(
    tx: &mut Transaction<'_, Sqlite>,
    request_id: &str,
) -> Result<(SqliteRow, CapturedOrdinaryObservation)> {
    let raw = sqlx::query("SELECT * FROM trades WHERE request_id=?")
        .bind(request_id)
        .fetch_one(&mut **tx)
        .await?;
    let key: String = raw.try_get("idempotency_key")?;
    let hash: String = raw.try_get("request_hash")?;
    let request_json: String = raw.try_get("request_json")?;
    let parent = row_to_record(raw)?;
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(request_id)
        .fetch_one(&mut **tx)
        .await?;
    ensure!(
        parent.request_id == request_id
            && row.try_get::<&str, _>("request_id")? == request_id
            && row.try_get::<&str, _>("request_hash")? == hash
            && row.try_get::<i64, _>("created_at_ms")? >= 0,
        "SDK observation claim or parent identity mismatch"
    );
    let candidate = validate_sdk_row(&row)?.context("SDK observation candidate is missing")?;
    validate_sdk_parent(&row, &parent)?;
    ensure!(
        ["signed", "broadcasting", "submitted", "unknown"]
            .contains(&row.try_get::<&str, _>("submission_stage")?),
        "SDK observation requires already-persisted original signed bytes"
    );
    let request: TradeRequest = serde_json::from_str(&request_json)?;
    let normalized = TradeValidator::new(&SdkAddressParser)
        .replay_sdk_observation(&request.validation_input(), candidate.metadata.details())?;
    ensure!(
        verify_stored_identity(&parent, &request_json, &key, false)?
            && parent.pool == request.pool
            && parent.side == request.side
            && parent.execution_method == normalized.execution_method
            && parent.mint == request.mint
            && parent.pool_id.as_deref().map(str::trim)
                == normalized.pool_id.map(|v| v.to_string()).as_deref()
            && SdkAddressParser.parse(&parent.wallet_pubkey)?
                == Address::from_bytes(candidate.metadata.details().wallet)
            && request_hash(&parent.wallet_id, &request, &normalized)? == hash,
        "SDK observation historical intent or original V1 hash mismatch"
    );
    // Mutable receipt/error/finality are excluded; original intent and signed artifacts are not.
    let fingerprint = sha(&serde_json::to_vec(&json!({
        "v":1, "operation":"ordinary-sdk-trade", "request_id":parent.request_id,
        "wallet_id":parent.wallet_id, "wallet_pubkey":parent.wallet_pubkey,
        "side":parent.side, "pool":parent.pool, "mint":parent.mint, "pool_id":parent.pool_id,
        "method":parent.execution_method, "parent_created_at_ms":parent.created_at_ms,
        "key":key, "request_hash":hash, "request_json":request_json,
        "claim_created_at_ms":row.try_get::<i64, _>("created_at_ms")?,
        "owner_nonce":row.try_get::<&str, _>("owner_nonce")?,
        "sdk_prepared_json":row.try_get::<&str, _>("sdk_prepared_json")?,
        "unsigned_sha256":row.try_get::<&str, _>("unsigned_sha256")?,
        "signature":row.try_get::<&str, _>("signed_signature")?,
        "signed_sha256":row.try_get::<&str, _>("signed_sha256")?
    }))?);
    let context = OrdinaryObservationContext::captured_sdk(&parent, key, hash, fingerprint)?;
    Ok((
        row,
        CapturedOrdinaryObservation {
            context,
            record: parent,
        },
    ))
}

async fn checked_context(
    tx: &mut Transaction<'_, Sqlite>,
    context: &OrdinaryObservationContext,
) -> Result<(SqliteRow, TradeRecordV1)> {
    let (row, captured) = capture_checked(tx, context.request_id()).await?;
    context.ensure_same(&captured.context)?;
    Ok((row, captured.record))
}

pub(in crate::adapters::persistence) async fn record_finality(
    store: &SqliteOrdinaryExecutionClaims,
    context: &OrdinaryObservationContext,
    observation: &ChainObservation,
) -> Result<TradeRecordV1> {
    ensure!(
        context.operation() == ClaimOperation::SdkTrade,
        "SDK observation operation changed"
    );
    let evidence = FinalObservationV1::new(context.signature(), observation)?;
    let json = serde_json::to_string(&evidence)?;
    ensure!(json.len() <= 65_536, "SDK finality exceeds limit");
    let mut tx = store.pool.begin().await?;
    // Serialize finality and receipt writers, then bind every immutable field before any mutation.
    sqlx::query("UPDATE ordinary_execution_claims SET request_id=request_id WHERE 0")
        .execute(&mut *tx)
        .await?;
    let (row, parent) = checked_context(&mut tx, context).await?;
    if let Some(previous) = row.try_get::<Option<&str>, _>("final_observation_json")? {
        ensure!(previous == json, "SDK finalized observation conflict");
        tx.commit().await?;
        return Ok(parent);
    }
    let changed = sqlx::query("UPDATE ordinary_execution_claims SET final_observation_json=? WHERE request_id=? AND operation='ordinary-sdk-trade' AND request_hash=? AND owner_nonce IS NOT NULL AND signed_signature=? AND final_observation_json IS NULL AND submission_stage IN ('signed','broadcasting','submitted','unknown')")
        .bind(&json).bind(context.request_id()).bind(context.request_hash()).bind(context.signature())
        .execute(&mut *tx).await?.rows_affected();
    ensure!(
        changed == 1,
        "SDK finality state changed; no transaction retry"
    );
    let status = if evidence.error().is_some() {
        "failed"
    } else {
        "confirmed"
    };
    let changed = sqlx::query("UPDATE trades SET status=?,error=?,updated_at_ms=? WHERE request_id=? AND request_hash=? AND signature=? AND status IN ('submitting','submitted','unknown')")
        .bind(status).bind(evidence.error()).bind(i64::try_from(crate::state::now_ms())?)
        .bind(context.request_id()).bind(context.request_hash()).bind(context.signature())
        .execute(&mut *tx).await?.rows_affected();
    ensure!(
        changed == 1,
        "SDK finalized parent changed; preserve original evidence"
    );
    let (row, parent) = checked_context(&mut tx, context).await?;
    ensure!(
        row.try_get::<Option<&str>, _>("final_observation_json")? == Some(json.as_str())
            && parent.status == evidence.status()
            && parent.error.as_deref() == evidence.error(),
        "SDK finality readback mismatch"
    );
    tx.commit().await?;
    store.read_observation(context).await
}
