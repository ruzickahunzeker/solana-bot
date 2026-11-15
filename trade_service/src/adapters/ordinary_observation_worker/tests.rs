use super::*;
use crate::{
    application::{
        ordinary_observation_context::{CapturedOrdinaryObservation, OrdinaryObservationContext},
        ordinary_observation_scan::{ObservationCursor, ObservationPage},
        trade_record::TradeRecordV1,
    },
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        trade::{Address, ExecutionMethod, Pool, Side, TradeStatus},
    },
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use tokio::sync::Notify;

#[cfg(unix)]
mod signals;

#[derive(Clone)]
struct Store(Arc<Data>);
struct Data {
    rows: Mutex<Vec<(ObservationCursor, TradeRecordV1)>>,
    afters: Mutex<Vec<Option<ObservationCursor>>>,
    cutoffs: Mutex<Vec<i64>>,
    captures: AtomicUsize,
    writes: AtomicUsize,
    pages: Notify,
    reject_first: bool,
}
impl Store {
    fn new(count: usize, reject_first: bool) -> Self {
        let rows = (0..count)
            .map(|index| {
                let id = format!("tr_scan_{index:03}");
                (
                    ObservationCursor::new(index as i64 + 10, id.clone()).unwrap(),
                    TradeRecordV1 {
                        request_id: id,
                        wallet_id: "fixture-wallet".into(),
                        wallet_pubkey: Address::from_bytes([1; 32]).to_string(),
                        side: Side::Buy,
                        pool: Pool::PumpAmm,
                        mint: Address::from_bytes([2; 32]).to_string(),
                        pool_id: None,
                        execution_method: ExecutionMethod::Sdk,
                        status: TradeStatus::Unknown,
                        signature: Some("fake-original-signature".into()),
                        error: None,
                        duplicate: false,
                        created_at_ms: 10,
                        updated_at_ms: 10,
                    },
                )
            })
            .collect();
        Self(Arc::new(Data {
            rows: Mutex::new(rows),
            afters: Mutex::new(vec![]),
            cutoffs: Mutex::new(vec![]),
            captures: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            pages: Notify::new(),
            reject_first,
        }))
    }
}
impl PendingOrdinaryObservations for Store {
    async fn pending_observations(
        &self,
        after: Option<&ObservationCursor>,
        cutoff: i64,
        limit: usize,
    ) -> Result<ObservationPage> {
        self.0.afters.lock().unwrap().push(after.cloned());
        self.0.cutoffs.lock().unwrap().push(cutoff);
        let eligible = self
            .0
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|(cursor, row)| {
                cursor.created_at_ms() <= cutoff
                    && after.is_none_or(|old| cursor > old)
                    && !matches!(row.status, TradeStatus::Confirmed | TradeStatus::Failed)
            })
            .map(|(cursor, _)| cursor.clone())
            .collect::<Vec<_>>();
        self.0.pages.notify_one();
        Ok(ObservationPage {
            has_more: eligible.len() > limit,
            entries: eligible.into_iter().take(limit).collect(),
        })
    }
}
impl RecoveredOrdinaryObservations for Store {
    async fn capture_observation(&self, id: &str) -> Result<CapturedOrdinaryObservation> {
        self.0.captures.fetch_add(1, Ordering::SeqCst);
        if self.0.reject_first && id.ends_with("000") {
            anyhow::bail!("synthetic damaged record");
        }
        let record = self
            .0
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|(_, row)| row.request_id == id)
            .unwrap()
            .1
            .clone();
        let context = OrdinaryObservationContext::captured(
            &record,
            id.into(),
            "a".repeat(64),
            "b".repeat(64),
        )?;
        Ok(CapturedOrdinaryObservation { context, record })
    }
    async fn read_observation(
        &self,
        context: &OrdinaryObservationContext,
    ) -> Result<TradeRecordV1> {
        let record = self
            .0
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|(_, row)| row.request_id == context.request_id())
            .unwrap()
            .1
            .clone();
        context.ensure_record(&record)?;
        Ok(record)
    }
    async fn record_captured_finality(
        &self,
        context: &OrdinaryObservationContext,
        observation: &ChainObservation,
    ) -> Result<TradeRecordV1> {
        assert_eq!(observation.signature, context.signature());
        assert_eq!(observation.commitment, ChainCommitment::Finalized);
        self.0.writes.fetch_add(1, Ordering::SeqCst);
        let mut rows = self.0.rows.lock().unwrap();
        let row = &mut rows
            .iter_mut()
            .find(|(_, row)| row.request_id == context.request_id())
            .unwrap()
            .1;
        row.status = TradeStatus::Confirmed;
        Ok(row.clone())
    }
}
struct Reader {
    calls: AtomicUsize,
    pending_on: Option<usize>,
    entered: Notify,
    dropped: Arc<AtomicBool>,
    finalize: bool,
}
impl Reader {
    fn new(finalize: bool, pending_on: Option<usize>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            pending_on,
            entered: Notify::new(),
            dropped: Arc::new(AtomicBool::new(false)),
            finalize,
        }
    }
}
struct PendingDrop(Arc<AtomicBool>);
impl Drop for PendingDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl SignatureStatusReader for Reader {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.pending_on == Some(call) {
            let _guard = PendingDrop(self.dropped.clone());
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(self.finalize.then(|| ChainObservation {
            signature: signature.into(),
            commitment: ChainCommitment::Finalized,
            slot: 123,
            error: None,
        }))
    }
}

#[tokio::test]
async fn ordinary_scan_cursor_advances_past_bad_rows_and_freezes_cycle_cutoff() {
    let store = Store::new(3, true);
    let reader = Arc::new(Reader::new(true, None));
    let mut scanner = OrdinaryObservationScanner::new(store.clone(), reader.clone());
    let first = scanner.step(100, 1, Duration::ZERO).await.unwrap();
    assert_eq!(
        (first.examined, first.rejected, first.wrapped),
        (1, 1, false)
    );
    let second = scanner.step(200, 1, Duration::ZERO).await.unwrap();
    assert_eq!(
        (second.examined, second.finalized, second.wrapped),
        (1, 1, false)
    );
    let third = scanner.step(300, 1, Duration::ZERO).await.unwrap();
    assert_eq!(
        (third.examined, third.finalized, third.wrapped),
        (1, 1, true)
    );
    assert_eq!(*store.0.cutoffs.lock().unwrap(), vec![100, 100, 100]);
    scanner.step(400, 1, Duration::ZERO).await.unwrap();
    assert_eq!(store.0.cutoffs.lock().unwrap()[3], 400);
    assert_eq!(reader.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ordinary_scan_rejects_bad_limits_before_loading_a_page() {
    let store = Store::new(2, false);
    let mut scanner =
        OrdinaryObservationScanner::new(store.clone(), Arc::new(Reader::new(true, None)));
    for (cutoff, limit) in [(-1, 1), (100, 0), (100, 65)] {
        assert!(scanner.step(cutoff, limit, Duration::ZERO).await.is_err());
    }
    assert!(store.0.afters.lock().unwrap().is_empty());
}

struct MalformedPage {
    entries: Vec<ObservationCursor>,
    has_more: bool,
}
impl PendingOrdinaryObservations for MalformedPage {
    async fn pending_observations(
        &self,
        _: Option<&ObservationCursor>,
        _: i64,
        _: usize,
    ) -> Result<ObservationPage> {
        Ok(ObservationPage {
            entries: self.entries.clone(),
            has_more: self.has_more,
        })
    }
}
impl RecoveredOrdinaryObservations for MalformedPage {
    async fn capture_observation(&self, _: &str) -> Result<CapturedOrdinaryObservation> {
        panic!("a malformed page must be rejected before capturing any candidate")
    }
    async fn read_observation(&self, _: &OrdinaryObservationContext) -> Result<TradeRecordV1> {
        unreachable!()
    }
    async fn record_captured_finality(
        &self,
        _: &OrdinaryObservationContext,
        _: &ChainObservation,
    ) -> Result<TradeRecordV1> {
        unreachable!()
    }
}

#[tokio::test]
async fn ordinary_scan_rejects_entire_malformed_page_before_capture_or_rpc() {
    let cursor = |time, id: &str| ObservationCursor::new(time, id.into()).unwrap();
    for (entries, has_more) in [
        (vec![cursor(10, "tr_a"), cursor(10, "tr_a")], false),
        (vec![cursor(11, "tr_b"), cursor(10, "tr_a")], false),
        (vec![cursor(10, "tr_a"), cursor(101, "tr_b")], false),
        (
            vec![cursor(10, "tr_a"), cursor(11, "tr_b"), cursor(12, "tr_c")],
            false,
        ),
        (vec![], true),
    ] {
        let reader = Arc::new(Reader::new(true, None));
        let mut scanner =
            OrdinaryObservationScanner::new(MalformedPage { entries, has_more }, reader.clone());
        assert!(scanner.step(100, 2, Duration::ZERO).await.is_err());
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    }
}

fn fast_config() -> OrdinaryObservationConfig {
    // Only the private loop is tested at accelerated intervals; production prepare validates bounds.
    OrdinaryObservationConfig {
        enabled: true,
        batch_size: 3,
        poll_interval_ms: 1,
        min_age_ms: 1,
        request_interval_ms: 0,
    }
}
fn now() -> u128 {
    1_000
}

#[tokio::test]
async fn ordinary_scan_shutdown_cancels_inflight_lookup_and_preserves_original_pending_record() {
    let store = Store::new(1, false);
    let reader = Arc::new(Reader::new(false, Some(1)));
    let scanner = OrdinaryObservationScanner::new(store.clone(), reader.clone());
    let worker = spawn_scanner(scanner, fast_config(), now, Duration::from_secs(10));
    tokio::time::timeout(Duration::from_secs(2), reader.entered.notified())
        .await
        .unwrap();
    worker.shutdown().await.unwrap();
    assert!(reader.dropped.load(Ordering::SeqCst));
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.0.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.0.rows.lock().unwrap()[0].1.status,
        TradeStatus::Unknown
    );
}

#[tokio::test]
async fn ordinary_scan_owner_drop_aborts_the_read_only_task() {
    let store = Store::new(1, false);
    let reader = Arc::new(Reader::new(false, Some(1)));
    let worker = spawn_scanner(
        OrdinaryObservationScanner::new(store.clone(), reader.clone()),
        fast_config(),
        now,
        Duration::from_secs(10),
    );
    tokio::time::timeout(Duration::from_secs(2), reader.entered.notified())
        .await
        .unwrap();
    drop(worker);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !reader.dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(store.0.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ordinary_scan_batch_deadline_retains_completed_cursor_without_skipping_unfinished_row() {
    let store = Store::new(3, false);
    let reader = Arc::new(Reader::new(false, Some(2)));
    let worker = spawn_scanner(
        OrdinaryObservationScanner::new(store.clone(), reader.clone()),
        fast_config(),
        now,
        Duration::from_millis(20),
    );
    tokio::time::timeout(Duration::from_secs(2), reader.entered.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if store.0.afters.lock().unwrap().len() >= 2 {
                break;
            }
            store.0.pages.notified().await;
        }
    })
    .await
    .unwrap();
    worker.shutdown().await.unwrap();
    let afters = store.0.afters.lock().unwrap();
    assert!(afters[0].is_none());
    assert_eq!(afters[1].as_ref().unwrap().request_id(), "tr_scan_000");
    assert!(reader.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn ordinary_scan_factory_is_opt_in_and_validates_before_creating_network_work() {
    let state = crate::http_contract_tests::state().await;
    let runtime = |urls: &[String]| Arc::new(SharedReadRuntime::new(urls).unwrap());
    assert!(
        ObservationPlan::prepare(&Default::default(), &state.persistence, runtime(&[]))
            .unwrap()
            .is_none()
    );
    let config = OrdinaryObservationConfig {
        enabled: true,
        ..Default::default()
    };
    assert!(ObservationPlan::prepare(
        &config,
        &state.persistence,
        runtime(&["https://node.example.invalid".into()])
    )
    .is_err());
    let persistence = TradePersistence::new(&crate::config::TradePersistenceConfig {
        mode: crate::config::TradePersistenceMode::Durable,
        database_url: "sqlite::memory:".into(),
        busy_timeout_ms: 100,
    })
    .await
    .unwrap();
    assert!(ObservationPlan::prepare(&config, &persistence, runtime(&[])).is_err());
    let plan = ObservationPlan::prepare(
        &config,
        &persistence,
        runtime(&["https://node.example.invalid".into()]),
    )
    .unwrap()
    .unwrap();
    let one = plan.reader();
    let two = plan.reader();
    assert!(Arc::ptr_eq(&one, &two));
    drop(plan);
    drop(one);
    drop(two);
}
