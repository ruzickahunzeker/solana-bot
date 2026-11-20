use super::*;
use crate::{
    adapters::sdk::prepared_candidate::{validate_candidate, validate_signed_candidate},
    application::{
        ordinary_claims::test_support::{admit_request_for_pubkey, fixture},
        ports::ordinary_claims::OrdinaryExecutionClaimer,
        sdk_preparation::{validate_candidate_intent, validate_origin},
    },
    config::TradePersistenceMode,
    domain::{
        pump_native_candidate::PumpNativeDetails, sdk_prepared_record::SdkPreparedRecord,
        submission::SignedSubmission, trade::MevProvider,
    },
    models::TradeRequest,
};
use sol_trade_sdk::swqos::{SwqosType, TradeType};

fn changed_record(
    native: &PumpNativeCandidateRecord,
    message: &[u8],
    edit: impl FnOnce(&mut PumpNativeDetails),
) -> SdkPreparedRecord {
    let mut details = native.details().clone();
    edit(&mut details);
    SdkPreparedRecord::PumpNative(PumpNativeCandidateRecord::new(details, message).unwrap())
}

#[tokio::test]
async fn pump_history_intent_matches_original_admission_without_claim_or_signature_permission() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (f, native, message) =
                PumpNativeCandidateRecord::history_fixture(side, provider).await;
            let metadata = SdkPreparedRecord::PumpNative(native);
            validate_candidate(&metadata, &message).unwrap();
            validate_candidate_intent(&f.execution, &metadata, &message).unwrap();
            assert!(validate_origin(&f.execution, &metadata, &message).is_err());
            assert!(f.claims().claim(&f.execution).await.is_err());
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM ordinary_execution_claims WHERE request_id=?",
            )
            .bind(f.execution.reservation().request_id())
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
            assert_eq!(count, 0);
            let error = validate_signed_candidate(
                &metadata,
                &message,
                &SignedSubmission {
                    bytes: vec![],
                    signature: String::new(),
                },
            )
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("historical Pump candidate cannot enter signing"));
        }
    }
}

#[tokio::test]
async fn pump_history_intent_rejects_changed_command_route_payment_mode_and_execution_policy() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (f, native, message) =
                PumpNativeCandidateRecord::history_fixture(side, provider).await;
            let metadata = SdkPreparedRecord::PumpNative(native);
            for field in [
                "wallet",
                "literal_key",
                "pool",
                "mint",
                "quote",
                "pool_id",
                "hop",
                "hop_id",
                "method",
                "mode",
                "aggregate",
                "amount",
                "asset",
                "direction",
                "close",
                "slippage",
                "tip",
                "use_mev",
                "provider",
            ] {
                let mut c = f.execution.command().clone();
                match field {
                    "wallet" => c.wallet_id.push('x'),
                    "literal_key" => c.idempotency_key = c.idempotency_key.trim().into(),
                    "pool" => c.route.pool = Pool::MeteoraDammV2,
                    "mint" => c.route.mint = Address::from_bytes([91; 32]),
                    "quote" => c.route.quote_mint = Address::from_bytes([91; 32]),
                    "pool_id" => c.route.pool_id = Some(Address::from_bytes([91; 32])),
                    "hop" => c.route.sol_hop_pool = Some(Pool::RaydiumCpmm),
                    "hop_id" => c.route.sol_hop_pool_id = Some(Address::from_bytes([91; 32])),
                    "method" => c.execution.method = ExecutionMethod::Pumpapi,
                    "mode" => c.execution.mode = ExecutionMode::Simulate,
                    "aggregate" => c.aggregator_sol = true,
                    "amount" => {
                        c.amount = match c.amount {
                            TradeAmount::BuySol { lamports } => TradeAmount::BuySol {
                                lamports: lamports + 1,
                            },
                            _ => TradeAmount::SellPercent { percent: 20 },
                        }
                    }
                    "asset" => {
                        c.amount = TradeAmount::BuyQuote {
                            mint: c.route.quote_mint,
                            units: 1_000_000,
                        }
                    }
                    "direction" => {
                        c.amount = if matches!(side, TradeType::Buy) {
                            TradeAmount::SellPercent { percent: 10 }
                        } else {
                            TradeAmount::BuySol {
                                lamports: 1_000_000,
                            }
                        }
                    }
                    "close" => c.amount = TradeAmount::SellPercent { percent: 100 },
                    "slippage" => c.execution.slippage_bps += 1,
                    "tip" => c.execution.tip_sol += 0.0001,
                    "use_mev" => c.execution.use_mev = None,
                    "provider" => c.execution.mev_provider = Some(MevProvider::Auto),
                    _ => unreachable!(),
                }
                assert!(
                    validate_candidate_intent(
                        &f.execution.with_command_for_test(c),
                        &metadata,
                        &message
                    )
                    .is_err(),
                    "{side:?}/{provider:?}/{field}"
                );
            }
        }
    }
}

#[tokio::test]
async fn pump_history_intent_rejects_nominally_valid_foreign_identity_and_execution_metadata() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (f, native, message) =
            PumpNativeCandidateRecord::history_fixture(side, SwqosType::Default).await;
        for field in [
            "request_id",
            "wallet_id",
            "wallet",
            "literal_key",
            "request_hash",
            "tip",
            "use_mev",
            "provider",
        ] {
            let metadata = changed_record(&native, &message, |d| match field {
                "request_id" => d.request_id.push('x'),
                "wallet_id" => d.wallet_id.push('x'),
                "wallet" => d.wallet = [91; 32],
                "literal_key" => d.idempotency_key = d.idempotency_key.trim().into(),
                "request_hash" => d.request_hash.push('x'),
                "tip" => d.tip_sol_bits = 0.0f64.to_bits(),
                "use_mev" => d.use_mev = None,
                "provider" => d.requested_mev_provider = Some(MevProvider::Auto),
                _ => unreachable!(),
            });
            metadata.validate(&message).unwrap();
            assert!(
                validate_candidate_intent(&f.execution, &metadata, &message).is_err(),
                "{side:?}/{field}"
            );
        }
    }
}

#[tokio::test]
async fn pump_history_intent_does_not_accept_a_coherent_different_percentage_for_the_same_input() {
    let (f, native, message) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Sell, SwqosType::Default).await;
    assert_eq!(native.details().sell_percent, Some(10));
    let metadata = changed_record(&native, &message, |d| {
        d.sell_percent = Some(20);
        d.source_token_units = Some(d.input_units.checked_mul(5).unwrap());
    });
    // Wire and nominal math can match while the original user's percentage does not.
    validate_candidate(&metadata, &message).unwrap();
    let error = validate_candidate_intent(&f.execution, &metadata, &message).unwrap_err();
    assert!(error
        .to_string()
        .contains("historical percentage differs from the original intent"));
    assert!(validate_origin(&f.execution, &metadata, &message).is_err());
}

#[tokio::test]
async fn pump_history_intent_does_not_rebind_to_an_identical_request_on_a_new_reservation_or_mode()
{
    let (original, native, message) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let json: String = sqlx::query_scalar("SELECT request_json FROM trades WHERE request_id=?")
        .bind(original.execution.reservation().request_id())
        .fetch_one(&original.state.persistence.pool_for_tests())
        .await
        .unwrap();
    let metadata = SdkPreparedRecord::PumpNative(native);
    for mode in [
        TradePersistenceMode::Durable,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Memory,
    ] {
        let state = fixture(mode).await.state;
        let request: TradeRequest = serde_json::from_str(&json).unwrap();
        let new = admit_request_for_pubkey(
            state,
            request,
            original.execution.reservation().wallet_pubkey().into(),
        )
        .await;
        assert_ne!(
            new.execution.reservation().request_id(),
            original.execution.reservation().request_id()
        );
        assert_eq!(
            new.execution.command().idempotency_key,
            original.execution.command().idempotency_key
        );
        assert_eq!(
            new.execution.command().route.mint,
            original.execution.command().route.mint
        );
        if matches!(mode, TradePersistenceMode::Durable) {
            assert_eq!(
                new.execution.reservation().request_hash(),
                original.execution.reservation().request_hash()
            );
        }
        assert!(validate_candidate_intent(&new.execution, &metadata, &message).is_err());
        assert!(validate_origin(&new.execution, &metadata, &message).is_err());
        assert!(new.claims().claim(&new.execution).await.is_err());
    }
}
