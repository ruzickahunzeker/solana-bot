use super::*;
use crate::{
    application::{
        ordinary_claims::test_support::NominalPumpPreparation,
        ports::ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
    },
    domain::pump_native_candidate::PumpNativeCandidateRecord,
};
use sol_trade_sdk::swqos::{SwqosType, TradeType};

#[tokio::test]
async fn pump_preclaim_failure_commits_only_original_unsigned_unclaimed_parent() {
    let (f, _, _) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let pool = f.state.persistence.pool_for_tests();
    let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
    let before: (String, String, String) = sqlx::query_as(
        "SELECT request_hash,idempotency_key,request_json FROM trades WHERE request_id=?",
    )
    .bind(&f.record.request_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let failed = store
        .fail_unclaimed_preparation(&f.execution, "fixture preparation rejected")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, TradeStatus::Failed);
    assert!(failed.signature.is_none());
    assert_eq!(
        failed.error.as_deref(),
        Some("fixture preparation rejected")
    );
    assert_eq!(failed.created_at_ms, f.record.created_at_ms);
    assert!(failed.updated_at_ms >= failed.created_at_ms);
    let after: (String, String, String) = sqlx::query_as(
        "SELECT request_hash,idempotency_key,request_json FROM trades WHERE request_id=?",
    )
    .bind(&f.record.request_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    assert!(store
        .fail_unclaimed_preparation(&f.execution, "must not overwrite first failure")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        f.state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .error,
        failed.error
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Accepted
    );
}

#[tokio::test]
async fn pump_preclaim_failure_never_overwrites_existing_owner_or_signed_parent() {
    for existing in ["owner", "signed", "building", "rejected"] {
        let (f, record, message) =
            PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
        let pool = f.state.persistence.pool_for_tests();
        let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
        if existing == "owner" {
            let nominal = NominalPumpPreparation::new(record, message);
            assert!(matches!(
                f.claims()
                    .claim_prepared_sdk(&f.execution, &nominal)
                    .await
                    .unwrap(),
                ExecutionClaim::Acquired(_)
            ));
        } else {
            let (status, signature) = match existing {
                "signed" => ("submitted", Some("original-signature")),
                "building" => ("building_tx", None),
                _ => ("rejected", None),
            };
            sqlx::query(
                "UPDATE trades SET status=?,signature=?,error='original-result' WHERE request_id=?",
            )
            .bind(status)
            .bind(signature)
            .bind(&f.record.request_id)
            .execute(&pool)
            .await
            .unwrap();
        }
        let before = f
            .state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .fail_unclaimed_preparation(&f.execution, "late failure")
                .await
                .unwrap()
                .is_none(),
            "{existing}"
        );
        let after = f
            .state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(before).unwrap(),
            "{existing}"
        );
    }
}

#[tokio::test]
async fn pump_preclaim_failure_rejects_identity_corruption_without_creating_owner() {
    for field in ["key", "hash", "pubkey", "json", "pool", "command-key"] {
        let (f, _, _) =
            PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
        let pool = f.state.persistence.pool_for_tests();
        let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
        let sql = match field {
            "key" => "UPDATE trades SET idempotency_key='foreign-key' WHERE request_id=?",
            "hash" => "UPDATE trades SET request_hash='foreign-hash' WHERE request_id=?",
            "pubkey" => "UPDATE trades SET wallet_pubkey='foreign-wallet' WHERE request_id=?",
            "json" => "UPDATE trades SET request_json='{}' WHERE request_id=?",
            "pool" => "UPDATE trades SET pool='pump-amm' WHERE request_id=?",
            _ => "UPDATE trades SET request_id=request_id WHERE request_id=?",
        };
        sqlx::query(sql)
            .bind(&f.record.request_id)
            .execute(&pool)
            .await
            .unwrap();
        let mut c = f.execution.command().clone();
        if field == "command-key" {
            c.idempotency_key.push('x');
        }
        let execution = f.execution.with_command_for_test(c);
        assert!(
            store
                .fail_unclaimed_preparation(&execution, "invalid identity")
                .await
                .is_err(),
            "{field}"
        );
        let saved = f
            .state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, TradeStatus::Accepted);
        assert!(saved.error.is_none() && saved.signature.is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn pump_preclaim_failure_and_original_claim_have_one_atomic_winner() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (f, record, message) =
            PumpNativeCandidateRecord::history_fixture(side, SwqosType::Temporal).await;
        let pool = f.state.persistence.pool_for_tests();
        let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
        let nominal = NominalPumpPreparation::new(record, message);
        let (failed, claimed) = tokio::join!(
            store.fail_unclaimed_preparation(&f.execution, "preparation race"),
            store.claim_prepared_sdk(&f.execution, &nominal),
        );
        let failed = failed.unwrap();
        let saved = f
            .state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap();
        match claimed {
            Ok(ExecutionClaim::Acquired(owner)) => {
                owner.ensure_binding(&f.execution).unwrap();
                assert!(failed.is_none());
                assert_eq!(saved.status, TradeStatus::Accepted);
                assert!(saved.error.is_none());
            }
            Err(_) => {
                assert_eq!(failed.unwrap().status, TradeStatus::Failed);
                assert_eq!(saved.status, TradeStatus::Failed);
                assert_eq!(
                    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
                        .fetch_one(&pool)
                        .await
                        .unwrap(),
                    0
                );
            }
            other => panic!("new fixture cannot have a pre-existing owner: {other:?}"),
        }
    }
}

#[tokio::test]
async fn pump_prepared_claim_expiring_under_write_lock_rolls_back_without_reconstructing_owner() {
    use crate::application::ports::ordinary_claims::ExecutionOwner;
    use crate::application::ports::ordinary_execution::{JournaledSdkFuture, PreparedSdkExecution};
    use crate::domain::sdk_prepared_record::SdkPreparedRecord;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Expires {
        nominal: NominalPumpPreparation,
        checks: AtomicUsize,
    }
    impl PreparedSdkExecution for Expires {
        fn metadata(&self) -> &SdkPreparedRecord {
            &self.nominal.metadata
        }
        fn message(&self) -> &[u8] {
            &self.nominal.message
        }
        fn validate_current(&self, execution: &AdmittedTrade) -> Result<()> {
            self.nominal.validate_current(execution)?;
            ensure!(
                self.checks.fetch_add(1, Ordering::SeqCst) < 2,
                "fixture proof expired before commit"
            );
            Ok(())
        }
        fn execute<'a>(
            self: Box<Self>,
            _: &'a AdmittedTrade,
            _: ExecutionOwner,
        ) -> JournaledSdkFuture<'a>
        where
            Self: 'a,
        {
            Box::pin(async { panic!("expired claim cannot execute") })
        }
    }
    let (f, record, message) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let expires = Expires {
        nominal: NominalPumpPreparation::new(record, message),
        checks: AtomicUsize::new(0),
    };
    let pool = f.state.persistence.pool_for_tests();
    let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
    assert!(store
        .claim_prepared_sdk(&f.execution, &expires)
        .await
        .unwrap_err()
        .to_string()
        .contains("expired before commit"));
    assert_eq!(expires.checks.load(Ordering::SeqCst), 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TradeStatus::Accepted
    );
    assert!(f.claims().claim(&f.execution).await.is_err());
}
