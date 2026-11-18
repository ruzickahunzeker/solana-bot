use super::*;
use crate::infrastructure::trade_tasks::TradeTasks;
use sol_trade_sdk::{
    swqos::{common::TradeError, SwqosClient, SwqosClientTrait},
    trading::core::{async_executor::execute_parallel, params::SenderConcurrencyConfig},
};
use solana_sdk::{signature::Keypair, transaction::VersionedTransaction};
use std::{pin::Pin, sync::atomic::AtomicUsize, time::Duration};
use tokio::{
    sync::{oneshot, watch, Notify},
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(15);

#[derive(Default)]
struct WorkerWitness {
    entered: Notify,
    release: watch::Sender<bool>,
    finished: Notify,
    calls: AtomicUsize,
    completed: AtomicBool,
    signature: Mutex<Option<Signature>>,
}

impl WorkerWitness {
    async fn started(&self) -> Signature {
        timeout(WAIT, self.entered.notified()).await.unwrap();
        self.signature.lock().unwrap().unwrap()
    }

    fn assert_unfinished_once(&self) {
        assert_eq!(self.calls.load(Ordering::SeqCst), 1);
        assert!(!self.completed.load(Ordering::SeqCst));
    }

    async fn release_and_check(&self, original: Signature) {
        self.release.send_replace(true);
        timeout(WAIT, self.finished.notified()).await.unwrap();
        assert!(self.completed.load(Ordering::SeqCst));
        assert_eq!(self.calls.load(Ordering::SeqCst), 1);
        assert_eq!(*self.signature.lock().unwrap(), Some(original));
    }
}

// Never hold a worker in the SDK's process-wide pool after an assertion panics.
struct ReleaseOnDrop(Arc<WorkerWitness>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release.send_replace(true);
    }
}

struct OfflineSender(Arc<WorkerWitness>);

impl SwqosClientTrait for OfflineSender {
    fn send_transaction<'life0, 'life1, 'async_trait>(
        &'life0 self,
        direction: TradeType,
        tx: &'life1 VersionedTransaction,
        confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            assert!(matches!(direction, TradeType::Buy));
            assert!(!confirm);
            assert_eq!(tx.signatures.len(), 1);
            assert_ne!(tx.signatures[0], Signature::default());
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            *self.0.signature.lock().unwrap() = Some(tx.signatures[0]);
            self.0.entered.notify_one();
            let mut release = self.0.release.subscribe();
            release.wait_for(|released| *released).await.unwrap();
            self.0.completed.store(true, Ordering::SeqCst);
            self.0.finished.notify_one();
            Ok(())
        })
    }

    fn send_transactions<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _: TradeType,
        _: &'life1 Vec<VersionedTransaction>,
        _: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { panic!("no SDK bundle expected") })
    }

    fn get_tip_account(&self) -> Result<String> {
        Ok(String::new())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Default
    }
}

struct QueuedLane {
    lane: Lane,
    worker: Option<Arc<WorkerWitness>>,
}

impl OrdinarySdkLane for QueuedLane {
    type Prepared = ();

    fn provider(&self) -> SwqosType {
        self.lane.provider
    }

    async fn prepare(&self, trade: &ExecuteTrade) -> Result<()> {
        self.lane.prepare(trade).await
    }

    async fn invoke(&self, _: ()) -> Result<SdkLaneReport, SdkInvocationFailure> {
        self.lane.stage(Stage::Invoke).await;
        let worker = self.worker.as_ref().expect("fallback must not run");
        let senders: [Arc<SwqosClient>; 1] = [Arc::new(OfflineSender(worker.clone()))];
        // No DEX instruction, RPC or relay. The pinned executor only constructs
        // a synthetic fixture packet and hands it to this in-memory sender.
        let raw = execute_parallel(
            &senders,
            Arc::new(Keypair::new_from_array([41; 32])),
            vec![],
            vec![],
            Some(Hash::new_from_array([73; 32])),
            None,
            None,
            "offline-worker-lifecycle",
            true,
            false,
            false,
            true,
            default_fee_strategy(SwqosType::Default, 0.0),
            false,
            SenderConcurrencyConfig {
                sender_thread_cores: None,
                effective_core_ids: Arc::default(),
                max_sender_concurrency: 2,
            },
            false,
        )
        .await;
        decode_sdk_report(raw.map(|(ok, signatures, error, timings)| {
            (
                ok,
                signatures,
                error.map(TradeError::from),
                timings
                    .into_iter()
                    .map(|timing| (timing.swqos_type, timing.submit_done_us))
                    .collect(),
            )
        }))
    }

    async fn observe(
        &self,
        signature_text: &str,
        trade: &ExecuteTrade,
    ) -> Result<SdkLaneObservation> {
        self.lane.observe(signature_text, trade).await
    }
}

struct Case {
    lanes: [QueuedLane; 2],
    worker: Arc<WorkerWitness>,
    events: Arc<Mutex<Vec<(SwqosType, Stage)>>>,
    _release: ReleaseOnDrop,
}

impl Case {
    fn new() -> Self {
        let worker = Arc::new(WorkerWitness::default());
        let events = Arc::new(Mutex::new(vec![]));
        let lanes = [
            QueuedLane {
                lane: Lane::new(SwqosType::Default, events.clone()),
                worker: Some(worker.clone()),
            },
            QueuedLane {
                lane: Lane::new(SwqosType::Temporal, events.clone()),
                worker: None,
            },
        ];
        Self {
            lanes,
            worker: worker.clone(),
            events,
            _release: ReleaseOnDrop(worker),
        }
    }
}

fn assert_only_original_invocation(events: &Mutex<Vec<(SwqosType, Stage)>>) {
    assert_eq!(
        *events.lock().unwrap(),
        [
            (SwqosType::Default, Stage::Prepare),
            (SwqosType::Default, Stage::Invoke),
        ]
    );
}

#[tokio::test]
async fn sdk_worker_lifecycle_caller_abort_keeps_original_queued_sender_alive() {
    let Case {
        lanes,
        worker,
        events,
        _release,
    } = Case::new();
    let caller = tokio::spawn(async move { execute(lanes, &trade()).await });
    let original = worker.started().await;
    caller.abort();
    let joined = timeout(WAIT, caller).await.unwrap();
    assert!(matches!(joined, Err(error) if error.is_cancelled()));
    assert_only_original_invocation(&events);
    worker.assert_unfinished_once();
    worker.release_and_check(original).await;
    assert_only_original_invocation(&events);
}

#[tokio::test]
async fn sdk_worker_lifecycle_clean_application_drain_does_not_prove_sdk_sender_completion() {
    let Case {
        lanes,
        worker,
        events,
        _release,
    } = Case::new();
    let group = TradeTasks::default();
    let (completed, result) = oneshot::channel();
    group
        .register()
        .unwrap()
        .spawn("offline-opaque-sdk".into(), async move {
            let _ = completed.send(classify_result(execute(lanes, &trade()).await, false));
        });
    let original = worker.started().await;
    worker.assert_unfinished_once();
    let (status, reported_signature, error) = timeout(WAIT, result).await.unwrap().unwrap();
    assert_eq!(status, TradeStatus::Unknown);
    assert_eq!(reported_signature, None);
    let error = error.unwrap();
    assert!(error.contains("submit result window"));
    assert!(error.contains("automatic lane fallback disabled"));
    let report = group.drain(WAIT).await;
    assert!(report.clean(), "{report:?}");
    assert!(report.request_ids.is_empty());
    assert!(group.register().is_none());
    worker.assert_unfinished_once();
    assert_only_original_invocation(&events);
    worker.release_and_check(original).await;
    assert_only_original_invocation(&events);
}
