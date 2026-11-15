use super::*;
use crate::application::ordinary_observation_scan::OrdinaryObservationScanner;

type OriginalEvidence = (
    String,
    String,
    String,
    String,
    String,
    Vec<u8>,
    Vec<u8>,
    String,
    String,
    i64,
);

async fn original_evidence(pool: &SqlitePool) -> OriginalEvidence {
    sqlx::query_as("SELECT t.request_hash,t.request_json,c.owner_nonce,c.bound_quote_json,c.preflight_json,c.unsigned_message,c.signed_transaction,c.signed_signature,c.submission_stage,c.created_at_ms FROM trades t JOIN ordinary_execution_claims c USING(request_id)")
        .fetch_one(pool).await.unwrap()
}

async fn restored_fixture(
    stage: &str,
) -> (
    TradePersistence,
    TradePersistenceConfig,
    PathBuf,
    Events,
    String,
) {
    let (f, owner, mut backend, journal, config, dir) = file_setup().await;
    match stage {
        "signed" => stage_signed(&f, &owner, &backend, &journal, SIGNATURE).await,
        "submitted" | "unknown" => {
            if stage == "unknown" {
                backend.failure = Some("broadcast");
            }
            execute(&journal, &backend, &backend, &f.execution, &owner).await;
        }
        _ => unreachable!(),
    }
    let id = f.record.request_id.clone();
    let events = backend.events.clone();
    journal.pool.close().await;
    drop(journal);
    drop(backend);
    drop(owner);
    drop(f);

    let restored = TradePersistence::new(&config).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER deny_scan_new_claim BEFORE INSERT ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no recovered claim'); END; CREATE TRIGGER deny_scan_new_trade BEFORE INSERT ON trades BEGIN SELECT RAISE(ABORT,'no recovered trade'); END; CREATE TRIGGER deny_scan_replacement BEFORE UPDATE OF owner_nonce,unsigned_message,signed_signature,signed_transaction ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no recovered execution authority'); END;")
        .execute(&restored.pool_for_tests()).await.unwrap();
    (restored, config, dir, events, id)
}

#[tokio::test]
async fn ordinary_scan_reopened_signed_submitted_and_unknown_finalize_only_original_evidence() {
    for stage in ["signed", "submitted", "unknown"] {
        for failed in [false, true] {
            let (restored, config, dir, events, id) = restored_fixture(stage).await;
            let original = original_evidence(&restored.pool_for_tests()).await;
            let before_events = events.lock().unwrap().clone();
            let reader = Arc::new(Reader::new(Answer::Found(observation(
                ChainCommitment::Finalized,
                failed.then_some("synthetic chain error"),
            ))));
            let mut scanner = OrdinaryObservationScanner::new(
                restored.ordinary_execution_claims().unwrap(),
                reader.clone(),
            );
            let report = scanner.step(i64::MAX, 8, Duration::ZERO).await.unwrap();
            assert_eq!(
                (
                    report.examined,
                    report.finalized,
                    report.rejected,
                    report.wrapped
                ),
                (1, 1, 0, true),
                "{stage} failed={failed}"
            );
            let expected = if failed {
                TradeStatus::Failed
            } else {
                TradeStatus::Confirmed
            };
            let record = restored.get(&id).await.unwrap().unwrap();
            assert_eq!(record.status, expected);
            assert_eq!(record.signature.as_deref(), Some(SIGNATURE));
            assert_eq!(
                record.error.as_deref(),
                failed.then_some("synthetic chain error")
            );
            assert_eq!(
                original_evidence(&restored.pool_for_tests()).await,
                original
            );
            assert_eq!(
                scanner
                    .step(i64::MAX, 8, Duration::ZERO)
                    .await
                    .unwrap()
                    .examined,
                0
            );
            assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
            drop(scanner);
            restored.pool_for_tests().close().await;
            drop(restored);

            // Another restart must not turn a finalized parent back into a pending observation.
            let again = TradePersistence::new(&config).await.unwrap();
            let mut scanner = OrdinaryObservationScanner::new(
                again.ordinary_execution_claims().unwrap(),
                reader.clone(),
            );
            assert_eq!(again.get(&id).await.unwrap().unwrap().status, expected);
            assert_eq!(
                scanner
                    .step(i64::MAX, 8, Duration::ZERO)
                    .await
                    .unwrap()
                    .examined,
                0
            );
            assert_eq!(original_evidence(&again.pool_for_tests()).await, original);
            assert_eq!(*events.lock().unwrap(), before_events);
            assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
            drop(scanner);
            again.pool_for_tests().close().await;
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[tokio::test]
async fn ordinary_scan_reopened_missing_or_unavailable_status_preserves_pending_without_new_send() {
    for stage in ["signed", "submitted", "unknown"] {
        for unavailable in [false, true] {
            let (restored, _, dir, events, id) = restored_fixture(stage).await;
            let original = original_evidence(&restored.pool_for_tests()).await;
            let before = serde_json::to_value(restored.get(&id).await.unwrap()).unwrap();
            let before_events = events.lock().unwrap().clone();
            let reader = Arc::new(Reader::new(if unavailable {
                Answer::Error
            } else {
                Answer::Missing
            }));
            let mut scanner = OrdinaryObservationScanner::new(
                restored.ordinary_execution_claims().unwrap(),
                reader.clone(),
            );
            for _ in 0..2 {
                let report = scanner.step(i64::MAX, 8, Duration::ZERO).await.unwrap();
                assert_eq!(
                    (
                        report.examined,
                        report.pending,
                        report.finalized,
                        report.rejected,
                        report.unavailable,
                        report.wrapped
                    ),
                    (1, 1, 0, 0, usize::from(unavailable), true)
                );
                assert_eq!(
                    serde_json::to_value(restored.get(&id).await.unwrap()).unwrap(),
                    before
                );
                assert_eq!(
                    original_evidence(&restored.pool_for_tests()).await,
                    original
                );
                assert_eq!(*events.lock().unwrap(), before_events);
            }
            assert_eq!(reader.calls.load(Ordering::SeqCst), 2);
            drop(scanner);
            restored.pool_for_tests().close().await;
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
