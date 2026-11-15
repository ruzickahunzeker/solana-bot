use super::*;
use crate::application::wsol_submission::tests::{corrupt_record, WsolStageGate};
use crate::{
    adapters::persistence::wsol_conversion::SqliteWsolJournal,
    application::{
        ports::wsol_conversion::WsolWalletMetadata, wsol_submission_types::WsolSubmissionRecord,
    },
    conversion::{ConversionStore, WsolQuote},
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        trade::Address,
        wsol_progress::{WsolObservedOutcome, WsolSignatureObservation},
    },
};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

pub(crate) struct Shared {
    pub store: Arc<ConversionStore>,
    events: Mutex<Vec<String>>,
    selected: Mutex<u8>,
    mode: AtomicUsize,
    lookup_gate: Mutex<Option<Arc<WsolStageGate>>>,
    reads: AtomicUsize,
    corrupt_readback: AtomicUsize,
    observation: Mutex<Option<WsolSignatureObservation>>,
    lookup_drops: AtomicUsize,
    recovery_record: Mutex<Option<WsolSubmissionRecord>>,
    status_records: Mutex<VecDeque<Option<WsolSubmissionRecord>>>,
    quote_estimate: Mutex<Option<WsolQuoteEstimate>>,
    estimate_gate: Mutex<Option<Arc<WsolStageGate>>>,
    quote_save_gate: Mutex<Option<Arc<WsolStageGate>>>,
    saved_quote: Mutex<Option<WsolQuoteSnapshot>>,
}
impl Shared {
    fn event(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }
    pub(crate) fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
    pub(crate) fn replace_recovery(&self, record: WsolSubmissionRecord) {
        *self.recovery_record.lock().unwrap() = Some(record);
    }
    pub(crate) fn mode(&self, mode: usize) {
        self.mode.store(mode, Ordering::SeqCst);
    }
    pub(crate) fn replace_estimate(&self, estimate: WsolQuoteEstimate) {
        *self.quote_estimate.lock().unwrap() = Some(estimate);
    }
    pub(crate) fn saved_quote(&self) -> Option<WsolQuoteSnapshot> {
        self.saved_quote.lock().unwrap().clone()
    }
    pub(crate) fn pause_estimate(&self) -> Arc<WsolStageGate> {
        let gate = Arc::new(WsolStageGate::default());
        *self.estimate_gate.lock().unwrap() = Some(gate.clone());
        gate
    }
    pub(crate) fn pause_quote_save(&self) -> Arc<WsolStageGate> {
        let gate = Arc::new(WsolStageGate::default());
        *self.quote_save_gate.lock().unwrap() = Some(gate.clone());
        gate
    }
    pub(crate) fn set_status_records(&self, records: Vec<Option<WsolSubmissionRecord>>) {
        *self.status_records.lock().unwrap() = records.into();
    }
    pub(crate) fn set_observation(&self, observation: WsolSignatureObservation) {
        *self.observation.lock().unwrap() = Some(observation);
    }
    pub(crate) fn lookup_drops(&self) -> usize {
        self.lookup_drops.load(Ordering::SeqCst)
    }
    pub(crate) fn pause_lookup(&self) -> Arc<WsolStageGate> {
        let gate = Arc::new(WsolStageGate::default());
        *self.lookup_gate.lock().unwrap() = Some(gate.clone());
        gate
    }
}

pub(crate) fn signature_observation(
    commitment: ChainCommitment,
    error: Option<&str>,
) -> WsolSignatureObservation {
    WsolSignatureObservation {
        chain: ChainObservation {
            signature: "synthetic-signature".into(),
            commitment,
            slot: 42,
            error: error.map(Into::into),
        },
        context_slot: 42,
    }
}

struct PendingLookup(Arc<Shared>);
impl Drop for PendingLookup {
    fn drop(&mut self) {
        self.0.lookup_drops.fetch_add(1, Ordering::SeqCst);
    }
}
pub(crate) struct Source(Arc<Shared>);
pub(crate) struct QuoteReader {
    shared: Arc<Shared>,
    id: u8,
}
pub(crate) struct StatusReader {
    shared: Arc<Shared>,
}
pub(crate) struct Parser(Arc<Shared>);
pub(crate) struct Journal(Arc<Shared>);
impl TradeAddressParser for Parser {
    fn parse(&self, raw: &str) -> anyhow::Result<Address> {
        self.0.event("parse_owner");
        if raw == "bad" {
            bail!("synthetic address parse error");
        }
        Ok(Address::from_bytes([1; 32]))
    }
}
impl WsolQuerySource for Source {
    type QuoteReader = QuoteReader;
    type StatusReader = StatusReader;
    async fn metadata(&self, id: &str) -> anyhow::Result<WsolWalletMetadata> {
        self.0.event("metadata");
        let mode = self.0.mode.load(Ordering::SeqCst);
        if mode == 1 {
            bail!("synthetic metadata failure");
        }
        if mode == 21 {
            bail!("wallet not found");
        }
        if mode == 22 {
            bail!(WsolFailure::rejected("wallet not found"));
        }
        Ok(WsolWalletMetadata {
            wallet_id: if mode == 20 { "foreign-wallet" } else { id }.into(),
            pubkey: if mode == 4 {
                "bad".into()
            } else {
                Address::from_bytes([1; 32]).to_string()
            },
            owner_type: "telegram".into(),
            owner_id: if mode == 3 { "foreign" } else { "42" }.into(),
            enabled: mode != 2,
        })
    }
    fn quote_reader(&self, _: &str) -> Option<QuoteReader> {
        self.0.event("quote_reader");
        if self.0.mode.load(Ordering::SeqCst) == 5 {
            return None;
        }
        Some(QuoteReader {
            shared: self.0.clone(),
            id: *self.0.selected.lock().unwrap(),
        })
    }
    fn status_reader(&self, wallet: &str, signature: &str) -> anyhow::Result<StatusReader> {
        assert_eq!(wallet, "fixture-wallet");
        assert_eq!(signature, "synthetic-signature");
        self.0.event("status_reader");
        if self.0.mode.load(Ordering::SeqCst) == 6 {
            bail!("synthetic status constructor failure");
        }
        Ok(StatusReader {
            shared: self.0.clone(),
        })
    }
}
impl WsolQuoteReader for QuoteReader {
    async fn estimate(
        &self,
        owner: Address,
        input: &WsolQuoteInput,
    ) -> anyhow::Result<WsolQuoteEstimate> {
        assert_eq!(owner, Address::from_bytes([1; 32]));
        self.shared.event(format!("estimate:{}", self.id));
        *self.shared.selected.lock().unwrap() = 2;
        if self.shared.mode.load(Ordering::SeqCst) == 7 {
            bail!("synthetic estimation failure");
        }
        if self.shared.mode.load(Ordering::SeqCst) == 23 {
            bail!(WsolFailure::rejected("synthetic insufficient SOL balance"));
        }
        let gate = self.shared.estimate_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.pause().await;
        }
        if let Some(estimate) = self.shared.quote_estimate.lock().unwrap().clone() {
            return Ok(estimate);
        }
        tokio::task::yield_now().await;
        Ok(WsolQuoteEstimate {
            amount_lamports: if input.all {
                2_000_000
            } else {
                input
                    .amount_lamports
                    .ok_or_else(|| WsolFailure::rejected("amount_lamports is required"))?
            },
            source_balance_lamports: 2_000_000,
            ata_exists: true,
            estimated_network_fee_lamports: 10_000,
            temporary_rent_lamports: 0,
            rent_refund_lamports: if input.all { 2_039_280 } else { 0 },
        })
    }
}
impl WsolStatusReader for StatusReader {
    async fn lookup(&self) -> anyhow::Result<Option<WsolSignatureObservation>> {
        self.shared.event("lookup");
        let mode = self.shared.mode.load(Ordering::SeqCst);
        let pending = (mode == 15).then(|| PendingLookup(self.shared.clone()));
        let mut observation = match mode {
            8 => bail!("synthetic lookup failure"),
            9 => None,
            10 => Some(signature_observation(ChainCommitment::Finalized, None)),
            11 => Some(signature_observation(
                ChainCommitment::Finalized,
                Some("synthetic chain failure"),
            )),
            _ => Some(signature_observation(ChainCommitment::Processed, None)),
        };
        if let Some(replacement) = self.shared.observation.lock().unwrap().clone() {
            observation = Some(replacement);
        }
        let gate = self.shared.lookup_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.pause().await;
        }
        if pending.is_some() {
            std::future::pending::<()>().await;
        }
        Ok(observation)
    }
}
impl WsolQueryJournal for Journal {
    async fn recovery(
        &self,
        input: &WsolRecoveryInput,
    ) -> anyhow::Result<Option<WsolSubmissionRecord>> {
        self.0.event("recovery");
        if self.0.mode.load(Ordering::SeqCst) == 16 {
            bail!("synthetic recovery failure https://fixture.invalid/private-api-key");
        }
        if let Some(record) = self.0.recovery_record.lock().unwrap().clone() {
            return Ok(Some(record));
        }
        WsolQueryJournal::recovery(&SqliteWsolJournal::new(self.0.store.clone()), input).await
    }
    async fn save_quote(&self, quote: &WsolQuoteSnapshot) -> anyhow::Result<()> {
        self.0.event("save");
        if self.0.mode.load(Ordering::SeqCst) == 12 {
            bail!("synthetic quote write failure");
        }
        WsolQueryJournal::save_quote(&SqliteWsolJournal::new(self.0.store.clone()), quote).await?;
        *self.0.saved_quote.lock().unwrap() = Some(quote.clone());
        let gate = self.0.quote_save_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.pause().await;
        }
        match self.0.mode.load(Ordering::SeqCst) {
            24 => bail!("synthetic quote commit acknowledgment lost"),
            25 => bail!(WsolFailure::rejected("synthetic late quote rejection")),
            _ => Ok(()),
        }
    }
    async fn submission(&self, id: &str) -> anyhow::Result<Option<WsolSubmissionRecord>> {
        self.0.event("submission");
        let reads = self.0.reads.fetch_add(1, Ordering::SeqCst);
        let mode = self.0.mode.load(Ordering::SeqCst);
        if (mode == 17 && reads == 0) || (mode == 18 && reads > 0) {
            bail!("synthetic status read failure https://fixture.invalid/private-api-key");
        }
        if let Some(record) = self.0.status_records.lock().unwrap().pop_front() {
            return Ok(record);
        }
        let mut result = self.0.store.get_by_id(id).await?;
        let corrupt = self.0.corrupt_readback.load(Ordering::SeqCst);
        if reads > 0 && corrupt > 0 {
            corrupt_record(result.as_mut().unwrap(), corrupt - 1);
        }
        if self.0.mode.load(Ordering::SeqCst) == 14 {
            result.as_mut().unwrap().signed_transaction = None;
        }
        Ok(result)
    }
    async fn record_observation(
        &self,
        expected: &WsolSubmissionRecord,
        status: WsolObservedOutcome,
        error: Option<&str>,
        now: u64,
    ) -> anyhow::Result<()> {
        self.0.event(format!("record:{}", status.state().as_str()));
        if self.0.mode.load(Ordering::SeqCst) == 13 {
            bail!("synthetic observation write failure");
        }
        self.0
            .store
            .apply_signed_update(
                crate::application::wsol_submission_types::WsolSignedOperation::from_record(
                    expected,
                )?,
                crate::domain::wsol_progress::WsolUpdate::Observation(status),
                error,
                now,
            )
            .await?;
        if self.0.mode.load(Ordering::SeqCst) == 19 {
            bail!("synthetic observation commit acknowledgment lost");
        }
        Ok(())
    }
}
pub(crate) type Service = WsolQueryService<Source, Journal, Parser>;
pub(crate) async fn harness() -> (Arc<Service>, Arc<Shared>) {
    harness_with_store(Arc::new(
        ConversionStore::new("sqlite::memory:").await.unwrap(),
    ))
    .await
}
pub(crate) async fn harness_with_store(store: Arc<ConversionStore>) -> (Arc<Service>, Arc<Shared>) {
    let shared = Arc::new(Shared {
        store,
        events: Mutex::new(vec![]),
        selected: Mutex::new(1),
        mode: AtomicUsize::new(0),
        lookup_gate: Mutex::new(None),
        reads: AtomicUsize::new(0),
        corrupt_readback: AtomicUsize::new(0),
        observation: Mutex::new(None),
        lookup_drops: AtomicUsize::new(0),
        recovery_record: Mutex::new(None),
        status_records: Mutex::new(VecDeque::new()),
        quote_estimate: Mutex::new(None),
        estimate_gate: Mutex::new(None),
        quote_save_gate: Mutex::new(None),
        saved_quote: Mutex::new(None),
    });
    let service = Arc::new(Service::new(
        Source(shared.clone()),
        Some(Journal(shared.clone())),
        Parser(shared.clone()),
    ));
    (service, shared)
}
pub(crate) fn input() -> WsolQuoteInput {
    WsolQuoteInput {
        wallet_id: "fixture-wallet".into(),
        owner_type: "telegram".into(),
        owner_id: "42".into(),
        direction: WsolDirection::Wrap,
        amount_lamports: Some(1_000_000),
        all: false,
    }
}
pub(crate) async fn seed(shared: &Shared, status: &str) -> WsolSubmissionRecord {
    let mut quote: WsolQuote = crate::conversion::contract_tests::quote();
    quote.expires_at_ms = 4_000_000_000_000;
    let request = crate::conversion::contract_tests::submission();
    let (record, _) = shared
        .store
        .reserve_live(&request, &quote, 1)
        .await
        .unwrap();
    if status == "failed" {
        shared
            .store
            .mark_failed_before_sign(&record.submission_id, "synthetic pre-sign failure", 2)
            .await
            .unwrap();
    } else if status != "reserved" {
        shared
            .store
            .record_signed(&record.submission_id, "synthetic-signature", &[1, 2, 3], 2)
            .await
            .unwrap();
        if status != "signed" {
            shared
                .store
                .mark_broadcast_result(&record.submission_id, status, None, 3)
                .await
                .unwrap();
        }
    }
    shared
        .store
        .get_by_id(&record.submission_id)
        .await
        .unwrap()
        .unwrap()
}

pub(crate) fn recovery_input() -> WsolRecoveryInput {
    WsolRecoveryInput {
        idempotency_key: "fixture-key".into(),
        wallet_id: "fixture-wallet".into(),
        owner_type: "telegram".into(),
        owner_id: "42".into(),
    }
}

#[tokio::test]
async fn wsol_recovery_is_a_single_ledger_read_for_every_state_without_runtime_or_observation() {
    for status in [
        "reserved",
        "signed",
        "unknown",
        "submitted",
        "confirmed",
        "failed",
    ] {
        let (service, shared) = harness().await;
        let record = seed(&shared, status).await;
        shared.mode(1); // Even unusable metadata must not affect ledger recovery.
        for _ in 0..3 {
            let result = service.recover(recovery_input()).await.unwrap();
            assert_eq!(result.record, record);
            assert!(!result.duplicate);
        }
        assert_eq!(shared.events(), ["recovery", "recovery", "recovery"]);
        assert_eq!(
            shared.store.get_by_id(&record.submission_id).await.unwrap(),
            Some(record)
        );
    }
}

#[tokio::test]
async fn wsol_recovery_validates_literal_inputs_and_hides_missing_foreign_and_misbound_rows() {
    let (service, shared) = harness().await;
    let record = seed(&shared, "signed").await;
    for field in 0..4 {
        let mut input = recovery_input();
        let value = match field {
            0 => &mut input.idempotency_key,
            1 => &mut input.wallet_id,
            2 => &mut input.owner_type,
            _ => &mut input.owner_id,
        };
        value.push('x');
        let error = service.recover(input.clone()).await.err().unwrap();
        assert_eq!(
            error.downcast_ref::<WsolRecoveryFailure>(),
            Some(&WsolRecoveryFailure::NotFound)
        );
        // Application checks the binding independently of the adapter's SQL filter.
        shared.replace_recovery(record.clone());
        let error = service.recover(input).await.err().unwrap();
        assert_eq!(
            error.downcast_ref::<WsolRecoveryFailure>(),
            Some(&WsolRecoveryFailure::NotFound)
        );
    }
    let count = shared.events().len();
    for field in 0..4 {
        let mut input = recovery_input();
        match field {
            0 => input.idempotency_key = " ".into(),
            1 => input.wallet_id = " ".into(),
            2 => input.owner_type.clear(),
            _ => input.owner_id.clear(),
        }
        let error = service.recover(input).await.err().unwrap();
        assert_eq!(
            error.downcast_ref::<WsolRecoveryFailure>(),
            Some(&WsolRecoveryFailure::Rejected)
        );
    }
    for key in ["x".repeat(129), "é".repeat(65)] {
        let mut input = recovery_input();
        input.idempotency_key = key;
        assert!(service.recover(input).await.is_err());
    }
    assert_eq!(shared.events().len(), count);
}

#[tokio::test]
async fn wsol_recovery_rejects_incoherent_evidence_without_repair_or_leaking_storage_errors() {
    let (service, shared) = harness().await;
    let original = seed(&shared, "reserved").await;
    for field in 0..12 {
        let mut record = original.clone();
        match field {
            0 => record.submission_id.clear(),
            1 => record.quote_id.clear(),
            2 => record.wallet_pubkey.clear(),
            3 => record.fingerprint.clear(),
            4 => record.status = "unsupported".into(),
            5 => record.signature = Some("unexpected".into()),
            6 => record.signed_transaction = Some(vec![1]),
            7 => record.error = Some("dirty reserved".into()),
            8 => record.status = "signed".into(),
            9 => record.status = "confirmed".into(),
            10 => {
                record.status = "failed".into();
                record.signature = Some("orphaned".into());
            }
            _ => {
                record.status = "submitted".into();
                record.signature = Some(String::new());
                record.signed_transaction = Some(vec![1]);
            }
        }
        shared.replace_recovery(record);
        let error = service.recover(recovery_input()).await.err().unwrap();
        assert_eq!(
            error.downcast_ref::<WsolRecoveryFailure>(),
            Some(&WsolRecoveryFailure::Integrity),
            "{field}"
        );
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original.clone())
        );
    }
    shared.mode(16);
    let error = service.recover(recovery_input()).await.err().unwrap();
    assert_eq!(
        error.downcast_ref::<WsolRecoveryFailure>(),
        Some(&WsolRecoveryFailure::Unavailable)
    );
    assert!(!format!("{error:?}").contains("private-api-key"));
    assert!(shared.events().iter().all(|event| event == "recovery"));
    let disabled = Service::new(
        Source(shared.clone()),
        None::<Journal>,
        Parser(shared.clone()),
    );
    assert_eq!(
        disabled
            .recover(recovery_input())
            .await
            .err()
            .unwrap()
            .downcast_ref::<WsolRecoveryFailure>(),
        Some(&WsolRecoveryFailure::Unavailable)
    );
}

#[tokio::test]
async fn wsol_recovery_redacts_historical_errors_in_response_without_rewriting_the_ledger() {
    let (service, shared) = harness().await;
    let mut record = seed(&shared, "failed").await;
    record.error = Some("historical error https://fixture.invalid/private-api-key".into());
    shared.replace_recovery(record.clone());
    let result = service.recover(recovery_input()).await.unwrap();
    assert!(!result
        .record
        .error
        .as_ref()
        .unwrap()
        .contains("private-api-key"));
    assert_eq!(
        shared.recovery_record.lock().unwrap().as_ref(),
        Some(&record)
    );
    assert_eq!(shared.events(), ["recovery"]);
}

#[tokio::test]
async fn wsol_quote_saves_project_snapshot_with_original_identity_and_ttl_after_estimation() {
    let (service, shared) = harness().await;
    let before = current_time_ms().unwrap();
    let quote = service.quote(input()).await.unwrap();
    let after = current_time_ms().unwrap();
    assert_eq!(
        shared.events(),
        [
            "metadata",
            "parse_owner",
            "quote_reader",
            "estimate:1",
            "save"
        ]
    );
    assert_eq!(*shared.selected.lock().unwrap(), 2);
    assert!(quote.quote_id.starts_with("cq_"));
    assert_eq!(quote.quote_id.len(), 35);
    assert_eq!(quote.owner_id, "42");
    assert_eq!(quote.wallet_id, "fixture-wallet");
    assert_eq!(quote.amount_lamports, 1_000_000);
    assert!((before + 60_000..=after + 60_000).contains(&quote.expires_at_ms));
    assert_eq!(
        shared.store.get_quote(&quote.quote_id).await.unwrap(),
        Some((&quote).into())
    );
    assert!(shared
        .store
        .get_by_key("fixture-key")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn wsol_quote_metadata_all_parser_runtime_and_estimate_error_order_is_preserved() {
    for (mode, message, last) in [
        (1, "synthetic metadata failure", "metadata"),
        (
            2,
            "wallet is disabled or does not belong to this owner",
            "metadata",
        ),
        (
            3,
            "wallet is disabled or does not belong to this owner",
            "metadata",
        ),
        (4, "synthetic address parse error", "parse_owner"),
        (5, "wallet is not loaded", "quote_reader"),
        (7, "synthetic estimation failure", "estimate:1"),
        (12, "synthetic quote write failure", "save"),
    ] {
        let (service, shared) = harness().await;
        shared.mode(mode);
        assert_eq!(
            service.quote(input()).await.err().unwrap().to_string(),
            message
        );
        assert_eq!(shared.events().last().unwrap(), last);
    }
    for (amount, direction, message) in [
        (
            Some(1),
            WsolDirection::Wrap,
            "provide either all or amount_lamports",
        ),
        (
            None,
            WsolDirection::Wrap,
            "all is only supported for WSOL unwrap",
        ),
    ] {
        let (service, shared) = harness().await;
        shared.mode(4);
        let mut input = input();
        input.all = true;
        input.amount_lamports = amount;
        input.direction = direction;
        assert_eq!(
            service.quote(input).await.err().unwrap().to_string(),
            message
        );
        assert_eq!(shared.events(), ["metadata"]);
    }
}

#[tokio::test]
async fn wsol_quote_all_unwrap_retains_refund_but_missing_amount_is_still_a_late_error() {
    let (service, shared) = harness().await;
    let mut request = input();
    request.direction = WsolDirection::Unwrap;
    request.all = true;
    request.amount_lamports = None;
    let quote = service.quote(request).await.unwrap();
    assert_eq!(quote.amount_lamports, 2_000_000);
    assert_eq!(quote.rent_refund_lamports, 2_039_280);
    let mut request = input();
    request.amount_lamports = None;
    assert_eq!(
        service.quote(request).await.err().unwrap().to_string(),
        "amount_lamports is required"
    );
    assert_eq!(shared.events().last().unwrap(), "estimate:2");
}

#[tokio::test]
async fn wsol_query_disabled_store_precedes_metadata_record_and_signature_access() {
    let (_, shared) = harness().await;
    let service = Service::new(Source(shared.clone()), None, Parser(shared.clone()));
    assert_eq!(
        service.quote(input()).await.err().unwrap().to_string(),
        "conversion quotes are disabled"
    );
    assert_eq!(
        service
            .status("missing", "telegram", "42")
            .await
            .err()
            .unwrap()
            .to_string(),
        "conversion quotes are disabled"
    );
    assert!(shared.events().is_empty());
}

#[tokio::test]
async fn wsol_status_terminal_or_unsigned_records_do_not_request_rpc_or_metadata() {
    for status in ["reserved", "failed", "confirmed"] {
        let (service, shared) = harness().await;
        let record = seed(&shared, status).await;
        let outcome = service
            .status(&record.submission_id, "telegram", "42")
            .await
            .unwrap();
        assert!(!outcome.duplicate);
        assert_eq!(outcome.record, record);
        assert_eq!(shared.events(), ["submission", "submission"]);
    }
}

#[tokio::test]
async fn wsol_status_rejects_other_owner_before_constructor_and_propagates_constructor_failure() {
    let (service, shared) = harness().await;
    let record = seed(&shared, "signed").await;
    shared.mode(6);
    assert_eq!(
        service
            .status(&record.submission_id, "telegram", "foreign")
            .await
            .err()
            .unwrap()
            .to_string(),
        "conversion submission does not belong to this owner"
    );
    assert_eq!(shared.events(), ["submission"]);
    assert_eq!(
        service
            .status(&record.submission_id, "telegram", "42")
            .await
            .err()
            .unwrap()
            .to_string(),
        "synthetic status constructor failure"
    );
    assert_eq!(&shared.events()[1..], ["submission", "status_reader"]);
}

#[tokio::test]
async fn wsol_status_rpc_errors_and_missing_observations_preserve_original_signed_evidence() {
    for mode in [8, 9] {
        let (service, shared) = harness().await;
        let record = seed(&shared, "signed").await;
        shared.mode(mode);
        assert_eq!(
            service
                .status(&record.submission_id, "telegram", "42")
                .await
                .unwrap()
                .record,
            record
        );
        assert_eq!(
            shared.events(),
            ["submission", "status_reader", "lookup", "submission"]
        );
    }
}

#[tokio::test]
async fn wsol_status_uses_explicit_finalized_outcomes_and_reads_back_written_record() {
    for (mode, status, error) in [
        (0, "submitted", None),
        (10, "confirmed", None),
        (11, "failed", Some("synthetic chain failure")),
    ] {
        let (service, shared) = harness().await;
        let record = seed(&shared, "signed").await;
        shared.mode(mode);
        let result = service
            .status(&record.submission_id, "telegram", "42")
            .await
            .unwrap();
        assert_eq!(result.record.status, status);
        assert_eq!(result.record.error.as_deref(), error);
        assert_eq!(result.record.signature, record.signature);
        assert_eq!(result.record.signed_transaction, record.signed_transaction);
        assert_eq!(
            shared.events(),
            [
                "submission".into(),
                "status_reader".into(),
                "lookup".into(),
                format!("record:{status}"),
                "submission".into()
            ]
        );
    }
}

#[tokio::test]
async fn wsol_status_observation_write_failure_is_explicit_and_does_not_fake_a_terminal_receipt() {
    let (service, shared) = harness().await;
    let record = seed(&shared, "signed").await;
    shared.mode(13);
    assert_eq!(
        service
            .status(&record.submission_id, "telegram", "42")
            .await
            .err()
            .unwrap()
            .to_string(),
        "synthetic observation write failure"
    );
    assert_eq!(
        shared.store.get_by_id(&record.submission_id).await.unwrap(),
        Some(record)
    );
    assert_eq!(
        shared.events(),
        ["submission", "status_reader", "lookup", "record:submitted"]
    );
}

#[tokio::test]
async fn wsol_status_late_weak_observation_reads_concurrent_terminal_result() {
    for mode in [10, 11] {
        let (service, shared) = harness().await;
        let signed = seed(&shared, "signed").await;
        let gate = shared.pause_lookup();
        let task = tokio::spawn({
            let service = service.clone();
            let id = signed.submission_id.clone();
            async move { service.status(&id, "telegram", "42").await }
        });
        gate.wait_until_entered().await;
        shared.mode(mode);
        let authoritative = service
            .status(&signed.submission_id, "telegram", "42")
            .await
            .unwrap();
        assert_eq!(
            authoritative.record.status,
            if mode == 10 { "confirmed" } else { "failed" }
        );
        gate.release();
        let late = task.await.unwrap().unwrap();
        assert_eq!(late.record, authoritative.record);
        assert_eq!(late.record.signature, signed.signature);
        assert_eq!(late.record.signed_transaction, signed.signed_transaction);
        assert_eq!(shared.events().iter().filter(|e| *e == "lookup").count(), 2);
        let trace = shared.events();
        assert_eq!(
            service
                .status(&signed.submission_id, "telegram", "42")
                .await
                .unwrap()
                .record,
            late.record
        );
        assert_eq!(
            &shared.events()[trace.len()..],
            ["submission", "submission"]
        );
    }
}

#[tokio::test]
async fn wsol_status_rejects_final_readback_changes_even_without_an_rpc_observation() {
    for mode in [0, 9] {
        for field in 0..10 {
            let (service, shared) = harness().await;
            let signed = seed(&shared, "signed").await;
            shared.mode(mode);
            shared.corrupt_readback.store(field + 1, Ordering::SeqCst);
            let error = service
                .status(&signed.submission_id, "telegram", "42")
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                if field < 8 {
                    "WSOL submission identity changed during readback"
                } else {
                    "WSOL signed operation identity or evidence mismatch"
                },
                "mode {mode}, field {field}"
            );
            let current = shared
                .store
                .get_by_id(&signed.submission_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                current.status,
                if mode == 0 { "submitted" } else { "signed" }
            );
            assert_eq!(current.signature, signed.signature);
            assert_eq!(current.signed_transaction, signed.signed_transaction);
        }
    }
}

#[tokio::test]
async fn wsol_status_requires_complete_signed_evidence_before_requesting_rpc() {
    let (service, shared) = harness().await;
    let signed = seed(&shared, "signed").await;
    shared.mode(14);
    assert_eq!(
        service
            .status(&signed.submission_id, "telegram", "42")
            .await
            .err()
            .unwrap()
            .to_string(),
        "WSOL outcome requires persisted signed evidence"
    );
    assert_eq!(shared.events(), ["submission"]);
    assert_eq!(
        shared.store.get_by_id(&signed.submission_id).await.unwrap(),
        Some(signed)
    );
}

async fn reserve_another(shared: &Shared) -> anyhow::Result<(WsolSubmissionRecord, bool)> {
    let mut request = crate::conversion::contract_tests::submission();
    request.idempotency_key = "another-key".into();
    let mut quote = crate::conversion::contract_tests::quote();
    quote.expires_at_ms = 4_000_000_000_000;
    shared.store.reserve_live(&request, &quote, 4).await
}

#[tokio::test]
async fn wsol_status_non_final_success_and_errors_keep_wallet_pending_until_finalized_evidence() {
    for initial in ["signed", "unknown", "submitted"] {
        for commitment in [ChainCommitment::Processed, ChainCommitment::Confirmed] {
            for error in [None, Some("synthetic chain failure")] {
                let (service, shared) = harness().await;
                let original = seed(&shared, initial).await;
                shared.set_observation(signature_observation(commitment, error));
                let outcome = service
                    .status(&original.submission_id, "telegram", "42")
                    .await
                    .unwrap();
                assert_eq!(outcome.record.status, "submitted");
                let provisional =
                    error.map(|e| format!("non-final {} observation: {e}", commitment.as_str()));
                assert_eq!(
                    outcome.record.error.as_deref(),
                    if initial == "submitted" {
                        None
                    } else {
                        provisional.as_deref()
                    }
                );
                assert_eq!(outcome.record.signature, original.signature);
                assert_eq!(
                    outcome.record.signed_transaction,
                    original.signed_transaction
                );
                assert!(reserve_another(&shared)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("pending conversion"));
                shared.set_observation(signature_observation(ChainCommitment::Finalized, error));
                let final_record = service
                    .status(&original.submission_id, "telegram", "42")
                    .await
                    .unwrap()
                    .record;
                assert_eq!(
                    final_record.status,
                    if error.is_some() {
                        "failed"
                    } else {
                        "confirmed"
                    }
                );
                assert_eq!(final_record.error.as_deref(), error);
                assert_eq!(final_record.signature, original.signature);
                assert_eq!(final_record.signed_transaction, original.signed_transaction);
                assert!(reserve_another(&shared).await.unwrap().1);
                let trace = shared.events();
                assert_eq!(
                    service
                        .status(&original.submission_id, "telegram", "42")
                        .await
                        .unwrap()
                        .record,
                    final_record
                );
                assert_eq!(
                    &shared.events()[trace.len()..],
                    ["submission", "submission"]
                );
            }
        }
    }
}

#[tokio::test]
async fn wsol_status_wrong_signature_and_invalid_slots_never_change_evidence_or_release_wallet() {
    for state in ["signed", "unknown", "submitted"] {
        for field in 0..4 {
            let (service, shared) = harness().await;
            let original = seed(&shared, state).await;
            let mut wrong =
                signature_observation(ChainCommitment::Finalized, Some("untrusted failure"));
            match field {
                0 => wrong.chain.signature = "another-signature".into(),
                1 => wrong.chain.slot = 0,
                2 => wrong.context_slot = 0,
                _ => wrong.chain.slot = wrong.context_slot + 1,
            }
            shared.set_observation(wrong);
            assert_eq!(
                service
                    .status(&original.submission_id, "telegram", "42")
                    .await
                    .unwrap()
                    .record,
                original
            );
            assert_eq!(
                shared.events(),
                ["submission", "status_reader", "lookup", "submission"]
            );
            assert!(reserve_another(&shared).await.is_err());
        }
    }
}

#[tokio::test]
async fn wsol_status_deadline_cancels_pending_lookup_without_changing_prior_error_or_retrying() {
    let (service, shared) = harness().await;
    let signed = seed(&shared, "signed").await;
    shared
        .store
        .apply_signed_update(
            crate::application::wsol_submission_types::WsolSignedOperation::from_record(&signed)
                .unwrap(),
            crate::domain::wsol_progress::WsolUpdate::SimulationUncertain,
            Some("original simulation uncertainty"),
            3,
        )
        .await
        .unwrap();
    let original = shared
        .store
        .get_by_id(&signed.submission_id)
        .await
        .unwrap()
        .unwrap();
    shared.mode(15);
    let start = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        WSOL_STATUS_TIMEOUT + Duration::from_secs(2),
        service.status(&signed.submission_id, "telegram", "42"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(start.elapsed() >= WSOL_STATUS_TIMEOUT);
    assert_eq!(outcome.record, original);
    assert_eq!(shared.lookup_drops(), 1);
    assert_eq!(
        shared.events(),
        ["submission", "status_reader", "lookup", "submission"]
    );
    assert!(reserve_another(&shared).await.is_err());
}

#[tokio::test]
async fn wsol_status_timed_out_lookup_reads_concurrent_finalized_result_without_downgrade() {
    for mode in [10, 11] {
        let (service, shared) = harness().await;
        let original = seed(&shared, "signed").await;
        shared.mode(15);
        let gate = shared.pause_lookup();
        let task = tokio::spawn({
            let service = service.clone();
            let id = original.submission_id.clone();
            async move { service.status(&id, "telegram", "42").await }
        });
        gate.wait_until_entered().await;
        shared.mode(mode);
        let final_record = service
            .status(&original.submission_id, "telegram", "42")
            .await
            .unwrap()
            .record;
        gate.release();
        assert_eq!(task.await.unwrap().unwrap().record, final_record);
        assert_eq!(shared.lookup_drops(), 1);
        assert_eq!(
            shared
                .events()
                .iter()
                .filter(|e| e.as_str() == "lookup")
                .count(),
            2
        );
        assert_eq!(final_record.signature, original.signature);
        assert_eq!(final_record.signed_transaction, original.signed_transaction);
    }
}

#[tokio::test]
async fn wsol_status_cancelled_lookup_does_not_write_a_result_or_launch_another_query() {
    let (service, shared) = harness().await;
    let original = seed(&shared, "signed").await;
    shared.mode(15);
    let gate = shared.pause_lookup();
    let task = tokio::spawn({
        let service = service.clone();
        let id = original.submission_id.clone();
        async move { service.status(&id, "telegram", "42").await }
    });
    gate.wait_until_entered().await;
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(shared.lookup_drops(), 1);
    assert_eq!(
        shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(original)
    );
    assert_eq!(shared.events(), ["submission", "status_reader", "lookup"]);
}

#[tokio::test]
async fn wsol_status_late_provisional_error_cannot_overwrite_finalized_success_or_failure() {
    for error in [None, Some("finalized failure")] {
        let (service, shared) = harness().await;
        let original = seed(&shared, "signed").await;
        shared.set_observation(signature_observation(
            ChainCommitment::Processed,
            Some("provisional error"),
        ));
        let gate = shared.pause_lookup();
        let task = tokio::spawn({
            let service = service.clone();
            let id = original.submission_id.clone();
            async move { service.status(&id, "telegram", "42").await }
        });
        gate.wait_until_entered().await;
        shared.set_observation(signature_observation(ChainCommitment::Finalized, error));
        let final_record = service
            .status(&original.submission_id, "telegram", "42")
            .await
            .unwrap()
            .record;
        gate.release();
        assert_eq!(task.await.unwrap().unwrap().record, final_record);
        assert_eq!(final_record.error.as_deref(), error);
    }
}
