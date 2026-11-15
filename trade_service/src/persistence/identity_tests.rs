use super::*;
use crate::{models::USDC_MINT, validation::validate_trade};

fn pair() -> (TradeRequest, TradeRequest) {
    let common = json!({
        "wallet_id":"main","side":"buy","pool":"pump-amm",
        "mint":"11111111111111111111111111111111","quote_mint":USDC_MINT,
        "idempotency_key":" literal-path-key ","amount_sol":0.001,
    });
    let sol: TradeRequest = serde_json::from_value(common).unwrap();
    let mut quote = sol.clone();
    quote.amount_sol = None;
    quote.amount_usdc = Some(1.0);
    (sol, quote)
}

fn record(id: &str, request: &TradeRequest) -> TradeRecord {
    TradeRecord {
        request_id: id.into(),
        wallet_id: request.wallet_id.clone().unwrap_or_else(|| "main".into()),
        wallet_pubkey: "11111111111111111111111111111111".into(),
        side: request.side,
        pool: request.pool,
        mint: request.mint.clone(),
        pool_id: request.pool_id.clone(),
        execution_method: request.execution_method.unwrap_or(ExecutionMethod::Sdk),
        status: TradeStatus::Accepted,
        signature: None,
        error: None,
        duplicate: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

async fn durable() -> TradePersistence {
    TradePersistence::new(&TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: "sqlite::memory:".into(),
        busy_timeout_ms: 1000,
    })
    .await
    .unwrap()
}

async fn count(persistence: &TradePersistence) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM trades")
        .fetch_one(persistence.pool.as_ref().unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn same_legacy_hash_but_different_payment_assets_conflict_in_both_orders() {
    let state = crate::http_contract_tests::state().await;
    let (sol, quote) = pair();
    for (first, second) in [(&sol, &quote), (&quote, &sol)] {
        let persistence = durable().await;
        let n1 = validate_trade(first, &state.config).unwrap();
        let n2 = validate_trade(second, &state.config).unwrap();
        assert_eq!(
            request_hash("main", first, &n1).unwrap(),
            request_hash("main", second, &n2).unwrap()
        );
        assert!(matches!(
            persistence
                .reserve(&record("tr_original", first), first, &n1)
                .await
                .unwrap(),
            DurableReserve::Created
        ));
        assert!(matches!(
            persistence
                .reserve(&record("tr_conflict", second), second, &n2)
                .await
                .unwrap(),
            DurableReserve::Conflict
        ));
        assert!(
            matches!(persistence.reserve(&record("tr_repeat",first),first,&n1).await.unwrap(),DurableReserve::Duplicate(r) if r.request_id == "tr_original")
        );
        assert_eq!(count(&persistence).await, 1);
        assert!(persistence.get("tr_conflict").await.unwrap().is_none());
    }
}

#[tokio::test]
async fn same_input_path_aliases_and_nontrade_metadata_still_deduplicate() {
    let state = crate::http_contract_tests::state().await;
    let (_, base_quote) = pair();
    let mut quote_alias = base_quote.clone();
    quote_alias.amount_usdc = None;
    quote_alias.amount_quote = Some(1.0);
    let mut quote_units = base_quote.clone();
    quote_units.amount_usdc = None;
    quote_units.amount_quote_units = Some(1_000_000);
    let mut native_sol = pair().0;
    native_sol.quote_mint = None;
    let mut native_lamports = native_sol.clone();
    native_lamports.amount_sol = None;
    native_lamports.amount_lamports = Some(1_000_000);
    native_lamports.quote_mint = Some(crate::models::SOL_MINT.into());
    for (first, mut repeated) in [
        (base_quote.clone(), quote_alias),
        (base_quote, quote_units),
        (native_sol, native_lamports),
    ] {
        let persistence = durable().await;
        repeated.source = Some("synthetic-other-source".into());
        repeated.source_signature = Some("synthetic-source-signature".into());
        let n1 = validate_trade(&first, &state.config).unwrap();
        let n2 = validate_trade(&repeated, &state.config).unwrap();
        persistence
            .reserve(&record("tr_original", &first), &first, &n1)
            .await
            .unwrap();
        assert!(
            matches!(persistence.reserve(&record("tr_repeat",&repeated),&repeated,&n2).await.unwrap(),DurableReserve::Duplicate(r) if r.request_id == "tr_original")
        );
        assert_eq!(count(&persistence).await, 1);
    }
}

#[tokio::test]
async fn sparse_legacy_request_json_replays_without_reapplying_defaults() {
    let state = crate::http_contract_tests::state().await;
    let (request, _) = pair();
    let n = validate_trade(&request, &state.config).unwrap();
    let persistence = durable().await;
    let mut legacy = serde_json::to_value(&request).unwrap();
    legacy.as_object_mut().unwrap().retain(|_, v| !v.is_null());
    legacy.as_object_mut().unwrap().remove("wallet_id");
    let old_hash = request_hash("main", &request, &n).unwrap();
    let mut saved = record("tr_legacy", &request);
    saved.status = TradeStatus::Submitted;
    saved.signature = Some("synthetic-existing-signature".into());
    assert!(insert_trade(
        persistence.pool.as_ref().unwrap(),
        &saved,
        &request.idempotency_key,
        &old_hash,
        &legacy.to_string()
    )
    .await
    .unwrap());
    assert!(
        matches!(persistence.reserve(&record("tr_repeat",&request),&request,&n).await.unwrap(),DurableReserve::Duplicate(r) if r.request_id == "tr_legacy" && r.signature == saved.signature && r.status == TradeStatus::Submitted)
    );
    let stored: String = sqlx::query_scalar("SELECT request_hash FROM trades")
        .fetch_one(persistence.pool.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(stored, old_hash);
    assert_eq!(count(&persistence).await, 1);
}

#[tokio::test]
async fn malformed_or_inconsistent_old_json_never_releases_or_replaces_the_key() {
    let state = crate::http_contract_tests::state().await;
    let (request, _) = pair();
    let n = validate_trade(&request, &state.config).unwrap();
    let persistence = durable().await;
    let original = serde_json::to_string(&request).unwrap();
    persistence
        .reserve(&record("tr_original", &request), &request, &n)
        .await
        .unwrap();
    let mut invalid = vec![
        "{".to_string(),
        "null".to_string(),
        "{}".to_string(),
        String::new(),
    ];
    for (field, value) in [
        ("wallet_id", json!("different-wallet")),
        ("idempotency_key", json!("different-key")),
        ("side", json!("sell")),
        ("pool", json!("bonk")),
        ("mint", json!(crate::models::WSOL_MINT)),
        ("quote_mint", json!("not-a-mint")),
        ("amount_quote", json!(1)),
    ] {
        let mut changed = serde_json::to_value(&request).unwrap();
        changed[field] = value;
        invalid.push(changed.to_string());
    }
    for bad in invalid {
        sqlx::query("UPDATE trades SET request_json=? WHERE request_id='tr_original'")
            .bind(&bad)
            .execute(persistence.pool.as_ref().unwrap())
            .await
            .unwrap();
        assert!(
            persistence
                .reserve(&record("tr_attempt", &request), &request, &n)
                .await
                .is_err(),
            "{bad}"
        );
        assert_eq!(count(&persistence).await, 1);
        assert!(persistence.get("tr_attempt").await.unwrap().is_none());
        let retained: String = sqlx::query_scalar("SELECT request_json FROM trades")
            .fetch_one(persistence.pool.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(retained, bad);
    }
    sqlx::query("UPDATE trades SET request_json=? WHERE request_id='tr_original'")
        .bind(&original)
        .execute(persistence.pool.as_ref().unwrap())
        .await
        .unwrap();
    assert!(
        matches!(persistence.reserve(&record("tr_retry",&request),&request,&n).await.unwrap(),DurableReserve::Duplicate(r) if r.request_id == "tr_original")
    );
}

#[tokio::test]
async fn conflicting_paths_stay_conflicting_after_restart_with_original_signature() {
    let state = crate::http_contract_tests::state().await;
    let (sol, quote) = pair();
    let sol_n = validate_trade(&sol, &state.config).unwrap();
    let quote_n = validate_trade(&quote, &state.config).unwrap();
    let dir = std::env::temp_dir().join(format!("trade-path-recovery-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("trades.sqlite").display()),
        busy_timeout_ms: 1000,
    };
    let persistence = TradePersistence::new(&config).await.unwrap();
    let mut original = record("tr_original", &sol);
    original.status = TradeStatus::Submitting;
    original.signature = Some("synthetic-saved-signature".into());
    persistence.reserve(&original, &sol, &sol_n).await.unwrap();
    persistence.pool.as_ref().unwrap().close().await;
    drop(persistence);
    let reopened = TradePersistence::new(&config).await.unwrap();
    assert!(matches!(
        reopened
            .reserve(&record("tr_conflict", &quote), &quote, &quote_n)
            .await
            .unwrap(),
        DurableReserve::Conflict
    ));
    assert!(
        matches!(reopened.reserve(&record("tr_repeat",&sol),&sol,&sol_n).await.unwrap(),DurableReserve::Duplicate(r) if r.request_id==original.request_id && r.status==TradeStatus::Unknown && r.signature==original.signature)
    );
    assert_eq!(count(&reopened).await, 1);
    reopened.pool.as_ref().unwrap().close().await;
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn concurrent_sol_and_quote_inputs_have_one_winner_and_never_alias() {
    let state = crate::http_contract_tests::state().await;
    let persistence = Arc::new(durable().await);
    let (sol, quote) = pair();
    let mut tasks = vec![];
    for index in 0..100 {
        let persistence = persistence.clone();
        let request = if index % 2 == 0 {
            sol.clone()
        } else {
            quote.clone()
        };
        let config = state.config.clone();
        tasks.push(tokio::spawn(async move {
            let n = validate_trade(&request, &config).unwrap();
            persistence
                .reserve(&record(&format!("tr_{index}"), &request), &request, &n)
                .await
                .unwrap()
        }));
    }
    let (mut created, mut duplicates, mut conflicts) = (0, 0, 0);
    for task in tasks {
        match task.await.unwrap() {
            DurableReserve::Created => created += 1,
            DurableReserve::Duplicate(_) => duplicates += 1,
            DurableReserve::Conflict => conflicts += 1,
        }
    }
    assert_eq!((created, duplicates, conflicts), (1, 49, 50));
    assert_eq!(count(&persistence).await, 1);
}

#[tokio::test]
async fn path_binding_covers_custom_quotes_and_cpmm_not_a_single_usdc_pool() {
    let state = crate::http_contract_tests::state().await;
    for pool in [Pool::PumpAmm, Pool::RaydiumCpmm, Pool::StonkFun] {
        for mint in [
            USDC_MINT,
            crate::models::USD1_MINT,
            "11111111111111111111111111111111",
        ] {
            let persistence = durable().await;
            let (mut sol, mut quote) = pair();
            for request in [&mut sol, &mut quote] {
                request.pool = pool;
                request.quote_mint = Some(mint.into());
                if pool != Pool::PumpAmm {
                    request.pool_id = Some("11111111111111111111111111111111".into());
                }
            }
            quote.amount_usdc = None;
            quote.amount_quote_units = Some(1_000_000);
            let sol_n = validate_trade(&sol, &state.config).unwrap();
            let quote_n = validate_trade(&quote, &state.config).unwrap();
            assert_eq!(
                request_hash("main", &sol, &sol_n).unwrap(),
                request_hash("main", &quote, &quote_n).unwrap()
            );
            persistence
                .reserve(&record("tr_sol", &sol), &sol, &sol_n)
                .await
                .unwrap();
            assert!(matches!(
                persistence
                    .reserve(&record("tr_quote", &quote), &quote, &quote_n)
                    .await
                    .unwrap(),
                DurableReserve::Conflict
            ));
        }
    }
}

#[tokio::test]
async fn same_key_remains_independent_between_wallets() {
    let state = crate::http_contract_tests::state().await;
    let (first, mut second) = pair();
    second.wallet_id = Some("other-wallet".into());
    let persistence = durable().await;
    for (id, request) in [("tr_one", first), ("tr_two", second)] {
        let n = validate_trade(&request, &state.config).unwrap();
        assert!(matches!(
            persistence
                .reserve(&record(id, &request), &request, &n)
                .await
                .unwrap(),
            DurableReserve::Created
        ));
    }
    assert_eq!(count(&persistence).await, 2);
}
