use super::*;
use crate::{
    application::ports::preparation_balance::PreparationBalanceRead,
    domain::{
        preparation_balance::{
            BalanceFailureKind, PreparationBalanceFailure, PreparationBalanceSnapshot,
        },
        trade::Address,
    },
};
use std::{
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum Mode {
    Good,
    Invalid,
    Unavailable,
    Expired,
    WrongWallet,
    WrongMint,
    Old,
    Missing,
    Overflow,
    Blocked,
    GateError,
}
struct Reader {
    mode: Mode,
    events: Arc<Mutex<Vec<String>>>,
    label: &'static str,
    entered: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}
impl PreparationBalanceRead for Reader {
    async fn snapshot(&self, mint: Address) -> Result<PreparationBalanceSnapshot> {
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:read", self.label));
        if matches!(self.mode, Mode::Blocked) {
            let _drop = DropFlag(self.dropped.clone());
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        let kind = match self.mode {
            Mode::Invalid => Some(BalanceFailureKind::Invalid),
            Mode::Unavailable => Some(BalanceFailureKind::Unavailable),
            Mode::Expired => Some(BalanceFailureKind::Expired),
            _ => None,
        };
        if let Some(kind) = kind {
            return Err(PreparationBalanceFailure::new(kind, "fixture balance failure").into());
        }
        let wallet = Address::from_bytes(
            [if matches!(self.mode, Mode::WrongWallet) {
                99
            } else {
                1
            }; 32],
        );
        let mint = if matches!(self.mode, Mode::WrongMint) {
            Address::from_bytes([99; 32])
        } else {
            mint
        };
        let started = if matches!(self.mode, Mode::Old) {
            Instant::now() - Duration::from_secs(4)
        } else {
            Instant::now()
        };
        Ok(PreparationBalanceSnapshot::fixture(
            wallet,
            mint,
            if matches!(self.mode, Mode::Overflow) {
                u64::MAX
            } else {
                1_000_000
            },
            started,
        ))
    }
}
struct EvidenceLane {
    reader: Reader,
    provider: SwqosType,
    fail_after_entry: bool,
    invocations: Arc<AtomicUsize>,
}
impl OrdinarySdkLane for EvidenceLane {
    type Prepared = Option<PreparationBalanceSnapshot>;
    fn provider(&self) -> SwqosType {
        self.provider
    }
    async fn prepare(&self, trade: &ExecuteTrade) -> Result<Self::Prepared> {
        self.reader
            .events
            .lock()
            .unwrap()
            .push(format!("{}:prepare", self.reader.label));
        if matches!(self.reader.mode, Mode::Missing) {
            return Ok(None);
        }
        self.reader
            .snapshot(Address::from_bytes(trade.mint.to_bytes()))
            .await
            .map(Some)
    }
    fn validate_prepared(&self, prepared: &Self::Prepared, trade: &ExecuteTrade) -> Result<()> {
        self.reader
            .events
            .lock()
            .unwrap()
            .push(format!("{}:gate", self.reader.label));
        if matches!(self.reader.mode, Mode::GateError) {
            bail!("gate https://fixture.invalid/?key=fixture-secret failed");
        }
        crate::adapters::sdk::preparation_balance::validate(
            prepared.as_ref(),
            Address::from_bytes([1; 32]),
            trade,
            crate::domain::trade::Side::Sell,
        )
    }
    async fn invoke(&self, _: Self::Prepared) -> Result<SdkLaneReport, SdkInvocationFailure> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        self.reader
            .events
            .lock()
            .unwrap()
            .push(format!("{}:invoke", self.reader.label));
        if self.fail_after_entry {
            return Err(SdkInvocationFailure {
                signatures: vec![signature(1)],
                message: "fixture post-entry failure".into(),
            });
        }
        Ok(SdkLaneReport {
            success: true,
            signatures: vec![signature(1)],
            error: None,
        })
    }
    async fn observe(&self, _: &str, _: &ExecuteTrade) -> Result<SdkLaneObservation> {
        Ok(SdkLaneObservation::Skipped)
    }
}
fn lane(
    provider: SwqosType,
    label: &'static str,
    mode: Mode,
    events: Arc<Mutex<Vec<String>>>,
) -> EvidenceLane {
    EvidenceLane {
        reader: Reader {
            mode,
            events,
            label,
            entered: Arc::new(Notify::new()),
            dropped: Arc::default(),
        },
        provider,
        fail_after_entry: false,
        invocations: Arc::default(),
    }
}

#[tokio::test]
async fn preparation_balance_invalid_or_expired_read_stops_before_any_sdk_or_next_lane() {
    for mode in [Mode::Invalid, Mode::Expired] {
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let first = lane(SwqosType::Temporal, "first", mode, events.clone());
        let second = lane(SwqosType::Jito, "second", Mode::Good, events.clone());
        let first_count = first.invocations.clone();
        let second_count = second.invocations.clone();
        let error = execute([first, second], &trade()).await.err().unwrap();
        assert!(error
            .downcast_ref::<crate::application::trade_execution::UncertainSdkSubmission>()
            .is_none());
        assert_eq!(*events.lock().unwrap(), ["first:prepare", "first:read"]);
        assert_eq!(
            first_count.load(Ordering::SeqCst) + second_count.load(Ordering::SeqCst),
            0
        );
    }
}

#[tokio::test]
async fn preparation_balance_unavailable_read_keeps_only_pre_entry_fallback() {
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let first = lane(
        SwqosType::Temporal,
        "first",
        Mode::Unavailable,
        events.clone(),
    );
    let second = lane(SwqosType::Jito, "second", Mode::Good, events.clone());
    let count = second.invocations.clone();
    assert!(execute([first, second], &trade()).await.unwrap().success);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "first:prepare",
            "first:read",
            "second:prepare",
            "second:read",
            "second:gate",
            "second:invoke"
        ]
    );
}

#[tokio::test]
async fn preparation_balance_gate_rejects_missing_wrong_scope_stale_and_overflow_without_fallback()
{
    for mode in [
        Mode::Missing,
        Mode::WrongWallet,
        Mode::WrongMint,
        Mode::Old,
        Mode::Overflow,
    ] {
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let first = lane(SwqosType::Temporal, "first", mode, events.clone());
        let second = lane(SwqosType::Jito, "second", Mode::Good, events.clone());
        let count = first.invocations.clone();
        assert!(execute([first, second], &trade()).await.is_err());
        let events = events.lock().unwrap();
        assert!(events.last().unwrap().ends_with(":gate"));
        assert!(!events.iter().any(|event| event.starts_with("second:")));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn preparation_balance_gate_errors_are_redacted_and_stop_before_sdk_entry() {
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let first = lane(
        SwqosType::Temporal,
        "first",
        Mode::GateError,
        events.clone(),
    );
    let second = lane(SwqosType::Jito, "second", Mode::Good, events.clone());
    let error = execute([first, second], &trade()).await.err().unwrap();
    assert!(!error.to_string().contains("fixture-secret"));
    assert!(!events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.ends_with(":invoke") || event.starts_with("second:")));
}

#[tokio::test]
async fn preparation_balance_valid_snapshot_still_cannot_allow_post_entry_fallback() {
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut first = lane(SwqosType::Temporal, "first", Mode::Good, events.clone());
    first.fail_after_entry = true;
    let second = lane(SwqosType::Jito, "second", Mode::Good, events.clone());
    let error = execute([first, second], &trade()).await.err().unwrap();
    assert!(error
        .downcast_ref::<crate::application::trade_execution::UncertainSdkSubmission>()
        .is_some());
    assert!(!events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.starts_with("second:")));
}

#[tokio::test]
async fn preparation_balance_cancel_before_gate_cannot_continue_query_or_other_lane() {
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let first = lane(SwqosType::Temporal, "first", Mode::Blocked, events.clone());
    let entered = first.reader.entered.clone();
    let dropped = first.reader.dropped.clone();
    let second = lane(SwqosType::Jito, "second", Mode::Good, events.clone());
    let task = tokio::spawn(async move { execute([first, second], &trade()).await });
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(*events.lock().unwrap(), ["first:prepare", "first:read"]);
}
