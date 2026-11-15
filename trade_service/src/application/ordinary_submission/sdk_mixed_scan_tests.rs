use super::*;
use crate::{
    adapters::persistence::ordinary_sdk_preparation::test_support::signed_observation,
    application::ordinary_observation_scan::OrdinaryObservationScanner,
};

#[tokio::test]
async fn sdk_scan_mixed_real_legacy_and_sdk_records_share_one_cursor_without_reexecuting_either() {
    struct MixedReader(std::sync::Mutex<Vec<String>>);
    impl SignatureStatusReader for MixedReader {
        async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
            self.0.lock().unwrap().push(signature.into());
            Ok(Some(ChainObservation {
                signature: signature.into(),
                slot: 123,
                commitment: ChainCommitment::Finalized,
                error: None,
            }))
        }
    }
    let (f, owner, backend, journal, config, dir) = file_setup().await;
    stage_signed(&f, &owner, &backend, &journal, SIGNATURE).await;
    let persistence = f.state.persistence.clone();
    let pool = persistence.pool_for_tests();
    let sdk = signed_observation(persistence.clone(), "broadcasting", 2).await;
    let legacy_id = f.record.request_id.clone();
    sqlx::query("UPDATE ordinary_execution_claims SET created_at_ms=1 WHERE request_id=?")
        .bind(&legacy_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE trades SET updated_at_ms=1 WHERE request_id=?")
        .bind(&legacy_id)
        .execute(&pool)
        .await
        .unwrap();
    let query = "SELECT request_id,operation,owner_nonce,unsigned_message,signed_transaction,signed_signature FROM ordinary_execution_claims ORDER BY request_id";
    let before: Vec<(String, String, String, Vec<u8>, Vec<u8>, String)> =
        sqlx::query_as(query).fetch_all(&pool).await.unwrap();
    let events = backend.events.clone();
    let original_events = events.lock().unwrap().clone();
    drop(journal);
    drop(backend);
    drop(owner);
    drop(f);
    let reader = Arc::new(MixedReader(std::sync::Mutex::new(Vec::new())));
    let mut scanner = OrdinaryObservationScanner::new(
        persistence.ordinary_execution_claims().unwrap(),
        reader.clone(),
    );
    for wrapped in [false, true] {
        let report = scanner.step(i64::MAX, 1, Duration::ZERO).await.unwrap();
        assert_eq!(
            (
                report.examined,
                report.finalized,
                report.rejected,
                report.wrapped
            ),
            (1, 1, 0, wrapped)
        );
    }
    assert_eq!(
        *reader.0.lock().unwrap(),
        [SIGNATURE.to_string(), sdk.signature]
    );
    assert_eq!(
        persistence.get(&legacy_id).await.unwrap().unwrap().status,
        TradeStatus::Confirmed
    );
    assert_eq!(
        persistence
            .get(&sdk.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TradeStatus::Confirmed
    );
    let after: Vec<(String, String, String, Vec<u8>, Vec<u8>, String)> =
        sqlx::query_as(query).fetch_all(&pool).await.unwrap();
    assert_eq!(after, before);
    assert_eq!(*events.lock().unwrap(), original_events);
    assert_eq!(
        scanner
            .step(i64::MAX, 1, Duration::ZERO)
            .await
            .unwrap()
            .examined,
        0
    );
    drop(scanner);
    pool.close().await;
    drop(persistence);
    let again = TradePersistence::new(&config).await.unwrap();
    assert_eq!(
        again.get(&legacy_id).await.unwrap().unwrap().status,
        TradeStatus::Confirmed
    );
    assert_eq!(
        again.get(&sdk.request_id).await.unwrap().unwrap().status,
        TradeStatus::Confirmed
    );
    again.pool_for_tests().close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
