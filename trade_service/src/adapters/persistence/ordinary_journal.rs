use crate::{
    application::{
        ports::ordinary_journal::{
            OrdinaryTradeJournal, SnapshotWrite, TradeJournalMode, TradeReservation,
        },
        trade_persistence_input::{RejectedTradeInput, ValidatedTradeInput},
        trade_record::TradeRecordV1,
    },
    config::TradePersistenceMode,
    persistence::TradePersistence,
};

impl crate::application::ports::ordinary_runtime::OrdinaryExecutionStorage for TradePersistence {
    type SdkJournal = crate::adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims;

    fn sdk_journal(&self) -> Option<Self::SdkJournal> {
        self.ordinary_execution_claims()
    }

    async fn fail_unclaimed_preparation(
        &self,
        execution: &crate::application::trade_admission::AdmittedTrade,
        error: &str,
    ) -> anyhow::Result<Option<TradeRecordV1>> {
        self.ordinary_execution_claims()
            .ok_or_else(|| anyhow::anyhow!("durable SDK preparation failure store unavailable"))?
            .fail_unclaimed_preparation(execution, error)
            .await
    }
}

impl OrdinaryTradeJournal for TradePersistence {
    fn ensure_snapshot_storage(&self, write: SnapshotWrite) -> anyhow::Result<()> {
        self.require_storage(match write {
            SnapshotWrite::Reservation => "durable database is unavailable",
            SnapshotWrite::Mirror => "trade database is unavailable",
        })
    }
    fn mode(&self) -> TradeJournalMode {
        match TradePersistence::mode(self) {
            TradePersistenceMode::Memory => TradeJournalMode::Memory,
            TradePersistenceMode::Shadow => TradeJournalMode::Shadow,
            TradePersistenceMode::Durable => TradeJournalMode::Durable,
        }
    }
    async fn reserve_record(
        &self,
        record: &TradeRecordV1,
        input: &ValidatedTradeInput,
    ) -> anyhow::Result<TradeReservation> {
        self.reserve_input(record, input).await
    }
    async fn mirror_accepted(
        &self,
        record: &TradeRecordV1,
        input: ValidatedTradeInput,
    ) -> anyhow::Result<()> {
        self.mirror_validated(record, input).await
    }
    async fn mirror_rejection(
        &self,
        record: &TradeRecordV1,
        input: RejectedTradeInput,
    ) -> anyhow::Result<()> {
        self.mirror_rejected(record, input).await
    }
    async fn update_record(&self, record: &TradeRecordV1) -> anyhow::Result<()> {
        self.update(record).await
    }
    async fn read_record(&self, request_id: &str) -> anyhow::Result<Option<TradeRecordV1>> {
        self.get(request_id).await
    }
    async fn recent_records(&self, limit: usize) -> anyhow::Result<Vec<TradeRecordV1>> {
        self.recent(limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapters::persistence::trade_snapshot::{rejected, validated},
        config::TradePersistenceConfig,
        domain::trade::TradeStatus,
        models::TradeRequest,
        state::TradeStore,
        validation::{validate_trade, NormalizedTrade},
    };

    async fn fixture(
        mode: TradePersistenceMode,
    ) -> (
        TradePersistence,
        TradeRequest,
        NormalizedTrade,
        TradeRecordV1,
    ) {
        let state = crate::http_contract_tests::state().await;
        let request: TradeRequest = serde_json::from_value(serde_json::json!({"side":"buy","pool":"pump",
            "mint":"11111111111111111111111111111111","amount_sol":0.001,"idempotency_key":"journal-port"})).unwrap();
        let normalized = validate_trade(&request, &state.config).unwrap();
        let mut record = TradeStore::default().create_with_id(
            "tr_journal_port".into(),
            &request,
            "fixture-wallet".into(),
            "11111111111111111111111111111111".into(),
        );
        record.created_at_ms = 1;
        record.updated_at_ms = 1;
        let persistence = TradePersistence::new(&TradePersistenceConfig {
            mode,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap();
        (persistence, request, normalized, record)
    }

    async fn wait_record(
        journal: &impl OrdinaryTradeJournal,
        id: &str,
        status: TradeStatus,
    ) -> TradeRecordV1 {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(record) = journal.read_record(id).await.unwrap() {
                    if record.status == status {
                        break record;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn real_adapter_maps_all_modes_and_executes_reserve_mirror_update_and_queries() {
        for (mode, expected) in [
            (TradePersistenceMode::Memory, TradeJournalMode::Memory),
            (TradePersistenceMode::Shadow, TradeJournalMode::Shadow),
            (TradePersistenceMode::Durable, TradeJournalMode::Durable),
        ] {
            let (journal, request, normalized, mut record) = fixture(mode).await;
            assert_eq!(OrdinaryTradeJournal::mode(&journal), expected);
            let input = validated(&record.wallet_id, &request, &normalized).unwrap();
            assert!(matches!(
                journal.reserve_record(&record, &input).await.unwrap(),
                TradeReservation::Created
            ));
            journal.mirror_accepted(&record, input).await.unwrap();
            if expected == TradeJournalMode::Memory {
                assert_eq!(
                    journal
                        .read_record(&record.request_id)
                        .await
                        .err()
                        .unwrap()
                        .to_string(),
                    "trade database is unavailable"
                );
                assert!(journal.recent_records(10).await.is_err());
                journal.update_record(&record).await.unwrap();
            } else {
                wait_record(&journal, &record.request_id, TradeStatus::Accepted).await;
                record.status = TradeStatus::Submitted;
                record.signature = Some("original-port-signature".into());
                journal.update_record(&record).await.unwrap();
                let saved = wait_record(&journal, &record.request_id, TradeStatus::Submitted).await;
                assert_eq!(saved.signature, record.signature);
                assert_eq!(journal.recent_records(10).await.unwrap().len(), 1);
            }
        }
    }

    #[tokio::test]
    async fn durable_port_keeps_duplicate_conflict_rejected_key_and_original_record() {
        let (journal, request, normalized, record) = fixture(TradePersistenceMode::Durable).await;
        let input = validated(&record.wallet_id, &request, &normalized).unwrap();
        assert!(matches!(
            journal.reserve_record(&record, &input).await.unwrap(),
            TradeReservation::Created
        ));
        assert!(
            matches!(journal.reserve_record(&record, &input).await.unwrap(), TradeReservation::Duplicate(r) if r.request_id == record.request_id)
        );
        let mut different = request.clone();
        different.amount_sol = Some(0.002);
        let state = crate::http_contract_tests::state().await;
        let changed = validated(
            &record.wallet_id,
            &different,
            &validate_trade(&different, &state.config).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            journal.reserve_record(&record, &changed).await.unwrap(),
            TradeReservation::Conflict
        ));
        let mut rejected_record = record.clone();
        rejected_record.request_id = "tr_rejection_port".into();
        rejected_record.status = TradeStatus::Rejected;
        rejected_record.created_at_ms = 2;
        journal
            .mirror_rejection(
                &rejected_record,
                rejected(&record.wallet_id, &rejected_record.request_id, &request).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(journal.recent_records(10).await.unwrap().len(), 2);
        assert_eq!(
            journal.recent_records(1).await.unwrap()[0].request_id,
            rejected_record.request_id
        );
        assert!(
            matches!(journal.reserve_record(&record, &input).await.unwrap(), TradeReservation::Duplicate(r) if r.status == TradeStatus::Accepted)
        );
    }

    #[tokio::test]
    async fn port_update_keeps_original_signature_and_pool_errors_are_not_masked() {
        let (journal, request, normalized, mut record) =
            fixture(TradePersistenceMode::Durable).await;
        let input = validated(&record.wallet_id, &request, &normalized).unwrap();
        journal.reserve_record(&record, &input).await.unwrap();
        record.status = TradeStatus::Submitted;
        record.signature = Some("original".into());
        journal.update_record(&record).await.unwrap();
        record.status = TradeStatus::Unknown;
        record.signature = None;
        record.error = Some("synthetic lost confirmation".into());
        record.duplicate = true;
        journal.update_record(&record).await.unwrap();
        let saved = journal
            .read_record(&record.request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.signature.as_deref(), Some("original"));
        assert_eq!(saved.status, TradeStatus::Unknown);
        assert_eq!(saved.error, record.error);
        assert!(!saved.duplicate);
        journal.pool_for_tests().close().await;
        assert!(journal.update_record(&record).await.is_err());
        assert!(journal.read_record(&record.request_id).await.is_err());
        assert!(journal.recent_records(10).await.is_err());
    }
}
