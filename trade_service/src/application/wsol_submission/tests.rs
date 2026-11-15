//! Keyless application execution with the actual temporary conversion store.
use super::*;
use crate::{
    adapters::persistence::wsol_conversion::SqliteWsolJournal,
    application::wsol_submission_types::*, conversion::ConversionStore, domain::trade::Address,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Stage {
    Reserved,
    Prepared,
    BeforeSignedWrite,
    AfterSignedWrite,
    Simulation,
    Broadcast,
    BeforeReceiptWrite,
    AfterReceiptWrite,
    BeforeFailureWrite,
    AfterFailureWrite,
    UnsignedSimulation,
}

pub(crate) struct Shared {
    pub store: Arc<ConversionStore>,
    trace: Mutex<Vec<String>>,
    selected: Mutex<Option<u8>>,
    mode: AtomicUsize,
    fail_write: AtomicUsize,
    gates: Mutex<HashMap<Stage, Arc<WsolStageGate>>>,
    corrupt_readback: AtomicUsize,
    corrupt_signed_receipt: AtomicUsize,
    corrupt_before_broadcast: AtomicUsize,
    corrupt_reservation: AtomicUsize,
    corrupt_failure_receipt: AtomicUsize,
    corrupt_failure_readback: AtomicUsize,
}

#[derive(Default)]
pub(crate) struct WsolStageGate {
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl WsolStageGate {
    pub(crate) async fn pause(&self) {
        self.entered.notify_one();
        self.resume.notified().await;
    }
    pub(crate) async fn wait_until_entered(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.notified())
            .await
            .expect("synthetic stage was not reached");
    }
    pub(crate) fn release(&self) {
        self.resume.notify_one();
    }
}

pub(crate) fn corrupt_record(record: &mut WsolSubmissionRecord, field: usize) {
    match field {
        0 => record.submission_id.push('x'),
        1 => record.idempotency_key.push('x'),
        2 => record.fingerprint.push('x'),
        3 => record.quote_id.push('x'),
        4 => record.wallet_id.push('x'),
        5 => record.wallet_pubkey.push('x'),
        6 => record.owner_type.push('x'),
        7 => record.owner_id.push('x'),
        8 => record.signature = Some("other-signature".into()),
        9 => record.signed_transaction = Some(vec![4, 5, 6]),
        _ => unreachable!(),
    }
}

fn corrupt_failed_record(record: &mut WsolSubmissionRecord, field: usize) {
    match field {
        11 => record.status = "reserved".into(),
        12 => record.error = Some("different failure".into()),
        13 => record.error = None,
        _ => corrupt_record(record, field - 1),
    }
}
impl Shared {
    fn event(&self, event: impl Into<String>) {
        self.trace.lock().unwrap().push(event.into());
    }
    pub(crate) fn events(&self) -> Vec<String> {
        self.trace.lock().unwrap().clone()
    }
    pub(crate) fn set_write_failure(&self, stage: usize) {
        self.fail_write.store(stage, Ordering::SeqCst);
    }
    pub(crate) fn set_runtime_mode(&self, mode: usize) {
        self.mode.store(mode, Ordering::SeqCst);
    }
    pub(crate) fn pause_broadcast(&self, uncertain: bool) -> Arc<WsolStageGate> {
        self.mode
            .store(if uncertain { 10 } else { 0 }, Ordering::SeqCst);
        self.pause_stage(Stage::Broadcast)
    }
    pub(crate) fn pause_stage(&self, stage: Stage) -> Arc<WsolStageGate> {
        let gate = Arc::new(WsolStageGate::default());
        self.gates.lock().unwrap().insert(stage, gate.clone());
        gate
    }
    async fn wait_at(&self, stage: Stage) {
        let gate = self.gates.lock().unwrap().remove(&stage);
        if let Some(gate) = gate {
            gate.pause().await;
        }
    }
    pub(crate) fn set_simulation_failure(&self, rpc_error: bool) {
        self.mode
            .store(if rpc_error { 9 } else { 8 }, Ordering::SeqCst);
    }
    pub(crate) fn set_reservation_corruption(&self, field: usize) {
        self.corrupt_reservation.store(field, Ordering::SeqCst);
    }
    pub(crate) fn set_prepare_failure(&self) {
        self.mode.store(6, Ordering::SeqCst);
    }
    pub(crate) fn set_failure_corruption(&self, field: usize, readback: bool) {
        if readback {
            &self.corrupt_failure_readback
        } else {
            &self.corrupt_failure_receipt
        }
        .store(field, Ordering::SeqCst);
    }
}
pub(crate) struct FakeRuntime(Arc<Shared>);
pub(crate) struct Wallet {
    id: u8,
    shared: Arc<Shared>,
}
pub(crate) struct Prepared {
    id: u8,
    quote: WsolQuoteSnapshot,
}
pub(crate) struct Signed {
    id: u8,
    signature: String,
    bytes: Vec<u8>,
}
impl WsolArtifact for Signed {
    fn signature(&self) -> &str {
        &self.signature
    }
    fn signed_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl WsolRuntime for FakeRuntime {
    type Wallet = Wallet;
    async fn metadata(&self, _: &str) -> Result<WsolWalletMetadata> {
        self.0.event("metadata");
        let mode = self.0.mode.load(Ordering::SeqCst);
        if mode == 1 {
            bail!(WsolFailure::rejected("wallet not found"));
        }
        if mode == 21 {
            bail!("synthetic metadata RPC failure https://fixture.invalid/private-api-key");
        }
        Ok(WsolWalletMetadata {
            wallet_id: if mode == 22 { "foreign-wallet" } else { "w1" }.into(),
            pubkey: Address::from_bytes([1; 32]).to_string(),
            owner_type: "telegram".into(),
            owner_id: if mode == 3 { "foreign" } else { "123" }.into(),
            enabled: mode != 2,
        })
    }
    fn resolve(&self, _: &str) -> Option<Self::Wallet> {
        let id = *self.0.selected.lock().unwrap();
        self.0.event(format!("resolve:{id:?}"));
        id.map(|id| Wallet {
            id,
            shared: self.0.clone(),
        })
    }
}
impl WsolExecutionWallet for Wallet {
    type Prepared = Prepared;
    type Signed = Signed;
    fn pubkey(&self) -> Address {
        Address::from_bytes(
            [if self.shared.mode.load(Ordering::SeqCst) == 4 {
                2
            } else {
                1
            }; 32],
        )
    }
    async fn prepare(&self, quote: &WsolQuoteSnapshot) -> Result<Prepared> {
        self.shared.event(format!("prepare:{}", self.id));
        *self.shared.selected.lock().unwrap() = Some(2);
        if self.shared.mode.load(Ordering::SeqCst) == 6 {
            bail!("synthetic prepare failure");
        }
        if self.shared.mode.load(Ordering::SeqCst) == 13 {
            bail!("synthetic prepare failure https://fixture.invalid/private-key");
        }
        tokio::task::yield_now().await;
        self.shared.wait_at(Stage::Prepared).await;
        Ok(Prepared {
            id: self.id,
            quote: quote.clone(),
        })
    }
    async fn simulate_unsigned(
        &self,
        prepared: &Prepared,
        quote: &WsolQuoteSnapshot,
    ) -> Result<Option<u64>> {
        assert_eq!(prepared.id, self.id);
        assert_eq!(&prepared.quote, quote);
        self.shared.event(format!("unsigned:{}", self.id));
        self.shared.wait_at(Stage::UnsignedSimulation).await;
        match self.shared.mode.load(Ordering::SeqCst) {
            14 => bail!("synthetic unsigned RPC failure https://fixture.invalid/private-api-key"),
            15 => bail!(WsolFailure::rejected(
                "synthetic unsigned simulation rejected"
            )),
            16 => bail!(WsolFailure::integrity("synthetic unsigned message changed")),
            _ => {}
        }
        Ok(Some(999))
    }
    fn sign(&self, prepared: Prepared, quote: &WsolQuoteSnapshot) -> Result<Signed> {
        assert_eq!(prepared.id, self.id);
        assert_eq!(&prepared.quote, quote);
        self.shared.event(format!("sign:{}", self.id));
        if self.shared.mode.load(Ordering::SeqCst) == 7 {
            bail!("synthetic sign failure");
        }
        Ok(Signed {
            id: self.id,
            signature: "synthetic-signature".into(),
            bytes: vec![1, 2, 3],
        })
    }
    async fn simulate_signed(&self, signed: &Signed) -> Result<bool> {
        assert_eq!(signed.id, self.id);
        let stored = self.shared.store.get_by_key("key").await?.unwrap();
        assert_eq!(stored.status, "signed");
        assert_eq!(stored.signature.as_deref(), Some(signed.signature()));
        assert_eq!(
            stored.signed_transaction.as_deref(),
            Some(signed.signed_bytes())
        );
        self.shared.event(format!("simulate:{}", self.id));
        self.shared.wait_at(Stage::Simulation).await;
        match self.shared.mode.load(Ordering::SeqCst) {
            8 => Ok(false),
            9 => bail!("synthetic simulation RPC failure"),
            12 => std::future::pending().await,
            _ => Ok(true),
        }
    }
    async fn broadcast(&self, signed: &Signed) -> Result<String> {
        assert_eq!(signed.id, self.id);
        self.shared.event(format!("broadcast:{}", self.id));
        let stored = self.shared.store.get_by_key("key").await?.unwrap();
        assert_eq!(stored.status, "signed");
        assert_eq!(stored.signature.as_deref(), Some(signed.signature()));
        assert_eq!(
            stored.signed_transaction.as_deref(),
            Some(signed.signed_bytes())
        );
        self.shared.wait_at(Stage::Broadcast).await;
        match self.shared.mode.load(Ordering::SeqCst) {
            10 => bail!("synthetic send failure https://fixture.invalid/private-key"),
            11 => Ok("different-signature".into()),
            _ => Ok(signed.signature.clone()),
        }
    }
}

pub(crate) struct Journal {
    inner: SqliteWsolJournal,
    shared: Arc<Shared>,
}
impl WsolJournal for Journal {
    async fn quote(&self, id: &str) -> Result<Option<WsolQuoteSnapshot>> {
        self.shared.event("quote");
        if self.shared.fail_write.load(Ordering::SeqCst) == 8 {
            bail!("synthetic quote storage failure https://fixture.invalid/private-api-key");
        }
        self.inner.quote(id).await
    }
    async fn reserve(
        &self,
        input: &WsolSubmissionInput,
        quote: &WsolQuoteSnapshot,
        now: u64,
    ) -> Result<(WsolSubmissionRecord, bool)> {
        self.shared.event("reserve");
        if self.shared.fail_write.load(Ordering::SeqCst) == 9 {
            bail!("conversion quote expired");
        }
        let mut result = self.inner.reserve(input, quote, now).await?;
        if result.1 && self.shared.fail_write.load(Ordering::SeqCst) == 7 {
            bail!("synthetic reservation commit acknowledgment lost");
        }
        self.shared.wait_at(Stage::Reserved).await;
        let corrupt = self.shared.corrupt_reservation.load(Ordering::SeqCst);
        if corrupt > 0 {
            match corrupt {
                1 => result.0.submission_id.clear(),
                3 => result.0.fingerprint.clear(),
                11 => result.0.error = Some("dirty reservation".into()),
                12 => result.0.status = "signed".into(),
                13 => result.0.status = "failed".into(),
                14 => result.0.status = "corrupt".into(),
                _ => corrupt_record(&mut result.0, corrupt - 1),
            }
        }
        Ok(result)
    }
    async fn record_signed(
        &self,
        operation: WsolSignedOperation<'_>,
        now: u64,
    ) -> Result<WsolSubmissionRecord> {
        self.shared.event("record_signed");
        if self.shared.fail_write.load(Ordering::SeqCst) == 1 {
            bail!("synthetic signed write failure");
        }
        if self.shared.fail_write.load(Ordering::SeqCst) == 10 {
            bail!(WsolFailure::rejected("synthetic late rejection"));
        }
        self.shared.wait_at(Stage::BeforeSignedWrite).await;
        let mut result = self.inner.record_signed(operation, now).await?;
        if self.shared.fail_write.load(Ordering::SeqCst) == 5 {
            bail!("synthetic signed commit acknowledgment lost");
        }
        self.shared.wait_at(Stage::AfterSignedWrite).await;
        let corrupt = self.shared.corrupt_signed_receipt.load(Ordering::SeqCst);
        if corrupt > 0 {
            match corrupt {
                11 => result.status = "unknown".into(),
                12 => result.error = Some("synthetic unexpected error".into()),
                _ => corrupt_record(&mut result, corrupt - 1),
            }
        }
        Ok(result)
    }
    async fn record_simulation_failure(
        &self,
        operation: WsolSignedOperation<'_>,
        error: &str,
        now: u64,
    ) -> Result<()> {
        self.shared.event("simulation_failed");
        if self.shared.fail_write.load(Ordering::SeqCst) == 4 {
            bail!("synthetic simulation receipt write failure");
        }
        self.inner
            .record_simulation_failure(operation, error, now)
            .await
    }
    async fn fail_before_sign(
        &self,
        expected: &WsolSubmissionRecord,
        error: &str,
        now: u64,
    ) -> Result<WsolSubmissionRecord> {
        self.shared.event("fail_before_sign");
        if self.shared.fail_write.load(Ordering::SeqCst) == 3 {
            bail!("synthetic failure receipt write failure");
        }
        self.shared.wait_at(Stage::BeforeFailureWrite).await;
        let mut result = self.inner.fail_before_sign(expected, error, now).await?;
        if self.shared.fail_write.load(Ordering::SeqCst) == 6 {
            bail!("synthetic unsigned failure commit acknowledgment lost");
        }
        self.shared.wait_at(Stage::AfterFailureWrite).await;
        let corrupt = self.shared.corrupt_failure_receipt.load(Ordering::SeqCst);
        if corrupt > 0 {
            corrupt_failed_record(&mut result, corrupt);
        }
        Ok(result)
    }
    async fn record_broadcast(
        &self,
        operation: WsolSignedOperation<'_>,
        status: WsolBroadcastOutcome,
        error: Option<&str>,
        now: u64,
    ) -> Result<()> {
        self.shared
            .event(format!("record:{}", status.state().as_str()));
        if self.shared.fail_write.load(Ordering::SeqCst) == 2 {
            bail!("synthetic receipt write failure");
        }
        self.shared.wait_at(Stage::BeforeReceiptWrite).await;
        self.inner
            .record_broadcast(operation, status, error, now)
            .await?;
        self.shared.wait_at(Stage::AfterReceiptWrite).await;
        Ok(())
    }
    async fn by_id(&self, id: &str) -> Result<Option<WsolSubmissionRecord>> {
        self.shared.event("by_id");
        let mut result = self.inner.by_id(id).await?;
        let corrupt = self.shared.corrupt_before_broadcast.load(Ordering::SeqCst);
        if corrupt > 0 {
            let record = result.as_mut().unwrap();
            match corrupt {
                11 => record.status = "corrupt".into(),
                12 => record.status = "reserved".into(),
                _ => corrupt_record(record, corrupt - 1),
            }
        }
        let corrupt = self.shared.corrupt_failure_readback.load(Ordering::SeqCst);
        if corrupt > 0 {
            corrupt_failed_record(result.as_mut().unwrap(), corrupt);
        }
        Ok(result)
    }
    async fn by_key(&self, key: &str) -> Result<Option<WsolSubmissionRecord>> {
        self.shared.event("by_key");
        let mut result = self.inner.by_key(key).await?;
        let corrupt = self.shared.corrupt_readback.load(Ordering::SeqCst);
        if corrupt > 0 {
            corrupt_record(result.as_mut().unwrap(), corrupt - 1);
        }
        Ok(result)
    }
}

pub(crate) fn input() -> WsolSubmissionInput {
    WsolSubmissionInput {
        quote_id: "q1".into(),
        wallet_id: "w1".into(),
        owner_type: "telegram".into(),
        owner_id: "123".into(),
        simulate: false,
        confirmed: true,
        idempotency_key: "key".into(),
    }
}
pub(crate) fn policy() -> WsolSubmitPolicy {
    WsolSubmitPolicy {
        broadcast: BroadcastPolicy::Enabled,
        live_enabled: true,
    }
}
pub(crate) type Service = WsolSubmissionService<FakeRuntime, Journal>;
pub(crate) async fn harness() -> (Arc<Service>, Arc<Shared>) {
    let store = Arc::new(ConversionStore::new("sqlite::memory:").await.unwrap());
    harness_with_store(store).await
}

pub(crate) async fn harness_with_store(store: Arc<ConversionStore>) -> (Arc<Service>, Arc<Shared>) {
    let mut quote = crate::conversion::contract_tests::quote();
    quote.quote_id = "q1".into();
    quote.wallet_id = "w1".into();
    quote.owner_id = "123".into();
    quote.expires_at_ms = 4_000_000_000_000;
    if store.get_quote(&quote.quote_id).await.unwrap().is_none() {
        store.save_quote(&quote).await.unwrap();
    }
    let shared = Arc::new(Shared {
        store: store.clone(),
        trace: Mutex::new(vec![]),
        selected: Mutex::new(Some(1)),
        mode: AtomicUsize::new(0),
        fail_write: AtomicUsize::new(0),
        gates: Mutex::new(HashMap::new()),
        corrupt_readback: AtomicUsize::new(0),
        corrupt_signed_receipt: AtomicUsize::new(0),
        corrupt_before_broadcast: AtomicUsize::new(0),
        corrupt_reservation: AtomicUsize::new(0),
        corrupt_failure_receipt: AtomicUsize::new(0),
        corrupt_failure_readback: AtomicUsize::new(0),
    });
    let service = Arc::new(Service::new(
        FakeRuntime(shared.clone()),
        Some(Journal {
            inner: SqliteWsolJournal::new(store),
            shared: shared.clone(),
        }),
    ));
    (service, shared)
}

#[tokio::test]
async fn wsol_live_retains_original_wallet_and_rpc_after_registry_replacement_and_journals_before_send(
) {
    let (service, shared) = harness().await;
    let result = service.submit(input(), policy()).await.unwrap();
    assert_eq!(result.record.status, "submitted");
    assert!(!result.duplicate);
    assert_eq!(
        shared.events(),
        [
            "quote",
            "reserve",
            "metadata",
            "resolve:Some(1)",
            "prepare:1",
            "sign:1",
            "record_signed",
            "simulate:1",
            "by_id",
            "broadcast:1",
            "record:submitted",
            "by_key"
        ]
    );
    assert_eq!(*shared.selected.lock().unwrap(), Some(2));
    let duplicate = service.submit(input(), policy()).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record, result.record);
    assert_eq!(&shared.events()[12..], ["quote", "reserve"]);
}

#[tokio::test]
async fn wsol_unsigned_simulation_retains_prepared_rpc_without_signing_or_live_reservation() {
    let (service, shared) = harness().await;
    let mut request = input();
    request.simulate = true;
    request.confirmed = false;
    let result = service.simulate(request).await.unwrap();
    assert_eq!(result.units_consumed, Some(999));
    assert_eq!(
        shared.events(),
        [
            "quote",
            "metadata",
            "resolve:Some(1)",
            "prepare:1",
            "unsigned:1"
        ]
    );
    assert!(shared.store.get_by_key("key").await.unwrap().is_none());
}

#[tokio::test]
async fn wsol_prepare_sign_metadata_and_runtime_failures_keep_original_failed_receipts() {
    for (mode, message) in [
        (1, "wallet not found"),
        (2, "wallet is disabled"),
        (
            3,
            "conversion quote does not belong to this wallet and owner",
        ),
        (4, "loaded wallet signer does not match quote"),
        (5, "wallet is not loaded"),
        (6, "synthetic prepare failure"),
        (7, "synthetic sign failure"),
    ] {
        let (service, shared) = harness().await;
        shared.mode.store(mode, Ordering::SeqCst);
        if mode == 5 {
            *shared.selected.lock().unwrap() = None;
        }
        let result = service.submit(input(), policy()).await.unwrap();
        assert_eq!(result.record.status, "failed", "mode {mode}");
        assert_eq!(result.record.error.as_deref(), Some(message));
        assert!(result.record.signature.is_none());
        assert!(result.record.signed_transaction.is_none());
        assert!(!shared
            .events()
            .iter()
            .any(|event| event.starts_with("broadcast:")));
        let trace = shared.events();
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, result.record);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_signed_and_receipt_write_failures_are_propagated_without_retry() {
    for failure in [1, 2] {
        let (service, shared) = harness().await;
        shared.fail_write.store(failure, Ordering::SeqCst);
        assert!(service.submit(input(), policy()).await.is_err());
        let stored = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(
            stored.status,
            if failure == 1 { "reserved" } else { "signed" }
        );
        assert_eq!(stored.signature.is_some(), failure == 2);
        let trace = shared.events();
        assert_eq!(
            trace
                .iter()
                .filter(|event| event.starts_with("simulate:"))
                .count(),
            usize::from(failure == 2)
        );
        assert_eq!(
            trace
                .iter()
                .filter(|event| event.starts_with("broadcast:"))
                .count(),
            usize::from(failure == 2)
        );
        let result = service.submit(input(), policy()).await.unwrap();
        assert!(result.duplicate);
        assert_eq!(result.record, stored);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_signed_simulation_failure_rpc_error_and_timeout_retain_evidence_without_send_or_unlock(
) {
    for mode in [8, 9, 12] {
        let (service, shared) = harness().await;
        shared.mode.store(mode, Ordering::SeqCst);
        let outcome = service.submit(input(), policy()).await.unwrap();
        assert_eq!(outcome.record.status, "unknown");
        assert_eq!(
            outcome.record.signature.as_deref(),
            Some("synthetic-signature")
        );
        assert_eq!(
            outcome.record.signed_transaction.as_deref(),
            Some(&[1, 2, 3][..])
        );
        assert!(outcome
            .record
            .error
            .as_ref()
            .unwrap()
            .contains("signed evidence retained"));
        let trace = shared.events();
        assert!(!trace
            .iter()
            .any(|event| event == "fail_before_sign" || event.starts_with("broadcast:")));
        assert!(
            trace.iter().position(|e| e == "record_signed").unwrap()
                < trace
                    .iter()
                    .position(|e| e.starts_with("simulate:"))
                    .unwrap()
        );
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, outcome.record);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
        let mut another = input();
        another.idempotency_key = "another-key".into();
        assert_eq!(
            service
                .submit(another, policy())
                .await
                .err()
                .unwrap()
                .to_string(),
            "wallet has a pending conversion; reconcile it before submitting another"
        );
    }
}

#[tokio::test]
async fn wsol_signed_simulation_receipt_write_failure_preserves_signed_record_and_blocks_retry() {
    let (service, shared) = harness().await;
    shared.set_simulation_failure(true);
    shared.set_write_failure(4);
    assert_eq!(
        service
            .submit(input(), policy())
            .await
            .err()
            .unwrap()
            .to_string(),
        "synthetic simulation receipt write failure"
    );
    let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
    assert_eq!(signed.status, "signed");
    assert_eq!(signed.signature.as_deref(), Some("synthetic-signature"));
    assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
    let trace = shared.events();
    assert!(!trace
        .iter()
        .any(|e| e.starts_with("broadcast:") || e == "fail_before_sign"));
    let duplicate = service.submit(input(), policy()).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record, signed);
    assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
}

#[tokio::test]
async fn wsol_signed_write_receipt_corruption_is_rejected_before_simulation_or_broadcast() {
    for field in 1..=12 {
        let (service, shared) = harness().await;
        shared.corrupt_signed_receipt.store(field, Ordering::SeqCst);
        assert!(
            service.submit(input(), policy()).await.is_err(),
            "field {field}"
        );
        let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(signed.status, "signed");
        assert_eq!(signed.signature.as_deref(), Some("synthetic-signature"));
        assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        let trace = shared.events();
        assert!(!trace
            .iter()
            .any(|e| e.starts_with("simulate:") || e.starts_with("broadcast:")));
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, signed);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_final_prebroadcast_identity_evidence_and_state_readback_must_be_valid() {
    for field in 1..=12 {
        let (service, shared) = harness().await;
        shared
            .corrupt_before_broadcast
            .store(field, Ordering::SeqCst);
        assert!(
            service.submit(input(), policy()).await.is_err(),
            "field {field}"
        );
        let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(signed.status, "signed");
        assert_eq!(signed.signature.as_deref(), Some("synthetic-signature"));
        assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        let trace = shared.events();
        assert_eq!(
            trace.iter().filter(|e| e.starts_with("simulate:")).count(),
            1
        );
        assert!(!trace.iter().any(|e| e.starts_with("broadcast:")));
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, signed);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_lost_signed_commit_ack_reopens_signed_evidence_without_rpc_or_new_signature() {
    let dir = std::env::temp_dir().join(format!("wsol-commit-ack-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let store = Arc::new(ConversionStore::new(&url).await.unwrap());
    let (service, shared) = harness_with_store(store).await;
    shared.set_write_failure(5);
    assert_eq!(
        service
            .submit(input(), policy())
            .await
            .err()
            .unwrap()
            .to_string(),
        "synthetic signed commit acknowledgment lost"
    );
    let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
    assert_eq!(signed.status, "signed");
    assert_eq!(signed.signature.as_deref(), Some("synthetic-signature"));
    assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
    assert!(!shared
        .events()
        .iter()
        .any(|e| e.starts_with("simulate:") || e.starts_with("broadcast:")));
    drop(service);
    drop(shared);
    let store = Arc::new(ConversionStore::new(&url).await.unwrap());
    let (service, shared) = harness_with_store(store).await;
    let duplicate = service.submit(input(), policy()).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record, signed);
    assert_eq!(shared.events(), ["quote", "reserve"]);
    drop(service);
    drop(shared);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn wsol_signed_simulation_failure_cannot_downgrade_concurrent_chain_observation() {
    use crate::domain::wsol_progress::{WsolObservedOutcome as O, WsolUpdate};
    for status in [O::Submitted, O::Confirmed, O::Failed] {
        let (service, shared) = harness().await;
        shared.set_simulation_failure(false);
        let gate = shared.pause_stage(Stage::Simulation);
        let task = tokio::spawn({
            let service = service.clone();
            async move { service.submit(input(), policy()).await }
        });
        gate.wait_until_entered().await;
        let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
        shared
            .store
            .apply_signed_update(
                WsolSignedOperation::from_record(&signed).unwrap(),
                WsolUpdate::Observation(status),
                None,
                current_time_ms().unwrap(),
            )
            .await
            .unwrap();
        let expected = shared.store.get_by_key("key").await.unwrap().unwrap();
        gate.release();
        let outcome = task.await.unwrap().unwrap();
        assert_eq!(outcome.record, expected);
        assert!(!shared.events().iter().any(|e| e.starts_with("broadcast:")));
    }
}

#[tokio::test]
async fn wsol_successful_signed_simulation_does_not_send_after_concurrent_observation_or_uncertainty(
) {
    use crate::domain::wsol_progress::{WsolObservedOutcome as O, WsolUpdate};
    for update in [
        WsolUpdate::Observation(O::Submitted),
        WsolUpdate::Observation(O::Confirmed),
        WsolUpdate::Observation(O::Failed),
        WsolUpdate::SimulationUncertain,
    ] {
        let (service, shared) = harness().await;
        let gate = shared.pause_stage(Stage::Simulation);
        let task = tokio::spawn({
            let service = service.clone();
            async move { service.submit(input(), policy()).await }
        });
        gate.wait_until_entered().await;
        let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
        shared
            .store
            .apply_signed_update(
                WsolSignedOperation::from_record(&signed).unwrap(),
                update,
                None,
                current_time_ms().unwrap(),
            )
            .await
            .unwrap();
        let expected = shared.store.get_by_key("key").await.unwrap().unwrap();
        gate.release();
        assert_eq!(task.await.unwrap().unwrap().record, expected);
        assert!(!shared.events().iter().any(|e| e.starts_with("broadcast:")));
    }
}

#[tokio::test]
async fn wsol_cancellation_at_eight_stages_reopens_original_evidence_without_resigning_or_resending(
) {
    for stage in [
        Stage::Reserved,
        Stage::Prepared,
        Stage::BeforeSignedWrite,
        Stage::AfterSignedWrite,
        Stage::Simulation,
        Stage::Broadcast,
        Stage::BeforeReceiptWrite,
        Stage::AfterReceiptWrite,
    ] {
        let dir = std::env::temp_dir().join(format!("wsol-cancellation-{}", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", dir.join("test.sqlite").display());
        let store = Arc::new(ConversionStore::new(&url).await.unwrap());
        let (service, shared) = harness_with_store(store).await;
        let gate = shared.pause_stage(stage);
        let task = tokio::spawn({
            let service = service.clone();
            async move { service.submit(input(), policy()).await }
        });
        gate.wait_until_entered().await;
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        let record = shared.store.get_by_key("key").await.unwrap().unwrap();
        let signed = !matches!(
            stage,
            Stage::Reserved | Stage::Prepared | Stage::BeforeSignedWrite
        );
        assert_eq!(
            record.status,
            if stage == Stage::AfterReceiptWrite {
                "submitted"
            } else if signed {
                "signed"
            } else {
                "reserved"
            },
            "{stage:?}"
        );
        assert_eq!(
            record.signature.as_deref(),
            signed.then_some("synthetic-signature")
        );
        assert_eq!(
            record.signed_transaction.as_deref(),
            signed.then_some(&[1, 2, 3][..])
        );
        if !signed {
            assert!(!shared
                .events()
                .iter()
                .any(|e| e.starts_with("simulate:") || e.starts_with("broadcast:")));
        }
        drop(service);
        drop(shared);
        let reopened = Arc::new(ConversionStore::new(&url).await.unwrap());
        assert_eq!(
            reopened.get_by_key("key").await.unwrap(),
            Some(record.clone())
        );
        let (service, shared) = harness_with_store(reopened).await;
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, record);
        assert_eq!(shared.events(), ["quote", "reserve"]);
        let mut another = input();
        another.idempotency_key = "another-key".into();
        assert!(service
            .submit(another, policy())
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("pending conversion"));
        assert!(!shared.events().iter().any(|e| e.starts_with("prepare:")
            || e.starts_with("sign:")
            || e.starts_with("simulate:")
            || e.starts_with("broadcast:")));
        drop(service);
        drop(shared);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn wsol_failure_receipt_write_failure_does_not_claim_success_or_execute_again() {
    let (service, shared) = harness().await;
    shared.mode.store(6, Ordering::SeqCst);
    shared.fail_write.store(3, Ordering::SeqCst);
    assert_eq!(
        service
            .submit(input(), policy())
            .await
            .err()
            .unwrap()
            .to_string(),
        "synthetic failure receipt write failure"
    );
    let trace = shared.events();
    assert_eq!(
        service
            .submit(input(), policy())
            .await
            .unwrap()
            .record
            .status,
        "reserved"
    );
    assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
}

#[tokio::test]
async fn wsol_uncertain_send_keeps_original_signature_and_does_not_rebroadcast() {
    for mode in [10, 11] {
        let (service, shared) = harness().await;
        shared.mode.store(mode, Ordering::SeqCst);
        let result = service.submit(input(), policy()).await.unwrap();
        assert_eq!(result.record.status, "unknown");
        assert_eq!(
            result.record.signature.as_deref(),
            Some("synthetic-signature")
        );
        assert_eq!(
            result.record.signed_transaction.as_deref(),
            Some(&[1, 2, 3][..])
        );
        assert!(!result.record.error.unwrap().contains("private-key"));
        service.submit(input(), policy()).await.unwrap();
        assert_eq!(
            shared
                .events()
                .iter()
                .filter(|event| event.starts_with("broadcast:"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn wsol_concurrent_requests_have_one_preparer_signer_and_sender() {
    let (service, shared) = harness().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let service = service.clone();
        tasks.push(tokio::spawn(async move {
            service.submit(input(), policy()).await.unwrap()
        }));
    }
    let mut created = 0;
    for task in tasks {
        created += usize::from(!task.await.unwrap().duplicate);
    }
    assert_eq!(created, 1);
    for prefix in ["resolve:", "prepare:", "sign:", "broadcast:"] {
        assert_eq!(
            shared
                .events()
                .iter()
                .filter(|event| event.starts_with(prefix))
                .count(),
            1,
            "{prefix}"
        );
    }
}

#[tokio::test]
async fn wsol_live_gates_and_simulation_binding_fail_before_runtime() {
    let (service, shared) = harness().await;
    let mut p = policy();
    p.broadcast = BroadcastPolicy::Disabled;
    assert!(service.submit(input(), p).await.is_err());
    p = policy();
    p.live_enabled = false;
    assert_eq!(
        service.submit(input(), p).await.err().unwrap().to_string(),
        "live WSOL conversion is disabled or request is simulation"
    );
    assert!(shared.events().is_empty());
    let mut request = input();
    request.simulate = true;
    request.owner_id = "foreign".into();
    assert_eq!(
        service.simulate(request).await.err().unwrap().to_string(),
        "submission does not match conversion quote"
    );
    assert_eq!(shared.events(), ["quote"]);
}

#[tokio::test]
async fn wsol_submit_late_broadcast_returns_authoritative_observation_without_downgrade() {
    use crate::domain::wsol_progress::{WsolObservedOutcome as O, WsolUpdate};
    for observed in [O::Submitted, O::Confirmed, O::Failed] {
        for uncertain in [false, true] {
            let (service, shared) = harness().await;
            let gate = shared.pause_broadcast(uncertain);
            let task = tokio::spawn({
                let service = service.clone();
                async move { service.submit(input(), policy()).await }
            });
            gate.wait_until_entered().await;
            let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
            let error = matches!(observed, O::Failed).then_some("synthetic chain failure");
            shared
                .store
                .apply_signed_update(
                    WsolSignedOperation::from_record(&signed).unwrap(),
                    WsolUpdate::Observation(observed),
                    error,
                    current_time_ms().unwrap(),
                )
                .await
                .unwrap();
            let expected = shared.store.get_by_key("key").await.unwrap().unwrap();
            gate.release();
            let outcome = task.await.unwrap().unwrap();
            assert_eq!(outcome.record, expected);
            assert!(!outcome.duplicate);
            assert_eq!(
                service.submit(input(), policy()).await.unwrap().record,
                expected
            );
            for prefix in ["sign:", "broadcast:"] {
                assert_eq!(
                    shared
                        .events()
                        .iter()
                        .filter(|e| e.starts_with(prefix))
                        .count(),
                    1
                );
            }
        }
    }
}

#[tokio::test]
async fn wsol_submit_rejects_identity_or_signed_evidence_changes_in_final_readback() {
    for field in 0..10 {
        let (service, shared) = harness().await;
        shared.corrupt_readback.store(field + 1, Ordering::SeqCst);
        assert_eq!(
            service
                .submit(input(), policy())
                .await
                .err()
                .unwrap()
                .to_string(),
            "WSOL signed operation identity or evidence mismatch",
            "field {field}"
        );
        let expected = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(expected.status, "submitted");
        shared.corrupt_readback.store(0, Ordering::SeqCst);
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, expected);
        assert_eq!(
            shared
                .events()
                .iter()
                .filter(|e| e.starts_with("broadcast:"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn wsol_readback_allows_concurrent_reservation_progress_but_rejects_incomplete_evidence() {
    let (service, _) = harness().await;
    let signed = service.submit(input(), policy()).await.unwrap().record;
    let mut reserved = signed.clone();
    reserved.status = "reserved".into();
    reserved.signature = None;
    reserved.signed_transaction = None;
    reserved.validate_readback(&signed).unwrap();
    for field in 0..8 {
        let mut wrong = signed.clone();
        corrupt_record(&mut wrong, field);
        assert!(reserved.validate_readback(&wrong).is_err());
    }
    for state in ["signed", "unknown", "submitted", "confirmed", "failed"] {
        let mut progressed = signed.clone();
        progressed.status = state.into();
        signed.validate_readback(&progressed).unwrap();
    }
    for field in 0..4 {
        let mut corrupt = signed.clone();
        match field {
            0 => corrupt.signature = None,
            1 => corrupt.signed_transaction = None,
            2 => corrupt.signature = Some(String::new()),
            _ => corrupt.signed_transaction = Some(vec![]),
        }
        assert!(corrupt.validate_readback(&signed).is_err());
        assert!(signed.validate_readback(&corrupt).is_err());
    }
}

#[tokio::test]
async fn wsol_reservation_receipt_must_match_request_before_wallet_or_signer_access() {
    for field in 1..=14 {
        let (service, shared) = harness().await;
        shared.set_reservation_corruption(field);
        assert!(
            service.submit(input(), policy()).await.is_err(),
            "field {field}"
        );
        assert_eq!(shared.events(), ["quote", "reserve"]);
        let reserved = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(reserved.status, "reserved");
        assert!(reserved.signature.is_none() && reserved.signed_transaction.is_none());
        shared.set_reservation_corruption(0);
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, reserved);
        assert_eq!(shared.events(), ["quote", "reserve", "quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_reservation_lost_commit_ack_reopens_same_key_without_wallet_or_signer_access() {
    let dir = std::env::temp_dir().join(format!("wsol-reserve-ack-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let (service, shared) =
        harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
    shared.set_write_failure(7);
    assert_eq!(
        service
            .submit(input(), policy())
            .await
            .err()
            .unwrap()
            .to_string(),
        "synthetic reservation commit acknowledgment lost"
    );
    assert_eq!(shared.events(), ["quote", "reserve"]);
    let reserved = shared.store.get_by_key("key").await.unwrap().unwrap();
    assert_eq!(reserved.status, "reserved");
    assert!(reserved.signature.is_none() && reserved.signed_transaction.is_none());
    drop(service);
    drop(shared);
    let (service, shared) =
        harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
    let duplicate = service.submit(input(), policy()).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record, reserved);
    assert_eq!(shared.events(), ["quote", "reserve"]);
    let mut another = input();
    another.idempotency_key = "another-key".into();
    assert!(service
        .submit(another, policy())
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("pending conversion"));
    assert_eq!(shared.events(), ["quote", "reserve", "quote", "reserve"]);
    drop(service);
    drop(shared);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn wsol_unsigned_failure_receipt_and_final_readback_cannot_substitute_identity_or_evidence() {
    for readback in [false, true] {
        for field in 1..=13 {
            let (service, shared) = harness().await;
            shared.set_prepare_failure();
            shared.set_failure_corruption(field, readback);
            assert!(
                service.submit(input(), policy()).await.is_err(),
                "readback {readback} field {field}"
            );
            let failed = shared.store.get_by_key("key").await.unwrap().unwrap();
            assert_eq!(failed.status, "failed");
            assert_eq!(failed.error.as_deref(), Some("synthetic prepare failure"));
            assert!(failed.signature.is_none() && failed.signed_transaction.is_none());
            assert!(!shared.events().iter().any(|e| e.starts_with("sign:")
                || e.starts_with("simulate:")
                || e.starts_with("broadcast:")));
            let trace = shared.events();
            let duplicate = service.submit(input(), policy()).await.unwrap();
            assert!(duplicate.duplicate);
            assert_eq!(duplicate.record, failed);
            assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
        }
    }
}

#[tokio::test]
async fn wsol_unsigned_failure_cancel_or_lost_commit_ack_reopens_only_original_receipt() {
    for stage in [
        Some(Stage::BeforeFailureWrite),
        Some(Stage::AfterFailureWrite),
        None,
    ] {
        let dir =
            std::env::temp_dir().join(format!("wsol-unsigned-cancel-{}", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", dir.join("test.sqlite").display());
        let (service, shared) =
            harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
        shared.set_prepare_failure();
        if let Some(stage) = stage {
            let gate = shared.pause_stage(stage);
            let task = tokio::spawn({
                let service = service.clone();
                async move { service.submit(input(), policy()).await }
            });
            gate.wait_until_entered().await;
            task.abort();
            assert!(task.await.err().unwrap().is_cancelled());
        } else {
            shared.set_write_failure(6);
            assert_eq!(
                service
                    .submit(input(), policy())
                    .await
                    .err()
                    .unwrap()
                    .to_string(),
                "synthetic unsigned failure commit acknowledgment lost"
            );
        }
        let original = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(
            original.status,
            if stage == Some(Stage::BeforeFailureWrite) {
                "reserved"
            } else {
                "failed"
            }
        );
        assert!(original.signature.is_none() && original.signed_transaction.is_none());
        assert!(!shared.events().iter().any(|e| e.starts_with("sign:")
            || e.starts_with("simulate:")
            || e.starts_with("broadcast:")));
        drop(service);
        drop(shared);
        let (service, shared) =
            harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
        let duplicate = service.submit(input(), policy()).await.unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record, original);
        assert_eq!(shared.events(), ["quote", "reserve"]);
        let mut another = input();
        another.idempotency_key = "another-key".into();
        let quote = shared.store.get_quote("q1").await.unwrap().unwrap();
        assert_eq!(
            shared
                .store
                .reserve_live(
                    &crate::conversion::WsolSubmission::from(&another),
                    &quote,
                    4
                )
                .await
                .is_ok(),
            original.status == "failed"
        );
        drop(service);
        drop(shared);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn wsol_unsigned_failure_cannot_overwrite_a_concurrent_signed_operation() {
    let (service, shared) = harness().await;
    shared.set_prepare_failure();
    let gate = shared.pause_stage(Stage::BeforeFailureWrite);
    let task = tokio::spawn({
        let service = service.clone();
        async move { service.submit(input(), policy()).await }
    });
    gate.wait_until_entered().await;
    let original = shared.store.get_by_key("key").await.unwrap().unwrap();
    shared
        .store
        .record_signed(
            &original.submission_id,
            "synthetic-concurrent-signature",
            &[4, 5, 6],
            current_time_ms().unwrap(),
        )
        .await
        .unwrap();
    let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
    gate.release();
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        shared.store.get_by_key("key").await.unwrap(),
        Some(signed.clone())
    );
    let trace = shared.events();
    let duplicate = service.submit(input(), policy()).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record, signed);
    assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
    assert!(!shared.events().iter().any(|e| e.starts_with("sign:")
        || e.starts_with("simulate:")
        || e.starts_with("broadcast:")));
}

#[tokio::test]
async fn wsol_unsigned_failure_redaction_is_stable_across_write_and_final_readback() {
    let (service, shared) = harness().await;
    shared.mode.store(13, Ordering::SeqCst);
    let result = service.submit(input(), policy()).await.unwrap();
    assert_eq!(result.record.status, "failed");
    assert!(!result
        .record
        .error
        .as_ref()
        .unwrap()
        .contains("private-key"));
    assert_eq!(
        result.record.error.as_deref(),
        Some(
            crate::sanitize::redact_urls(
                "synthetic prepare failure https://fixture.invalid/private-key"
            )
            .as_str()
        )
    );
    let duplicate = service.submit(input(), policy()).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record, result.record);
}

#[tokio::test]
async fn wsol_submit_fault_stages_distinguish_reads_and_uncertain_mutations_without_retry() {
    use crate::application::wsol_failure::{WsolFailureKind as Kind, WsolFailureStage as Phase};
    for (fault, kind, stage, known_id, known_signature) in [
        (
            1,
            Kind::OutcomeUnknown,
            Phase::SignedPersistence,
            true,
            false,
        ),
        (2, Kind::OutcomeUnknown, Phase::BroadcastReceipt, true, true),
        (3, Kind::OutcomeUnknown, Phase::UnsignedFailure, true, false),
        (
            4,
            Kind::OutcomeUnknown,
            Phase::SimulationReceipt,
            true,
            true,
        ),
        (
            5,
            Kind::OutcomeUnknown,
            Phase::SignedPersistence,
            true,
            false,
        ),
        (6, Kind::OutcomeUnknown, Phase::UnsignedFailure, true, false),
        (7, Kind::OutcomeUnknown, Phase::Reservation, false, false),
        (8, Kind::Unavailable, Phase::QuoteLookup, false, false),
        (9, Kind::OutcomeUnknown, Phase::Reservation, false, false),
        (
            10,
            Kind::OutcomeUnknown,
            Phase::SignedPersistence,
            true,
            false,
        ),
    ] {
        let (service, shared) = harness().await;
        shared.set_write_failure(fault);
        if matches!(fault, 3 | 6) {
            shared.set_prepare_failure();
        }
        if fault == 4 {
            shared.set_simulation_failure(false);
        }
        let error = service.submit(input(), policy()).await.err().unwrap();
        let failure = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!(failure.kind, kind, "fault {fault}");
        assert_eq!(failure.stage, stage);
        assert_eq!(failure.submission_id.is_some(), known_id);
        assert_eq!(failure.signature.is_some(), known_signature);
        assert!(!format!("{failure:?}").contains("private-api-key"));
        let trace = shared.events();
        assert_eq!(
            trace.iter().filter(|e| e.starts_with("broadcast:")).count(),
            usize::from(fault == 2)
        );
        if shared.store.get_by_key("key").await.unwrap().is_some() {
            assert!(service.submit(input(), policy()).await.unwrap().duplicate);
            assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
        }
    }
}

#[tokio::test]
async fn wsol_submit_fault_context_never_uses_identifiers_from_a_misbound_receipt() {
    let (service, shared) = harness().await;
    shared.set_reservation_corruption(8);
    let error = service.submit(input(), policy()).await.err().unwrap();
    let failure = error.downcast_ref::<WsolFailure>().unwrap();
    assert_eq!(
        failure.kind,
        crate::application::wsol_failure::WsolFailureKind::Integrity
    );
    assert!(failure.submission_id.is_none() && failure.signature.is_none());
    let (service, shared) = harness().await;
    shared.set_prepare_failure();
    shared.set_failure_corruption(1, false);
    let error = service.submit(input(), policy()).await.err().unwrap();
    let failure = error.downcast_ref::<WsolFailure>().unwrap();
    let original = shared.store.get_by_key("key").await.unwrap().unwrap();
    assert_eq!(
        failure.submission_id.as_deref(),
        Some(original.submission_id.as_str())
    );
    assert!(failure.signature.is_none());
}
