use super::*;
use crate::{models::TradeRequest, state::TradeStore, validation::validate_trade};

#[tokio::test]
async fn ordinary_persistence_snapshot_bytes_before_extraction() {
    let state = crate::http_contract_tests::state().await;
    let mut accepted_config = (*state.config).clone();
    accepted_config.stonkfun_via_sol.enabled = true;
    accepted_config.default_tip_sol = 0.0001;
    accepted_config.pumpapi.default_priority_fee_sol = 0.0002;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/contracts/trade_normalization_v1.json"
    ))
    .unwrap();
    let persistence = TradePersistence::new(&TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: "sqlite::memory:".into(),
        busy_timeout_ms: 100,
    })
    .await
    .unwrap();
    let trades = TradeStore::default();
    let mut rows = Vec::new();
    for (category, config) in [
        ("accepted", &accepted_config),
        ("rejected", state.config.as_ref()),
    ] {
        for (index, case) in fixture[category].as_array().unwrap().iter().enumerate() {
            let mut value = json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111","idempotency_key":format!("snapshot-{category}-{index}")});
            value
                .as_object_mut()
                .unwrap()
                .extend(case["patch"].as_object().unwrap().clone());
            let request: TradeRequest = serde_json::from_value(value).unwrap();
            let mut record = trades.create_with_id(
                format!("tr_snapshot_{category}_{index}"),
                &request,
                "fixture-wallet".into(),
                "11111111111111111111111111111111".into(),
            );
            if category == "accepted" {
                let normalized = validate_trade(&request, config).unwrap();
                assert!(matches!(
                    persistence
                        .reserve(&record, &request, &normalized)
                        .await
                        .unwrap(),
                    DurableReserve::Created
                ));
            } else {
                let error = validate_trade(&request, config)
                    .err()
                    .expect("fixture must still reject");
                record.status = TradeStatus::Rejected;
                record.error = Some(error.to_string());
                persistence.mirror(&record, &request, None).await.unwrap();
            }
            let row = sqlx::query(
                "SELECT idempotency_key, request_hash, request_json FROM trades WHERE request_id=?",
            )
            .bind(&record.request_id)
            .fetch_one(persistence.pool.as_ref().unwrap())
            .await
            .unwrap();
            let request_json: String = row.get("request_json");
            assert_eq!(request_json, serde_json::to_string(&request).unwrap());
            let key: String = row.get("idempotency_key");
            assert_eq!(
                key,
                if category == "accepted" {
                    request.idempotency_key.clone()
                } else {
                    format!("__rejected__:{}", record.request_id)
                }
            );
            rows.push(json!({"category":category,"case":case["name"],"key":key,"hash":row.get::<String,_>("request_hash"),"request_json":request_json}));
        }
    }
    assert_eq!(rows.len(), 37);
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&rows).unwrap()));
    assert_eq!(
        hash,
        "d322ae4c28b37aae3bd3db33c136fec523e75792a65d997d25e1da8f015e8653"
    );
}

async fn typed_fixture(
    mode: TradePersistenceMode,
) -> (TradePersistence, TradeRequest, NormalizedTrade, TradeRecord) {
    let state = crate::http_contract_tests::state().await;
    let request: TradeRequest = serde_json::from_value(json!({
        "side":"buy","pool":"pump","mint":"11111111111111111111111111111111",
        "amount_sol":0.001,"idempotency_key":" literal-shared-key ","simulate":false,
    }))
    .unwrap();
    let normalized = validate_trade(&request, &state.config).unwrap();
    let record = TradeStore::default().create_with_id(
        "tr_typed_core".into(),
        &request,
        "fixture-wallet".into(),
        "11111111111111111111111111111111".into(),
    );
    let persistence = TradePersistence::new(&TradePersistenceConfig {
        mode,
        database_url: "sqlite::memory:".into(),
        busy_timeout_ms: 100,
    })
    .await
    .unwrap();
    (persistence, request, normalized, record)
}

async fn count(persistence: &TradePersistence) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM trades")
        .fetch_one(persistence.pool.as_ref().unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn validated_snapshot_cannot_be_reserved_or_mirrored_for_another_wallet() {
    use crate::adapters::persistence::trade_snapshot::validated;
    let (persistence, request, normalized, record) =
        typed_fixture(TradePersistenceMode::Durable).await;
    let input = validated("another-wallet", &request, &normalized).unwrap();
    assert_eq!(
        persistence
            .reserve_input(&record, &input)
            .await
            .err()
            .unwrap()
            .to_string(),
        "trade snapshot wallet does not match record"
    );
    assert_eq!(
        persistence
            .mirror_validated(&record, input)
            .await
            .unwrap_err()
            .to_string(),
        "trade snapshot wallet does not match record"
    );
    assert_eq!(count(&persistence).await, 0);
}

#[tokio::test]
async fn rejected_snapshot_is_bound_to_both_wallet_and_rejected_request_id() {
    use crate::adapters::persistence::trade_snapshot::rejected;
    let (persistence, request, _, record) = typed_fixture(TradePersistenceMode::Durable).await;
    for (wallet, id) in [
        ("another-wallet", record.request_id.as_str()),
        (record.wallet_id.as_str(), "another-request"),
    ] {
        let input = rejected(wallet, id, &request).unwrap();
        assert_eq!(
            persistence
                .mirror_rejected(&record, input)
                .await
                .unwrap_err()
                .to_string(),
            "rejected trade snapshot does not match record"
        );
    }
    assert_eq!(count(&persistence).await, 0);
}

#[tokio::test]
async fn typed_rejected_record_does_not_reserve_the_users_valid_trade_key() {
    use crate::adapters::persistence::trade_snapshot::{rejected, validated};
    let (persistence, request, normalized, record) =
        typed_fixture(TradePersistenceMode::Durable).await;
    let mut rejected_request = request.clone();
    rejected_request.amount_sol = Some(-1.0);
    let mut rejected_record = record.clone();
    rejected_record.request_id = "tr_rejected_core".into();
    rejected_record.status = TradeStatus::Rejected;
    persistence
        .mirror_rejected(
            &rejected_record,
            rejected(
                &record.wallet_id,
                &rejected_record.request_id,
                &rejected_request,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let input = validated(&record.wallet_id, &request, &normalized).unwrap();
    assert!(matches!(
        persistence.reserve_input(&record, &input).await.unwrap(),
        DurableReserve::Created
    ));
    assert_eq!(count(&persistence).await, 2);
    let key: String = sqlx::query_scalar("SELECT idempotency_key FROM trades WHERE request_id=?")
        .bind(&rejected_record.request_id)
        .fetch_one(persistence.pool.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(key, "__rejected__:tr_rejected_core");
    assert!(
        matches!(persistence.reserve_input(&record, &input).await.unwrap(), DurableReserve::Duplicate(r) if r.request_id == record.request_id)
    );
}

#[tokio::test]
async fn typed_alias_replay_keeps_original_snapshot_bytes_without_rewriting_metadata() {
    use crate::adapters::persistence::trade_snapshot::validated;
    let (persistence, request, normalized, record) =
        typed_fixture(TradePersistenceMode::Durable).await;
    let input = validated(&record.wallet_id, &request, &normalized).unwrap();
    assert!(matches!(
        persistence.reserve_input(&record, &input).await.unwrap(),
        DurableReserve::Created
    ));
    let mut alias = request.clone();
    alias.amount_sol = None;
    alias.amount_lamports = Some(1_000_000);
    alias.source = Some("different-metadata".into());
    let state = crate::http_contract_tests::state().await;
    let alias_input = validated(
        &record.wallet_id,
        &alias,
        &validate_trade(&alias, &state.config).unwrap(),
    )
    .unwrap();
    assert_eq!(input.snapshot().hash(), alias_input.snapshot().hash());
    assert_ne!(input.snapshot().json(), alias_input.snapshot().json());
    let mut retry = record.clone();
    retry.request_id = "tr_alias_attempt".into();
    assert!(
        matches!(persistence.reserve_input(&retry, &alias_input).await.unwrap(), DurableReserve::Duplicate(r) if r.request_id == record.request_id)
    );
    let stored: String = sqlx::query_scalar("SELECT request_json FROM trades WHERE request_id=?")
        .bind(&record.request_id)
        .fetch_one(persistence.pool.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(stored, input.snapshot().json());
    assert_eq!(count(&persistence).await, 1);
}

#[tokio::test]
async fn memory_skips_snapshot_binding_and_storage_availability_precedes_binding_for_shadow() {
    use crate::adapters::persistence::trade_snapshot::{rejected, validated};
    let (memory, request, normalized, record) = typed_fixture(TradePersistenceMode::Memory).await;
    let input = validated("another-wallet", &request, &normalized).unwrap();
    assert!(matches!(
        memory.reserve_input(&record, &input).await.unwrap(),
        DurableReserve::Created
    ));
    memory.mirror_validated(&record, input).await.unwrap();
    memory
        .mirror_rejected(
            &record,
            rejected("another-wallet", "another-id", &request).unwrap(),
        )
        .await
        .unwrap();
    let unavailable = TradePersistence::unavailable_shadow_for_tests();
    assert_eq!(
        unavailable
            .mirror_validated(
                &record,
                validated("another-wallet", &request, &normalized).unwrap()
            )
            .await
            .unwrap_err()
            .to_string(),
        "trade database is unavailable"
    );
    assert_eq!(
        unavailable
            .mirror(&record, &request, Some(&normalized))
            .await
            .unwrap_err()
            .to_string(),
        "trade database is unavailable"
    );
}

#[tokio::test]
async fn shadow_writer_consumes_typed_snapshots_and_preserves_both_storage_keys() {
    use crate::adapters::persistence::trade_snapshot::{rejected, validated};
    let (persistence, request, normalized, record) =
        typed_fixture(TradePersistenceMode::Shadow).await;
    persistence
        .mirror_validated(
            &record,
            validated(&record.wallet_id, &request, &normalized).unwrap(),
        )
        .await
        .unwrap();
    let mut rejected_record = record.clone();
    rejected_record.request_id = "tr_shadow_rejected".into();
    rejected_record.status = TradeStatus::Rejected;
    persistence
        .mirror_rejected(
            &rejected_record,
            rejected(&record.wallet_id, &rejected_record.request_id, &request).unwrap(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if count(&persistence).await == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT idempotency_key FROM trades ORDER BY idempotency_key")
            .fetch_all(persistence.pool.as_ref().unwrap())
            .await
            .unwrap();
    assert_eq!(
        keys,
        vec![
            request.idempotency_key.clone(),
            "__rejected__:tr_shadow_rejected".to_string()
        ]
    );
    let json: String = sqlx::query_scalar("SELECT request_json FROM trades WHERE request_id=?")
        .bind(&record.request_id)
        .fetch_one(persistence.pool.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(json, serde_json::to_string(&request).unwrap());
}
