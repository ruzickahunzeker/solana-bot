use super::*;
use crate::adapters::sdk::damm_probe::test_support::buy_record_fixture;
use crate::application::ports::ordinary_reconciliation::RecoveredOrdinaryObservations;

#[tokio::test]
async fn sdk_prepared_record_wrong_direction_rejects_before_database_access() {
    let c = setup().await;
    let (buy, message) = buy_record_fixture().await;
    let record = SdkPreparedRecord::DammBuy(buy);
    record.validate(&message).unwrap();
    c.store.pool.close().await;
    for result in [
        c.store
            .prepare_sdk_candidate(&c.f.execution, &c.owner, &record, &message)
            .await,
        c.store
            .begin_sdk_signing(&c.f.execution, &c.owner, &record, &message)
            .await,
    ] {
        // A closed pool would report an acquisition error if either path had reached SQL.
        assert!(result.unwrap_err().to_string().contains("not DAMM sell"));
    }
}

#[tokio::test]
async fn sdk_prepared_record_buy_cannot_replace_prepared_sell_or_advance_signing_cas() {
    let c = setup().await;
    prepare(&c).await.unwrap();
    let before = snapshot(&c).await;
    let (buy, message) = buy_record_fixture().await;
    let record = SdkPreparedRecord::DammBuy(buy);
    assert!(c
        .store
        .prepare_sdk_candidate(&c.f.execution, &c.owner, &record, &message)
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
    assert!(c
        .store
        .begin_sdk_signing(&c.f.execution, &c.owner, &record, &message)
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
    let stored = c
        .store
        .read_sdk_candidate(&c.f.execution, &c.owner)
        .await
        .unwrap();
    assert_eq!(stored.metadata, *c.candidate.metadata());
    assert_eq!(stored.message, c.candidate.message_bytes());
}

#[tokio::test]
async fn sdk_prepared_record_corrupt_buy_row_does_not_become_sell_recovery_or_owner() {
    let c = setup().await;
    prepare(&c).await.unwrap();
    let (buy, message) = buy_record_fixture().await;
    let json = buy.encode(&message).unwrap();
    assert!(matches!(
        SdkPreparedRecord::decode(&json, &message).unwrap(),
        SdkPreparedRecord::DammBuy(_)
    ));
    sqlx::query("UPDATE ordinary_execution_claims SET sdk_prepared_json=?,unsigned_message=?,unsigned_sha256=? WHERE request_id=?")
        .bind(json).bind(&message).bind(sha(&message)).bind(&c.f.record.request_id)
        .execute(&c.store.pool).await.unwrap();
    let before = snapshot(&c).await;
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id)
        .fetch_one(&c.store.pool)
        .await
        .unwrap();
    // A valid nominal buy can now reconstruct, but cannot bind to this sell parent or owner.
    assert!(validate_sdk_row(&row).unwrap().is_some());
    assert!(validate_origin(
        &c.f.execution,
        &SdkPreparedRecord::decode(row.get("sdk_prepared_json"), &message).unwrap(),
        &message
    )
    .is_err());
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
    assert_eq!(snapshot(&c).await, before);
    assert!(c.f.claims().claim(&c.f.execution).await.is_err());
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_prepared_record_sell_journal_stores_exact_legacy_json_not_an_enum_envelope() {
    let c = setup().await;
    let message = c.candidate.message_bytes();
    let original = c
        .candidate
        .metadata()
        .sell()
        .unwrap()
        .encode(&message)
        .unwrap();
    prepare(&c).await.unwrap();
    let json: String = sqlx::query_scalar(
        "SELECT sdk_prepared_json FROM ordinary_execution_claims WHERE request_id=?",
    )
    .bind(&c.f.record.request_id)
    .fetch_one(&c.store.pool)
    .await
    .unwrap();
    assert_eq!(json.as_bytes(), original.as_bytes());
    assert_eq!(
        SdkPreparedRecord::decode(&json, &message).unwrap(),
        *c.candidate.metadata()
    );
    let stored = c
        .store
        .read_sdk_candidate(&c.f.execution, &c.owner)
        .await
        .unwrap();
    assert_eq!(
        stored.metadata.sell().unwrap(),
        c.candidate.metadata().sell().unwrap()
    );
}
