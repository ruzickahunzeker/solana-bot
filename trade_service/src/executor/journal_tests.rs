use super::*;
use crate::application::{
    ports::ordinary_journal::TradeReservation,
    trade_persistence_input::{RejectedTradeInput, ValidatedTradeInput},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Journal {
    mode: TradeJournalMode,
    fail: bool,
    writes: AtomicUsize,
    mode_reads: AtomicUsize,
}

impl OrdinaryTradeJournal for Journal {
    fn ensure_snapshot_storage(
        &self,
        _: crate::application::ports::ordinary_journal::SnapshotWrite,
    ) -> anyhow::Result<()> {
        panic!("execution update helpers must not prepare snapshots")
    }
    fn mode(&self) -> TradeJournalMode {
        self.mode_reads.fetch_add(1, Ordering::SeqCst);
        self.mode
    }
    async fn reserve_record(
        &self,
        _: &TradeRecord,
        _: &ValidatedTradeInput,
    ) -> anyhow::Result<TradeReservation> {
        panic!("not a reservation test")
    }
    async fn mirror_accepted(&self, _: &TradeRecord, _: ValidatedTradeInput) -> anyhow::Result<()> {
        panic!("not a mirror test")
    }
    async fn mirror_rejection(&self, _: &TradeRecord, _: RejectedTradeInput) -> anyhow::Result<()> {
        panic!("not a mirror test")
    }
    async fn update_record(&self, _: &TradeRecord) -> anyhow::Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("synthetic journal write failure");
        }
        Ok(())
    }
    async fn read_record(&self, _: &str) -> anyhow::Result<Option<TradeRecord>> {
        panic!("not a query test")
    }
    async fn recent_records(&self, _: usize) -> anyhow::Result<Vec<TradeRecord>> {
        panic!("not a query test")
    }
}

fn record() -> TradeRecord {
    use crate::domain::trade::{Pool, Side};
    TradeRecord {
        request_id: "tr_generic_journal".into(),
        wallet_id: "fixture-wallet".into(),
        wallet_pubkey: "public-identity".into(),
        side: Side::Buy,
        pool: Pool::Pump,
        mint: "synthetic-mint".into(),
        pool_id: None,
        execution_method: ExecutionMethod::Sdk,
        status: TradeStatus::Submitting,
        signature: None,
        error: None,
        duplicate: false,
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

fn journal(mode: TradeJournalMode, fail: bool) -> Journal {
    Journal {
        mode,
        fail,
        writes: AtomicUsize::new(0),
        mode_reads: AtomicUsize::new(0),
    }
}

#[tokio::test]
async fn missing_record_never_calls_generic_journal_or_reads_its_mode() {
    let journal = journal(TradeJournalMode::Durable, true);
    assert!(!persist_before_execution(&journal, None).await);
    persist_update(&journal, None).await;
    assert_eq!(journal.writes.load(Ordering::SeqCst), 0);
    assert_eq!(journal.mode_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn successful_writes_do_not_probe_mode_or_repeat_storage_calls() {
    for mode in [
        TradeJournalMode::Memory,
        TradeJournalMode::Shadow,
        TradeJournalMode::Durable,
    ] {
        let journal = journal(mode, false);
        let record = record();
        assert!(persist_before_execution(&journal, Some(&record)).await);
        persist_update(&journal, Some(&record)).await;
        assert_eq!(journal.writes.load(Ordering::SeqCst), 2);
        assert_eq!(journal.mode_reads.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn durable_failure_stops_and_shadow_failure_continues_without_retrying_the_journal() {
    for mode in [TradeJournalMode::Shadow, TradeJournalMode::Durable] {
        let journal = journal(mode, true);
        let record = record();
        assert_eq!(
            persist_before_execution(&journal, Some(&record)).await,
            mode != TradeJournalMode::Durable
        );
        assert_eq!(journal.writes.load(Ordering::SeqCst), 1);
        assert_eq!(journal.mode_reads.load(Ordering::SeqCst), 1);
        persist_update(&journal, Some(&record)).await;
        assert_eq!(journal.writes.load(Ordering::SeqCst), 2);
        assert_eq!(journal.mode_reads.load(Ordering::SeqCst), 1);
    }
}
