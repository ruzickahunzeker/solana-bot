use super::*;
use crate::{
    application::{
        ordinary_claims::test_support::fixture,
        ports::{ordinary_claims::ExecutionClaim, ordinary_journal::TradeJournalMode},
    },
    config::{TradePersistenceConfig, TradePersistenceMode},
    domain::trade::TradeStatus,
};
use std::time::Duration;

#[tokio::test]
async fn ordinary_storage_ownership_preserves_read_authority_and_claims_in_each_mode() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let original = fixture(mode).await;
        let storage = OrdinaryStorage::new(
            original.state.persistence.clone(),
            original.state.trades.clone(),
            original.state.idempotency.clone(),
        );
        let retained = storage.view().retain();
        assert!(Arc::ptr_eq(&retained.journal, &original.state.persistence));
        assert!(Arc::ptr_eq(&retained.index, &original.state.trades));
        assert!(Arc::ptr_eq(
            &retained.reservations,
            &original.state.idempotency
        ));

        let port = retained.view();
        let id = &original.record.request_id;
        assert!(matches!(
            port.claims().claim(&original.execution).await.unwrap(),
            ExecutionClaim::Acquired(_)
        ));
        assert!(matches!(
            original.claims().claim(&original.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        ));
        let changed = port
            .execution_index()
            .update_status(id, TradeStatus::Unknown, None, Some("fixture only".into()))
            .unwrap();
        assert_eq!(
            port.admission_index().get_record(id).unwrap().status,
            changed.status
        );
        let effective = if mode == TradePersistenceMode::Durable {
            original.record.status
        } else {
            changed.status
        };
        assert_eq!(
            port.reads().read_record(id).await.unwrap().unwrap().status,
            effective
        );
        assert_eq!(
            port.reads().recent_records(1).await.unwrap()[0].status,
            effective
        );
        port.execution_journal()
            .update_record(&changed)
            .await
            .unwrap();
        original
            .state
            .persistence
            .shutdown_shadow(Duration::from_secs(2))
            .await
            .unwrap();
        if mode != TradePersistenceMode::Memory {
            assert_eq!(
                original
                    .state
                    .persistence
                    .get(id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                changed.status
            );
        }
    }
}

#[tokio::test]
async fn ordinary_storage_ownership_fixture_replacement_does_not_repoint_earlier_snapshot() {
    let mut updated = crate::http_contract_tests::state().await;
    let old = updated.clone();
    let old_storage = old.ordinary_storage().retain();
    let replacement = Arc::new(
        TradePersistence::new(&TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap(),
    );
    updated.persistence = replacement.clone();
    let new_storage = updated.ordinary_storage().retain();
    assert!(Arc::ptr_eq(&new_storage.journal, &replacement));
    assert!(Arc::ptr_eq(&old_storage.journal, &old.persistence));
    assert!(!Arc::ptr_eq(&old_storage.journal, &new_storage.journal));
    assert!(Arc::ptr_eq(&old_storage.index, &new_storage.index));
    assert!(Arc::ptr_eq(
        &old_storage.reservations,
        &new_storage.reservations
    ));
    assert_eq!(
        old_storage.view().admission_journal().mode(),
        TradeJournalMode::Memory
    );
    assert_eq!(
        new_storage.view().admission_journal().mode(),
        TradeJournalMode::Durable
    );
}
