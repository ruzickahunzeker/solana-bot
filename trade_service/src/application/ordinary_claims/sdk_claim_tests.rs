use super::{test_support::*, *};
use crate::{
    adapters::persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
    application::ports::{
        ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
        ordinary_submission::OrdinarySubmissionJournal,
    },
    config::TradePersistenceMode,
    domain::trade::{Address, Pool, USDC_MINT, WSOL_MINT},
};

fn sdk_request() -> crate::models::TradeRequest {
    serde_json::from_value(serde_json::json!({
        "wallet_id":"fixture-wallet", "side":"sell", "pool":"meteora-damm-v2",
        "mint":Address::from_bytes([2;32]).to_string(), "quote_mint":WSOL_MINT,
        "pool_id":Address::from_bytes([3;32]).to_string(),
        "sell_percent":100, "simulate":false, "execution_method":"sdk",
        "idempotency_key":" sdk-literal-key "
    }))
    .unwrap()
}

async fn sdk_fixture(mode: TradePersistenceMode) -> Fixture {
    let f = fixture(mode).await;
    admit_request(f.state, sdk_request()).await
}

fn sdk_buy_request(native: bool) -> crate::models::TradeRequest {
    let mut request = serde_json::to_value(sdk_request()).unwrap();
    request["side"] = serde_json::json!("buy");
    request["sell_percent"] = serde_json::Value::Null;
    request["quote_mint"] = serde_json::json!(if native { WSOL_MINT } else { USDC_MINT });
    request[if native {
        "amount_lamports"
    } else {
        "amount_quote_units"
    }] = serde_json::json!(100);
    serde_json::from_value(request).unwrap()
}

#[tokio::test]
async fn sdk_buy_claim_uses_only_the_original_durable_owner_for_both_integer_payment_assets() {
    for native in [false, true] {
        let f = admit_request(
            fixture(TradePersistenceMode::Durable).await.state,
            sdk_buy_request(native),
        )
        .await;
        assert_eq!(
            operation_for_claim(&f.execution, TradeJournalMode::Durable).unwrap(),
            ClaimOperation::SdkTrade
        );
        let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
            panic!("original buyer owner required")
        };
        owner.ensure_binding(&f.execution).unwrap();
        let row:(String,String,Option<String>,Option<String>)=sqlx::query_as("SELECT operation,owner_nonce,bound_quote_json,sdk_prepared_json FROM ordinary_execution_claims WHERE request_id=?").bind(&f.record.request_id).fetch_one(&f.state.persistence.pool_for_tests()).await.unwrap();
        assert_eq!(
            row,
            (
                "ordinary-sdk-trade".into(),
                owner.nonce().into(),
                None,
                None
            )
        );
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_buy_claim_scope_rejects_other_units_assets_backends_modes_protocols_and_hops() {
    for native in [false, true] {
        for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Shadow] {
            let f = admit_request(fixture(mode).await.state, sdk_buy_request(native)).await;
            assert!(f.claims().claim(&f.execution).await.is_err());
        }
        let f = admit_request(
            fixture(TradePersistenceMode::Durable).await.state,
            sdk_buy_request(native),
        )
        .await;
        for mutation in [
            "zero",
            "wrong-asset",
            "wrong-units",
            "pumpapi",
            "simulate",
            "pool",
            "hop",
            "hop-id",
            "aggregate",
        ] {
            let mut command = f.execution.command().clone();
            match mutation {
                "zero" => {
                    command.amount = if native {
                        TradeAmount::BuySol { lamports: 0 }
                    } else {
                        TradeAmount::BuyQuote {
                            mint: command.route.quote_mint,
                            units: 0,
                        }
                    }
                }
                "wrong-asset" => command.route.quote_mint = Address::from_bytes([99; 32]),
                "wrong-units" => {
                    command.amount = if native {
                        TradeAmount::BuyQuote {
                            mint: command.route.quote_mint,
                            units: 100,
                        }
                    } else {
                        TradeAmount::BuySol { lamports: 100 }
                    }
                }
                "pumpapi" => command.execution.method = ExecutionMethod::Pumpapi,
                "simulate" => {
                    command.execution.mode = crate::domain::trade::ExecutionMode::Simulate
                }
                "pool" => command.route.pool = Pool::RaydiumCpmm,
                "hop" => command.route.sol_hop_pool = Some(Pool::RaydiumCpmm),
                "hop-id" => command.route.sol_hop_pool_id = Some(Address::from_bytes([4; 32])),
                "aggregate" => command.aggregator_sol = true,
                _ => unreachable!(),
            }
            let ex = f.execution.with_command_for_test(command);
            assert_ne!(
                operation_for_claim(&ex, TradeJournalMode::Durable).ok(),
                Some(ClaimOperation::SdkTrade),
                "{native} {mutation}"
            );
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ordinary_execution_claims")
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn sdk_buy_claim_two_pools_compete_once_and_file_reopen_cannot_reissue_either_asset_owner() {
    use crate::{config::TradePersistenceConfig, persistence::TradePersistence};
    use std::sync::Arc;
    for native in [false, true] {
        let dir = std::env::temp_dir().join(format!("sdk-buy-owner-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let config = TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
            busy_timeout_ms: 2000,
        };
        let mut state = crate::http_contract_tests::state().await;
        state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
        let f = admit_request(state, sdk_buy_request(native)).await;
        let first = f.state.persistence.pool_for_tests();
        let second = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&config.database_url)
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for i in 0..16 {
            let pool = if i % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            let ex = f.execution.clone();
            tasks.push(tokio::spawn(async move {
                SqliteOrdinaryExecutionClaims::new(pool)
                    .claim(&ex)
                    .await
                    .unwrap()
            }));
        }
        let mut owner = None;
        for task in tasks {
            if let ExecutionClaim::Acquired(o) = task.await.unwrap() {
                assert!(owner.replace(o).is_none());
            }
        }
        let owner = owner.unwrap();
        first.close().await;
        second.close().await;
        let restored = TradePersistence::new(&config).await.unwrap();
        assert_eq!(
            restored
                .ordinary_execution_claims()
                .unwrap()
                .claim(&f.execution)
                .await
                .unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        let nonce: String = sqlx::query_scalar(
            "SELECT owner_nonce FROM ordinary_execution_claims WHERE request_id=?",
        )
        .bind(&f.record.request_id)
        .fetch_one(&restored.pool_for_tests())
        .await
        .unwrap();
        assert_eq!(nonce, owner.nonce());
        restored.pool_for_tests().close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn durable_sdk_sell_acquires_the_original_owner_without_quote_or_signed_metadata() {
    let f = sdk_fixture(TradePersistenceMode::Durable).await;
    assert_eq!(
        operation_for_claim(&f.execution, TradeJournalMode::Durable).unwrap(),
        ClaimOperation::SdkTrade
    );
    let before = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original SDK owner required")
    };
    owner.ensure_binding(&f.execution).unwrap();
    assert_eq!(owner.operation(), ClaimOperation::SdkTrade);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    let pool = f.state.persistence.pool_for_tests();
    let (operation, stage, nonce, quote, signature): (String, String, String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT operation,submission_stage,owner_nonce,bound_quote_json,signed_signature FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&f.record.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (operation.as_str(), stage.as_str(), nonce.as_str()),
        ("ordinary-sdk-trade", "claimed", owner.nonce())
    );
    assert!(quote.is_none() && signature.is_none());
    assert_eq!(
        serde_json::to_value(
            f.state
                .persistence
                .get(&f.record.request_id)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(before).unwrap()
    );
    // A new operation kind does not permit SDK execution through the quote journal.
    assert!(ensure_scope(&f.execution, TradeJournalMode::Durable).is_err());
    let journal = SqliteOrdinaryExecutionClaims::new(pool);
    assert!(journal.begin(&f.execution, &owner).await.is_err());
}

#[tokio::test]
async fn sdk_claims_remain_closed_for_memory_shadow_simulation_other_protocols_and_hops() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Shadow] {
        let f = sdk_fixture(mode).await;
        assert!(f.claims().claim(&f.execution).await.is_err());
    }
    // Test the scope classifier against each independently changed command without
    // granting these fixtures a durable key or an execution owner.
    let f = sdk_fixture(TradePersistenceMode::Durable).await;
    for name in [
        "simulate",
        "pumpapi",
        "pool",
        "hop",
        "hop_id",
        "zero_buy",
        "zero_percent",
        "large_percent",
        "aggregate",
    ] {
        let mut command = f.execution.command().clone();
        match name {
            "simulate" => command.execution.mode = crate::domain::trade::ExecutionMode::Simulate,
            "pumpapi" => command.execution.method = ExecutionMethod::Pumpapi,
            "pool" => command.route.pool = Pool::PumpAmm,
            "hop" => command.route.sol_hop_pool = Some(Pool::RaydiumCpmm),
            "hop_id" => command.route.sol_hop_pool_id = Some(Address::from_bytes([4; 32])),
            "zero_buy" => command.amount = TradeAmount::BuySol { lamports: 0 },
            "zero_percent" => command.amount = TradeAmount::SellPercent { percent: 0 },
            "large_percent" => command.amount = TradeAmount::SellPercent { percent: 101 },
            "aggregate" => command.aggregator_sol = true,
            _ => unreachable!(),
        }
        let ex = f.execution.with_command_for_test(command);
        assert!(
            operation_for_claim(&ex, TradeJournalMode::Durable).is_err(),
            "{name}"
        );
        assert!(
            operation_for_claim(&ex, TradeJournalMode::Memory).is_err(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn sixteen_durable_sdk_callers_acquire_only_one_owner() {
    let f = sdk_fixture(TradePersistenceMode::Durable).await;
    let claims = std::sync::Arc::new(f.claims());
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

#[tokio::test]
async fn sdk_parent_or_payment_branch_errors_rollback_the_provisional_claim() {
    for (column, value) in [
        ("wallet_id", "another-wallet"),
        ("wallet_pubkey", "another-public-key"),
        ("idempotency_key", "sdk-literal-key"),
        ("request_hash", "different-hash"),
        ("pool", "pump-amm"),
        ("status", "unknown"),
        ("signature", "original-signature"),
        ("request_json", "{}"),
        ("execution_method", "pumpapi"),
    ] {
        let f = sdk_fixture(TradePersistenceMode::Durable).await;
        let pool = f.state.persistence.pool_for_tests();
        sqlx::query(&format!("UPDATE trades SET {column}=? WHERE request_id=?"))
            .bind(value)
            .bind(&f.record.request_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(f.claims().claim(&f.execution).await.is_err(), "{column}");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM ordinary_execution_claims WHERE request_id=?"
            )
            .bind(&f.record.request_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            0,
            "{column}"
        );
    }
}

#[tokio::test]
async fn sdk_held_claim_is_never_reissued_or_reinterpreted_as_an_aggregator_claim() {
    let f = sdk_fixture(TradePersistenceMode::Durable).await;
    assert!(f.claims().claim(&f.execution).await.unwrap().is_acquired());
    let pool = f.state.persistence.pool_for_tests();
    let nonce: String =
        sqlx::query_scalar("SELECT owner_nonce FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&f.record.request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    for (column, value) in [
        ("owner_nonce", "invalid-owner"),
        ("submission_stage", "signed"),
        ("operation", "ordinary-sol-buy"),
        ("signed_signature", "unexpected-signature"),
    ] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(&format!(
            "UPDATE ordinary_execution_claims SET {column}=? WHERE request_id=?"
        ))
        .bind(value)
        .bind(&f.record.request_id)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(f.claims().claim(&f.execution).await.is_err(), "{column}");
        sqlx::query("UPDATE ordinary_execution_claims SET owner_nonce=?,submission_stage='claimed',operation='ordinary-sdk-trade',signed_signature=NULL WHERE request_id=?")
            .bind(&nonce).bind(&f.record.request_id).execute(&pool).await.unwrap();
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
    SqliteOrdinaryExecutionClaims::new(pool.clone())
        .initialize()
        .await
        .unwrap();
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT owner_nonce FROM ordinary_execution_claims WHERE request_id=?"
        )
        .bind(&f.record.request_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        nonce
    );
}

#[tokio::test]
async fn sdk_file_reopen_preserves_the_original_nonce_and_does_not_authorize_a_second_execution() {
    use crate::{config::TradePersistenceConfig, persistence::TradePersistence};
    use std::sync::Arc;
    let dir = std::env::temp_dir().join(format!("sdk-claim-reopen-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2_000,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let f = admit_request(state, sdk_request()).await;
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
    let mut owner = None;
    for task in tasks {
        if let ExecutionClaim::Acquired(acquired) = task.await.unwrap() {
            assert!(owner.replace(acquired).is_none());
        }
    }
    let owner = owner.expect("one original execution owner");
    first.close().await;
    second.close().await;
    let restored = TradePersistence::new(&config).await.unwrap();
    let store = restored.ordinary_execution_claims().unwrap();
    assert_eq!(
        store.claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    let row: (String, String, Option<String>) = sqlx::query_as("SELECT owner_nonce,submission_stage,signed_signature FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&f.record.request_id).fetch_one(&restored.pool_for_tests()).await.unwrap();
    assert_eq!(row, (owner.nonce().into(), "claimed".into(), None));
    let record = restored.get(&f.record.request_id).await.unwrap().unwrap();
    assert_eq!(record.status, TradeStatus::Unknown);
    assert!(record.signature.is_none());
    assert!(store.begin(&f.execution, &owner).await.is_err());
    restored.pool_for_tests().close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
