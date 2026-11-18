use super::submission::{begin, save_signed, signing_case, signing_case_with_defaults};
use super::*;
use crate::{
    application::{
        ordinary_reconciliation::reconcile_captured,
        ports::{
            ordinary_reconciliation::RecoveredOrdinaryObservations,
            reconciliation::SignatureStatusReader,
        },
    },
    config::TradePersistenceConfig,
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        submission::SignedSubmission,
    },
    persistence::TradePersistence,
};
use serde_json::{json, Value};
use solana_sdk::signature::Signer;

mod races;
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

async fn stage(c: &Case, signed: &SignedSubmission, name: &str) {
    if name != "signed" {
        c.store
            .take_sdk_broadcast(&c.f.execution, &c.owner, signed)
            .await
            .unwrap();
        if name != "broadcasting" {
            c.store
                .record_sdk_broadcast(
                    &c.f.execution,
                    &c.owner,
                    signed,
                    name == "submitted",
                    (name == "unknown").then_some("synthetic lost receipt"),
                )
                .await
                .unwrap();
        }
    }
}
fn observation(
    signature: &str,
    commitment: ChainCommitment,
    error: Option<&str>,
) -> ChainObservation {
    ChainObservation {
        signature: signature.into(),
        commitment,
        slot: 42,
        error: error.map(str::to_owned),
    }
}
struct Reader {
    value: Option<ChainObservation>,
    fail: bool,
    calls: AtomicUsize,
    pending: bool,
    dropped: Arc<AtomicBool>,
}
struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl Reader {
    fn new(value: Option<ChainObservation>) -> Self {
        Self {
            value,
            fail: false,
            calls: AtomicUsize::new(0),
            pending: false,
            dropped: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl SignatureStatusReader for Reader {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _dropped = Dropped(self.dropped.clone());
        if let Some(value) = &self.value {
            assert_eq!(value.signature, signature);
        }
        if self.pending {
            std::future::pending::<()>().await;
        }
        ensure!(
            !self.fail,
            "synthetic RPC 429 at https://rpc.invalid/private-key"
        );
        Ok(self.value.clone())
    }
}
async fn immutable(pool: &sqlx::SqlitePool, id: &str) -> Value {
    let row = sqlx::query("SELECT owner_nonce,operation,request_hash,sdk_prepared_json,unsigned_message,unsigned_sha256,signed_signature,signed_transaction,signed_sha256 FROM ordinary_execution_claims WHERE request_id=?")
        .bind(id).fetch_one(pool).await.unwrap();
    json!({
        "owner":row.get::<String,_>("owner_nonce"), "operation":row.get::<String,_>("operation"),
        "hash":row.get::<String,_>("request_hash"), "metadata":row.get::<String,_>("sdk_prepared_json"),
        "unsigned":row.get::<Vec<u8>,_>("unsigned_message"), "unsigned_hash":row.get::<String,_>("unsigned_sha256"),
        "signature":row.get::<String,_>("signed_signature"), "packet":row.get::<Vec<u8>,_>("signed_transaction"),
        "packet_hash":row.get::<String,_>("signed_sha256")
    })
}

#[tokio::test]
async fn sdk_recovery_context_captures_four_signed_stages_without_changing_original_evidence() {
    for name in ["signed", "broadcasting", "submitted", "unknown"] {
        let (c, key) = signing_case(None).await;
        let signed = save_signed(&c, &key).await;
        stage(&c, &signed, name).await;
        let before = snapshot(&c).await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        assert_eq!(captured.context.operation(), ClaimOperation::SdkTrade);
        assert_eq!(captured.context.signature(), signed.signature);
        assert_eq!(captured.context.wallet_pubkey(), key.pubkey().to_string());
        assert_eq!(snapshot(&c).await, before);
        c.store.read_observation(&captured.context).await.unwrap();
        assert_eq!(snapshot(&c).await, before);
        assert_eq!(
            c.f.claims().claim(&c.f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_recovery_context_never_recovers_a_signing_or_unsigned_preparation() {
    for name in ["claimed", "prepared", "signing", "failed", "rejected"] {
        let c = setup().await;
        if name != "claimed" {
            prepare(&c).await.unwrap();
        }
        match name {
            "signing" => begin(&c).await.unwrap(),
            "failed" => c
                .store
                .fail_sdk_before_sign(&c.f.execution, &c.owner, "synthetic")
                .await
                .unwrap(),
            "rejected" => c
                .store
                .reject_sdk_before_sign(&c.f.execution, &c.owner, "synthetic")
                .await
                .unwrap(),
            _ => (),
        }
        let before = snapshot(&c).await;
        assert!(
            c.store
                .capture_observation(&c.f.record.request_id)
                .await
                .is_err(),
            "{name}"
        );
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sdk_recovery_context_replays_original_defaults_and_literal_key_not_current_config() {
    let (mut c, key) = signing_case_with_defaults(None, true).await;
    save_signed(&c, &key).await;
    let before = snapshot(&c).await;
    c.f.state.edit_fixture_config(|config| {
        config.default_slippage_percent = 99.0;
        config.default_tip_sol = 10.0;
        config.enable_simulate = true;
    });
    let captured = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    assert_eq!(
        captured.context.signature(),
        before.0[7].as_deref().unwrap()
    );
    let key: String = sqlx::query_scalar("SELECT idempotency_key FROM trades WHERE request_id=?")
        .bind(&c.f.record.request_id)
        .fetch_one(&c.store.pool)
        .await
        .unwrap();
    assert_eq!(key, " sdk-candidate-literal ");
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_recovery_context_rejects_corrupt_original_request_and_signed_fields_without_writes() {
    for (table, column, value) in [
        ("trades", "idempotency_key", "trimmed"),
        ("trades", "request_hash", "bad-hash"),
        ("trades", "wallet_id", "other-wallet"),
        ("trades", "wallet_pubkey", "public-invalid"),
        ("trades", "mint", "public-invalid"),
        ("trades", "pool_id", "public-invalid"),
        ("trades", "execution_method", "pumpapi"),
        ("trades", "signature", "different"),
        ("trades", "status", "failed"),
        ("ordinary_execution_claims", "request_hash", "bad-hash"),
        ("ordinary_execution_claims", "owner_nonce", "bad-owner"),
        ("ordinary_execution_claims", "sdk_prepared_json", "{}"),
        ("ordinary_execution_claims", "unsigned_sha256", "bad-digest"),
        ("ordinary_execution_claims", "signed_signature", "different"),
        ("ordinary_execution_claims", "signed_sha256", "bad-digest"),
        ("ordinary_execution_claims", "final_observation_json", "{}"),
        ("ordinary_execution_claims", "operation", "ordinary-sol-buy"),
    ] {
        let (c, key) = signing_case(None).await;
        save_signed(&c, &key).await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        sqlx::query(&format!("UPDATE {table} SET {column}=? WHERE request_id=?"))
            .bind(value)
            .bind(&c.f.record.request_id)
            .execute(&c.store.pool)
            .await
            .unwrap();
        let before = snapshot(&c).await;
        assert!(
            c.store
                .capture_observation(&c.f.record.request_id)
                .await
                .is_err(),
            "{table}.{column}"
        );
        assert!(
            c.store.read_observation(&captured.context).await.is_err(),
            "{table}.{column}"
        );
        assert_eq!(snapshot(&c).await, before, "{table}.{column}");
    }
}

#[tokio::test]
async fn sdk_recovery_context_rejects_changed_request_intent_before_lookup_or_finality() {
    for (field, value) in [
        ("sell_percent", json!(50)),
        ("slippage_bps", json!(1234)),
        ("tip_sol", json!(0.2)),
        ("use_mev", json!(true)),
        ("mev_provider", json!("temporal")),
        ("simulate", json!(true)),
        ("amount_sol", json!(0.1)),
        ("settlement_asset", json!("sol")),
        ("sol_hop_pool", json!("raydium-cpmm")),
        ("quote_mint", json!(crate::domain::trade::USDC_MINT)),
    ] {
        let (c, key) = signing_case(None).await;
        let signed = save_signed(&c, &key).await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        let raw: String = sqlx::query_scalar("SELECT request_json FROM trades WHERE request_id=?")
            .bind(&c.f.record.request_id)
            .fetch_one(&c.store.pool)
            .await
            .unwrap();
        let mut parsed: Value = serde_json::from_str(&raw).unwrap();
        parsed[field] = value;
        sqlx::query("UPDATE trades SET request_json=? WHERE request_id=?")
            .bind(serde_json::to_string(&parsed).unwrap())
            .bind(&c.f.record.request_id)
            .execute(&c.store.pool)
            .await
            .unwrap();
        let before = snapshot(&c).await;
        let reader = Reader::new(Some(observation(
            &signed.signature,
            ChainCommitment::Finalized,
            None,
        )));
        assert!(
            c.store
                .capture_observation(&c.f.record.request_id)
                .await
                .is_err(),
            "{field}"
        );
        assert!(
            reconcile_captured(&c.store, &reader, &captured.context, Duration::from_secs(1))
                .await
                .is_err(),
            "{field}"
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sdk_recovery_context_freezes_valid_but_changed_owner_timestamp_and_raw_request_bytes() {
    for change in 0..3 {
        let (c, key) = signing_case(None).await;
        save_signed(&c, &key).await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        match change {
            0 => {
                sqlx::query(
                    "UPDATE ordinary_execution_claims SET owner_nonce=? WHERE request_id=?",
                )
                .bind("b".repeat(32))
                .bind(&c.f.record.request_id)
                .execute(&c.store.pool)
                .await
                .unwrap();
            }
            1 => {
                sqlx::query("UPDATE ordinary_execution_claims SET created_at_ms=created_at_ms+1 WHERE request_id=?")
                .bind(&c.f.record.request_id).execute(&c.store.pool).await.unwrap();
            }
            _ => {
                sqlx::query("UPDATE trades SET request_json=' '||request_json WHERE request_id=?")
                    .bind(&c.f.record.request_id)
                    .execute(&c.store.pool)
                    .await
                    .unwrap();
            }
        }
        let before = snapshot(&c).await;
        // These still describe a nominally valid row, but never the previously captured identity.
        c.store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        assert!(c.store.read_observation(&captured.context).await.is_err());
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sdk_recovery_finality_requires_matching_signature_nonzero_slot_and_finalized_commitment() {
    let (c, key) = signing_case(None).await;
    let signed = save_signed(&c, &key).await;
    let captured = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let before = snapshot(&c).await;
    for change in 0..5 {
        let mut obs = observation(&signed.signature, ChainCommitment::Finalized, None);
        match change {
            0 => obs.signature = solana_sdk::signature::Signature::new_unique().to_string(),
            1 => obs.slot = 0,
            2 => obs.commitment = ChainCommitment::Processed,
            3 => obs.commitment = ChainCommitment::Confirmed,
            _ => obs.error = Some("x".repeat(65_537)),
        }
        assert!(c
            .store
            .record_captured_finality(&captured.context, &obs)
            .await
            .is_err());
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sdk_recovery_finality_records_success_or_chain_failure_without_changing_original_packet() {
    for error in [
        None,
        Some("custom program error at https://rpc.invalid/private-key"),
    ] {
        let (c, key) = signing_case(None).await;
        let signed = save_signed(&c, &key).await;
        let before = immutable(&c.store.pool, &c.f.record.request_id).await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        let obs = observation(&signed.signature, ChainCommitment::Finalized, error);
        let record = c
            .store
            .record_captured_finality(&captured.context, &obs)
            .await
            .unwrap();
        assert_eq!(
            record.status,
            if error.is_some() {
                TradeStatus::Failed
            } else {
                TradeStatus::Confirmed
            }
        );
        assert_eq!(record.error, error.map(crate::sanitize::redact_urls));
        assert_eq!(record.signature.as_deref(), Some(signed.signature.as_str()));
        assert_eq!(
            immutable(&c.store.pool, &c.f.record.request_id).await,
            before
        );
        assert_eq!(
            c.store
                .read_observation(&captured.context)
                .await
                .unwrap()
                .status,
            record.status
        );
        c.store
            .record_captured_finality(&captured.context, &obs)
            .await
            .unwrap();
        let saved = snapshot(&c).await;
        let conflict = observation(
            &signed.signature,
            ChainCommitment::Finalized,
            Some("different outcome"),
        );
        assert!(c
            .store
            .record_captured_finality(&captured.context, &conflict)
            .await
            .is_err());
        assert!(c
            .store
            .take_sdk_broadcast(&c.f.execution, &c.owner, &signed)
            .await
            .is_err());
        assert!(c
            .store
            .fail_sdk_before_sign(&c.f.execution, &c.owner, "late error")
            .await
            .is_err());
        assert_eq!(
            c.store.claim(&c.f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        assert_eq!(snapshot(&c).await, saved);
    }
}

#[tokio::test]
async fn sdk_recovery_finality_rejects_damaged_finalized_metadata_and_unproved_terminal_parent() {
    for (field, value) in [
        ("schema_version", json!(2)),
        ("signature", json!("different")),
        ("slot", json!(0)),
        ("commitment", json!("confirmed")),
        ("unexpected", json!(true)),
        ("error", json!("https://rpc.invalid/private-key")),
    ] {
        let (c, key) = signing_case(None).await;
        let signed = save_signed(&c, &key).await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        c.store
            .record_captured_finality(
                &captured.context,
                &observation(&signed.signature, ChainCommitment::Finalized, None),
            )
            .await
            .unwrap();
        let raw: String = sqlx::query_scalar(
            "SELECT final_observation_json FROM ordinary_execution_claims WHERE request_id=?",
        )
        .bind(&c.f.record.request_id)
        .fetch_one(&c.store.pool)
        .await
        .unwrap();
        let mut parsed: Value = serde_json::from_str(&raw).unwrap();
        parsed[field] = value;
        sqlx::query(
            "UPDATE ordinary_execution_claims SET final_observation_json=? WHERE request_id=?",
        )
        .bind(serde_json::to_string(&parsed).unwrap())
        .bind(&c.f.record.request_id)
        .execute(&c.store.pool)
        .await
        .unwrap();
        let before = snapshot(&c).await;
        assert!(
            c.store
                .capture_observation(&c.f.record.request_id)
                .await
                .is_err(),
            "{field}"
        );
        assert!(
            c.store.read_observation(&captured.context).await.is_err(),
            "{field}"
        );
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sdk_recovery_finality_nonfinal_missing_or_failed_rpc_preserves_pending_original_signature()
{
    let (c, key) = signing_case(None).await;
    let signed = save_signed(&c, &key).await;
    let captured = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let before = snapshot(&c).await;
    for value in [
        None,
        Some(observation(
            &signed.signature,
            ChainCommitment::Processed,
            None,
        )),
        Some(observation(
            &signed.signature,
            ChainCommitment::Confirmed,
            Some("nonfinal chain error"),
        )),
    ] {
        let reader = Reader::new(value);
        let result =
            reconcile_captured(&c.store, &reader, &captured.context, Duration::from_secs(1))
                .await
                .unwrap();
        assert_eq!(
            result.record.signature.as_deref(),
            Some(signed.signature.as_str())
        );
        assert_eq!(snapshot(&c).await, before);
    }
    let mut reader = Reader::new(None);
    reader.fail = true;
    let result = reconcile_captured(&c.store, &reader, &captured.context, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(result.lookup_error.unwrap().contains("429"));
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_recovery_finality_timeout_drops_only_readonly_lookup_without_any_state_write() {
    let (c, key) = signing_case(None).await;
    save_signed(&c, &key).await;
    let captured = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let before = snapshot(&c).await;
    let mut reader = Reader::new(None);
    reader.pending = true;
    let result = reconcile_captured(
        &c.store,
        &reader,
        &captured.context,
        Duration::from_millis(5),
    )
    .await
    .unwrap();
    assert!(result.lookup_error.unwrap().contains("timed out"));
    assert!(reader.dropped.load(Ordering::SeqCst));
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_recovery_finality_parent_write_failure_rolls_back_all_finalized_evidence() {
    let (c, key) = signing_case(None).await;
    let signed = save_signed(&c, &key).await;
    let captured = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let before = snapshot(&c).await;
    sqlx::raw_sql("CREATE TRIGGER deny_final_parent BEFORE UPDATE OF status ON trades WHEN NEW.status IN ('confirmed','failed') BEGIN SELECT RAISE(ABORT,'synthetic final parent failure'); END;")
        .execute(&c.store.pool).await.unwrap();
    let obs = observation(&signed.signature, ChainCommitment::Finalized, None);
    assert!(c
        .store
        .record_captured_finality(&captured.context, &obs)
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
    sqlx::query("DROP TRIGGER deny_final_parent")
        .execute(&c.store.pool)
        .await
        .unwrap();
    assert_eq!(
        c.store
            .record_captured_finality(&captured.context, &obs)
            .await
            .unwrap()
            .status,
        TradeStatus::Confirmed
    );
}

#[tokio::test]
async fn sdk_recovery_finality_beats_late_receipt_without_reverting_terminal_result() {
    for error in [None, Some("finalized chain failure")] {
        let (c, key) = signing_case(None).await;
        let signed = save_signed(&c, &key).await;
        stage(&c, &signed, "broadcasting").await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        let obs = observation(&signed.signature, ChainCommitment::Finalized, error);
        let (finality, _receipt) = tokio::join!(
            c.store.record_captured_finality(&captured.context, &obs),
            c.store
                .record_sdk_broadcast(&c.f.execution, &c.owner, &signed, true, None)
        );
        let finalized = finality.unwrap();
        assert_eq!(
            finalized.status,
            if error.is_some() {
                TradeStatus::Failed
            } else {
                TradeStatus::Confirmed
            }
        );
        let before = snapshot(&c).await;
        assert!(c
            .store
            .record_sdk_broadcast(
                &c.f.execution,
                &c.owner,
                &signed,
                false,
                Some("late failure")
            )
            .await
            .is_err());
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sdk_recovery_context_file_reopen_drops_all_original_signing_objects_and_never_reexecutes()
{
    for name in ["signed", "broadcasting", "submitted", "unknown"] {
        let dir = std::env::temp_dir().join(format!("sdk-observe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let config = TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
            busy_timeout_ms: 2000,
        };
        let mut state = crate::http_contract_tests::state().await;
        state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
        let (c, key) = signing_case(Some(state)).await;
        let signed = save_signed(&c, &key).await;
        stage(&c, &signed, name).await;
        let id = c.f.record.request_id.clone();
        let signature = signed.signature.clone();
        let before = immutable(&c.store.pool, &id).await;
        c.store.pool.close().await;
        drop(signed);
        drop(key);
        drop(c);
        let restored = TradePersistence::new(&config).await.unwrap();
        let pool = restored.pool_for_tests();
        sqlx::raw_sql("CREATE TRIGGER no_recovery_owner BEFORE UPDATE OF owner_nonce ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'recovery cannot issue owner'); END; CREATE TRIGGER no_recovery_artifact BEFORE UPDATE OF signed_signature,signed_transaction,unsigned_message ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'recovery cannot replace artifacts'); END; CREATE TRIGGER no_recovery_claim BEFORE INSERT ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'recovery cannot execute'); END;")
            .execute(&pool).await.unwrap();
        let store = restored.ordinary_execution_claims().unwrap();
        let captured = store.capture_observation(&id).await.unwrap();
        let missing = Reader::new(None);
        reconcile_captured(&store, &missing, &captured.context, Duration::from_secs(1))
            .await
            .unwrap();
        let reader = Reader::new(Some(observation(
            &signature,
            ChainCommitment::Finalized,
            None,
        )));
        let finalized =
            reconcile_captured(&store, &reader, &captured.context, Duration::from_secs(1))
                .await
                .unwrap();
        assert_eq!(finalized.record.status, TradeStatus::Confirmed);
        assert_eq!(immutable(&pool, &id).await, before);
        assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
        pool.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
