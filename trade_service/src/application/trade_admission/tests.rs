use super::*;
use crate::{
    application::{
        ports::ordinary_journal::SnapshotWrite,
        trade_persistence_input::{RejectedTradeInput, StoredRequestV1, ValidatedTradeInput},
    },
    domain::trade::{ExecutionMethod, Pool, Side, TradeStatus},
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

type Trace = Arc<Mutex<Vec<&'static str>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reply {
    Created,
    Duplicate,
    Conflict,
    Error,
}

struct Journal {
    mode: TradeJournalMode,
    reply: Reply,
    storage_error: bool,
    mirror_error: bool,
    trace: Trace,
}
impl OrdinaryTradeJournal for Journal {
    fn mode(&self) -> TradeJournalMode {
        self.trace.lock().unwrap().push("mode");
        self.mode
    }
    fn ensure_snapshot_storage(&self, write: SnapshotWrite) -> anyhow::Result<()> {
        self.trace.lock().unwrap().push(match write {
            SnapshotWrite::Reservation => "check-reserve",
            SnapshotWrite::Mirror => "check-mirror",
        });
        if self.storage_error {
            anyhow::bail!("synthetic unavailable storage");
        }
        Ok(())
    }
    async fn reserve_record(
        &self,
        _: &TradeRecordV1,
        input: &ValidatedTradeInput,
    ) -> anyhow::Result<TradeReservation> {
        self.trace.lock().unwrap().push("db-reserve");
        assert_eq!(input.idempotency_key(), " literal-key ");
        assert!(input.aggregator_sol());
        match self.reply {
            Reply::Created => Ok(TradeReservation::Created),
            Reply::Conflict => Ok(TradeReservation::Conflict),
            Reply::Duplicate => {
                let mut original = record("tr_original");
                original.signature = Some("original-signature".into());
                Ok(TradeReservation::Duplicate(original))
            }
            Reply::Error => anyhow::bail!("synthetic reserve failure"),
        }
    }
    async fn mirror_accepted(
        &self,
        _: &TradeRecordV1,
        _: ValidatedTradeInput,
    ) -> anyhow::Result<()> {
        self.trace.lock().unwrap().push("db-mirror");
        if self.mirror_error {
            anyhow::bail!("synthetic mirror failure");
        }
        Ok(())
    }
    async fn mirror_rejection(
        &self,
        record: &TradeRecordV1,
        input: RejectedTradeInput,
    ) -> anyhow::Result<()> {
        self.trace.lock().unwrap().push("db-rejection");
        assert_eq!(record.status, TradeStatus::Rejected);
        input.ensure_record(&record.wallet_id, &record.request_id)?;
        if self.mirror_error {
            anyhow::bail!("synthetic mirror failure");
        }
        Ok(())
    }
    async fn update_record(&self, _: &TradeRecordV1) -> anyhow::Result<()> {
        panic!("admission must not update execution status")
    }
    async fn read_record(&self, _: &str) -> anyhow::Result<Option<TradeRecordV1>> {
        panic!("admission must not perform additional durable queries")
    }
    async fn recent_records(&self, _: usize) -> anyhow::Result<Vec<TradeRecordV1>> {
        panic!("admission must not list orders")
    }
}

struct Index {
    rows: Mutex<HashMap<String, TradeRecordV1>>,
    history: Mutex<Vec<TradeRecordV1>>,
    trace: Trace,
}
impl TradeRecordIndex for Index {
    fn insert_record(&self, record: &TradeRecordV1) {
        self.trace.lock().unwrap().push("index-insert");
        self.history.lock().unwrap().push(record.clone());
        self.rows
            .lock()
            .unwrap()
            .insert(record.request_id.clone(), record.clone());
    }
    fn get_record(&self, id: &str) -> Option<TradeRecordV1> {
        self.trace.lock().unwrap().push("index-get");
        self.rows.lock().unwrap().get(id).cloned()
    }
    fn remove_record(&self, id: &str) {
        self.trace.lock().unwrap().push("index-remove");
        self.rows.lock().unwrap().remove(id);
    }
}

struct Reservations {
    reply: Reply,
    trace: Trace,
}
impl MemoryTradeReservations for Reservations {
    fn reserve_key(
        &self,
        wallet_id: &str,
        key: &str,
        id: &str,
        hash: &str,
        aggregator: bool,
    ) -> MemoryReservation {
        self.trace.lock().unwrap().push("memory-reserve");
        assert_eq!(
            (wallet_id, key, id, hash, aggregator),
            ("wallet", " literal-key ", "tr_attempt", "legacy-hash", true)
        );
        match self.reply {
            Reply::Created => MemoryReservation::Reserved,
            Reply::Duplicate => MemoryReservation::Duplicate("tr_original".into()),
            Reply::Conflict => MemoryReservation::Conflict,
            Reply::Error => panic!("memory port has no error variant"),
        }
    }
}

struct Payload {
    hash_error: bool,
    snapshot_error: bool,
    trace: Trace,
    command_override: Option<TradeCommand>,
    command_reads: AtomicUsize,
}
impl ValidatedAdmissionPayload for Payload {
    fn execution_command(&self) -> crate::application::trade_command::TradeCommand {
        self.command_reads.fetch_add(1, Ordering::SeqCst);
        if let Some(command) = &self.command_override {
            return command.clone();
        }
        let mut command = crate::application::trade_command::test_support::command();
        command.wallet_id = "wallet".into();
        command.aggregator_sol = true;
        command.execution.method = ExecutionMethod::Sdk;
        command
    }
    fn wallet_id(&self) -> &str {
        "wallet"
    }
    fn idempotency_key(&self) -> &str {
        " literal-key "
    }
    fn aggregator_sol(&self) -> bool {
        true
    }
    fn fingerprint(&self) -> anyhow::Result<String> {
        self.trace.lock().unwrap().push("hash");
        if self.hash_error {
            anyhow::bail!("synthetic hash failure");
        }
        Ok("legacy-hash".into())
    }
    fn new_record(&self, id: String) -> TradeRecordV1 {
        self.trace.lock().unwrap().push("new-record");
        record(&id)
    }
    fn snapshot(&self) -> anyhow::Result<ValidatedTradeInput> {
        self.trace.lock().unwrap().push("snapshot");
        if self.snapshot_error {
            anyhow::bail!("synthetic snapshot failure");
        }
        Ok(ValidatedTradeInput::new(
            "wallet".into(),
            " literal-key ".into(),
            true,
            StoredRequestV1::new("legacy-hash".into(), "synthetic-opaque-json".into()),
        ))
    }
}

impl RejectedAdmissionPayload for Payload {
    fn new_record(&self, id: String) -> TradeRecordV1 {
        ValidatedAdmissionPayload::new_record(self, id)
    }
    fn snapshot(&self, record: &TradeRecordV1) -> anyhow::Result<RejectedTradeInput> {
        self.trace.lock().unwrap().push("rejection-snapshot");
        if self.snapshot_error {
            anyhow::bail!("synthetic snapshot failure");
        }
        Ok(RejectedTradeInput::new(
            record.wallet_id.clone(),
            record.request_id.clone(),
            StoredRequestV1::new("legacy-rejected-hash".into(), "opaque-rejected-json".into()),
        ))
    }
}

fn record(id: &str) -> TradeRecordV1 {
    TradeRecordV1 {
        request_id: id.into(),
        wallet_id: "wallet".into(),
        wallet_pubkey: "public-identity".into(),
        side: Side::Buy,
        pool: Pool::Pump,
        mint: "synthetic-mint".into(),
        pool_id: None,
        execution_method: ExecutionMethod::Sdk,
        status: TradeStatus::Accepted,
        signature: None,
        error: None,
        duplicate: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

struct Fixture {
    journal: Journal,
    index: Index,
    reservations: Reservations,
    payload: Payload,
    trace: Trace,
}
fn fixture(mode: TradeJournalMode) -> Fixture {
    let trace = Arc::new(Mutex::new(vec![]));
    Fixture {
        journal: Journal {
            mode,
            reply: Reply::Created,
            storage_error: false,
            mirror_error: false,
            trace: trace.clone(),
        },
        index: Index {
            rows: Mutex::new(HashMap::new()),
            history: Mutex::new(vec![]),
            trace: trace.clone(),
        },
        reservations: Reservations {
            reply: Reply::Created,
            trace: trace.clone(),
        },
        payload: Payload {
            hash_error: false,
            snapshot_error: false,
            trace: trace.clone(),
            command_override: None,
            command_reads: AtomicUsize::new(0),
        },
        trace,
    }
}

#[tokio::test]
async fn created_execution_keeps_original_reservation_and_freezes_all_command_fields() {
    for mode in [
        TradeJournalMode::Memory,
        TradeJournalMode::Shadow,
        TradeJournalMode::Durable,
    ] {
        let mut f = fixture(mode);
        let mut expected = f.payload.execution_command();
        f.payload.command_reads.store(0, Ordering::SeqCst);
        expected.amount = crate::domain::trade::TradeAmount::BuySol {
            lamports: 9_007_199_254_740_993,
        };
        expected.execution.tip_sol = f64::from_bits(1);
        expected.pumpapi_priority_fee_sol = -0.0;
        f.payload.command_override = Some(expected.clone());
        let AdmissionDecision::Created {
            record, execution, ..
        } = f.accept().await.unwrap()
        else {
            panic!("created required")
        };
        assert_eq!(f.payload.command_reads.load(Ordering::SeqCst), 1);
        f.payload.command_override.as_mut().unwrap().wallet_id = "different-wallet".into();
        f.payload.command_override.as_mut().unwrap().amount =
            crate::domain::trade::TradeAmount::SellPercent { percent: 1 };
        let anchor = execution.reservation();
        assert_eq!(anchor.request_id(), record.request_id);
        assert_eq!(anchor.wallet_id(), "wallet");
        assert_eq!(anchor.idempotency_key(), " literal-key ");
        assert_eq!(anchor.request_hash(), "legacy-hash");
        assert_eq!(anchor.mode(), mode);
        assert_eq!(
            format!("{:?}", execution.command()),
            format!("{expected:?}")
        );
        assert_eq!(
            execution.command().execution.tip_sol.to_bits(),
            expected.execution.tip_sol.to_bits()
        );
        assert_eq!(
            execution.command().pumpapi_priority_fee_sol.to_bits(),
            expected.pumpapi_priority_fee_sol.to_bits()
        );
    }
}

#[tokio::test]
async fn replays_conflicts_and_failed_reservations_never_obtain_an_execution_input() {
    for mode in [
        TradeJournalMode::Memory,
        TradeJournalMode::Shadow,
        TradeJournalMode::Durable,
    ] {
        for reply in [Reply::Duplicate, Reply::Conflict, Reply::Error] {
            let mut f = fixture(mode);
            f.journal.reply = reply;
            f.reservations.reply = reply;
            f.index
                .rows
                .lock()
                .unwrap()
                .insert("tr_original".into(), record("tr_original"));
            if mode != TradeJournalMode::Durable && reply == Reply::Error {
                f.payload.hash_error = true;
            }
            assert!(!matches!(
                f.accept().await,
                Ok(AdmissionDecision::Created { .. })
            ));
            assert_eq!(f.payload.command_reads.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn inconsistent_commands_cannot_be_executed_from_a_created_reservation() {
    for mode in [
        TradeJournalMode::Memory,
        TradeJournalMode::Shadow,
        TradeJournalMode::Durable,
    ] {
        for field in ["wallet", "key", "branch", "pool", "side", "method"] {
            let mut f = fixture(mode);
            let mut command = f.payload.execution_command();
            match field {
                "wallet" => command.wallet_id = "other".into(),
                "key" => command.idempotency_key = "literal-key".into(),
                "branch" => command.aggregator_sol = false,
                "pool" => command.route.pool = Pool::PumpAmm,
                "side" => {
                    command.amount = crate::domain::trade::TradeAmount::SellPercent { percent: 1 }
                }
                "method" => command.execution.method = ExecutionMethod::Pumpapi,
                _ => unreachable!(),
            }
            f.payload.command_override = Some(command);
            let error = f.accept().await.err().unwrap();
            assert!(matches!(error, AdmissionError::Internal(_)));
            assert!(f.index.rows.lock().unwrap().contains_key("tr_attempt"));
            assert!(!f.events().contains(&"index-remove"));
        }
    }
}

#[test]
fn wallet_identity_guard_rejects_remapped_ids_and_pubkeys() {
    let f = fixture(TradeJournalMode::Memory);
    let original = Address::from_bytes([3; 32]);
    let mut row = record("tr_identity");
    row.wallet_pubkey = original.to_string();
    let execution = AdmittedTrade::created(
        &row,
        &f.payload,
        "legacy-hash".into(),
        TradeJournalMode::Memory,
    )
    .unwrap();
    assert!(execution
        .reservation()
        .ensure_wallet("wallet", original)
        .is_ok());
    for (wallet_id, pubkey) in [
        ("wallet", Address::from_bytes([4; 32])),
        (" wallet ", original),
        ("other", original),
    ] {
        assert_eq!(
            execution
                .reservation()
                .ensure_wallet(wallet_id, pubkey)
                .unwrap_err()
                .to_string(),
            "ordinary execution wallet identity changed after admission"
        );
    }
}

#[tokio::test]
async fn shadow_mirror_failure_keeps_non_durable_anchor_and_original_key() {
    let mut f = fixture(TradeJournalMode::Shadow);
    f.journal.mirror_error = true;
    let AdmissionDecision::Created {
        execution,
        shadow_error,
        ..
    } = f.accept().await.unwrap()
    else {
        panic!("created required")
    };
    assert!(shadow_error.is_some());
    assert_eq!(execution.reservation().mode(), TradeJournalMode::Shadow);
    assert_eq!(execution.reservation().request_hash(), "legacy-hash");
    assert_eq!(execution.reservation().idempotency_key(), " literal-key ");
    assert_eq!(f.payload.command_reads.load(Ordering::SeqCst), 1);
}

impl Fixture {
    fn reject(&self) -> TradeRecordV1 {
        reject(
            &self.index,
            &self.payload,
            "tr_rejected".into(),
            "literal error\n".into(),
            || {
                self.trace.lock().unwrap().push("rejection-time");
                let rows = self.index.rows.lock().unwrap();
                assert_eq!(rows["tr_rejected"].status, TradeStatus::Accepted);
                assert_eq!(rows["tr_rejected"].updated_at_ms, 1);
                2
            },
        )
    }
}

#[tokio::test]
async fn rejection_preserves_initial_index_write_second_timestamp_and_log_position() {
    let f = fixture(TradeJournalMode::Memory);
    let record = f.reject();
    assert_eq!(record.request_id, "tr_rejected");
    assert_eq!(record.status, TradeStatus::Rejected);
    assert_eq!(record.error.as_deref(), Some("literal error\n"));
    assert_eq!((record.created_at_ms, record.updated_at_ms), (1, 2));
    assert!(record.signature.is_none());
    assert!(!record.duplicate);
    {
        let history = f.index.history.lock().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].status, TradeStatus::Accepted);
        assert_eq!(history[0].error, None);
        assert_eq!(history[1].status, TradeStatus::Rejected);
        assert_eq!(history[1].error, record.error);
    }
    f.trace.lock().unwrap().push("handler-log");
    mirror_rejection(&f.journal, &f.payload, &record)
        .await
        .unwrap();
    assert_eq!(
        f.events(),
        [
            "new-record",
            "index-insert",
            "rejection-time",
            "index-insert",
            "handler-log",
            "mode"
        ]
    );
}

#[tokio::test]
async fn memory_rejection_does_not_construct_a_snapshot_or_check_storage() {
    let mut f = fixture(TradeJournalMode::Memory);
    f.payload.hash_error = true;
    f.payload.snapshot_error = true;
    f.journal.storage_error = true;
    f.journal.mirror_error = true;
    let record = f.reject();
    mirror_rejection(&f.journal, &f.payload, &record)
        .await
        .unwrap();
    assert_eq!(
        f.events(),
        [
            "new-record",
            "index-insert",
            "rejection-time",
            "index-insert",
            "mode"
        ]
    );
    assert_eq!(f.index.rows.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn nonmemory_rejection_checks_storage_before_snapshot_and_keeps_record_on_failure() {
    for mode in [TradeJournalMode::Shadow, TradeJournalMode::Durable] {
        for failure in ["none", "storage", "snapshot", "mirror"] {
            let mut f = fixture(mode);
            f.payload.hash_error = true;
            f.payload.snapshot_error = failure == "snapshot";
            f.journal.storage_error = failure == "storage";
            f.journal.mirror_error = failure == "mirror";
            let record = f.reject();
            let result = mirror_rejection(&f.journal, &f.payload, &record).await;
            match failure {
                "none" => result.unwrap(),
                "storage" => assert_eq!(
                    result.unwrap_err().to_string(),
                    "synthetic unavailable storage"
                ),
                "snapshot" => assert_eq!(
                    result.unwrap_err().to_string(),
                    "synthetic snapshot failure"
                ),
                "mirror" => assert_eq!(result.unwrap_err().to_string(), "synthetic mirror failure"),
                _ => unreachable!(),
            }
            let mut expected = vec![
                "new-record",
                "index-insert",
                "rejection-time",
                "index-insert",
                "mode",
                "check-mirror",
            ];
            if failure != "storage" {
                expected.push("rejection-snapshot");
            }
            if !matches!(failure, "storage" | "snapshot") {
                expected.push("db-rejection");
            }
            assert_eq!(f.events(), expected);
            assert_eq!(
                f.index.rows.lock().unwrap()["tr_rejected"].status,
                TradeStatus::Rejected
            );
            assert_eq!(f.index.history.lock().unwrap().len(), 2);
        }
    }
}
impl Fixture {
    async fn accept(&self) -> Result<AdmissionDecision, AdmissionError> {
        accept(
            &self.journal,
            &self.index,
            &self.reservations,
            &self.payload,
            "tr_attempt".into(),
        )
        .await
    }
    fn events(&self) -> Vec<&'static str> {
        self.trace.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn memory_admission_does_not_construct_or_check_persistence_payloads() {
    let f = fixture(TradeJournalMode::Memory);
    assert!(matches!(
        f.accept().await.unwrap(),
        AdmissionDecision::Created {
            shadow_error: None,
            ..
        }
    ));
    assert_eq!(
        f.events(),
        [
            "mode",
            "hash",
            "memory-reserve",
            "new-record",
            "index-insert"
        ]
    );
    assert_eq!(f.index.rows.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn memory_replays_and_missing_index_records_do_not_construct_new_records_or_json() {
    for present in [false, true] {
        let mut f = fixture(TradeJournalMode::Memory);
        f.reservations.reply = Reply::Duplicate;
        if present {
            f.index
                .rows
                .lock()
                .unwrap()
                .insert("tr_original".into(), record("tr_original"));
        }
        match f.accept().await {
            Ok(AdmissionDecision::Duplicate(record)) => {
                assert!(present);
                assert!(record.duplicate);
                assert_eq!(record.request_id, "tr_original");
                assert!(!f.index.rows.lock().unwrap()["tr_original"].duplicate);
            }
            Err(AdmissionError::Conflict(message)) => {
                assert!(!present);
                assert_eq!(message, "duplicate idempotency key points to missing trade");
            }
            other => panic!("unexpected replay result: {other:?}"),
        }
        assert_eq!(f.events(), ["mode", "hash", "memory-reserve", "index-get"]);
    }
}

#[tokio::test]
async fn fingerprint_error_and_memory_conflict_stop_before_any_index_change() {
    let mut hash = fixture(TradeJournalMode::Memory);
    hash.payload.hash_error = true;
    assert!(matches!(
        hash.accept().await,
        Err(AdmissionError::Internal(_))
    ));
    assert_eq!(hash.events(), ["mode", "hash"]);
    let mut conflict = fixture(TradeJournalMode::Memory);
    conflict.reservations.reply = Reply::Conflict;
    assert!(matches!(
        conflict.accept().await,
        Err(AdmissionError::Conflict(CONFLICT))
    ));
    assert_eq!(conflict.events(), ["mode", "hash", "memory-reserve"]);
    assert!(conflict.index.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn durable_outcomes_preserve_original_record_and_remove_only_temporary_attempts() {
    for reply in [
        Reply::Created,
        Reply::Duplicate,
        Reply::Conflict,
        Reply::Error,
    ] {
        let mut f = fixture(TradeJournalMode::Durable);
        f.journal.reply = reply;
        f.payload.hash_error = true; // Durable must not invoke the separate memory hash path.
        match f.accept().await {
            Ok(AdmissionDecision::Created {
                shadow_error: None, ..
            }) => assert!(reply == Reply::Created),
            Ok(AdmissionDecision::Duplicate(record)) => {
                assert!(reply == Reply::Duplicate);
                assert_eq!(record.request_id, "tr_original");
                assert!(record.duplicate);
                assert_eq!(record.signature.as_deref(), Some("original-signature"));
            }
            Err(AdmissionError::Conflict(CONFLICT)) => assert!(reply == Reply::Conflict),
            Err(AdmissionError::PersistenceUnavailable(_)) => assert!(reply == Reply::Error),
            other => panic!("unexpected durable result: {other:?}"),
        }
        let mut expected = vec![
            "mode",
            "new-record",
            "index-insert",
            "check-reserve",
            "snapshot",
            "db-reserve",
        ];
        if reply != Reply::Created {
            expected.push("index-remove");
        }
        assert_eq!(f.events(), expected);
        assert_eq!(
            f.index.rows.lock().unwrap().len(),
            usize::from(reply == Reply::Created)
        );
    }
}

#[tokio::test]
async fn durable_storage_precedes_snapshot_and_both_failures_roll_back_the_index() {
    for storage_error in [false, true] {
        let mut f = fixture(TradeJournalMode::Durable);
        f.journal.storage_error = storage_error;
        f.payload.snapshot_error = true;
        assert!(matches!(
            f.accept().await,
            Err(AdmissionError::PersistenceUnavailable(_))
        ));
        let mut expected = vec!["mode", "new-record", "index-insert", "check-reserve"];
        if !storage_error {
            expected.push("snapshot");
        }
        expected.push("index-remove");
        assert_eq!(f.events(), expected);
        assert!(f.index.rows.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn shadow_failure_at_each_stage_keeps_memory_admission_and_original_operation_order() {
    for failure in ["none", "storage", "snapshot", "mirror"] {
        let mut f = fixture(TradeJournalMode::Shadow);
        f.journal.storage_error = failure == "storage";
        f.journal.mirror_error = failure == "mirror";
        f.payload.snapshot_error = failure == "snapshot";
        let AdmissionDecision::Created { shadow_error, .. } = f.accept().await.unwrap() else {
            panic!("shadow cannot become duplicate here")
        };
        assert_eq!(shadow_error.is_some(), failure != "none");
        let mut expected = vec![
            "mode",
            "hash",
            "memory-reserve",
            "new-record",
            "index-insert",
            "check-mirror",
        ];
        if failure != "storage" {
            expected.push("snapshot");
        }
        if !matches!(failure, "storage" | "snapshot") {
            expected.push("db-mirror");
        }
        assert_eq!(f.events(), expected);
        assert_eq!(f.index.rows.lock().unwrap().len(), 1);
    }
}
