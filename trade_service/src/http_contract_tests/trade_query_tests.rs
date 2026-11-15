//! Query baselines use public rows and temporary SQLite; no execution source is invoked.
use super::*;
use crate::{
    adapters::persistence::trade_snapshot,
    application::trade_record::TradeRecordV1,
    domain::trade::{ExecutionMethod, Pool, Side, TradeStatus},
};

fn record(id: &str, time: u128, status: TradeStatus) -> TradeRecordV1 {
    TradeRecordV1 {
        request_id: id.into(),
        wallet_id: "fixture-wallet".into(),
        wallet_pubkey: "11111111111111111111111111111111".into(),
        side: Side::Buy,
        pool: Pool::Pump,
        mint: "11111111111111111111111111111111".into(),
        pool_id: None,
        execution_method: ExecutionMethod::Sdk,
        status,
        signature: Some(format!("synthetic-signature-{id}")),
        error: None,
        duplicate: false,
        created_at_ms: time,
        updated_at_ms: time + 1,
    }
}

async fn for_mode(mode: TradePersistenceMode) -> AppState {
    let mut state = state().await;
    state.persistence = Arc::new(
        TradePersistence::new(&TradePersistenceConfig {
            mode,
            database_url: "sqlite::memory:".into(),
            busy_timeout_ms: 100,
        })
        .await
        .unwrap(),
    );
    state
}

async fn seed_database(state: &AppState, record: &TradeRecordV1) {
    let mut request = trade();
    request["idempotency_key"] = json!(record.request_id);
    let request: crate::models::TradeRequest = serde_json::from_value(request).unwrap();
    let normalized = crate::validation::validate_trade(&request, &state.config).unwrap();
    let input = trade_snapshot::validated(&record.wallet_id, &request, &normalized).unwrap();
    state
        .persistence
        .mirror_validated(record, input)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if state
                .persistence
                .get(&record.request_id)
                .await
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ordinary_query_authority_keeps_durable_database_and_memory_shadow_index_distinct() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let state = for_mode(mode).await;
        let memory = record("tr_shared", 1, TradeStatus::Accepted);
        let memory_only = record("tr_memory", 2, TradeStatus::Unknown);
        state.trades.insert_record(&memory);
        state.trades.insert_record(&memory_only);
        let database = record("tr_shared", 3, TradeStatus::Confirmed);
        let database_only = record("tr_database", 4, TradeStatus::Submitted);
        if mode != TradePersistenceMode::Memory {
            seed_database(&state, &database).await;
            seed_database(&state, &database_only).await;
        }
        let app = http::router(state.clone());
        let expected = if mode == TradePersistenceMode::Durable {
            &database
        } else {
            &memory
        };
        assert_eq!(
            json_body(
                send(
                    &app,
                    "GET",
                    "/v1/trades/tr_shared",
                    LOCAL,
                    Some(TOKEN),
                    None
                )
                .await,
                StatusCode::OK
            )
            .await,
            serde_json::to_value(expected).unwrap(),
        );
        let (missing, rows) = if mode == TradePersistenceMode::Durable {
            ("tr_memory", vec![database_only, database])
        } else {
            ("tr_database", vec![memory_only, memory])
        };
        error_body(
            send(
                &app,
                "GET",
                &format!("/v1/trades/{missing}"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::NOT_FOUND,
            "trade not found",
        )
        .await;
        assert_eq!(
            json_body(
                send(&app, "GET", "/v1/trades", LOCAL, Some(TOKEN), None).await,
                StatusCode::OK
            )
            .await,
            serde_json::to_value(rows).unwrap(),
        );
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}

#[tokio::test]
async fn ordinary_query_limit_default_clamp_order_and_wire_rows_are_frozen() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Durable] {
        let state = for_mode(mode).await;
        let mut rows = Vec::new();
        for i in 0..105 {
            let row = record(&format!("tr_limit_{i:03}"), i, TradeStatus::Unknown);
            if mode == TradePersistenceMode::Durable {
                seed_database(&state, &row).await;
            } else {
                state.trades.insert_record(&row);
            }
            rows.push(row);
        }
        rows.reverse();
        let app = http::router(state);
        for (limit, expected) in [
            (None, 20),
            (Some(0), 1),
            (Some(1), 1),
            (Some(100), 100),
            (Some(101), 100),
            (Some(usize::MAX), 100),
        ] {
            let path = limit
                .map(|v| format!("/v1/trades?limit={v}"))
                .unwrap_or_else(|| "/v1/trades".into());
            assert_eq!(
                json_body(
                    send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await,
                    StatusCode::OK
                )
                .await,
                serde_json::to_value(&rows[..expected]).unwrap(),
            );
        }
    }
}

#[tokio::test]
async fn ordinary_query_closed_or_corrupt_durable_storage_never_falls_back_to_memory() {
    for corrupt in [false, true] {
        let state = for_mode(TradePersistenceMode::Durable).await;
        let row = record("tr_storage_error", 1, TradeStatus::Confirmed);
        state.trades.insert_record(&row);
        seed_database(&state, &row).await;
        let pool = state.persistence.pool_for_tests();
        if corrupt {
            sqlx::query("UPDATE trades SET pool='unknown-synthetic-pool'")
                .execute(&pool)
                .await
                .unwrap();
        } else {
            pool.close().await;
        }
        let get_error = state
            .persistence
            .get(&row.request_id)
            .await
            .unwrap_err()
            .to_string();
        let list_error = state.persistence.recent(20).await.unwrap_err().to_string();
        let app = http::router(state.clone());
        for (path, detail) in [
            ("/v1/trades/tr_storage_error", get_error),
            ("/v1/trades", list_error),
        ] {
            error_body(
                send(&app, "GET", path, LOCAL, None, None).await,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            )
            .await;
            error_body(
                send(&app, "GET", path, LOCAL, Some(TOKEN), None).await,
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("trade persistence unavailable: {detail}"),
            )
            .await;
        }
        assert_eq!(
            state.trades.get(&row.request_id).unwrap().status,
            TradeStatus::Confirmed
        );
    }
}
