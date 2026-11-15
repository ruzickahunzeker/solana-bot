//! Wire/domain/snapshot compatibility; only temporary journals, no environment, RPC or wallets.

use crate::aggregator_quote::{SolBuyQuoteRequest, SolBuySubmitRequest};
use crate::conversion::{WsolDirection, WsolQuoteRequest, WsolSubmission};
use crate::models::*;
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/contracts/trade_v1.json")).unwrap()
}

fn sol_buy_fixture() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/contracts/sol_buy_v1.json")).unwrap()
}

fn legacy_bound_quote() -> crate::aggregator_quote::BoundSolBuyQuote {
    use crate::aggregator_quote::{BoundSolBuyQuote, QuoteLeg, SolBuyQuoteResponse};
    BoundSolBuyQuote {
        quote_id: "fixture-quote-v1".into(),
        wallet_id: "fixture-wallet".into(),
        wallet_pubkey: "fixture-public-key".into(),
        quote_slippage_bps: 1500,
        expires_at_ms: u64::MAX,
        quote: SolBuyQuoteResponse {
            input_mint: "fixture-wsol".into(),
            output_mint: "fixture-target".into(),
            amount_lamports: 9_007_199_254_740_993,
            expected_output_units: u64::MAX,
            minimum_output_units: 15_679_732_462_653_118_872,
            price_impact_percent: "0.01234567890123456789".into(),
            route: vec![
                QuoteLeg {
                    label: "first-hop".into(),
                    input_mint: "fixture-wsol".into(),
                    output_mint: "fixture-quote".into(),
                },
                QuoteLeg {
                    label: "second-hop".into(),
                    input_mint: "fixture-quote".into(),
                    output_mint: "fixture-target".into(),
                },
            ],
            executable: false,
        },
    }
}

#[test]
fn sol_buy_v1_bound_snapshot_and_original_fingerprint() {
    use crate::domain::submission::{SubmissionIntent, SubmissionKey};
    let fixture = sol_buy_fixture();
    let bound = legacy_bound_quote();
    let quote = serde_json::to_value(&bound).unwrap();
    assert_eq!(quote, fixture["bound"]);
    let intent = SubmissionIntent {
        key: SubmissionKey::new(&bound.wallet_id, "fixture-quote-v1").unwrap(),
        wallet_pubkey: bound.wallet_pubkey,
        quote_id: bound.quote_id,
        quote,
    };
    assert_eq!(
        intent.fingerprint().unwrap(),
        fixture["original_fingerprint"].as_str().unwrap()
    );
}

#[test]
fn sol_buy_v1_preflight_snapshot_preserves_every_field() {
    let fixture = sol_buy_fixture();
    let report: crate::aggregator_quote::SolBuyPreflightResponse =
        serde_json::from_value(fixture["preflight"].clone()).unwrap();
    assert_eq!(serde_json::to_value(report).unwrap(), fixture["preflight"]);
}

#[test]
fn sol_buy_v1_submit_response_preserves_envelope_and_null_signature() {
    let fixture = sol_buy_fixture();
    let response = crate::aggregator_quote::SolBuySubmitResponse {
        quote_id: "fixture-quote-v1".into(),
        wallet_id: "fixture-wallet".into(),
        status: "unknown".into(),
        signature: None,
        error: Some("synthetic timeout".into()),
        preflight: serde_json::from_value(fixture["preflight"].clone()).unwrap(),
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "quote_id":"fixture-quote-v1","wallet_id":"fixture-wallet","status":"unknown",
            "signature":null,"error":"synthetic timeout","preflight":fixture["preflight"],
        })
    );
}

#[test]
fn internal_bound_quote_preserves_wire_shape_raw_units_and_original_identity() {
    use crate::api::dto::sol_buy::BoundSolBuyQuote;
    use crate::domain::{
        quote::BoundQuote,
        submission::{SubmissionIntent, SubmissionKey},
    };
    let fixture = sol_buy_fixture();
    let internal = BoundQuote::from(legacy_bound_quote());
    assert_eq!(internal.quote.amount_lamports, 9_007_199_254_740_993);
    assert_eq!(internal.quote.expected_output_units, u64::MAX);
    assert_eq!(
        internal.quote.price_impact_percent,
        "0.01234567890123456789"
    );
    assert_eq!(serde_json::to_value(&internal).unwrap(), fixture["bound"]);
    let intent = SubmissionIntent {
        key: SubmissionKey::new(&internal.wallet_id, "fixture-quote-v1").unwrap(),
        wallet_pubkey: internal.wallet_pubkey.clone(),
        quote_id: internal.quote_id.clone(),
        quote: serde_json::to_value(&internal).unwrap(),
    };
    assert_eq!(
        intent.fingerprint().unwrap(),
        fixture["original_fingerprint"].as_str().unwrap()
    );
    assert_eq!(
        serde_json::to_value(BoundSolBuyQuote::from(internal)).unwrap(),
        fixture["bound"]
    );
}

#[test]
fn internal_preflight_maps_all_snapshot_fields_without_swapping_evidence() {
    use crate::{api::dto::sol_buy::SolBuyPreflightResponse, domain::quote::QuotePreflight};
    let baseline = sol_buy_fixture()["preflight"].clone();
    let mut variants = vec![baseline.clone()];
    for (field, value) in baseline.as_object().unwrap() {
        if value.is_boolean() {
            let mut changed = baseline.clone();
            changed[field] = json!(!value.as_bool().unwrap());
            variants.push(changed);
        } else if value.is_number() {
            let mut changed = baseline.clone();
            changed[field] = json!(424_242);
            variants.push(changed);
        }
    }
    for value in variants {
        let wire: SolBuyPreflightResponse = serde_json::from_value(value.clone()).unwrap();
        let internal = QuotePreflight::from(wire);
        assert_eq!(serde_json::to_value(&internal).unwrap(), value);
        assert_eq!(
            serde_json::to_value(SolBuyPreflightResponse::from(internal)).unwrap(),
            value
        );
    }
}

#[test]
fn internal_preflight_preserves_nulls_and_legacy_absent_optional_evidence() {
    use crate::{api::dto::sol_buy::SolBuyPreflightResponse, domain::quote::QuotePreflight};
    let mut value = sol_buy_fixture()["preflight"].clone();
    let optional = [
        "simulated_fee_lamports",
        "simulated_units_consumed",
        "simulated_payer_debit_lamports",
        "simulated_inner_instruction_groups",
        "simulated_target_token_delta_units",
        "simulated_target_token_post_units",
        "observed_same_slot_payer_debit_lamports",
        "observed_same_slot_target_token_delta_units",
        "simulated_inner_instruction_count",
        "simulated_inner_programs",
        "simulated_inner_payer_system_spend_lamports",
    ];
    for field in optional {
        value[field] = Value::Null;
    }
    let internal = QuotePreflight::from(
        serde_json::from_value::<SolBuyPreflightResponse>(value.clone()).unwrap(),
    );
    assert_eq!(serde_json::to_value(&internal).unwrap(), value);
    let mut old_sparse = value.clone();
    for field in optional {
        old_sparse.as_object_mut().unwrap().remove(field);
    }
    let decoded: QuotePreflight = serde_json::from_value(old_sparse).unwrap();
    assert_eq!(
        serde_json::to_value(SolBuyPreflightResponse::from(decoded)).unwrap(),
        value
    );
}

#[test]
fn internal_command_keeps_explicit_modes_and_literal_idempotency_identity() {
    use crate::{
        api::dto::sol_buy::SolBuySubmitRequest,
        application::quote_submission_types::QuoteSubmissionCommand,
    };
    for confirmed in [false, true] {
        for simulate in [false, true] {
            let wire: SolBuySubmitRequest = serde_json::from_value(json!({"wallet_id":"fixture-wallet","idempotency_key":" key-with-spaces ","confirmed":confirmed,"simulate":simulate})).unwrap();
            let command = QuoteSubmissionCommand::from(wire);
            assert_eq!(command.wallet_id, "fixture-wallet");
            assert_eq!(command.idempotency_key, " key-with-spaces ");
            assert_eq!((command.confirmed, command.simulate), (confirmed, simulate));
        }
    }
}

#[test]
fn internal_outcomes_map_to_all_existing_public_status_names() {
    use crate::{
        api::dto::sol_buy::SolBuySubmitResponse,
        application::quote_submission_types::{
            QuoteSubmissionOutcome as Outcome, QuoteSubmissionResult,
        },
    };
    let fixture = sol_buy_fixture();
    for (outcome, status) in [
        (Outcome::Pending, "submitting"),
        (Outcome::Simulated, "simulated"),
        (Outcome::Submitted, "submitted"),
        (Outcome::Confirmed, "confirmed"),
        (Outcome::Finalized, "finalized"),
        (Outcome::Unknown, "unknown"),
        (Outcome::Failed, "failed"),
    ] {
        let result = QuoteSubmissionResult {
            quote_id: "fixture-quote-v1".into(),
            wallet_id: "fixture-wallet".into(),
            status: outcome,
            signature: None,
            error: None,
            preflight: serde_json::from_value(fixture["preflight"].clone()).unwrap(),
        };
        assert_eq!(
            serde_json::to_value(SolBuySubmitResponse::from(result)).unwrap(),
            json!({
                "quote_id":"fixture-quote-v1","wallet_id":"fixture-wallet","status":status,
                "signature":null,"error":null,"preflight":fixture["preflight"],
            })
        );
    }
}

struct ReplayOnlyBackend;
impl crate::application::ports::quote_execution::QuoteSubmissionBackend for ReplayOnlyBackend {
    type Prepared = ();
    fn ensure_live_allowed(&self) -> anyhow::Result<()> {
        panic!("stored orders must not re-enter execution")
    }
    async fn prepare(
        &self,
        _: &crate::domain::quote::BoundQuote,
    ) -> anyhow::Result<((), crate::domain::quote::QuotePreflight)> {
        panic!("stored orders must not re-quote")
    }
    fn sign(&self, _: ()) -> anyhow::Result<crate::domain::submission::SignedSubmission> {
        panic!("stored orders must not re-sign")
    }
    async fn broadcast(
        &self,
        _: &crate::domain::submission::SignedSubmission,
    ) -> anyhow::Result<String> {
        panic!("stored orders must not re-broadcast")
    }
}

fn legacy_quote_intent() -> crate::domain::submission::SubmissionIntent {
    let fixture = sol_buy_fixture();
    crate::domain::submission::SubmissionIntent {
        key: crate::domain::submission::SubmissionKey::new("fixture-wallet", "fixture-quote-v1")
            .unwrap(),
        wallet_pubkey: "fixture-public-key".into(),
        quote_id: "fixture-quote-v1".into(),
        quote: fixture["bound"].clone(),
    }
}

#[tokio::test]
async fn internal_types_resume_legacy_file_journal_without_changing_fingerprint_or_execution() {
    use crate::{
        api::dto::sol_buy::{SolBuySubmitRequest, SolBuySubmitResponse},
        application::{
            ports::submission_journal::SubmissionJournal, quote_submission,
            quote_submission_types::QuoteSubmissionCommand,
        },
        config::{TradePersistenceConfig, TradePersistenceMode},
        domain::{
            quote::BoundQuote,
            submission::{Reservation, SignedSubmission, SubmissionStatus},
        },
        persistence::TradePersistence,
    };
    let fixture = sol_buy_fixture();
    let intent = legacy_quote_intent();
    assert_eq!(
        intent.fingerprint().unwrap(),
        fixture["original_fingerprint"].as_str().unwrap()
    );
    let dir = std::env::temp_dir().join(format!("quote-dto-contract-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("trades.sqlite").display()),
        busy_timeout_ms: 1000,
    };
    let persistence = TradePersistence::new(&config).await.unwrap();
    let journal = persistence.submission_journal().unwrap();
    let Reservation::Created(record) = journal
        .reserve(&intent, &fixture["preflight"])
        .await
        .unwrap()
    else {
        panic!("expected new legacy fixture")
    };
    journal
        .record_signed(
            &record.submission_id,
            &SignedSubmission {
                signature: "fixture-original-signature".into(),
                bytes: vec![9, 8, 7],
            },
        )
        .await
        .unwrap();
    journal
        .record_broadcast(
            &record.submission_id,
            SubmissionStatus::Unknown,
            Some("synthetic timeout"),
        )
        .await
        .unwrap();
    drop(journal);
    drop(persistence);
    let persistence = TradePersistence::new(&config).await.unwrap();
    let journal = persistence.submission_journal().unwrap();
    let bound = BoundQuote::from(legacy_bound_quote());
    let command = QuoteSubmissionCommand::from(
        serde_json::from_value::<SolBuySubmitRequest>(fixture["submit"].clone()).unwrap(),
    );
    let result = quote_submission::execute(Some(&journal), &ReplayOnlyBackend, &bound, &command)
        .await
        .unwrap();
    let response = serde_json::to_value(SolBuySubmitResponse::from(result)).unwrap();
    assert_eq!(response["signature"], "fixture-original-signature");
    assert_eq!(response["status"], "unknown");
    assert_eq!(response["error"], "synthetic timeout");
    assert_eq!(response["preflight"], fixture["preflight"]);
    let mut internal_intent = legacy_quote_intent();
    internal_intent.quote = serde_json::to_value(bound).unwrap();
    assert_eq!(
        internal_intent.fingerprint().unwrap(),
        intent.fingerprint().unwrap()
    );
    assert!(
        matches!(journal.reserve(&internal_intent, &fixture["preflight"]).await.unwrap(), Reservation::Existing(existing) if existing.submission_id == record.submission_id)
    );
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&config.database_url)
        .await
        .unwrap();
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT signed_transaction FROM quote_submissions WHERE submission_id=?",
    )
    .bind(&record.submission_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bytes, vec![9, 8, 7]);
    pool.close().await;
    drop(journal);
    drop(persistence);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn malformed_legacy_preflight_does_not_fall_back_to_signing_a_new_transaction() {
    use crate::{
        adapters::persistence::submission_journal::SqliteSubmissionJournal,
        api::dto::sol_buy::SolBuySubmitRequest,
        application::{
            ports::submission_journal::SubmissionJournal,
            quote_submission::{self, FailureKind},
            quote_submission_types::QuoteSubmissionCommand,
        },
        domain::quote::BoundQuote,
    };
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let journal = SqliteSubmissionJournal::new(pool);
    journal.initialize().await.unwrap();
    let mut malformed = sol_buy_fixture()["preflight"].clone();
    malformed.as_object_mut().unwrap().remove("executable");
    journal
        .reserve(&legacy_quote_intent(), &malformed)
        .await
        .unwrap();
    let command = QuoteSubmissionCommand::from(
        serde_json::from_value::<SolBuySubmitRequest>(sol_buy_fixture()["submit"].clone()).unwrap(),
    );
    let error = quote_submission::execute(
        Some(&journal),
        &ReplayOnlyBackend,
        &BoundQuote::from(legacy_bound_quote()),
        &command,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, FailureKind::Unavailable);
}

#[test]
fn minimal_trade_preserves_optional_fields_and_null_serialization() {
    let fixture = fixture();
    let request: TradeRequest = serde_json::from_value(fixture["minimal_buy"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        fixture["serialized_minimal_buy"]
    );
}

#[test]
fn trade_requires_identity_and_rejects_unknown_enum_values() {
    for field in ["mint", "side", "pool", "idempotency_key"] {
        let mut request = fixture()["minimal_buy"].clone();
        request.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<TradeRequest>(request).is_err(),
            "{field}"
        );
    }
    let mut request = fixture()["minimal_buy"].clone();
    request["pool"] = json!("not-a-protocol");
    assert!(serde_json::from_value::<TradeRequest>(request).is_err());
}

#[test]
fn aliases_keep_canonical_wire_names() {
    for (alias, canonical) in [
        ("stonk-fun", "stonkfun"),
        ("stonk-fun-swap", "stonkfun-swap"),
    ] {
        let pool: Pool = serde_json::from_value(json!(alias)).unwrap();
        assert_eq!(serde_json::to_value(pool).unwrap(), json!(canonical));
    }
    let method: ExecutionMethod = serde_json::from_value(json!("pump-api")).unwrap();
    assert_eq!(serde_json::to_value(method).unwrap(), json!("pumpapi"));
    for (alias, canonical) in [
        ("rpc", "default"),
        ("flash-block", "flashblock"),
        ("block-razor", "blockrazor"),
        ("speed-landing", "speedlanding"),
    ] {
        let provider: MevProvider = serde_json::from_value(json!(alias)).unwrap();
        assert_eq!(serde_json::to_value(provider).unwrap(), json!(canonical));
    }
}

#[test]
fn raw_quote_units_preserve_u64_precision() {
    let mut value = fixture()["minimal_buy"].clone();
    value["amount_quote_units"] = json!(u64::MAX);
    let request: TradeRequest = serde_json::from_value(value).unwrap();
    assert_eq!(request.amount_quote_units, Some(u64::MAX));
    assert_eq!(
        serde_json::to_value(request).unwrap()["amount_quote_units"].as_u64(),
        Some(u64::MAX)
    );
    for invalid in [json!(-1), json!(0.5), json!("1000")] {
        let mut value = fixture()["minimal_buy"].clone();
        value["amount_quote_units"] = invalid;
        assert!(serde_json::from_value::<TradeRequest>(value).is_err());
    }
}

#[test]
fn trade_response_does_not_leak_record_only_fields() {
    let record = TradeRecord {
        request_id: "tr_fixture".to_string(),
        wallet_id: "fixture-wallet".to_string(),
        wallet_pubkey: "11111111111111111111111111111111".to_string(),
        side: Side::Buy,
        pool: Pool::Pump,
        mint: "fixture-mint".to_string(),
        pool_id: None,
        execution_method: ExecutionMethod::Sdk,
        status: TradeStatus::Accepted,
        signature: None,
        error: None,
        duplicate: false,
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    assert_eq!(
        serde_json::to_value(TradeResponse::from(record)).unwrap(),
        fixture()["accepted_response"]
    );
}

#[test]
fn batch_keeps_nested_trade_and_per_wallet_results() {
    let batch: BatchTradeRequest = serde_json::from_value(json!({
        "wallet_ids": ["a", "b"], "trade": fixture()["minimal_buy"].clone()
    }))
    .unwrap();
    assert_eq!(batch.wallet_ids, ["a", "b"]);
    assert_eq!(batch.trade.idempotency_key, "fixture-buy");
    let result = BatchTradeResponse {
        items: vec![BatchTradeItemResponse {
            wallet_id: "b".to_string(),
            request_id: None,
            status: None,
            error: Some("wallet_id not found".to_string()),
            duplicate: false,
        }],
    };
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        json!({"items": [{
            "wallet_id": "b", "request_id": null, "status": null,
            "error": "wallet_id not found", "duplicate": false
        }]})
    );
}

#[test]
fn wallet_creation_and_listing_preserve_identity_contract() {
    let request: CreateWalletRequest = serde_json::from_value(json!({
        "owner_type": "fixture", "owner_id": "owner", "label": "Example"
    }))
    .unwrap();
    assert!(request.private_key_base58.is_none());
    let response = WalletsResponse {
        default_wallet_id: "fixture-wallet".to_string(),
        wallets: vec![WalletInfo {
            wallet_id: "fixture-wallet".to_string(),
            pubkey: "public-key".to_string(),
        }],
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "default_wallet_id": "fixture-wallet",
            "wallets": [{"wallet_id": "fixture-wallet", "pubkey": "public-key"}]
        })
    );
}

#[test]
fn pool_resolution_preserves_quote_and_absent_hop() {
    let route = ResolvePoolResponse {
        mint: "base".to_string(),
        pool: Pool::RaydiumCpmm,
        pool_id: Some("pool".to_string()),
        quote_mint: "custom-quote".to_string(),
    };
    assert_eq!(
        serde_json::to_value(route).unwrap(),
        json!({
            "mint": "base", "pool": "raydium-cpmm", "pool_id": "pool", "quote_mint": "custom-quote"
        })
    );
    let hop = ResolveSolHopResponse {
        available: false,
        pool: None,
        pool_id: None,
        reason: Some("timeout".to_string()),
    };
    assert_eq!(
        serde_json::to_value(hop).unwrap(),
        json!({
            "available": false, "pool": null, "pool_id": null, "reason": "timeout"
        })
    );
}

#[test]
fn aggregator_submit_requires_explicit_confirmation_and_simulation_fields() {
    let quote: SolBuyQuoteRequest = serde_json::from_value(json!({
        "wallet_id": "fixture-wallet", "mint": "base", "amount_lamports": 1000000, "slippage_bps": 1500
    })).unwrap();
    assert_eq!(quote.amount_lamports, 1_000_000);
    let request = json!({"wallet_id": "fixture-wallet", "confirmed": false, "simulate": true, "idempotency_key": "q1"});
    let decoded: SolBuySubmitRequest = serde_json::from_value(request.clone()).unwrap();
    assert!(!decoded.confirmed);
    assert!(decoded.simulate);
    for field in ["confirmed", "simulate", "idempotency_key"] {
        let mut missing = request.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<SolBuySubmitRequest>(missing).is_err());
    }
}

#[test]
fn wsol_defaults_do_not_imply_all_or_live_confirmation() {
    let quote: WsolQuoteRequest = serde_json::from_value(json!({
        "wallet_id": "fixture-wallet", "owner_type": "fixture", "owner_id": "owner", "direction": "wrap"
    })).unwrap();
    assert!(!quote.all);
    assert_eq!(quote.direction, WsolDirection::Wrap);
    assert_eq!(quote.amount_lamports, None);
    let mut value = json!({
        "quote_id": "q1", "wallet_id": "fixture-wallet", "owner_type": "fixture", "owner_id": "owner",
        "simulate": true, "idempotency_key": "w1"
    });
    let submission: WsolSubmission = serde_json::from_value(value.clone()).unwrap();
    assert!(!submission.confirmed);
    assert!(submission.simulate);
    value.as_object_mut().unwrap().remove("simulate");
    assert!(serde_json::from_value::<WsolSubmission>(value).is_err());
}

#[test]
fn transfer_amount_mode_stays_separate_from_trade_amount() {
    let request: SolTransferRequest = serde_json::from_value(json!({
        "owner_type": "fixture", "owner_id": "owner", "to_address": "receiver",
        "amount_mode": "percent", "percent": 100, "idempotency_key": "transfer1"
    }))
    .unwrap();
    assert_eq!(request.amount_mode.as_str(), "percent");
    assert_eq!(request.percent, Some(100));
    assert_eq!(request.amount_lamports, None);
}

#[test]
fn health_and_error_envelopes_remain_stable() {
    assert_eq!(
        serde_json::to_value(HealthResponse { ok: true }).unwrap(),
        json!({"ok": true})
    );
    assert_eq!(
        serde_json::to_value(ErrorResponse {
            error: "fixture error".to_string()
        })
        .unwrap(),
        json!({"error": "fixture error"})
    );
}

#[test]
fn readiness_v1_full_response_baseline_preserves_all_fields() {
    let expected: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/contracts/readiness_v1.json"
    ))
    .unwrap();
    let response = ReadyResponse {
        ready: true,
        wallet: "11111111111111111111111111111111".into(),
        default_wallet_id: "fixture-main".into(),
        wallet_count: 2,
        wallets: vec![
            WalletInfo {
                wallet_id: "fixture-main".into(),
                pubkey: "11111111111111111111111111111111".into(),
            },
            WalletInfo {
                wallet_id: "fixture-other".into(),
                pubkey: WSOL_MINT.into(),
            },
        ],
        has_blockhash: true,
        blockhash_age_ms: Some(15),
        rpc_ok: true,
        paused: false,
        rpc_lanes: 3,
        swqos_lanes: vec!["Jito".into(), "Temporal".into(), "Default".into()],
        active_swqos_lanes: vec!["Jito".into(), "Temporal".into()],
        mev_protection: true,
        require_mev_submit: true,
        pumpapi_executor: false,
        trade_persistence_mode: "durable".into(),
        trade_database_healthy: true,
    };
    assert_eq!(serde_json::to_value(response).unwrap(), expected);
}
