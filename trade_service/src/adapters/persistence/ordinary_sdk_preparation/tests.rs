use super::*;
use crate::{
    adapters::sdk::damm_sell_message::{candidate_fixture, DammSellCandidate},
    application::{
        ordinary_claims::test_support::{admit_request_for_pubkey, fixture, Fixture},
        ports::{
            ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
            ordinary_submission::OrdinarySubmissionJournal,
        },
    },
    config::TradePersistenceMode,
};
use sol_trade_sdk::swqos::SwqosType;
use std::sync::Arc;

mod submission;

struct Case {
    f: Fixture,
    owner: ExecutionOwner,
    candidate: DammSellCandidate,
    store: SqliteOrdinaryExecutionClaims,
}

async fn admit_sdk(state: crate::AppState) -> Fixture {
    let c = candidate_fixture(SwqosType::Default, false, 100);
    admit_candidate(state, &c).await
}

async fn admit_candidate(state: crate::AppState, c: &DammSellCandidate) -> Fixture {
    let d = c.metadata().details();
    admit_request_for_pubkey(state, serde_json::from_value(serde_json::json!({
        "wallet_id":"fixture-wallet", "side":"sell", "pool":"meteora-damm-v2",
        "mint": crate::domain::trade::Address::from_bytes(d.mint).to_string(),
        "quote_mint":crate::domain::trade::Address::from_bytes(d.quote_mint).to_string(),
        "pool_id":crate::domain::trade::Address::from_bytes(d.pool.pool).to_string(),
        "sell_percent":d.sell_percent, "slippage_bps":d.slippage_bps, "tip_sol":f64::from_bits(d.tip_sol_bits),
        "use_mev": d.use_mev, "mev_provider":d.requested_mev_provider,
        "simulate":false, "execution_method":"sdk", "idempotency_key":" sdk-candidate-literal "
    })).unwrap(), crate::domain::trade::Address::from_bytes(d.wallet).to_string()).await
}

async fn setup() -> Case {
    let base = fixture(TradePersistenceMode::Durable).await;
    let f = admit_sdk(base.state).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original SDK owner required")
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let store = SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests());
    Case {
        f,
        owner,
        candidate: candidate_fixture(SwqosType::Default, false, 100),
        store,
    }
}

async fn prepare(c: &Case) -> Result<()> {
    c.store
        .prepare_sdk_candidate(
            &c.f.execution,
            &c.owner,
            c.candidate.metadata(),
            &c.candidate.message_bytes(),
        )
        .await
}

async fn snapshot(c: &Case) -> (Vec<Option<String>>, serde_json::Value) {
    let row = sqlx::query("SELECT submission_stage,owner_nonce,sdk_prepared_json,hex(unsigned_message),unsigned_sha256,bound_quote_json,preflight_json,signed_signature,hex(signed_transaction),signed_sha256,submission_error,final_observation_json FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id).fetch_one(&c.store.pool).await.unwrap();
    let columns = (0..12).map(|i| row.try_get(i).unwrap()).collect();
    (
        columns,
        serde_json::to_value(
            c.f.state
                .persistence
                .get(&c.f.record.request_id)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn sdk_prepared_candidate_records_and_reads_the_original_message_without_quote_or_signing_permission(
) {
    let c = setup().await;
    let parent_before = snapshot(&c).await.1;
    prepare(&c).await.unwrap();
    let stored = c
        .store
        .read_sdk_candidate(&c.f.execution, &c.owner)
        .await
        .unwrap();
    assert_eq!(stored.metadata, *c.candidate.metadata());
    assert_eq!(stored.message, c.candidate.message_bytes());
    assert_eq!(snapshot(&c).await.1, parent_before);
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id)
        .fetch_one(&c.store.pool)
        .await
        .unwrap();
    assert_eq!(
        row.try_get::<&str, _>("submission_stage").unwrap(),
        "prepared"
    );
    assert!(row
        .try_get::<Option<String>, _>("bound_quote_json")
        .unwrap()
        .is_none());
    assert!(row
        .try_get::<Option<String>, _>("signed_signature")
        .unwrap()
        .is_none());
    assert!(c.store.begin(&c.f.execution, &c.owner).await.is_err());
}

#[tokio::test]
async fn sdk_candidate_prepare_is_single_assignment_and_readback_does_not_change_any_row() {
    let c = setup().await;
    prepare(&c).await.unwrap();
    let before = snapshot(&c).await;
    assert!(prepare(&c).await.is_err());
    for _ in 0..3 {
        c.store
            .read_sdk_candidate(&c.f.execution, &c.owner)
            .await
            .unwrap();
        assert_eq!(snapshot(&c).await, before);
        assert_eq!(
            c.f.claims().claim(&c.f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
    assert!(c.store.begin(&c.f.execution, &c.owner).await.is_err());
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_candidate_prepare_binds_every_reserved_command_field_and_original_wallet() {
    let c = setup().await;
    let before = snapshot(&c).await;
    for field in [
        "wallet",
        "mint",
        "quote",
        "pool",
        "percent",
        "slippage",
        "tip",
        "use_mev",
        "requested_provider",
        "simulate",
    ] {
        let mut d = c.candidate.metadata().details().clone();
        match field {
            "wallet" => d.wallet = [30; 32],
            "mint" => d.mint = [30; 32],
            "quote" => d.quote_mint = [30; 32],
            "pool" => {
                d.pool_hint = None;
            }
            "percent" => {
                d.sell_percent = 50;
                d.input_units /= 2;
            }
            "slippage" => d.slippage_bps += 1,
            "tip" => d.tip_sol_bits = 0.002_f64.to_bits(),
            "use_mev" => d.use_mev = None,
            "requested_provider" => {
                d.requested_mev_provider = Some(crate::domain::trade::MevProvider::Jito)
            }
            "simulate" => d.simulate = true,
            _ => unreachable!(),
        }
        let metadata = SdkCandidateMetadata::damm_sell(d, &c.candidate.message_bytes()).unwrap();
        assert!(
            c.store
                .prepare_sdk_candidate(
                    &c.f.execution,
                    &c.owner,
                    &metadata,
                    &c.candidate.message_bytes()
                )
                .await
                .is_err(),
            "{field}"
        );
        assert_eq!(snapshot(&c).await, before, "{field}");
    }
}

#[tokio::test]
async fn sdk_candidate_wrong_owner_parent_or_claim_metadata_cannot_write_a_prepared_record() {
    for (table, column, value) in [
        ("trades", "wallet_id", "another-wallet"),
        ("trades", "wallet_pubkey", "other-key"),
        ("trades", "idempotency_key", "sdk-candidate-literal"),
        ("trades", "request_hash", "changed"),
        ("trades", "status", "accepted"),
        ("trades", "signature", "previous-signature"),
        ("trades", "request_json", "{}"),
        (
            "ordinary_execution_claims",
            "owner_nonce",
            "different-owner",
        ),
        (
            "ordinary_execution_claims",
            "unsigned_sha256",
            "partial-metadata",
        ),
        ("ordinary_execution_claims", "sdk_prepared_json", "{}"),
    ] {
        let c = setup().await;
        sqlx::query(&format!("UPDATE {table} SET {column}=? WHERE request_id=?"))
            .bind(value)
            .bind(&c.f.record.request_id)
            .execute(&c.store.pool)
            .await
            .unwrap();
        let before = snapshot(&c).await;
        assert!(prepare(&c).await.is_err(), "{column}");
        assert_eq!(snapshot(&c).await, before, "{column}");
    }
    let c = setup().await;
    let wrong = ExecutionOwner::fixture(&c.f.execution);
    let before = snapshot(&c).await;
    assert!(c
        .store
        .prepare_sdk_candidate(
            &c.f.execution,
            &wrong,
            c.candidate.metadata(),
            &c.candidate.message_bytes()
        )
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_candidate_write_failure_or_after_write_corruption_rolls_back_stage_and_metadata() {
    for sql in [
        "CREATE TRIGGER deny_sdk BEFORE UPDATE OF sdk_prepared_json ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END",
        "CREATE TRIGGER corrupt_sdk AFTER UPDATE OF sdk_prepared_json ON ordinary_execution_claims BEGIN UPDATE ordinary_execution_claims SET sdk_prepared_json='{}' WHERE request_id=NEW.request_id; END",
        "CREATE TRIGGER corrupt_sdk AFTER UPDATE OF sdk_prepared_json ON ordinary_execution_claims BEGIN UPDATE ordinary_execution_claims SET unsigned_message=x'FF' WHERE request_id=NEW.request_id; END",
        "CREATE TRIGGER corrupt_sdk AFTER UPDATE OF sdk_prepared_json ON ordinary_execution_claims BEGIN UPDATE trades SET status='unknown' WHERE request_id=NEW.request_id; END",
    ] {
        let c = setup().await;
        sqlx::raw_sql(sql).execute(&c.store.pool).await.unwrap();
        let before = snapshot(&c).await;
        assert!(prepare(&c).await.is_err());
        assert_eq!(snapshot(&c).await, before);
        assert!(c.store.read_sdk_candidate(&c.f.execution, &c.owner).await.is_err());
    }
}

#[tokio::test]
async fn sdk_candidate_readback_rejects_corrupted_or_partial_metadata_and_never_reissues_owner() {
    for (column, value) in [
        ("sdk_prepared_json", "{}"),
        ("unsigned_message", "wrong-message"),
        ("unsigned_sha256", "wrong-digest"),
        ("submission_stage", "signing"),
        ("signed_signature", "unexpected-signature"),
        ("preflight_json", "{}"),
        ("submission_error", "unexpected-error"),
        ("final_observation_json", "{}"),
    ] {
        let c = setup().await;
        prepare(&c).await.unwrap();
        sqlx::query("PRAGMA ignore_check_constraints=ON")
            .execute(&c.store.pool)
            .await
            .unwrap();
        sqlx::query(&format!(
            "UPDATE ordinary_execution_claims SET {column}=? WHERE request_id=?"
        ))
        .bind(value)
        .bind(&c.f.record.request_id)
        .execute(&c.store.pool)
        .await
        .unwrap();
        let before = snapshot(&c).await;
        assert!(
            c.store
                .read_sdk_candidate(&c.f.execution, &c.owner)
                .await
                .is_err(),
            "{column}"
        );
        assert!(
            c.f.claims().claim(&c.f.execution).await.is_err(),
            "{column}"
        );
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sixteen_sdk_candidate_writers_cannot_replace_the_first_prepared_message() {
    let c = Arc::new(setup().await);
    let mut tasks = vec![];
    for _ in 0..16 {
        let c = c.clone();
        tasks.push(tokio::spawn(async move { prepare(&c).await }));
    }
    let mut prepared = 0;
    for task in tasks {
        prepared += usize::from(task.await.unwrap().is_ok());
    }
    assert_eq!(prepared, 1);
    assert_eq!(
        c.store
            .read_sdk_candidate(&c.f.execution, &c.owner)
            .await
            .unwrap()
            .message,
        c.candidate.message_bytes()
    );
}

#[tokio::test]
async fn sdk_metadata_does_not_enter_the_legacy_aggregate_claim_or_journal() {
    let f = fixture(TradePersistenceMode::Durable).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original aggregate owner")
    };
    let store = SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests());
    sqlx::query("UPDATE ordinary_execution_claims SET sdk_prepared_json='{}' WHERE request_id=?")
        .bind(&f.record.request_id)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(f.claims().claim(&f.execution).await.is_err());
    assert!(store.observe(&f.execution, &owner).await.is_err());
    assert!(store
        .read_sdk_candidate(&f.execution, &owner)
        .await
        .is_err());
}

#[tokio::test]
async fn sdk_candidate_row_validator_rejects_wrong_operation_without_relying_on_its_caller() {
    let c = setup().await;
    sqlx::query(
        "UPDATE ordinary_execution_claims SET operation='ordinary-sol-buy' WHERE request_id=?",
    )
    .bind(&c.f.record.request_id)
    .execute(&c.store.pool)
    .await
    .unwrap();
    let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id)
        .fetch_one(&c.store.pool)
        .await
        .unwrap();
    assert!(validate_sdk_row(&row).is_err());
}

#[tokio::test]
async fn sdk_candidate_readback_rejects_noncanonical_or_missing_optional_metadata_fields() {
    for variant in ["whitespace", "missing_option"] {
        let c = setup().await;
        prepare(&c).await.unwrap();
        let json = c
            .candidate
            .metadata()
            .encode(&c.candidate.message_bytes())
            .unwrap();
        let altered = if variant == "whitespace" {
            format!(" {json}")
        } else {
            let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
            value["details"]
                .as_object_mut()
                .unwrap()
                .remove("requested_mev_provider");
            serde_json::to_string(&value).unwrap()
        };
        sqlx::query("UPDATE ordinary_execution_claims SET sdk_prepared_json=? WHERE request_id=?")
            .bind(altered)
            .bind(&c.f.record.request_id)
            .execute(&c.store.pool)
            .await
            .unwrap();
        let before = snapshot(&c).await;
        assert!(
            c.store
                .read_sdk_candidate(&c.f.execution, &c.owner)
                .await
                .is_err(),
            "{variant}"
        );
        assert!(
            c.f.claims().claim(&c.f.execution).await.is_err(),
            "{variant}"
        );
        assert_eq!(snapshot(&c).await, before);
    }
}

#[tokio::test]
async fn sdk_candidate_file_reopen_preserves_metadata_but_cannot_issue_a_recovery_owner() {
    use crate::{config::TradePersistenceConfig, persistence::TradePersistence};
    let dir = std::env::temp_dir().join(format!("sdk-prepared-reopen-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2_000,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let f = admit_sdk(state).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original owner required")
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let c = Case {
        store: SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests()),
        f,
        owner,
        candidate: candidate_fixture(SwqosType::Default, false, 100),
    };
    prepare(&c).await.unwrap();
    let before = snapshot(&c).await;
    c.store.pool.close().await;
    let restored = TradePersistence::new(&config).await.unwrap();
    let store = restored.ordinary_execution_claims().unwrap();
    // Retain the actual original owner only for this continuity test. No code
    // reconstructs it from the persisted nominal candidate after a real restart.
    let stored = store
        .read_sdk_candidate(&c.f.execution, &c.owner)
        .await
        .unwrap();
    assert_eq!(stored.message, c.candidate.message_bytes());
    assert_eq!(stored.metadata, *c.candidate.metadata());
    assert_eq!(
        store.claim(&c.f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    let parent = restored.get(&c.f.record.request_id).await.unwrap().unwrap();
    assert_eq!(parent.status, TradeStatus::Unknown);
    assert!(parent.signature.is_none());
    assert!(store
        .prepare_sdk_candidate(&c.f.execution, &c.owner, &stored.metadata, &stored.message)
        .await
        .is_err());
    assert!(store.begin(&c.f.execution, &c.owner).await.is_err());
    let row: (String, String, String) = sqlx::query_as("SELECT owner_nonce,submission_stage,sdk_prepared_json FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id).fetch_one(&store.pool).await.unwrap();
    assert_eq!(Some(row.0), before.0[1]);
    assert_eq!(row.1, "prepared");
    assert_eq!(Some(row.2), before.0[2]);
    store.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
