use super::test_support::*;
use super::*;
use crate::{
    adapters::persistence::ordinary_claims::{
        RuntimeExecutionClaims, SqliteOrdinaryExecutionClaims,
    },
    application::ports::ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
    config::{TradePersistenceConfig, TradePersistenceMode},
    persistence::TradePersistence,
};
use std::sync::Arc;

#[tokio::test]
async fn every_mode_claims_the_original_parent_once_without_changing_order_or_key() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let f = fixture(mode).await;
        let claims = f.claims();
        assert!(claims.claim(&f.execution).await.unwrap().is_acquired());
        assert_eq!(
            claims.claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        assert_eq!(
            f.state.trades.get(&f.record.request_id).unwrap().status,
            TradeStatus::Accepted
        );
        if mode == TradePersistenceMode::Durable {
            let row: (String, String, i64) = sqlx::query_as(
                "SELECT request_id,request_hash,schema_version FROM ordinary_execution_claims",
            )
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
            assert_eq!(
                row,
                (
                    f.record.request_id.clone(),
                    f.execution.reservation().request_hash().into(),
                    1
                )
            );
        }
    }
}

#[tokio::test]
async fn sixteen_concurrent_callers_acquire_one_claim_in_each_authority_mode() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let f = fixture(mode).await;
        let claims = Arc::new(f.claims());
        let mut tasks = vec![];
        for _ in 0..16 {
            let claims = claims.clone();
            let execution = f.execution.clone();
            tasks.push(tokio::spawn(async move {
                claims.claim(&execution).await.unwrap()
            }));
        }
        let mut acquired = 0;
        for task in tasks {
            acquired += usize::from(task.await.unwrap().is_acquired());
        }
        assert_eq!(acquired, 1);
    }
}

#[tokio::test]
async fn memory_parent_errors_do_not_consume_the_original_key_claim() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Shadow] {
        for field in [
            "missing",
            "wallet",
            "pubkey",
            "mint",
            "pool",
            "pool_id",
            "side",
            "method",
            "status",
            "signature",
        ] {
            let f = fixture(mode).await;
            let mut parent = f.record.clone();
            match field {
                "missing" => f.state.trades.remove(&parent.request_id),
                "wallet" => parent.wallet_id = "other".into(),
                "pubkey" => {
                    parent.wallet_pubkey =
                        crate::domain::trade::Address::from_bytes([3; 32]).to_string()
                }
                "mint" => {
                    parent.mint = crate::domain::trade::Address::from_bytes([3; 32]).to_string()
                }
                "pool" => parent.pool = crate::domain::trade::Pool::Pump,
                "pool_id" => {
                    parent.pool_id =
                        Some(crate::domain::trade::Address::from_bytes([3; 32]).to_string())
                }
                "side" => parent.side = crate::domain::trade::Side::Sell,
                "method" => {
                    parent.execution_method = crate::domain::trade::ExecutionMethod::Pumpapi
                }
                "status" => parent.status = TradeStatus::Submitted,
                "signature" => parent.signature = Some("original-not-replaceable".into()),
                _ => unreachable!(),
            }
            if field != "missing" {
                f.state.trades.insert_record(&parent);
            }
            assert!(f.claims().claim(&f.execution).await.is_err(), "{field}");
            f.state.trades.insert_record(&f.record);
            assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
        }
    }
}

#[tokio::test]
async fn memory_missing_or_wrong_key_authority_never_falls_back_to_a_new_reservation() {
    let f = fixture(TradePersistenceMode::Memory).await;
    let runtime = RuntimeExecutionClaims::new(
        f.state.persistence.clone(),
        f.state.trades.clone(),
        Arc::default(),
    );
    assert!(runtime.claim(&f.execution).await.is_err());
    let forged = crate::application::trade_admission::AdmittedTrade::unreserved_fixture(
        &f.record,
        f.execution.command().clone(),
    );
    assert!(f.claims().claim(&forged).await.is_err());
    assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
}

#[tokio::test]
async fn mode_mismatch_and_non_live_or_non_aggregate_operations_are_rejected() {
    let f = fixture(TradePersistenceMode::Durable).await;
    for mode in [TradeJournalMode::Memory, TradeJournalMode::Shadow] {
        assert!(ensure_scope(&f.execution, mode).is_err());
    }
    for field in ["simulate", "branch", "amount"] {
        let mut command = f.execution.command().clone();
        match field {
            "simulate" => command.execution.mode = crate::domain::trade::ExecutionMode::Simulate,
            "branch" => command.aggregator_sol = false,
            "amount" => command.amount = TradeAmount::SellPercent { percent: 100 },
            _ => unreachable!(),
        }
        let fixture = crate::application::trade_admission::AdmittedTrade::unreserved_fixture(
            &f.record, command,
        );
        assert!(ensure_scope(&fixture, TradeJournalMode::Memory).is_err());
    }
}

#[tokio::test]
async fn durable_rejects_parent_key_hash_identity_json_branch_or_state_changes_and_rolls_back() {
    for (column, value) in [
        ("wallet_id", "other"),
        ("wallet_pubkey", "other-public-key"),
        ("idempotency_key", "claim-literal-key"),
        ("request_hash", "wrong-hash"),
        ("mint", "11111111111111111111111111111111"),
        ("pool", "pump"),
        ("request_json", "not-json"),
        ("status", "unknown"),
        ("signature", "original"),
    ] {
        let f = fixture(TradePersistenceMode::Durable).await;
        let pool = f.state.persistence.pool_for_tests();
        // Column names are fixed test constants, never request input.
        sqlx::query(&format!("UPDATE trades SET {column}=? WHERE request_id=?"))
            .bind(value)
            .bind(&f.record.request_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(f.claims().claim(&f.execution).await.is_err(), "{column}");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{column} must roll back provisional INSERT");
    }
    let f = fixture(TradePersistenceMode::Durable).await;
    let pool = f.state.persistence.pool_for_tests();
    let json: String = sqlx::query_scalar("SELECT request_json FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut direct: serde_json::Value = serde_json::from_str(&json).unwrap();
    direct.as_object_mut().unwrap().remove("amount_sol");
    direct["amount_quote_units"] = serde_json::json!(1_000_000);
    sqlx::query("UPDATE trades SET request_json=?")
        .bind(serde_json::to_string(&direct).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert!(f.claims().claim(&f.execution).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE trades SET request_json=?")
        .bind(json)
        .execute(&pool)
        .await
        .unwrap();
    assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
}

#[tokio::test]
async fn durable_claim_errors_and_corrupt_claim_evidence_never_authorize_execution() {
    let f = fixture(TradePersistenceMode::Durable).await;
    f.state.persistence.pool_for_tests().close().await;
    assert!(f.claims().claim(&f.execution).await.is_err());
    for column in [
        "request_hash",
        "schema_version",
        "operation",
        "created_at_ms",
    ] {
        let f = fixture(TradePersistenceMode::Durable).await;
        assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
        let pool = f.state.persistence.pool_for_tests();
        sqlx::query("PRAGMA ignore_check_constraints=ON")
            .execute(&pool)
            .await
            .unwrap();
        let literal = match column {
            "request_hash" => "'corrupt'",
            "schema_version" => "2",
            "operation" => "'other'",
            _ => "-1",
        };
        sqlx::query(&format!(
            "UPDATE ordinary_execution_claims SET {column}={literal}"
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert!(f.claims().claim(&f.execution).await.is_err(), "{column}");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn durable_claim_survives_file_reopen_and_does_not_reset_on_initialize() {
    let dir = std::env::temp_dir().join(format!("ordinary-claim-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 100,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let f = admit(state).await;
    assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
    f.state.persistence.pool_for_tests().close().await;
    let mut restored = crate::http_contract_tests::state().await;
    restored.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let pool = restored.persistence.pool_for_tests();
    SqliteOrdinaryExecutionClaims::new(pool.clone())
        .initialize()
        .await
        .unwrap();
    let claims = RuntimeExecutionClaims::new(
        restored.persistence.clone(),
        restored.trades.clone(),
        restored.idempotency.clone(),
    );
    assert_eq!(
        claims.claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    let original = restored
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.status, TradeStatus::Unknown);
    assert!(original.signature.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn shadow_claim_uses_original_memory_authority_even_if_mirror_storage_is_closed() {
    let f = fixture(TradePersistenceMode::Shadow).await;
    f.state.persistence.pool_for_tests().close().await;
    assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
}

#[tokio::test]
async fn two_independent_sqlite_pools_cannot_both_acquire_the_same_parent_operation() {
    let dir = std::env::temp_dir().join(format!("ordinary-two-writers-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2_000,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let f = admit(state).await;
    let first = f.state.persistence.pool_for_tests();
    let second = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&config.database_url)
        .await
        .unwrap();
    let mut tasks = vec![];
    for i in 0..16 {
        let pool = if i % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let execution = f.execution.clone();
        tasks.push(tokio::spawn(async move {
            SqliteOrdinaryExecutionClaims::new(pool)
                .claim(&execution)
                .await
                .unwrap()
        }));
    }
    let mut acquired = 0;
    for task in tasks {
        acquired += usize::from(task.await.unwrap().is_acquired());
    }
    assert_eq!(acquired, 1);
    first.close().await;
    second.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn claim_schema_links_only_the_original_parent_and_preserves_legacy_rows() {
    let f = fixture(TradePersistenceMode::Durable).await;
    let pool = f.state.persistence.pool_for_tests();
    let before = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    SqliteOrdinaryExecutionClaims::new(pool.clone())
        .initialize()
        .await
        .unwrap();
    assert!(sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms) VALUES('missing-parent',1,'ordinary-sol-buy','hash',1)").execute(&pool).await.is_err());
    assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
    SqliteOrdinaryExecutionClaims::new(pool.clone())
        .initialize()
        .await
        .unwrap();
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    let after = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert!(sqlx::query("DELETE FROM trades WHERE request_id=?")
        .bind(&f.record.request_id)
        .execute(&pool)
        .await
        .is_err());
}
