use super::*;
use crate::{
    adapters::{
        persistence::ordinary_sdk_preparation::test_support::signed_observation,
        test_support::{
            read_rpc_server::{LocalServer, Reply},
            IsolatedTestProcess,
        },
    },
    config::{TradePersistenceConfig, TradePersistenceMode},
};
use serde_json::json;
use sqlx::Row;

async fn persistence() -> Arc<TradePersistence> {
    Arc::new(
        TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 2000,
        })
        .await
        .unwrap(),
    )
}
async fn evidence(
    pool: &sqlx::SqlitePool,
) -> Vec<(String, String, String, String, Vec<u8>, Vec<u8>)> {
    sqlx::query_as("SELECT request_id,owner_nonce,sdk_prepared_json,signed_signature,unsigned_message,signed_transaction FROM ordinary_execution_claims ORDER BY request_id")
        .fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn sdk_scan_worker_handles_every_original_signed_stage_and_never_looks_up_finalized_twice() {
    let p = persistence().await;
    let mut ids = Vec::new();
    for (i, stage) in ["signed", "broadcasting", "submitted", "unknown"]
        .into_iter()
        .enumerate()
    {
        ids.push(signed_observation(p.clone(), stage, 10 + i as i64).await);
    }
    let pool = p.pool_for_tests();
    let before = evidence(&pool).await;
    let reader = Arc::new(Reader::new(true, None));
    let mut scanner =
        OrdinaryObservationScanner::new(p.ordinary_execution_claims().unwrap(), reader.clone());
    for (examined, more) in [(2, false), (2, true)] {
        let report = scanner.step(i64::MAX, 2, Duration::ZERO).await.unwrap();
        assert_eq!(
            (
                report.examined,
                report.finalized,
                report.rejected,
                report.wrapped
            ),
            (examined, examined, 0, more)
        );
    }
    assert_eq!(
        scanner
            .step(i64::MAX, 2, Duration::ZERO)
            .await
            .unwrap()
            .examined,
        0
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 4);
    for identity in ids {
        let record = p.get(&identity.request_id).await.unwrap().unwrap();
        assert_eq!(record.status, TradeStatus::Confirmed);
        assert_eq!(
            record.signature.as_deref(),
            Some(identity.signature.as_str())
        );
    }
    assert_eq!(evidence(&pool).await, before);
}

#[tokio::test]
async fn sdk_scan_worker_rejects_damaged_candidate_without_rpc_and_continues_to_next_original() {
    let p = persistence().await;
    let bad = signed_observation(p.clone(), "broadcasting", 10).await;
    let good = signed_observation(p.clone(), "submitted", 11).await;
    sqlx::query("UPDATE ordinary_execution_claims SET sdk_prepared_json='{}' WHERE request_id=?")
        .bind(&bad.request_id)
        .execute(&p.pool_for_tests())
        .await
        .unwrap();
    let reader = Arc::new(Reader::new(true, None));
    let mut scanner =
        OrdinaryObservationScanner::new(p.ordinary_execution_claims().unwrap(), reader.clone());
    let report = scanner.step(i64::MAX, 8, Duration::ZERO).await.unwrap();
    assert_eq!(
        (report.examined, report.rejected, report.finalized),
        (2, 1, 1)
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        p.get(&bad.request_id).await.unwrap().unwrap().status,
        TradeStatus::Submitting
    );
    assert_eq!(
        p.get(&good.request_id).await.unwrap().unwrap().status,
        TradeStatus::Confirmed
    );
}

#[tokio::test]
async fn sdk_scan_worker_shutdown_cancels_only_original_signature_lookup_without_writing() {
    let p = persistence().await;
    let identity = signed_observation(p.clone(), "broadcasting", 10).await;
    let pool = p.pool_for_tests();
    let before = evidence(&pool).await;
    let reader = Arc::new(Reader::new(false, Some(1)));
    let worker = spawn_scanner(
        OrdinaryObservationScanner::new(p.ordinary_execution_claims().unwrap(), reader.clone()),
        fast_config(),
        now,
        Duration::from_secs(10),
    );
    tokio::time::timeout(Duration::from_secs(2), reader.entered.notified())
        .await
        .unwrap();
    worker.shutdown().await.unwrap();
    assert!(reader.dropped.load(Ordering::SeqCst));
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(evidence(&pool).await, before);
    assert_eq!(
        p.get(&identity.request_id).await.unwrap().unwrap().status,
        TradeStatus::Submitting
    );
}

#[tokio::test]
async fn sdk_scan_worker_reopens_file_and_finalizes_without_an_original_owner_or_wallet() {
    let dir = std::env::temp_dir().join(format!("sdk-worker-file-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2000,
    };
    let p = Arc::new(TradePersistence::new(&config).await.unwrap());
    let identity = signed_observation(p.clone(), "broadcasting", 10).await;
    let before = evidence(&p.pool_for_tests()).await;
    p.pool_for_tests().close().await;
    drop(p);
    let restored = TradePersistence::new(&config).await.unwrap();
    let pool = restored.pool_for_tests();
    sqlx::raw_sql("CREATE TRIGGER no_sdk_scan_claim BEFORE INSERT ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no recovered claim'); END; CREATE TRIGGER no_sdk_scan_artifact BEFORE UPDATE OF owner_nonce,unsigned_message,signed_signature,signed_transaction ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'no recovered signing or sending'); END;")
        .execute(&pool).await.unwrap();
    let reader = Arc::new(Reader::new(true, None));
    let mut scanner = OrdinaryObservationScanner::new(
        restored.ordinary_execution_claims().unwrap(),
        reader.clone(),
    );
    assert_eq!(
        scanner
            .step(i64::MAX, 8, Duration::ZERO)
            .await
            .unwrap()
            .finalized,
        1
    );
    assert_eq!(
        restored
            .get(&identity.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TradeStatus::Confirmed
    );
    assert_eq!(evidence(&pool).await, before);
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    drop(scanner);
    pool.close().await;
    drop(restored);
    let again = TradePersistence::new(&config).await.unwrap();
    let mut scanner =
        OrdinaryObservationScanner::new(again.ordinary_execution_claims().unwrap(), reader.clone());
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
    again.pool_for_tests().close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

const FLAG: &str = "SOL_BOT_SDK_SCAN_LOOPBACK_CASE";
const CHILD: &str =
    "adapters::ordinary_observation_worker::tests::sdk::sdk_scan_worker_loopback_child";
const DONE: &str = "SDK_SCAN_LOOPBACK_DONE";

#[test]
fn sdk_scan_worker_real_factory_shared_runtime_and_loopback_rpc_finalize_original_signature() {
    let child = IsolatedTestProcess::spawn(CHILD, FLAG, "run");
    child.await_line(DONE);
    child.finish();
}

#[tokio::test]
async fn sdk_scan_worker_loopback_child() {
    if std::env::var(FLAG).as_deref() != Ok("run") {
        return;
    }
    let p = persistence().await;
    let identity = signed_observation(p.clone(), "broadcasting", 10).await;
    let pool = p.pool_for_tests();
    let before = evidence(&pool).await;
    let server = LocalServer::new(Reply::ObservedSignature(identity.signature.clone())).await;
    let runtime = Arc::new(SharedReadRuntime::new(&[server.url.clone()]).unwrap());
    assert!(
        ObservationPlan::prepare(&Default::default(), &p, runtime.clone())
            .unwrap()
            .is_none()
    );
    assert_eq!(server.connections.load(Ordering::SeqCst), 0);
    let config = OrdinaryObservationConfig {
        enabled: true,
        batch_size: 1,
        poll_interval_ms: 1000,
        min_age_ms: 1000,
        request_interval_ms: 1000,
    };
    let worker = ObservationPlan::prepare(&config, &p, runtime)
        .unwrap()
        .unwrap()
        .start();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if p.get(&identity.request_id).await.unwrap().unwrap().status == TradeStatus::Confirmed
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    worker.shutdown().await.unwrap();
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(
        requests,
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"getSignatureStatuses","params":[[identity.signature],{"searchTransactionHistory":true}]})
        ]
    );
    assert_eq!(evidence(&pool).await, before);
    let stage: String =
        sqlx::query("SELECT submission_stage FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&identity.request_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("submission_stage");
    assert_eq!(stage, "broadcasting");
    server.finish().await;
    println!("{DONE}");
}
