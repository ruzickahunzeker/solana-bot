use super::*;
use crate::{
    adapters::sdk::{
        address_parser::SdkAddressParser, pump_native_message::assert_native_history_only,
    },
    application::{
        ports::ordinary_reconciliation::RecoveredOrdinaryObservations,
        trade_validation::TradeValidator,
    },
    config::TradePersistenceConfig,
    domain::pump_native_candidate::PumpNativeCandidateRecord,
    models::TradeRequest,
    persistence::TradePersistence,
};
use sol_trade_sdk::swqos::TradeType;

async fn install_history(c: &Case, json: &str, message: &[u8], digest: &str) {
    sqlx::query("UPDATE ordinary_execution_claims SET sdk_prepared_json=?,unsigned_message=?,unsigned_sha256=? WHERE request_id=?")
        .bind(json).bind(message).bind(digest).bind(&c.f.record.request_id)
        .execute(&c.store.pool).await.unwrap();
}

#[tokio::test]
async fn pump_native_history_original_codec_recognizes_nominal_data_but_claim_and_recovery_stay_closed(
) {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (f, native, message) =
                PumpNativeCandidateRecord::history_fixture(side, provider).await;
            let encoded = native.encode(&message).unwrap();
            assert_native_history_only(&encoded, &message);
            let decoded = SdkPreparedRecord::decode(&encoded, &message).unwrap();
            assert_eq!(decoded.wallet(), native.details().wallet);
            assert_eq!(decoded.encode(&message).unwrap(), encoded);
            assert!(validate_origin(&f.execution, &decoded, &message).is_err());
            assert!(f.claims().claim(&f.execution).await.is_err());
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM ordinary_execution_claims WHERE request_id=?",
            )
            .bind(&f.record.request_id)
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
            assert_eq!(count, 0);
            let json: String =
                sqlx::query_scalar("SELECT request_json FROM trades WHERE request_id=?")
                    .bind(&f.record.request_id)
                    .fetch_one(&f.state.persistence.pool_for_tests())
                    .await
                    .unwrap();
            let request: TradeRequest = serde_json::from_str(&json).unwrap();
            let error = TradeValidator::new(&SdkAddressParser)
                .replay_sdk_candidate(&request.validation_input(), &decoded)
                .err()
                .expect("historical Pump recovery unexpectedly enabled");
            assert!(error
                .to_string()
                .contains("historical Pump candidate recovery is not enabled"));
        }
    }
}

#[tokio::test]
async fn pump_native_history_raw_sqlite_readback_never_replaces_an_original_damm_owner_or_advances_signing(
) {
    let c = setup().await;
    prepare(&c).await.unwrap();
    let (_, native, message) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let encoded = native.encode(&message).unwrap();
    install_history(&c, &encoded, &message, &sha(&message)).await;
    let before = snapshot(&c).await;
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id)
        .fetch_one(&c.store.pool)
        .await
        .unwrap();
    let stored = validate_sdk_row(&row).unwrap().unwrap();
    assert_eq!(stored.message, message);
    assert_eq!(stored.metadata.encode(&message).unwrap(), encoded);
    assert!(validate_origin(&c.f.execution, &stored.metadata, &message).is_err());
    assert!(c
        .store
        .read_sdk_candidate(&c.f.execution, &c.owner)
        .await
        .is_err());
    assert!(c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .is_err());
    assert!(c.f.claims().claim(&c.f.execution).await.is_err());
    assert_eq!(snapshot(&c).await, before);
    // Both mutating entry points must reject the foreign profile before touching SQL.
    c.store.pool.close().await;
    for result in [
        c.store
            .prepare_sdk_candidate(&c.f.execution, &c.owner, &stored.metadata, &message)
            .await,
        c.store
            .begin_sdk_signing(&c.f.execution, &c.owner, &stored.metadata, &message)
            .await,
    ] {
        assert!(result.unwrap_err().to_string().contains("not DAMM sell"));
    }
}

#[tokio::test]
async fn pump_native_history_original_row_validator_rejects_bad_tags_partial_hashes_and_rehashed_wire(
) {
    let c = setup().await;
    prepare(&c).await.unwrap();
    let (_, native, message) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Temporal).await;
    let encoded = native.encode(&message).unwrap();
    for damage in 0..6 {
        let mut json = encoded.clone();
        let mut bytes = message.clone();
        let mut digest = sha(&bytes);
        match damage {
            0 => json = encoded.replacen("\"version\":1", "\"version\":2", 1),
            1 => json = encoded.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
            2 => json = format!("{{\"unknown\":true,{}", &encoded[1..]),
            3 => digest = "0".repeat(64),
            4 => {
                bytes[0] ^= 1;
                digest = sha(&bytes);
            }
            _ => {
                let mut wire: solana_sdk::message::VersionedMessage =
                    bincode::deserialize(&bytes).unwrap();
                let solana_sdk::message::VersionedMessage::Legacy(ref mut legacy) = wire else {
                    unreachable!()
                };
                legacy.instructions.last_mut().unwrap().data[8] ^= 1;
                bytes = wire.serialize();
                let rehashed =
                    PumpNativeCandidateRecord::new(native.details().clone(), &bytes).unwrap();
                json = rehashed.encode(&bytes).unwrap();
                digest = sha(&bytes);
            }
        }
        install_history(&c, &json, &bytes, &digest).await;
        let before = snapshot(&c).await;
        let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&c.f.record.request_id)
            .fetch_one(&c.store.pool)
            .await
            .unwrap();
        assert!(validate_sdk_row(&row).is_err(), "damage {damage}");
        assert!(c
            .store
            .read_sdk_candidate(&c.f.execution, &c.owner)
            .await
            .is_err());
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn pump_native_history_file_reopen_preserves_original_columns_without_restoring_an_execution_owner(
) {
    let dir = std::env::temp_dir().join(format!("native-history-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2_000,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let f = admit_sdk(state).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        unreachable!()
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let c = Case {
        store: SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests()),
        f,
        owner,
        candidate: candidate_fixture(SwqosType::Default, false, 100),
    };
    prepare(&c).await.unwrap();
    let (_, native, message) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Sell, SwqosType::Default).await;
    let encoded = native.encode(&message).unwrap();
    install_history(&c, &encoded, &message, &sha(&message)).await;
    let before = snapshot(&c).await;
    c.store.pool.close().await;
    let restored = TradePersistence::new(&config).await.unwrap();
    let journal = restored.ordinary_execution_claims().unwrap();
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id)
        .fetch_one(&journal.pool)
        .await
        .unwrap();
    let candidate = validate_sdk_row(&row).unwrap().unwrap();
    assert_eq!(candidate.metadata.encode(&message).unwrap(), encoded);
    assert_eq!(candidate.message, message);
    assert_eq!(row.get::<&str, _>("submission_stage"), "prepared");
    assert_eq!(row.get::<Option<String>, _>("owner_nonce"), before.0[1]);
    assert!(row.get::<Option<String>, _>("signed_signature").is_none());
    assert!(row
        .get::<Option<Vec<u8>>, _>("signed_transaction")
        .is_none());
    assert!(journal.claim(&c.f.execution).await.is_err());
    assert!(journal
        .read_sdk_candidate(&c.f.execution, &c.owner)
        .await
        .is_err());
    assert!(journal
        .begin_sdk_signing(&c.f.execution, &c.owner, &candidate.metadata, &message)
        .await
        .is_err());
    assert!(journal
        .capture_observation(&c.f.record.request_id)
        .await
        .is_err());
    let row = sqlx::query("SELECT sdk_prepared_json,unsigned_message,signed_signature FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id).fetch_one(&journal.pool).await.unwrap();
    assert_eq!(row.get::<&str, _>("sdk_prepared_json"), encoded);
    assert_eq!(row.get::<Vec<u8>, _>("unsigned_message"), message);
    assert!(row.get::<Option<String>, _>("signed_signature").is_none());
    journal.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
