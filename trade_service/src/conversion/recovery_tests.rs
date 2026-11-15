//! Scoped reads against disposable databases; no production files or signing.
use super::*;
use crate::application::wsol_recovery::WsolRecoveryInput;

fn input(request: &WsolSubmission) -> WsolRecoveryInput {
    WsolRecoveryInput {
        idempotency_key: request.idempotency_key.clone(),
        wallet_id: request.wallet_id.clone(),
        owner_type: request.owner_type.clone(),
        owner_id: request.owner_id.clone(),
    }
}

#[tokio::test]
async fn wsol_recovery_sql_scopes_all_identity_fields_and_preserves_literal_keys() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let quote = contract_tests::quote();
    let mut request = contract_tests::submission();
    request.idempotency_key = "  literal '+ /?&=é  ".into();
    let (record, _) = store.reserve_live(&request, &quote, 1).await.unwrap();
    let original = input(&request);
    assert_eq!(
        store.recover_by_key(&original).await.unwrap(),
        Some(record.clone())
    );
    for field in 0..4 {
        let mut foreign = original.clone();
        match field {
            0 => foreign.idempotency_key.push('x'),
            1 => foreign.wallet_id.push('x'),
            2 => foreign.owner_type.push('x'),
            _ => foreign.owner_id.push('x'),
        }
        assert!(
            store.recover_by_key(&foreign).await.unwrap().is_none(),
            "{field}"
        );
    }
    let mut trimmed = original.clone();
    trimmed.idempotency_key = trimmed.idempotency_key.trim().into();
    assert!(store.recover_by_key(&trimmed).await.unwrap().is_none());
    let mut injection = original.clone();
    injection.idempotency_key = "' OR 1=1 --".into();
    assert!(store.recover_by_key(&injection).await.unwrap().is_none());
    assert_eq!(
        store.get_by_id(&record.submission_id).await.unwrap(),
        Some(record)
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversion_submissions")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn wsol_recovery_does_not_decode_foreign_corrupt_rows_or_turn_them_into_an_oracle() {
    let store = ConversionStore::new("sqlite::memory:").await.unwrap();
    let request = contract_tests::submission();
    store
        .reserve_live(&request, &contract_tests::quote(), 1)
        .await
        .unwrap();
    sqlx::query("UPDATE conversion_submissions SET quote_id=X'FF'")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store.recover_by_key(&input(&request)).await.is_err());
    for field in 0..3 {
        let mut foreign = input(&request);
        match field {
            0 => foreign.wallet_id.push('x'),
            1 => foreign.owner_type.push('x'),
            _ => foreign.owner_id.push('x'),
        }
        assert!(store.recover_by_key(&foreign).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn wsol_recovery_reopens_every_state_on_a_read_only_pool_without_any_ledger_mutation() {
    let dir = std::env::temp_dir().join(format!("wsol-recovery-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let store = ConversionStore::new(&url).await.unwrap();
    let mut records = vec![];
    for (index, state) in [
        "reserved",
        "signed",
        "unknown",
        "submitted",
        "confirmed",
        "failed",
        "signed_failed",
    ]
    .into_iter()
    .enumerate()
    {
        let mut quote = contract_tests::quote();
        quote.wallet_id = format!("fixture-{index}");
        quote.quote_id = format!("cq-{index}");
        let mut request = contract_tests::submission();
        request.wallet_id = quote.wallet_id.clone();
        request.quote_id = quote.quote_id.clone();
        request.idempotency_key = format!("key-{index}");
        let (record, _) = store.reserve_live(&request, &quote, 1).await.unwrap();
        if state != "reserved" {
            let signed = !matches!(state, "failed");
            sqlx::query("UPDATE conversion_submissions SET status=?,signature=?,signed_transaction=?,error=?,updated_at_ms=2 WHERE submission_id=?")
                .bind(if state == "signed_failed" { "failed" } else { state })
                .bind(signed.then_some("synthetic-signature"))
                .bind(signed.then_some(vec![1u8,2,3]))
                .bind((state == "failed" || state == "signed_failed").then_some("synthetic failure"))
                .bind(&record.submission_id).execute(&store.pool).await.unwrap();
        }
        records.push((
            input(&request),
            store
                .get_by_id(&record.submission_id)
                .await
                .unwrap()
                .unwrap(),
        ));
    }
    store.pool.close().await;
    let options = SqliteConnectOptions::from_str(&url)
        .unwrap()
        .read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let readonly = ConversionStore { pool };
    let before: Vec<(String, i64, i64)> = sqlx::query_as("SELECT submission_id,created_at_ms,updated_at_ms FROM conversion_submissions ORDER BY submission_id")
        .fetch_all(&readonly.pool).await.unwrap();
    for (lookup, record) in records {
        for _ in 0..3 {
            let recovered = readonly.recover_by_key(&lookup).await.unwrap().unwrap();
            lookup.validate_record(&recovered).unwrap();
            assert_eq!(recovered, record);
        }
    }
    assert!(
        sqlx::query("UPDATE conversion_submissions SET updated_at_ms=99")
            .execute(&readonly.pool)
            .await
            .is_err()
    );
    let after: Vec<(String, i64, i64)> = sqlx::query_as("SELECT submission_id,created_at_ms,updated_at_ms FROM conversion_submissions ORDER BY submission_id")
        .fetch_all(&readonly.pool).await.unwrap();
    assert_eq!(before, after);
    readonly.pool.close().await;
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
