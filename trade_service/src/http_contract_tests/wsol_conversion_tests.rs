//! WSOL contract baselines using temporary SQL and an empty, keyless SDK registry.
use super::*;
use crate::conversion::{ConversionStore, WsolQuote, WsolSubmission};

const BASE: &str = "/internal/conversions/wsol";

fn quote() -> WsolQuote {
    serde_json::from_value(json!({
        "quote_id":"cq_fixture", "wallet_id":"fixture-wallet",
        "wallet_pubkey":crate::domain::trade::Address::from_bytes([1;32]).to_string(),
        "owner_type":"telegram", "owner_id":"42", "direction":"unwrap",
        "amount_lamports":1_000_000, "source_balance_lamports":2_000_000,
        "ata_exists":true, "estimated_network_fee_lamports":10_000,
        "temporary_rent_lamports":2_039_280, "rent_refund_lamports":0,
        "expires_at_ms":4_000_000_000_000u64,
    }))
    .unwrap()
}

fn submission(patch: Value) -> Value {
    let mut request = json!({
        "quote_id":"cq_fixture", "wallet_id":"fixture-wallet",
        "owner_type":"telegram", "owner_id":"42", "simulate":false,
        "confirmed":true, "idempotency_key":"fixture-key",
    });
    request
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    request
}

async fn fixture() -> (AppState, Arc<ConversionStore>) {
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.conversion_quotes_enabled = true;
    config.conversion_live_enabled = true;
    state.config = Arc::new(config);
    let store = Arc::new(ConversionStore::new("sqlite::memory:").await.unwrap());
    store.save_quote(&quote()).await.unwrap();
    state.conversions = Some(store.clone());
    let (repository, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    sqlx::query("INSERT INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES('fixture-wallet','telegram','42','fixture',?,'not-a-secret-or-valid-wallet',1)")
        .bind(quote().wallet_pubkey).execute(&pool).await.unwrap();
    state.wallet_repository = Arc::new(repository);
    state.wsol_queries = Arc::new(crate::application::wsol_queries::WsolQueryService::new(
        crate::adapters::sdk::wsol_queries::SdkWsolQuerySource::new(
            state.wallets.clone(),
            state.wallet_repository.clone(),
        ),
        Some(crate::adapters::persistence::wsol_conversion::SqliteWsolJournal::new(store.clone())),
        crate::adapters::sdk::address_parser::SdkAddressParser,
    ));
    state.wsol_submissions = Arc::new(
        crate::application::wsol_submission::WsolSubmissionService::new(
            crate::adapters::sdk::wsol_conversion::SdkWsolRuntime::new(
                state.wallets.clone(),
                state.wallet_repository.clone(),
            ),
            Some(
                crate::adapters::persistence::wsol_conversion::SqliteWsolJournal::new(
                    store.clone(),
                ),
            ),
        ),
    );
    (state, store)
}

async fn post(app: &Router, suffix: &str, input: Value) -> Response {
    send(
        app,
        "POST",
        &format!("{BASE}/{suffix}"),
        LOCAL,
        Some(TOKEN),
        Some(input),
    )
    .await
}

async fn submit_fault(response: Response, code: &str, stage: &str, message: &str) {
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-wsol-error-code"], code);
    assert_eq!(response.headers()["x-wsol-error-stage"], stage);
    assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(response.headers().get("retry-after").is_none());
    error_body(response, StatusCode::SERVICE_UNAVAILABLE,
        &format!("{message}; WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key")).await;
}

async fn application_fixture() -> (
    Router,
    Arc<crate::application::wsol_submission::tests::Shared>,
    Value,
) {
    let (service, shared) = crate::application::wsol_submission::tests::harness().await;
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.conversion_live_enabled = true;
    config.conversion_quotes_enabled = true;
    state.config = Arc::new(config);
    state.conversions = Some(shared.store.clone());
    state.wsol_submissions = service;
    let wire = WsolSubmission::from(&crate::application::wsol_submission::tests::input());
    (
        http::router(state),
        shared,
        serde_json::to_value(wire).unwrap(),
    )
}

#[tokio::test]
async fn wsol_router_simulation_and_live_receipts_use_ports_without_any_wallet_keys() {
    let (app, shared, input) = application_fixture().await;
    let mut simulation = input.clone();
    simulation["simulate"] = json!(true);
    simulation["confirmed"] = json!(false);
    let result = json_body(post(&app, "simulate", simulation).await, StatusCode::OK).await;
    assert_eq!(
        result,
        json!({"status":"simulated","quote_id":"q1","network_fee_lamports":10000,"units_consumed":999})
    );
    assert!(shared.store.get_by_key("key").await.unwrap().is_none());
    let result = json_body(post(&app, "submit", input.clone()).await, StatusCode::OK).await;
    assert_eq!(result.as_object().unwrap().len(), 7);
    assert_eq!(result["status"], "submitted");
    assert_eq!(result["signature"], "synthetic-signature");
    assert_eq!(result["duplicate"], false);
    let mut expected = result.clone();
    expected["duplicate"] = json!(true);
    assert_eq!(
        json_body(post(&app, "submit", input).await, StatusCode::OK).await,
        expected
    );
    assert_eq!(
        shared
            .events()
            .iter()
            .filter(|event| event.starts_with("broadcast:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn wsol_router_signed_simulation_failure_returns_original_signature_and_never_reopens_wallet()
{
    for rpc_error in [false, true] {
        let (app, shared, input) = application_fixture().await;
        shared.set_simulation_failure(rpc_error);
        let body = json_body(post(&app, "submit", input.clone()).await, StatusCode::OK).await;
        let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(
            body,
            json!({
                "submission_id":signed.submission_id,"quote_id":"q1","wallet_id":"w1",
                "status":"unknown","signature":"synthetic-signature","duplicate":false,
                "error":"pre-submit simulation failed or timed out; signed evidence retained; do not resubmit with a new key",
            })
        );
        assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        let duplicate = json_body(post(&app, "submit", input.clone()).await, StatusCode::OK).await;
        let mut expected = body;
        expected["duplicate"] = json!(true);
        assert_eq!(duplicate, expected);
        let mut another = input;
        another["idempotency_key"] = json!("another-key");
        error_body(
            post(&app, "submit", another).await,
            StatusCode::CONFLICT,
            "wallet has a pending conversion; reconcile it before submitting another",
        )
        .await;
        assert!(!shared
            .events()
            .iter()
            .any(|e| e == "fail_before_sign" || e.starts_with("broadcast:")));
        assert_eq!(
            shared
                .events()
                .iter()
                .filter(|e| e.starts_with("sign:"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn wsol_router_request_cancellation_after_signed_commit_keeps_evidence_and_never_retries() {
    use crate::application::wsol_submission::tests::Stage;
    for stage in [
        Stage::AfterSignedWrite,
        Stage::Simulation,
        Stage::BeforeReceiptWrite,
    ] {
        let (app, shared, input) = application_fixture().await;
        let gate = shared.pause_stage(stage);
        let task = tokio::spawn({
            let app = app.clone();
            let input = input.clone();
            async move { post(&app, "submit", input).await }
        });
        gate.wait_until_entered().await;
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(signed.status, "signed");
        assert_eq!(signed.signature.as_deref(), Some("synthetic-signature"));
        assert_eq!(signed.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        let trace = shared.events();
        let duplicate = json_body(post(&app, "submit", input).await, StatusCode::OK).await;
        assert_eq!(duplicate["status"], "signed");
        assert_eq!(duplicate["signature"], "synthetic-signature");
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_router_rejects_misbound_reservation_before_wallet_access_and_never_replays_it() {
    for field in [3, 6, 8] {
        let (app, shared, input) = application_fixture().await;
        shared.set_reservation_corruption(field);
        submit_fault(
            post(&app, "submit", input.clone()).await,
            "wsol_integrity_error",
            "reservation",
            "WSOL reservation does not match the original request and quote",
        )
        .await;
        assert_eq!(shared.events(), ["quote", "reserve"]);
        let record = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(record.status, "reserved");
        assert!(record.signature.is_none() && record.signed_transaction.is_none());
        shared.set_reservation_corruption(0);
        let duplicate = json_body(post(&app, "submit", input).await, StatusCode::OK).await;
        assert_eq!(duplicate["status"], "reserved");
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(shared.events(), ["quote", "reserve", "quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_router_reservation_ack_loss_is_not_a_license_to_execute_again_or_change_key() {
    let (app, shared, input) = application_fixture().await;
    shared.set_write_failure(7);
    submit_fault(
        post(&app, "submit", input.clone()).await,
        "wsol_outcome_unknown",
        "reservation",
        "synthetic reservation commit acknowledgment lost",
    )
    .await;
    assert_eq!(shared.events(), ["quote", "reserve"]);
    let duplicate = json_body(post(&app, "submit", input.clone()).await, StatusCode::OK).await;
    assert_eq!(duplicate.as_object().unwrap().len(), 7);
    assert_eq!(duplicate["status"], "reserved");
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(duplicate["signature"], Value::Null);
    let mut another = input;
    another["idempotency_key"] = json!("another-key");
    error_body(
        post(&app, "submit", another).await,
        StatusCode::CONFLICT,
        "wallet has a pending conversion; reconcile it before submitting another",
    )
    .await;
    assert!(shared
        .events()
        .iter()
        .all(|e| e == "quote" || e == "reserve"));
}

#[tokio::test]
async fn wsol_router_rejects_unsigned_failure_receipt_or_readback_from_another_identity() {
    for readback in [false, true] {
        let (app, shared, input) = application_fixture().await;
        shared.set_prepare_failure();
        shared.set_failure_corruption(6, readback);
        submit_fault(
            post(&app, "submit", input.clone()).await,
            "wsol_integrity_error",
            "unsigned_failure",
            "WSOL submission identity changed during readback",
        )
        .await;
        let failed = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error.as_deref(), Some("synthetic prepare failure"));
        assert!(failed.signature.is_none() && failed.signed_transaction.is_none());
        let trace = shared.events();
        let duplicate = json_body(post(&app, "submit", input).await, StatusCode::OK).await;
        assert_eq!(duplicate["status"], "failed");
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(duplicate["signature"], Value::Null);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
        assert!(!shared.events().iter().any(|e| e.starts_with("sign:")
            || e.starts_with("simulate:")
            || e.starts_with("broadcast:")));
    }
}

#[tokio::test]
async fn wsol_router_unsigned_failure_cancel_and_commit_ack_loss_keep_original_key_immutable() {
    use crate::application::wsol_submission::tests::Stage;
    for stage in [
        Some(Stage::BeforeFailureWrite),
        Some(Stage::AfterFailureWrite),
        None,
    ] {
        let (app, shared, input) = application_fixture().await;
        shared.set_prepare_failure();
        if let Some(stage) = stage {
            let gate = shared.pause_stage(stage);
            let task = tokio::spawn({
                let app = app.clone();
                let input = input.clone();
                async move { post(&app, "submit", input).await }
            });
            gate.wait_until_entered().await;
            task.abort();
            assert!(task.await.err().unwrap().is_cancelled());
        } else {
            shared.set_write_failure(6);
            submit_fault(
                post(&app, "submit", input.clone()).await,
                "wsol_outcome_unknown",
                "unsigned_failure",
                "synthetic unsigned failure commit acknowledgment lost",
            )
            .await;
        }
        let record = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(
            record.status,
            if stage == Some(Stage::BeforeFailureWrite) {
                "reserved"
            } else {
                "failed"
            }
        );
        let trace = shared.events();
        let duplicate = json_body(post(&app, "submit", input).await, StatusCode::OK).await;
        assert_eq!(duplicate.as_object().unwrap().len(), 7);
        assert_eq!(duplicate["status"], record.status);
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(duplicate["signature"], Value::Null);
        assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
        assert!(!shared.events().iter().any(|e| e.starts_with("sign:")
            || e.starts_with("simulate:")
            || e.starts_with("broadcast:")));
    }
}

#[tokio::test]
async fn wsol_router_concurrent_requests_share_one_reservation_signer_and_sender() {
    let (app, shared, input) = application_fixture().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let app = app.clone();
        let input = input.clone();
        tasks.push(tokio::spawn(async move {
            json_body(post(&app, "submit", input).await, StatusCode::OK).await
        }));
    }
    let mut created = 0;
    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        let result = task.await.unwrap();
        created += usize::from(result["duplicate"] == false);
        ids.insert(result["submission_id"].as_str().unwrap().to_string());
    }
    assert_eq!(created, 1);
    assert_eq!(ids.len(), 1);
    for prefix in ["prepare:", "sign:", "broadcast:"] {
        assert_eq!(
            shared
                .events()
                .iter()
                .filter(|event| event.starts_with(prefix))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn wsol_router_late_send_receipt_preserves_observed_state_and_original_signature() {
    use crate::{
        application::wsol_submission_types::WsolSignedOperation,
        domain::wsol_progress::{WsolObservedOutcome as O, WsolUpdate},
    };
    for observed in [O::Submitted, O::Confirmed, O::Failed] {
        let (app, shared, input) = application_fixture().await;
        let gate = shared.pause_broadcast(true);
        let task = tokio::spawn({
            let app = app.clone();
            let input = input.clone();
            async move { json_body(post(&app, "submit", input).await, StatusCode::OK).await }
        });
        gate.wait_until_entered().await;
        let signed = shared.store.get_by_key("key").await.unwrap().unwrap();
        let error = matches!(observed, O::Failed).then_some("synthetic chain failure");
        shared
            .store
            .apply_signed_update(
                WsolSignedOperation::from_record(&signed).unwrap(),
                WsolUpdate::Observation(observed),
                error,
                100,
            )
            .await
            .unwrap();
        gate.release();
        let body = task.await.unwrap();
        assert_eq!(
            body,
            json!({
                "submission_id":signed.submission_id,"quote_id":signed.quote_id,"wallet_id":signed.wallet_id,
                "status":observed.state().as_str(),"signature":signed.signature,"error":error,"duplicate":false
            })
        );
        let duplicate = json_body(post(&app, "submit", input).await, StatusCode::OK).await;
        assert_eq!(duplicate["status"], body["status"]);
        assert_eq!(duplicate["duplicate"], true);
        let current = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(current.signed_transaction, signed.signed_transaction);
        assert_eq!(
            shared
                .events()
                .iter()
                .filter(|e| e.starts_with("broadcast:"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn wsol_router_storage_errors_are_503_and_duplicates_never_retry_or_rebroadcast() {
    for stage in [1, 2] {
        let (app, shared, input) = application_fixture().await;
        shared.set_write_failure(stage);
        submit_fault(
            post(&app, "submit", input.clone()).await,
            "wsol_outcome_unknown",
            if stage == 1 {
                "signed_persistence"
            } else {
                "broadcast_receipt"
            },
            if stage == 1 {
                "synthetic signed write failure"
            } else {
                "synthetic receipt write failure"
            },
        )
        .await;
        let stored = shared.store.get_by_key("key").await.unwrap().unwrap();
        assert_eq!(
            stored.status,
            if stage == 1 { "reserved" } else { "signed" }
        );
        let before = shared.events();
        let result = json_body(post(&app, "submit", input).await, StatusCode::OK).await;
        assert_eq!(result["duplicate"], true);
        assert_eq!(result["status"], stored.status);
        assert_eq!(&shared.events()[before.len()..], ["quote", "reserve"]);
    }
}

#[tokio::test]
async fn wsol_router_submit_fault_matrix_has_typed_stages_and_only_verified_recovery_identifiers() {
    for (fault, code, stage, message, known_id, known_signature) in [
        (
            1,
            "wsol_outcome_unknown",
            "signed_persistence",
            "synthetic signed write failure",
            true,
            false,
        ),
        (
            2,
            "wsol_outcome_unknown",
            "broadcast_receipt",
            "synthetic receipt write failure",
            true,
            true,
        ),
        (
            3,
            "wsol_outcome_unknown",
            "unsigned_failure",
            "synthetic failure receipt write failure",
            true,
            false,
        ),
        (
            4,
            "wsol_outcome_unknown",
            "simulation_receipt",
            "synthetic simulation receipt write failure",
            true,
            true,
        ),
        (
            5,
            "wsol_outcome_unknown",
            "signed_persistence",
            "synthetic signed commit acknowledgment lost",
            true,
            false,
        ),
        (
            6,
            "wsol_outcome_unknown",
            "unsigned_failure",
            "synthetic unsigned failure commit acknowledgment lost",
            true,
            false,
        ),
        (
            7,
            "wsol_outcome_unknown",
            "reservation",
            "synthetic reservation commit acknowledgment lost",
            false,
            false,
        ),
        (
            8,
            "wsol_unavailable",
            "quote_lookup",
            "synthetic quote storage failure https://fixture.invalid/private-api-key",
            false,
            false,
        ),
        (
            9,
            "wsol_outcome_unknown",
            "reservation",
            "conversion quote expired",
            false,
            false,
        ),
        (
            10,
            "wsol_outcome_unknown",
            "signed_persistence",
            "synthetic late rejection",
            true,
            false,
        ),
    ] {
        let (app, shared, input) = application_fixture().await;
        shared.set_write_failure(fault);
        if matches!(fault, 3 | 6) {
            shared.set_prepare_failure();
        }
        if fault == 4 {
            shared.set_simulation_failure(false);
        }
        let response = post(&app, "submit", input.clone()).await;
        let stored = shared.store.get_by_key("key").await.unwrap();
        assert_eq!(
            response.headers().get("x-wsol-submission-id").is_some(),
            known_id,
            "fault {fault}"
        );
        if known_id {
            assert_eq!(
                response.headers()["x-wsol-submission-id"],
                stored.as_ref().unwrap().submission_id.as_str()
            );
        }
        assert_eq!(
            response.headers().get("x-wsol-signature").is_some(),
            known_signature
        );
        if known_signature {
            assert_eq!(
                response.headers()["x-wsol-signature"],
                "synthetic-signature"
            );
        }
        assert!(response.headers().get("x-wsol-idempotency-key").is_none());
        submit_fault(
            response,
            code,
            stage,
            &crate::sanitize::redact_urls(message),
        )
        .await;
        let trace = shared.events();
        assert_eq!(
            trace.iter().filter(|e| e.starts_with("broadcast:")).count(),
            usize::from(fault == 2)
        );
        if stored.is_some() {
            let duplicate = json_body(post(&app, "submit", input).await, StatusCode::OK).await;
            assert_eq!(duplicate["duplicate"], true);
            assert_eq!(&shared.events()[trace.len()..], ["quote", "reserve"]);
        }
    }
}

#[tokio::test]
async fn wsol_router_submit_business_conflicts_and_expired_quotes_remain_distinct_from_server_faults(
) {
    let (app, shared, input) = application_fixture().await;
    json_body(post(&app, "submit", input.clone()).await, StatusCode::OK).await;
    let original = shared.store.get_quote("q1").await.unwrap().unwrap();
    let mut changed = original.clone();
    changed.quote_id = "q2".into();
    shared.store.save_quote(&changed).await.unwrap();
    let mut conflict = input.clone();
    conflict["quote_id"] = json!("q2");
    let mut pending = input.clone();
    pending["idempotency_key"] = json!("another-key");
    for (request, message) in [
        (
            conflict,
            "idempotency key was used with different conversion parameters",
        ),
        (
            pending,
            "wallet has a pending conversion; reconcile it before submitting another",
        ),
    ] {
        let response = post(&app, "submit", request).await;
        assert_eq!(response.headers()["x-wsol-error-code"], "wsol_conflict");
        assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
        assert_eq!(
            response.headers()["x-wsol-recovery"],
            "inspect-original-operation"
        );
        assert!(response.headers().get("x-wsol-submission-id").is_none());
        assert!(response.headers().get("x-wsol-signature").is_none());
        error_body(response, StatusCode::CONFLICT, message).await;
    }
    let mut expired = original;
    expired.quote_id = "expired-for-submit".into();
    expired.expires_at_ms = 0;
    shared.store.save_quote(&expired).await.unwrap();
    let mut request = input.clone();
    request["quote_id"] = json!(expired.quote_id);
    request["idempotency_key"] = json!("expired-key");
    let response = post(&app, "submit", request).await;
    assert_eq!(response.headers()["x-wsol-error-code"], "wsol_rejected");
    assert_eq!(response.headers()["x-wsol-error-stage"], "reservation");
    error_body(
        response,
        StatusCode::BAD_REQUEST,
        "conversion quote expired",
    )
    .await;
    let mut missing = input;
    missing["quote_id"] = json!("missing-quote");
    let response = post(&app, "submit", missing).await;
    assert_eq!(response.headers()["x-wsol-error-code"], "wsol_rejected");
    assert_eq!(response.headers()["x-wsol-error-stage"], "quote_lookup");
    error_body(
        response,
        StatusCode::BAD_REQUEST,
        "conversion quote not found",
    )
    .await;
    assert_eq!(
        shared
            .events()
            .iter()
            .filter(|e| e.starts_with("sign:"))
            .count(),
        1
    );
    assert_eq!(
        shared
            .events()
            .iter()
            .filter(|e| e.starts_with("broadcast:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn wsol_router_submit_recovery_context_is_not_taken_from_corrupt_reservation_or_failure_receipt(
) {
    let (app, shared, input) = application_fixture().await;
    shared.set_reservation_corruption(8);
    let response = post(&app, "submit", input).await;
    assert!(response.headers().get("x-wsol-submission-id").is_none());
    assert!(response.headers().get("x-wsol-signature").is_none());
    submit_fault(
        response,
        "wsol_integrity_error",
        "reservation",
        "WSOL reservation does not match the original request and quote",
    )
    .await;
    let (app, shared, input) = application_fixture().await;
    shared.set_prepare_failure();
    shared.set_failure_corruption(1, false);
    let response = post(&app, "submit", input).await;
    let original = shared.store.get_by_key("key").await.unwrap().unwrap();
    assert_eq!(
        response.headers()["x-wsol-submission-id"],
        original.submission_id.as_str()
    );
    assert!(response.headers().get("x-wsol-signature").is_none());
    submit_fault(
        response,
        "wsol_integrity_error",
        "unsigned_failure",
        "WSOL submission identity changed during readback",
    )
    .await;
}

#[tokio::test]
async fn wsol_store_and_live_policy_errors_precede_keys_and_wallets() {
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.conversion_live_enabled = true;
    state.config = Arc::new(config);
    let app = http::router(state);
    let input = submission(json!({"idempotency_key":""}));
    error_body(
        post(&app, "submit", input.clone()).await,
        StatusCode::BAD_REQUEST,
        "conversion quotes are disabled",
    )
    .await;
    error_body(
        post(&app, "simulate", input).await,
        StatusCode::BAD_REQUEST,
        "simulation endpoint requires simulate=true",
    )
    .await;
    submit_fault(
        post(&app, "simulate", submission(json!({"simulate":true}))).await,
        "wsol_unavailable",
        "unsigned_admission",
        "conversion quotes are disabled",
    )
    .await;
    error_body(
        post(&app, "submit", submission(json!({"simulate":true}))).await,
        StatusCode::BAD_REQUEST,
        "live WSOL conversion is disabled or request is simulation",
    )
    .await;
    error_body(
        post(&app, "submit", submission(json!({"confirmed":false}))).await,
        StatusCode::BAD_REQUEST,
        "live WSOL conversion is disabled or request is simulation",
    )
    .await;
}

#[tokio::test]
async fn wsol_quote_metadata_and_all_guards_precede_empty_runtime() {
    let (state, _) = fixture().await;
    let app = http::router(state);
    let mut input = json!({"wallet_id":"fixture-wallet","owner_type":"telegram","owner_id":"42","direction":"wrap","amount_lamports":1});
    for (patch, message, stage) in [
        (json!({"wallet_id":"missing"}), "wallet not found", None),
        (
            json!({"owner_id":"other"}),
            "wallet is disabled or does not belong to this owner",
            None,
        ),
        (
            json!({"all":true}),
            "provide either all or amount_lamports",
            None,
        ),
        (
            json!({"all":true,"amount_lamports":null}),
            "all is only supported for WSOL unwrap",
            None,
        ),
        (json!({}), "wallet is not loaded", Some("quote_estimate")),
    ] {
        let mut request = input.clone();
        request
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let response = post(&app, "quotes", request).await;
        if let Some(stage) = stage {
            submit_fault(response, "wsol_unavailable", stage, message).await;
        } else {
            error_body(response, StatusCode::BAD_REQUEST, message).await;
        }
    }
    // Missing/zero amounts are still checked after the original runtime lookup.
    input.as_object_mut().unwrap().remove("amount_lamports");
    submit_fault(
        post(&app, "quotes", input).await,
        "wsol_unavailable",
        "quote_estimate",
        "wallet is not loaded",
    )
    .await;
}

#[tokio::test]
async fn wsol_simulation_binding_errors_do_not_reserve_a_live_key() {
    let (state, store) = fixture().await;
    let mut expired = quote();
    expired.quote_id = "expired".into();
    expired.expires_at_ms = 1;
    store.save_quote(&expired).await.unwrap();
    let app = http::router(state);
    for (patch, message, stage) in [
        (
            json!({"quote_id":"missing"}),
            "conversion quote not found",
            None,
        ),
        (
            json!({"wallet_id":"other"}),
            "submission does not match conversion quote",
            None,
        ),
        (
            json!({"idempotency_key":""}),
            "idempotency_key must be 1..128 characters",
            None,
        ),
        (
            json!({"quote_id":"expired"}),
            "conversion quote expired",
            None,
        ),
        (json!({}), "wallet is not loaded", Some("unsigned_wallet")),
    ] {
        let mut input = submission(patch);
        input["simulate"] = json!(true);
        let response = post(&app, "simulate", input).await;
        if let Some(stage) = stage {
            submit_fault(response, "wsol_unavailable", stage, message).await;
        } else {
            error_body(response, StatusCode::BAD_REQUEST, message).await;
        }
    }
    assert!(store.get_by_key("fixture-key").await.unwrap().is_none());
}

#[tokio::test]
async fn wsol_reserved_prepare_failure_is_stored_once_and_duplicate_is_seven_field_receipt() {
    let (state, store) = fixture().await;
    let app = http::router(state);
    let first = json_body(
        post(&app, "submit", submission(json!({}))).await,
        StatusCode::OK,
    )
    .await;
    let id = first["submission_id"].as_str().unwrap();
    assert_eq!(
        first,
        json!({
            "submission_id":id,"quote_id":"cq_fixture","wallet_id":"fixture-wallet",
            "status":"failed","signature":null,"error":"wallet is not loaded","duplicate":false,
        })
    );
    let record = store.get_by_id(id).await.unwrap().unwrap();
    assert!(record.signed_transaction.is_none());
    let mut expected = first.clone();
    expected["duplicate"] = json!(true);
    assert_eq!(
        json_body(
            post(&app, "submit", submission(json!({}))).await,
            StatusCode::OK
        )
        .await,
        expected
    );
    let mut changed = quote();
    changed.quote_id = "other-quote".into();
    store.save_quote(&changed).await.unwrap();
    error_body(
        post(
            &app,
            "submit",
            submission(json!({"quote_id":"other-quote"})),
        )
        .await,
        StatusCode::CONFLICT,
        "idempotency key was used with different conversion parameters",
    )
    .await;
    assert_eq!(store.get_by_id(id).await.unwrap(), Some(record));
}

#[tokio::test]
async fn wsol_status_requires_owner_and_unsigned_reserved_reads_do_not_load_runtime() {
    let (state, store) = fixture().await;
    let input: WsolSubmission = serde_json::from_value(submission(json!({}))).unwrap();
    let (record, _) = store.reserve_live(&input, &quote(), 1).await.unwrap();
    let app = http::router(state);
    let route = format!("{BASE}/submissions/{}", record.submission_id);
    assert_eq!(
        send(&app, "GET", &route, LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    error_body(
        send(
            &app,
            "GET",
            &format!("{route}?owner_type=telegram&owner_id=other"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "conversion submission does not belong to this owner",
    )
    .await;
    let result = json_body(
        send(
            &app,
            "GET",
            &format!("{route}?owner_type=telegram&owner_id=42"),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(result["status"], "reserved");
    assert_eq!(result["signature"], Value::Null);
    assert_eq!(result["duplicate"], false);
    assert_eq!(result.as_object().unwrap().len(), 7);
    assert_eq!(
        store.get_by_id(&record.submission_id).await.unwrap(),
        Some(record)
    );
}

#[tokio::test]
async fn wsol_status_owner_gate_precedes_classified_signature_and_runtime_failures() {
    for valid_signature in [false, true] {
        let (state, store) = fixture().await;
        let input: WsolSubmission = serde_json::from_value(submission(json!({}))).unwrap();
        let (record, _) = store.reserve_live(&input, &quote(), 1).await.unwrap();
        let signature = if valid_signature {
            solana_sdk::signature::Signature::from([7; 64]).to_string()
        } else {
            "synthetic-signature".into()
        };
        store
            .record_signed(&record.submission_id, &signature, &[1, 2, 3], 2)
            .await
            .unwrap();
        let app = http::router(state);
        let route = format!("{BASE}/submissions/{}", record.submission_id);
        error_body(
            send(
                &app,
                "GET",
                &format!("{route}?owner_type=telegram&owner_id=other"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::BAD_REQUEST,
            "conversion submission does not belong to this owner",
        )
        .await;
        let expected = if valid_signature {
            "wallet is not loaded".into()
        } else {
            signature
                .parse::<solana_sdk::signature::Signature>()
                .unwrap_err()
                .to_string()
        };
        submit_fault(
            send(
                &app,
                "GET",
                &format!("{route}?owner_type=telegram&owner_id=42"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            "wsol_unavailable",
            "status_lookup",
            &expected,
        )
        .await;
        assert_eq!(
            store
                .get_by_id(&record.submission_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "signed"
        );
    }
}

async fn query_fixture(
    mode: usize,
) -> (Router, Arc<crate::application::wsol_queries::tests::Shared>) {
    let (queries, shared) = crate::application::wsol_queries::tests::harness().await;
    shared.mode(mode);
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.security.broadcast = crate::domain::execution_policy::BroadcastPolicy::Disabled;
    config.conversion_live_enabled = false;
    state.config = Arc::new(config);
    state.conversions = Some(shared.store.clone());
    state.wsol_queries = queries;
    (http::router(state), shared)
}

fn recovery_route(input: &crate::application::wsol_recovery::WsolRecoveryInput) -> String {
    let mut url =
        reqwest::Url::parse("http://fixture.invalid/internal/conversions/wsol/submissions/by-key")
            .unwrap();
    url.query_pairs_mut()
        .append_pair("idempotency_key", &input.idempotency_key)
        .append_pair("wallet_id", &input.wallet_id)
        .append_pair("owner_type", &input.owner_type)
        .append_pair("owner_id", &input.owner_id);
    format!("{}?{}", url.path(), url.query().unwrap())
}

#[tokio::test]
async fn wsol_recovery_router_preserves_encoded_literal_keys_and_owner_values() {
    use crate::application::wsol_recovery::WsolRecoveryInput;
    let (app, shared) = query_fixture(0).await;
    let mut quote = quote();
    quote.owner_id = " 42 /?+&=é ".into();
    let request = WsolSubmission {
        quote_id: quote.quote_id.clone(),
        wallet_id: quote.wallet_id.clone(),
        owner_type: quote.owner_type.clone(),
        owner_id: quote.owner_id.clone(),
        simulate: false,
        confirmed: true,
        idempotency_key: "  café /?+%&=  ".into(),
    };
    let (original, _) = shared
        .store
        .reserve_live(&request, &quote, 1)
        .await
        .unwrap();
    let input = WsolRecoveryInput {
        idempotency_key: request.idempotency_key.clone(),
        wallet_id: request.wallet_id.clone(),
        owner_type: request.owner_type.clone(),
        owner_id: request.owner_id.clone(),
    };
    let body = json_body(recovery_get(&app, &input).await, StatusCode::OK).await;
    assert_eq!(body["submission_id"], original.submission_id);
    assert_eq!(
        shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(original)
    );
    let mut trimmed = input.clone();
    trimmed.idempotency_key = trimmed.idempotency_key.trim().into();
    assert_eq!(
        recovery_get(&app, &trimmed).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(shared.events(), ["recovery", "recovery"]);
}

#[tokio::test]
async fn wsol_recovery_router_during_inflight_submit_and_cancellation_does_not_advance_or_unlock() {
    use crate::application::{
        wsol_recovery::WsolRecoveryInput,
        wsol_submission::tests::{harness, input, Stage},
    };
    let (submissions, submitted) = harness().await;
    let (queries, queried) =
        crate::application::wsol_queries::tests::harness_with_store(submitted.store.clone()).await;
    let mut state = state().await;
    let mut config = (*state.config).clone();
    config.conversion_live_enabled = true;
    state.config = Arc::new(config);
    state.wsol_submissions = submissions;
    state.wsol_queries = queries;
    let app = http::router(state);
    let gate = submitted.pause_stage(Stage::Reserved);
    let running_app = app.clone();
    let task = tokio::spawn(async move {
        post(
            &running_app,
            "submit",
            serde_json::to_value(WsolSubmission::from(&input())).unwrap(),
        )
        .await
    });
    gate.wait_until_entered().await;
    let lookup = WsolRecoveryInput {
        idempotency_key: "key".into(),
        wallet_id: "w1".into(),
        owner_type: "telegram".into(),
        owner_id: "123".into(),
    };
    let original = submitted.store.get_by_key("key").await.unwrap().unwrap();
    let body = json_body(recovery_get(&app, &lookup).await, StatusCode::OK).await;
    assert_eq!(body["status"], "reserved");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let body = json_body(recovery_get(&app, &lookup).await, StatusCode::OK).await;
    assert_eq!(body["status"], "reserved");
    assert_eq!(body["signature"], Value::Null);
    assert_eq!(submitted.events(), ["quote", "reserve"]);
    assert_eq!(queried.events(), ["recovery", "recovery"]);
    assert_eq!(
        submitted
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(original)
    );
    let mut another = input();
    another.idempotency_key = "new-key".into();
    let quote = submitted.store.get_quote("q1").await.unwrap().unwrap();
    assert!(submitted
        .store
        .reserve_live(&WsolSubmission::from(&another), &quote, 2)
        .await
        .is_err());
}

async fn recovery_get(
    app: &Router,
    input: &crate::application::wsol_recovery::WsolRecoveryInput,
) -> Response {
    send(app, "GET", &recovery_route(input), LOCAL, Some(TOKEN), None).await
}

#[tokio::test]
async fn wsol_recovery_router_reads_all_states_while_live_disabled_without_rpc_or_broadcast() {
    use crate::application::wsol_queries::tests::{recovery_input, seed};
    for status in [
        "reserved",
        "signed",
        "submitted",
        "unknown",
        "confirmed",
        "failed",
    ] {
        let (app, shared) = query_fixture(1).await;
        let original = seed(&shared, status).await;
        for _ in 0..2 {
            let response = recovery_get(&app, &recovery_input()).await;
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
            assert!(response.headers().get("retry-after").is_none());
            let body = json_body(response, StatusCode::OK).await;
            assert_eq!(
                body,
                serde_json::to_value(
                    crate::api::dto::wsol_conversions::WsolSubmissionResult::from_record(
                        original.clone(),
                        false
                    )
                )
                .unwrap()
            );
            assert_eq!(body.as_object().unwrap().len(), 7);
            assert!(body.get("signed_transaction").is_none());
            assert!(body.get("idempotency_key").is_none());
        }
        assert_eq!(shared.events(), ["recovery", "recovery"]);
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
    }
}

#[tokio::test]
async fn wsol_recovery_router_hides_foreign_rows_and_validates_without_reinterpreting_keys() {
    use crate::application::{
        wsol_queries::tests::{recovery_input, seed},
        wsol_recovery::WsolRecoveryFailure,
    };
    let (app, shared) = query_fixture(0).await;
    seed(&shared, "signed").await;
    for field in 0..4 {
        let mut input = recovery_input();
        match field {
            0 => input.idempotency_key.push('x'),
            1 => input.wallet_id.push('x'),
            2 => input.owner_type.push('x'),
            _ => input.owner_id.push('x'),
        }
        let response = recovery_get(&app, &input).await;
        assert_eq!(
            response.headers()["x-wsol-error-code"],
            "wsol_recovery_not_found"
        );
        assert_eq!(response.headers()["x-wsol-error-stage"], "recovery_lookup");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-wsol-auto-retry"], "false");
        error_body(
            response,
            StatusCode::NOT_FOUND,
            &WsolRecoveryFailure::NotFound.to_string(),
        )
        .await;
    }
    for key in [String::new(), " ".into(), "x".repeat(129), "é".repeat(65)] {
        let mut input = recovery_input();
        input.idempotency_key = key;
        error_body(
            recovery_get(&app, &input).await,
            StatusCode::BAD_REQUEST,
            &WsolRecoveryFailure::Rejected.to_string(),
        )
        .await;
    }
    assert_eq!(
        shared.events(),
        ["recovery", "recovery", "recovery", "recovery"]
    );
}

#[tokio::test]
async fn wsol_recovery_router_auth_extractors_and_methods_precede_any_ledger_access() {
    use crate::application::wsol_queries::tests::recovery_input;
    let (app, shared) = query_fixture(0).await;
    let route = recovery_route(&recovery_input());
    assert_eq!(
        send(&app, "GET", &route, LOCAL, None, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "GET", &route, ALLOWED, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, "POST", &route, LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("{BASE}/submissions/by-key?owner_type=telegram"),
            LOCAL,
            Some(TOKEN),
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(shared.events().is_empty());
}

#[tokio::test]
async fn wsol_recovery_router_storage_and_integrity_faults_do_not_leak_ids_or_allow_resubmission() {
    use crate::application::{
        wsol_queries::tests::{recovery_input, seed},
        wsol_recovery::WsolRecoveryFailure,
    };
    for (mode, failure) in [
        (16, WsolRecoveryFailure::Unavailable),
        (0, WsolRecoveryFailure::Integrity),
    ] {
        let (app, shared) = query_fixture(mode).await;
        let original = seed(&shared, "signed").await;
        if mode == 0 {
            let mut corrupt = original.clone();
            corrupt.signed_transaction = None;
            shared.replace_recovery(corrupt);
        }
        let response = recovery_get(&app, &recovery_input()).await;
        assert_eq!(response.headers()["x-wsol-error-code"], failure.code());
        assert!(response.headers().get("x-wsol-submission-id").is_none());
        assert!(response.headers().get("x-wsol-signature").is_none());
        error_body(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            &failure.to_string(),
        )
        .await;
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
        assert_eq!(shared.events(), ["recovery"]);
    }
}

#[tokio::test]
async fn wsol_recovery_router_recovers_lost_reservation_ack_after_file_reopen_without_reposting() {
    use crate::application::{
        wsol_recovery::WsolRecoveryInput, wsol_submission::tests::harness_with_store,
    };
    let dir = std::env::temp_dir().join(format!("wsol-http-recovery-{}", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", dir.join("test.sqlite").display());
    let (submit, submitted) =
        harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
    submitted.set_write_failure(7);
    let mut initial_state = state().await;
    let mut config = (*initial_state.config).clone();
    config.conversion_live_enabled = true;
    initial_state.config = Arc::new(config);
    initial_state.wsol_submissions = submit;
    let app = http::router(initial_state);
    let response = post(
        &app,
        "submit",
        serde_json::to_value(WsolSubmission::from(
            &crate::application::wsol_submission::tests::input(),
        ))
        .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get("x-wsol-submission-id").is_none());
    assert!(response.headers().get("x-wsol-signature").is_none());
    drop(response);
    assert_eq!(submitted.events(), ["quote", "reserve"]);
    let original = submitted.store.get_by_key("key").await.unwrap().unwrap();
    drop(app);
    drop(submitted);
    let (queries, shared) = crate::application::wsol_queries::tests::harness_with_store(Arc::new(
        ConversionStore::new(&url).await.unwrap(),
    ))
    .await;
    let mut state = state().await;
    state.wsol_queries = queries;
    let app = http::router(state);
    let input = WsolRecoveryInput {
        idempotency_key: "key".into(),
        wallet_id: "w1".into(),
        owner_type: "telegram".into(),
        owner_id: "123".into(),
    };
    for _ in 0..3 {
        let body = json_body(recovery_get(&app, &input).await, StatusCode::OK).await;
        assert_eq!(body["submission_id"], original.submission_id);
        assert_eq!(body["status"], "reserved");
        assert_eq!(body["signature"], Value::Null);
    }
    assert_eq!(shared.events(), ["recovery", "recovery", "recovery"]);
    assert_eq!(
        shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(original)
    );
    let mut another = crate::application::wsol_submission::tests::input();
    another.idempotency_key = "another-key".into();
    let quote = shared.store.get_quote("q1").await.unwrap().unwrap();
    assert!(shared
        .store
        .reserve_live(&WsolSubmission::from(&another), &quote, 2)
        .await
        .is_err());
    drop(app);
    drop(shared);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[tokio::test]
async fn wsol_status_router_keeps_non_final_errors_pending_and_requires_finalized_to_release_wallet(
) {
    use crate::{
        application::wsol_queries::tests::signature_observation,
        domain::reconciliation::ChainCommitment,
    };
    for commitment in [ChainCommitment::Processed, ChainCommitment::Confirmed] {
        let (app, shared) = query_fixture(0).await;
        let original = crate::application::wsol_queries::tests::seed(&shared, "signed").await;
        shared.set_observation(signature_observation(
            commitment,
            Some("provisional chain error"),
        ));
        let route = format!(
            "{BASE}/submissions/{}?owner_type=telegram&owner_id=42",
            original.submission_id
        );
        let body = json_body(
            send(&app, "GET", &route, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(
            body,
            json!({
                "submission_id":original.submission_id,"quote_id":original.quote_id,"wallet_id":original.wallet_id,
                "status":"submitted","signature":"synthetic-signature","error":format!("non-final {} observation: provisional chain error", commitment.as_str()),"duplicate":false,
            })
        );
        let mut another = crate::conversion::contract_tests::submission();
        another.idempotency_key = "another-key".into();
        assert!(shared
            .store
            .reserve_live(&another, &quote(), 4)
            .await
            .is_err());
        shared.set_observation(signature_observation(ChainCommitment::Finalized, None));
        let final_body = json_body(
            send(&app, "GET", &route, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(final_body["status"], "confirmed");
        assert_eq!(final_body["error"], Value::Null);
        assert_eq!(final_body["signature"], body["signature"]);
        let stored = shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.signed_transaction, original.signed_transaction);
        assert!(
            shared
                .store
                .reserve_live(&another, &quote(), 4)
                .await
                .unwrap()
                .1
        );
        let trace = shared.events();
        assert_eq!(
            json_body(
                send(&app, "GET", &route, LOCAL, Some(TOKEN), None).await,
                StatusCode::OK
            )
            .await,
            final_body
        );
        assert_eq!(
            &shared.events()[trace.len()..],
            ["submission", "submission"]
        );
    }
}

#[tokio::test]
async fn wsol_status_router_ignores_unbound_or_invalid_slot_evidence_without_changing_receipt() {
    use crate::{
        application::wsol_queries::tests::signature_observation,
        domain::reconciliation::ChainCommitment,
    };
    for variant in 0..4 {
        let (app, shared) = query_fixture(0).await;
        let original = crate::application::wsol_queries::tests::seed(&shared, "signed").await;
        let mut wrong =
            signature_observation(ChainCommitment::Finalized, Some("untrusted failure"));
        match variant {
            0 => wrong.chain.signature = "other-signature".into(),
            1 => wrong.chain.slot = 0,
            2 => wrong.context_slot = 0,
            _ => wrong.chain.slot = wrong.context_slot + 1,
        }
        shared.set_observation(wrong);
        let route = format!(
            "{BASE}/submissions/{}?owner_type=telegram&owner_id=42",
            original.submission_id
        );
        let body = json_body(
            send(&app, "GET", &route, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(body.as_object().unwrap().len(), 7);
        assert_eq!(body["status"], "signed");
        assert_eq!(body["signature"], "synthetic-signature");
        assert_eq!(body["error"], Value::Null);
        assert_eq!(
            shared
                .store
                .get_by_id(&original.submission_id)
                .await
                .unwrap(),
            Some(original)
        );
        assert_eq!(
            shared.events(),
            ["submission", "status_reader", "lookup", "submission"]
        );
    }
}

#[tokio::test]
async fn wsol_status_router_bounds_rpc_lookup_and_preserves_original_signed_receipt() {
    let (app, shared) = query_fixture(15).await;
    let original = crate::application::wsol_queries::tests::seed(&shared, "signed").await;
    let route = format!(
        "{BASE}/submissions/{}?owner_type=telegram&owner_id=42",
        original.submission_id
    );
    let start = std::time::Instant::now();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send(&app, "GET", &route, LOCAL, Some(TOKEN), None),
    )
    .await
    .unwrap();
    let body = json_body(response, StatusCode::OK).await;
    assert!(start.elapsed() >= std::time::Duration::from_secs(3));
    assert_eq!(body["status"], "signed");
    assert_eq!(body["signature"], "synthetic-signature");
    assert_eq!(body["error"], Value::Null);
    assert_eq!(shared.lookup_drops(), 1);
    assert_eq!(
        shared
            .store
            .get_by_id(&original.submission_id)
            .await
            .unwrap(),
        Some(original)
    );
    assert_eq!(
        shared.events(),
        ["submission", "status_reader", "lookup", "submission"]
    );
}

#[tokio::test]
async fn wsol_status_router_validates_actual_rpc_shape_and_explicit_finality_without_wallet_keys() {
    use crate::application::{
        ports::wsol_conversion::WsolWalletMetadata, ports::wsol_queries::WsolQuerySource,
        wsol_queries::WsolQueryService,
    };
    use solana_client::{nonblocking::rpc_client::RpcClient, rpc_request::RpcRequest};
    struct Source(Arc<RpcClient>);
    impl WsolQuerySource for Source {
        type QuoteReader = crate::application::wsol_queries::tests::QuoteReader;
        type StatusReader = crate::adapters::rpc::wsol_status::WsolSignatureReader;
        async fn metadata(&self, _: &str) -> anyhow::Result<WsolWalletMetadata> {
            anyhow::bail!("status must not read wallet metadata");
        }
        fn quote_reader(&self, _: &str) -> Option<Self::QuoteReader> {
            None
        }
        fn status_reader(
            &self,
            wallet: &str,
            signature: &str,
        ) -> anyhow::Result<Self::StatusReader> {
            assert_eq!(wallet, "fixture-wallet");
            Ok(crate::adapters::rpc::wsol_status::WsolSignatureReader::new(
                self.0.clone(),
                signature.parse()?,
            ))
        }
    }
    for (commitment, confirmations, slot, failure, inconsistent, expected) in [
        ("processed", json!(null), 42, true, false, "signed"),
        ("finalized", json!(1), 42, false, false, "signed"),
        ("finalized", json!(null), 43, false, false, "signed"),
        ("finalized", json!(null), 42, true, true, "signed"),
        ("processed", json!(0), 42, true, false, "submitted"),
        ("confirmed", json!(1), 42, true, false, "submitted"),
        ("finalized", json!(null), 42, false, false, "confirmed"),
        ("finalized", json!(null), 42, true, false, "failed"),
    ] {
        let (mut state, store) = fixture().await;
        let mut config = (*state.config).clone();
        config.conversion_live_enabled = false;
        config.security.broadcast = crate::domain::execution_policy::BroadcastPolicy::Disabled;
        state.config = Arc::new(config);
        let request: WsolSubmission = serde_json::from_value(submission(json!({}))).unwrap();
        let (original, _) = store.reserve_live(&request, &quote(), 1).await.unwrap();
        let signature = solana_sdk::signature::Signature::from([7; 64]).to_string();
        store
            .record_signed(&original.submission_id, &signature, &[1, 2, 3], 2)
            .await
            .unwrap();
        let error = if failure {
            json!({"InstructionError":[0,{"Custom":42}]})
        } else {
            Value::Null
        };
        let legacy = if failure && !inconsistent {
            json!({"Err":error})
        } else {
            json!({"Ok":null})
        };
        let mut mocks = std::collections::HashMap::new();
        mocks.insert(RpcRequest::GetSignatureStatuses, json!({"context":{"slot":42},"value":[{
            "slot":slot,"confirmationStatus":commitment,"confirmations":confirmations,"err":error,"status":legacy,
        }]}));
        let rpc = Arc::new(RpcClient::new_mock_with_mocks("fails".into(), mocks));
        state.wsol_queries = Arc::new(WsolQueryService::new(
            Source(rpc),
            Some(
                crate::adapters::persistence::wsol_conversion::SqliteWsolJournal::new(
                    store.clone(),
                ),
            ),
            crate::adapters::sdk::address_parser::SdkAddressParser,
        ));
        let app = http::router(state);
        let route = format!(
            "{BASE}/submissions/{}?owner_type=telegram&owner_id=42",
            original.submission_id
        );
        let body = json_body(
            send(&app, "GET", &route, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(body.as_object().unwrap().len(), 7);
        assert_eq!(body["status"], expected, "{commitment}, {confirmations}");
        assert_eq!(body["signature"], signature);
        let stored = store
            .get_by_id(&original.submission_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        assert_eq!(body["error"], serde_json::to_value(&stored.error).unwrap());
        if expected == "signed" {
            assert!(stored.error.is_none());
        }
        if expected == "submitted" {
            assert!(stored.error.as_ref().unwrap().starts_with("non-final "));
        }
    }
}

#[tokio::test]
async fn wsol_quote_router_keeps_exact_wire_and_shared_store_with_broadcast_disabled() {
    let (app, shared) = query_fixture(0).await;
    let input = json!({"wallet_id":"fixture-wallet","owner_type":"telegram","owner_id":"42","direction":"wrap","amount_lamports":1_000_000});
    assert_eq!(
        send(
            &app,
            "POST",
            &format!("{BASE}/quotes"),
            LOCAL,
            None,
            Some(input.clone())
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(shared.events().is_empty());
    let body = json_body(post(&app, "quotes", input).await, StatusCode::OK).await;
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert_eq!(body["executable"], false);
    let quote = &body["quote"];
    assert_eq!(
        *quote,
        json!({
            "quote_id":quote["quote_id"],"expires_at_ms":quote["expires_at_ms"],
            "wallet_id":"fixture-wallet","wallet_pubkey":crate::domain::trade::Address::from_bytes([1;32]).to_string(),
            "owner_type":"telegram","owner_id":"42","direction":"wrap","amount_lamports":1_000_000,
            "source_balance_lamports":2_000_000,"ata_exists":true,"estimated_network_fee_lamports":10_000,
            "temporary_rent_lamports":0,"rent_refund_lamports":0,
        })
    );
    assert_eq!(
        serde_json::to_value(
            shared
                .store
                .get_quote(quote["quote_id"].as_str().unwrap())
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        *quote
    );
    assert!(shared
        .store
        .get_by_key("fixture-key")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        shared.events(),
        [
            "metadata",
            "parse_owner",
            "quote_reader",
            "estimate:1",
            "save"
        ]
    );
}

#[tokio::test]
async fn wsol_status_router_keeps_receipt_owner_gate_and_original_signature_without_submission() {
    for (mode, status, error) in [
        (0, "submitted", None),
        (8, "signed", None),
        (10, "confirmed", None),
        (11, "failed", Some("synthetic chain failure")),
    ] {
        let (app, shared) = query_fixture(mode).await;
        let record = crate::application::wsol_queries::tests::seed(&shared, "signed").await;
        let route = format!("{BASE}/submissions/{}", record.submission_id);
        error_body(
            send(
                &app,
                "GET",
                &format!("{route}?owner_type=telegram&owner_id=foreign"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::BAD_REQUEST,
            "conversion submission does not belong to this owner",
        )
        .await;
        assert_eq!(shared.events(), ["submission"]);
        let body = json_body(
            send(
                &app,
                "GET",
                &format!("{route}?owner_type=telegram&owner_id=42"),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(
            body,
            json!({"submission_id":record.submission_id,"quote_id":record.quote_id,"wallet_id":record.wallet_id,
            "status":status,"signature":"synthetic-signature","error":error,"duplicate":false})
        );
        let stored = shared
            .store
            .get_by_key("fixture-key")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.signed_transaction.as_deref(), Some(&[1, 2, 3][..]));
        assert_eq!(stored.status, status);
    }
}

#[tokio::test]
async fn wsol_status_router_concurrent_weak_query_cannot_overwrite_terminal_result() {
    for mode in [10, 11] {
        let (app, shared) = query_fixture(0).await;
        let signed = crate::application::wsol_queries::tests::seed(&shared, "signed").await;
        let route = format!(
            "{BASE}/submissions/{}?owner_type=telegram&owner_id=42",
            signed.submission_id
        );
        let gate = shared.pause_lookup();
        let task = tokio::spawn({
            let app = app.clone();
            let route = route.clone();
            async move {
                json_body(
                    send(&app, "GET", &route, LOCAL, Some(TOKEN), None).await,
                    StatusCode::OK,
                )
                .await
            }
        });
        gate.wait_until_entered().await;
        shared.mode(mode);
        let authoritative = json_body(
            send(&app, "GET", &route, LOCAL, Some(TOKEN), None).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(
            authoritative["status"],
            if mode == 10 { "confirmed" } else { "failed" }
        );
        gate.release();
        assert_eq!(task.await.unwrap(), authoritative);
        let current = shared
            .store
            .get_by_id(&signed.submission_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.signature, signed.signature);
        assert_eq!(current.signed_transaction, signed.signed_transaction);
        assert!(shared.events().iter().all(|e| matches!(
            e.as_str(),
            "submission"
                | "status_reader"
                | "lookup"
                | "record:submitted"
                | "record:confirmed"
                | "record:failed"
        )));
    }
}

#[tokio::test]
async fn wsol_query_router_classifies_quote_and_status_storage_failures() {
    let (app, shared) = query_fixture(12).await;
    submit_fault(post(&app,"quotes",json!({"wallet_id":"fixture-wallet","owner_type":"telegram","owner_id":"42","direction":"unwrap","all":true})).await,
        "wsol_outcome_unknown", "quote_save", "synthetic quote write failure").await;
    assert!(shared
        .store
        .get_by_key("fixture-key")
        .await
        .unwrap()
        .is_none());
    let (app, shared) = query_fixture(13).await;
    let record = crate::application::wsol_queries::tests::seed(&shared, "signed").await;
    submit_fault(
        send(
            &app,
            "GET",
            &format!(
                "{BASE}/submissions/{}?owner_type=telegram&owner_id=42",
                record.submission_id
            ),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        "wsol_outcome_unknown",
        "status_observation",
        "synthetic observation write failure",
    )
    .await;
    assert_eq!(
        shared.store.get_by_id(&record.submission_id).await.unwrap(),
        Some(record)
    );
}

#[tokio::test]
async fn standalone_wsol_route_group_matches_integrated_methods_auth_and_disabled_policy() {
    let state = state().await;
    let group = crate::api::routes::wsol_conversion_routes().with_state(state.clone());
    let integrated = http::router(state.clone());
    let quote = json!({"wallet_id":"missing","owner_type":"telegram","owner_id":"42","direction":"wrap","amount_lamports":1});
    let submit = submission(json!({"wallet_id":"missing"}));
    let mut simulate = submit.clone();
    simulate["simulate"] = json!(true);
    let cases = [
        (
            "POST",
            "GET",
            format!("{BASE}/quotes"),
            Some(quote),
            "conversion quotes are disabled",
            Some("quote_store"),
        ),
        (
            "POST",
            "GET",
            format!("{BASE}/simulate"),
            Some(simulate),
            "conversion quotes are disabled",
            Some("unsigned_admission"),
        ),
        (
            "POST",
            "GET",
            format!("{BASE}/submit"),
            Some(submit),
            "live WSOL conversion is disabled or request is simulation",
            None,
        ),
        (
            "GET",
            "POST",
            format!("{BASE}/submissions/missing?owner_type=telegram&owner_id=42"),
            None,
            "conversion quotes are disabled",
            Some("status_read"),
        ),
    ];
    for app in [&group, &integrated] {
        for (method, wrong_method, path, body, message, fault_stage) in &cases {
            assert_eq!(
                send(app, method, path, LOCAL, None, body.clone())
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                send(app, method, path, ALLOWED, Some(TOKEN), body.clone())
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
            let response = send(app, method, path, LOCAL, Some(TOKEN), body.clone()).await;
            if let Some(stage) = fault_stage {
                submit_fault(response, "wsol_unavailable", stage, message).await;
            } else {
                error_body(response, StatusCode::BAD_REQUEST, message).await;
            }
            assert_eq!(
                send(app, wrong_method, path, LOCAL, Some(TOKEN), body.clone())
                    .await
                    .status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
            if *method == "POST" {
                assert_eq!(
                    send(app, method, path, LOCAL, None, Some(json!({})))
                        .await
                        .status(),
                    StatusCode::UNPROCESSABLE_ENTITY
                );
            }
        }
        assert_eq!(
            send(
                app,
                "GET",
                &format!("{BASE}/submissions/missing"),
                LOCAL,
                Some(TOKEN),
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(state.wallets.wallet_count(), 0);
    assert!(state.trades.recent(10).is_empty());
}
