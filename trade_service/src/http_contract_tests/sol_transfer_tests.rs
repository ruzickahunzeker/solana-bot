//! V1 validation, corrected API safety, and legacy storage oracles with public fixtures only.
use super::*;
use crate::{domain::trade::Address, wallet_repository::SolTransferRecord};
use sqlx::SqlitePool;

const PATH: &str = "/internal/wallets/source/transfer-sol";

fn address(byte: u8) -> String {
    Address::from_bytes([byte; 32]).to_string()
}

fn transfer(patch: Value) -> Value {
    let mut request = json!({
        "owner_type":"telegram", "owner_id":"42", "to_address":address(2),
        "amount_mode":"fixed", "amount_lamports":1_000_000,
        "idempotency_key":"fixture-transfer-key",
    });
    request
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    request
}

async fn fixture() -> (AppState, SqlitePool) {
    let mut state = state().await;
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    for (id, owner_type, owner_id, byte, enabled) in [
        ("source", "telegram", "42", 1, true),
        ("target", "telegram", "42", 2, true),
        ("foreign", "telegram", "7", 3, true),
        ("system", "system", "global", 4, true),
        ("disabled", "telegram", "42", 5, false),
    ] {
        sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES(?,?,?,?,?,'not-a-secret-or-valid-wallet',?)")
            .bind(id).bind(owner_type).bind(owner_id).bind(id).bind(address(byte)).bind(enabled)
            .execute(&pool).await.unwrap();
    }
    state.wallet_repository = Arc::new(repository);
    state.sol_transfers = Arc::new(crate::application::sol_transfers::SolTransferService::new(
        crate::adapters::sdk::sol_transfer::SdkSolTransferRuntime::new(
            state.wallets.clone(),
            state.wallet_repository.clone(),
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap()),
        ),
        crate::adapters::sdk::address_parser::SdkAddressParser,
    ));
    (state, pool)
}

async fn count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM wallet_sol_transfers")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn body(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn legacy_transfer_auth_extractors_broadcast_policy_and_key_error_priority() {
    let (mut state, pool) = fixture().await;
    let app = http::router(state.clone());
    assert_eq!(
        send(&app, "POST", PATH, LOCAL, None, Some(transfer(json!({}))))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "POST",
            PATH,
            ALLOWED,
            Some(TOKEN),
            Some(transfer(json!({})))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, "POST", PATH, LOCAL, None, Some(json!({})))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    error_body(
        send(
            &app,
            "POST",
            "/internal/wallets/missing/transfer-sol",
            LOCAL,
            Some(TOKEN),
            Some(transfer(json!({"idempotency_key":""}))),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "idempotency_key must be 1..128 characters",
    )
    .await;
    state.config = Arc::new({
        let mut config = (*state.config).clone();
        config.security.broadcast = crate::domain::execution_policy::BroadcastPolicy::Disabled;
        config
    });
    let app = http::router(state);
    error_body(
        send(
            &app,
            "POST",
            PATH,
            LOCAL,
            Some(TOKEN),
            Some(transfer(json!({"idempotency_key":"", "simulate":true}))),
        )
        .await,
        StatusCode::FORBIDDEN,
        crate::domain::execution_policy::BROADCAST_DISABLED_ERROR,
    )
    .await;
    assert_eq!(count(&pool).await, 0);
}

#[tokio::test]
async fn legacy_transfer_owner_destination_and_amount_validation_order() {
    let (state, pool) = fixture().await;
    let app = http::router(state);
    for (id, patch, status, message) in [
        (
            "missing",
            json!({}),
            StatusCode::NOT_FOUND,
            "wallet not found",
        ),
        (
            "disabled",
            json!({"to_address":"bad"}),
            StatusCode::FORBIDDEN,
            "source wallet is disabled or does not belong to this owner",
        ),
        (
            "foreign",
            json!({"to_address":"bad"}),
            StatusCode::FORBIDDEN,
            "source wallet is disabled or does not belong to this owner",
        ),
        (
            "system",
            json!({}),
            StatusCode::FORBIDDEN,
            "source wallet is disabled or does not belong to this owner",
        ),
        (
            "source",
            json!({"to_address":null}),
            StatusCode::BAD_REQUEST,
            "provide exactly one destination",
        ),
        (
            "source",
            json!({"to_wallet_id":"target"}),
            StatusCode::BAD_REQUEST,
            "provide exactly one destination",
        ),
        (
            "source",
            json!({"to_address":null,"to_wallet_id":"missing"}),
            StatusCode::BAD_REQUEST,
            "wallet not found",
        ),
        (
            "source",
            json!({"to_address":null,"to_wallet_id":"foreign"}),
            StatusCode::FORBIDDEN,
            "target wallet does not belong to this owner",
        ),
        (
            "source",
            json!({"to_address":null,"to_wallet_id":"disabled"}),
            StatusCode::FORBIDDEN,
            "target wallet does not belong to this owner",
        ),
        (
            "source",
            json!({"to_address":null,"to_wallet_id":"system"}),
            StatusCode::FORBIDDEN,
            "target wallet does not belong to this owner",
        ),
        (
            "source",
            json!({"to_address":"bad", "amount_lamports":0}),
            StatusCode::BAD_REQUEST,
            "destination is not a valid Solana address",
        ),
        (
            "source",
            json!({"amount_lamports":0}),
            StatusCode::BAD_REQUEST,
            "fixed requires amount_lamports; percent supports only 50 or 100",
        ),
        (
            "source",
            json!({"amount_lamports":null}),
            StatusCode::BAD_REQUEST,
            "fixed requires amount_lamports; percent supports only 50 or 100",
        ),
        (
            "source",
            json!({"percent":50}),
            StatusCode::BAD_REQUEST,
            "fixed requires amount_lamports; percent supports only 50 or 100",
        ),
        (
            "source",
            json!({"amount_mode":"percent", "amount_lamports":null, "percent":75}),
            StatusCode::BAD_REQUEST,
            "fixed requires amount_lamports; percent supports only 50 or 100",
        ),
        (
            "source",
            json!({"amount_mode":"percent", "percent":50}),
            StatusCode::BAD_REQUEST,
            "fixed requires amount_lamports; percent supports only 50 or 100",
        ),
    ] {
        error_body(
            send(
                &app,
                "POST",
                &format!("/internal/wallets/{id}/transfer-sol"),
                LOCAL,
                Some(TOKEN),
                Some(transfer(patch)),
            )
            .await,
            status,
            message,
        )
        .await;
        assert_eq!(count(&pool).await, 0);
    }
}

#[tokio::test]
async fn legacy_transfer_reserves_exact_units_before_missing_runtime_and_duplicate_returns_v1_record(
) {
    for mode in 0..5 {
        let (mut state, pool) = fixture().await;
        let mut request = transfer(json!({"to_address":format!(" \t{}\n", address(2))}));
        let id = if mode == 4 { "system" } else { "source" };
        if mode == 4 {
            state.config = Arc::new({
                let mut config = (*state.config).clone();
                config.allow_internal_system_wallet_transfer = true;
                config
            });
        }
        if mode == 1 || mode == 2 {
            request["amount_mode"] = json!("percent");
            request["amount_lamports"] = Value::Null;
            request["percent"] = json!(if mode == 1 { 50 } else { 100 });
        } else if mode == 3 {
            request["amount_lamports"] = json!(u64::MAX);
        }
        let app = http::router(state.clone());
        let path = format!("/internal/wallets/{id}/transfer-sol");
        error_body(
            send(
                &app,
                "POST",
                &path,
                LOCAL,
                Some(TOKEN),
                Some(request.clone()),
            )
            .await,
            StatusCode::SERVICE_UNAVAILABLE,
            "source wallet is not loaded",
        )
        .await;
        let record = state
            .wallet_repository
            .get_sol_transfer_by_idempotency("fixture-transfer-key")
            .await
            .unwrap();
        assert_eq!(record.status, "failed");
        assert_eq!(record.to_pubkey, address(2));
        assert_eq!(
            record.requested_amount_lamports,
            if mode == 1 || mode == 2 {
                None
            } else if mode == 3 {
                Some(u64::MAX)
            } else {
                Some(1_000_000)
            }
        );
        assert_eq!(
            record.requested_percent,
            if mode == 1 {
                Some(50)
            } else if mode == 2 {
                Some(100)
            } else {
                None
            }
        );
        let response =
            body(send(&app, "POST", &path, LOCAL, Some(TOKEN), Some(request)).await).await;
        assert_eq!(
            response,
            json!({
                "transfer_id":record.transfer_id, "wallet_id":id,
                "from_pubkey":address(if mode == 4 { 4 } else { 1 }), "to_pubkey":address(2),
                "status":"failed", "amount_lamports":null, "fee_lamports":null,
                "balance_before_lamports":null, "signature":null,
                "error":"source wallet is not loaded", "duplicate":true,
            })
        );
        assert_eq!(count(&pool).await, 1);
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}

#[tokio::test]
async fn transfer_duplicate_compares_complete_intent_instead_of_only_owner() {
    let (state, pool) = fixture().await;
    let app = http::router(state);
    assert_eq!(
        send(
            &app,
            "POST",
            PATH,
            LOCAL,
            Some(TOKEN),
            Some(transfer(json!({})))
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    error_body(
        send(
            &app,
            "POST",
            "/internal/wallets/target/transfer-sol",
            LOCAL,
            Some(TOKEN),
            Some(transfer(
                json!({"to_address":address(9),"amount_lamports":2}),
            )),
        )
        .await,
        StatusCode::CONFLICT,
        "idempotency key was used for another SOL transfer intent",
    )
    .await;
    error_body(
        send(
            &app,
            "POST",
            "/internal/wallets/foreign/transfer-sol",
            LOCAL,
            Some(TOKEN),
            Some(transfer(json!({"owner_id":"7"}))),
        )
        .await,
        StatusCode::CONFLICT,
        "idempotency key belongs to another owner",
    )
    .await;
    assert_eq!(count(&pool).await, 1);
}

#[tokio::test]
async fn transfer_finish_and_reservation_failures_are_reported_without_claiming_completion() {
    for finish in [false, true] {
        let (state, pool) = fixture().await;
        sqlx::raw_sql(if finish {
            "CREATE TRIGGER reject_transfer_write BEFORE UPDATE ON wallet_sol_transfers BEGIN SELECT RAISE(FAIL,'synthetic finish failure'); END;"
        } else {
            "CREATE TRIGGER reject_transfer_write BEFORE INSERT ON wallet_sol_transfers BEGIN SELECT RAISE(FAIL,'synthetic reserve failure'); END;"
        }).execute(&pool).await.unwrap();
        let app = http::router(state.clone());
        let response = send(
            &app,
            "POST",
            PATH,
            LOCAL,
            Some(TOKEN),
            Some(transfer(json!({}))),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap())
                .unwrap();
        if finish {
            assert_eq!(
                value["error"],
                "SOL transfer journal unavailable; do not resubmit or use a new key"
            );
            let record = state
                .wallet_repository
                .get_sol_transfer_by_idempotency("fixture-transfer-key")
                .await
                .unwrap();
            assert_eq!(record.status, "processing");
            assert!(record.signature.is_none());
            let replay = body(
                send(
                    &app,
                    "POST",
                    PATH,
                    LOCAL,
                    Some(TOKEN),
                    Some(transfer(json!({}))),
                )
                .await,
            )
            .await;
            assert_eq!(replay["status"], "processing");
        } else {
            assert_eq!(
                value["error"],
                "SOL transfer journal unavailable; do not resubmit or use a new key"
            );
            assert_eq!(count(&pool).await, 0);
        }
    }
}

#[tokio::test]
async fn legacy_transfer_literal_key_owner_and_json_integer_rules_remain_explicit() {
    let (state, pool) = fixture().await;
    let app = http::router(state.clone());
    assert_eq!(
        send(
            &app,
            "POST",
            PATH,
            LOCAL,
            Some(TOKEN),
            Some(transfer(json!({"idempotency_key":" literal-key "})))
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(state
        .wallet_repository
        .get_sol_transfer_by_idempotency(" literal-key ")
        .await
        .is_ok());
    assert!(state
        .wallet_repository
        .get_sol_transfer_by_idempotency("literal-key")
        .await
        .is_err());
    error_body(
        send(
            &app,
            "POST",
            PATH,
            LOCAL,
            Some(TOKEN),
            Some(transfer(json!({"owner_id":" 42 "}))),
        )
        .await,
        StatusCode::FORBIDDEN,
        "source wallet is disabled or does not belong to this owner",
    )
    .await;
    for patch in [
        json!({"amount_lamports":1.5}),
        json!({"amount_lamports":-1}),
        json!({"amount_mode":"Fixed"}),
    ] {
        assert_eq!(
            send(&app, "POST", PATH, LOCAL, None, Some(transfer(patch)))
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(count(&pool).await, 1);
}

#[tokio::test]
async fn legacy_transfer_repository_concurrent_reservation_and_unchecked_missing_finish_baseline() {
    let (state, pool) = fixture().await;
    let mut tasks = Vec::new();
    for n in 0..8 {
        let repository = state.wallet_repository.clone();
        tasks.push(tokio::spawn(async move {
            repository
                .reserve_sol_transfer(&SolTransferRecord {
                    transfer_id: format!("fixture-transfer-{n}"),
                    idempotency_key: "shared-key".into(),
                    owner_type: "telegram".into(),
                    owner_id: "42".into(),
                    wallet_id: "source".into(),
                    from_pubkey: address(1),
                    to_pubkey: address(2),
                    amount_mode: "fixed".into(),
                    requested_amount_lamports: Some(n + 1),
                    requested_percent: None,
                    actual_amount_lamports: None,
                    fee_lamports: None,
                    balance_before_lamports: None,
                    status: "processing".into(),
                    signature: None,
                    error: None,
                })
                .await
                .unwrap()
        }));
    }
    let mut created = 0;
    for task in tasks {
        created += usize::from(task.await.unwrap());
    }
    assert_eq!(created, 1);
    assert_eq!(count(&pool).await, 1);
    state
        .wallet_repository
        .finish_sol_transfer(
            "missing-id",
            "confirmed",
            None,
            None,
            None,
            Some("synthetic-signature"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(count(&pool).await, 1);
}
