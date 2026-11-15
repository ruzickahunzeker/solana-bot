use super::*;
use crate::domain::trade::{ExecutionMethod, Pool, Side};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

enum Read {
    Row(TradeRecordV1),
    Missing,
    Error,
    Delayed(TradeRecordV1),
    Blocked(Arc<AtomicBool>),
}

struct Source {
    reads: Mutex<VecDeque<Read>>,
    ids: Mutex<Vec<String>>,
    limits: Mutex<Vec<usize>>,
    list_error: bool,
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl TradeRecordReadSource for Source {
    async fn read_record(&self, id: &str) -> anyhow::Result<Option<TradeRecordV1>> {
        self.ids.lock().unwrap().push(id.into());
        let next = self
            .reads
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra read");
        match next {
            Read::Row(row) => Ok(Some(row)),
            Read::Missing => Ok(None),
            Read::Error => anyhow::bail!("synthetic read failure"),
            Read::Delayed(row) => {
                sleep(Duration::from_millis(25)).await;
                Ok(Some(row))
            }
            Read::Blocked(flag) => {
                let _drop = DropFlag(flag);
                std::future::pending().await
            }
        }
    }

    async fn recent_records(&self, limit: usize) -> anyhow::Result<Vec<TradeRecordV1>> {
        self.limits.lock().unwrap().push(limit);
        if self.list_error {
            anyhow::bail!("synthetic list failure");
        }
        Ok(vec![record(TradeStatus::Unknown)])
    }
}

fn record(status: TradeStatus) -> TradeRecordV1 {
    TradeRecordV1 {
        request_id: " literal request ".into(),
        wallet_id: "fixture-wallet".into(),
        wallet_pubkey: "11111111111111111111111111111111".into(),
        side: Side::Buy,
        pool: Pool::Pump,
        mint: "11111111111111111111111111111111".into(),
        pool_id: None,
        execution_method: ExecutionMethod::Sdk,
        status,
        signature: Some("synthetic-original-signature".into()),
        error: Some("literal record metadata".into()),
        duplicate: true,
        created_at_ms: 17,
        updated_at_ms: 23,
    }
}

fn service(reads: impl IntoIterator<Item = Read>) -> TradeQueryService<Source> {
    TradeQueryService::new(Source {
        reads: Mutex::new(reads.into_iter().collect()),
        ids: Mutex::new(vec![]),
        limits: Mutex::new(vec![]),
        list_error: false,
    })
}

fn statuses() -> [TradeStatus; 10] {
    [
        TradeStatus::Accepted,
        TradeStatus::WarmingParams,
        TradeStatus::BuildingTx,
        TradeStatus::Submitting,
        TradeStatus::Simulated,
        TradeStatus::Submitted,
        TradeStatus::Confirmed,
        TradeStatus::Failed,
        TradeStatus::Rejected,
        TradeStatus::Unknown,
    ]
}

#[test]
fn query_pending_policy_keeps_all_ten_legacy_statuses() {
    for (i, status) in statuses().into_iter().enumerate() {
        assert_eq!(is_pending_status(status), i < 4);
    }
}

#[tokio::test]
async fn get_keeps_literal_ids_full_rows_missing_and_original_errors() {
    let expected = record(TradeStatus::Unknown);
    let service = service([Read::Row(expected.clone()), Read::Missing, Read::Error]);
    let result = service.get(" literal request ").await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert!(service.get("").await.unwrap().is_none());
    assert_eq!(
        service.get("not a uuid").await.unwrap_err().to_string(),
        "synthetic read failure"
    );
    assert_eq!(
        *service.source.ids.lock().unwrap(),
        [" literal request ", "", "not a uuid"]
    );
    assert!(service.source.limits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn recent_bounds_only_the_requested_limit_and_preserves_backend_errors() {
    let mut service = service([]);
    for input in [
        None,
        Some(0),
        Some(1),
        Some(20),
        Some(100),
        Some(101),
        Some(usize::MAX),
    ] {
        let rows = service.recent(input).await.unwrap();
        assert_eq!(
            serde_json::to_value(rows).unwrap(),
            serde_json::to_value([record(TradeStatus::Unknown)]).unwrap()
        );
    }
    service.source.list_error = true;
    assert_eq!(
        service.recent(None).await.unwrap_err().to_string(),
        "synthetic list failure"
    );
    assert_eq!(
        *service.source.limits.lock().unwrap(),
        [20, 1, 1, 20, 100, 100, 100, 20]
    );
    assert!(service.source.ids.lock().unwrap().is_empty());
}

#[tokio::test]
async fn zero_wait_reads_once_even_for_pending_and_unconfirmed_statuses() {
    for status in statuses() {
        let row = record(status);
        let service = service([Read::Row(row.clone())]);
        let result = service.wait(" literal request ", 0).await.unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::to_value(row).unwrap()
        );
        assert_eq!(*service.source.ids.lock().unwrap(), [" literal request "]);
        assert!(service.source.limits.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn positive_wait_polls_pending_rows_and_stops_at_the_first_nonpending_snapshot() {
    let service = service([
        Read::Row(record(TradeStatus::Accepted)),
        Read::Row(record(TradeStatus::WarmingParams)),
        Read::Row(record(TradeStatus::BuildingTx)),
        Read::Row(record(TradeStatus::Submitting)),
        Read::Row(record(TradeStatus::Unknown)),
        Read::Error,
    ]);
    let result = service.wait("same-original-id", 60_000).await.unwrap();
    assert_eq!(result.status, TradeStatus::Unknown);
    assert_eq!(
        result.signature.as_deref(),
        Some("synthetic-original-signature")
    );
    assert_eq!(
        *service.source.ids.lock().unwrap(),
        vec!["same-original-id"; 5]
    );
    assert_eq!(service.source.reads.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn deadline_is_checked_after_the_read_without_discarding_its_pending_or_terminal_row() {
    for status in [TradeStatus::Submitting, TradeStatus::Submitted] {
        let service = service([Read::Delayed(record(status)), Read::Error]);
        let result = service.wait("slow-original-id", 1).await.unwrap();
        assert_eq!(result.status, status);
        assert_eq!(*service.source.ids.lock().unwrap(), ["slow-original-id"]);
        assert_eq!(service.source.reads.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn missing_or_failed_wait_read_returns_none_without_retry_or_an_alternate_source() {
    for wait_ms in [0, 60_000] {
        for missing in [false, true] {
            let service = service([if missing { Read::Missing } else { Read::Error }]);
            assert!(service.wait("original-id", wait_ms).await.is_none());
            assert_eq!(*service.source.ids.lock().unwrap(), ["original-id"]);
        }
    }
    for missing in [false, true] {
        let service = service([
            Read::Row(record(TradeStatus::Submitting)),
            if missing { Read::Missing } else { Read::Error },
        ]);
        assert!(service.wait("original-id", 60_000).await.is_none());
        assert_eq!(
            *service.source.ids.lock().unwrap(),
            ["original-id", "original-id"]
        );
    }
}

#[tokio::test]
async fn cancellation_drops_the_current_read_without_a_detached_poll_or_deadline_replacement() {
    let dropped = Arc::new(AtomicBool::new(false));
    let service = service([Read::Blocked(dropped.clone())]);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), service.wait("original-id", 1))
            .await
            .is_err()
    );
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(*service.source.ids.lock().unwrap(), ["original-id"]);
    assert!(service.source.reads.lock().unwrap().is_empty());
}
