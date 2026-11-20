use super::*;
use crate::{
    adapters::sdk::pump_curve_evidence::native_funding::probe::observation::shared::constraints::envelope::simulation::bank_swap::tests::final_for_intent,
    application::{
        ordinary_claims::test_support::{
            admit_request_for_pubkey, fixture as state_fixture, Fixture,
        },
        ports::ordinary_claims::OrdinaryExecutionClaimer,
    },
    config::TradePersistenceMode,
    domain::{preparation_balance::MAX_BALANCE_AGE, trade::MevProvider},
    models::TradeRequest,
};
use serde_json::json;
use sol_trade_sdk::swqos::{SwqosType, TradeType};
use solana_sdk::signature::Signature;
use std::time::{Duration, Instant};

pub(super) async fn admitted(
    mode: TradePersistenceMode,
    side: TradeType,
    provider: SwqosType,
) -> (Fixture, NativeFinalObservation) {
    let state = state_fixture(mode).await.state;
    let observation = final_for_intent(side, provider).await;
    let funding = &observation.template.source.source.observation.probe.funding;
    let buy = matches!(side, TradeType::Buy);
    let mut request = json!({
        "wallet_id":"fixture-wallet", "pool":"pump", "side":if buy {"buy"} else {"sell"},
        "mint":funding.keys[1].to_string(), "quote_mint":WSOL_MINT,
        "execution_method":"sdk", "simulate":false,
        "slippage_bps":1500, "tip_sol":0.0001,
        "use_mev":provider != SwqosType::Default,
        "mev_provider":if provider == SwqosType::Default {"default"} else {"temporal"},
        "idempotency_key":format!(" literal-native-{side:?}-{provider:?} ")
    });
    if buy {
        request["amount_sol"] = json!(0.001);
    } else {
        request["sell_percent"] = json!(10);
    }
    let request: TradeRequest = serde_json::from_value(request).unwrap();
    let fixture = admit_request_for_pubkey(state, request, funding.keys[6].to_string()).await;
    (fixture, observation)
}

#[tokio::test]
async fn pump_intent_binds_actual_admission_native_amounts_and_original_wire_without_claim_permission(
) {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (f, observation) = admitted(TradePersistenceMode::Durable, side, provider).await;
            let frozen = bincode::serialize(observation.packet().unwrap()).unwrap();
            let bound = observation.bind_intent(&f.execution).unwrap();
            assert_eq!(bincode::serialize(bound.packet().unwrap()).unwrap(), frozen);
            assert_eq!(bound.packet().unwrap().signatures, [Signature::default()]);
            assert!(std::ptr::eq(bound.anchor, f.execution.reservation()));
            assert!(std::ptr::eq(bound.execution, f.execution.as_ref()));
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM ordinary_execution_claims WHERE request_id=?",
            )
            .bind(&f.record.request_id)
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
            assert_eq!(count, 0);
            // Binding cannot widen the unchanged original DAMM-only SDK claim selector.
            assert!(f.claims().claim(&f.execution).await.is_err());
        }
    }
}

#[tokio::test]
async fn pump_intent_rejects_original_identity_route_amount_mode_fee_and_provider_mismatches() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (f, observation) = admitted(TradePersistenceMode::Durable, side, provider).await;
            check(&observation, &f.execution).unwrap();
            for field in [
                "wallet",
                "key",
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
                "slippage",
                "tip",
                "use_mev",
                "provider",
            ] {
                let mut command = f.execution.command().clone();
                match field {
                    "wallet" => command.wallet_id.push('x'),
                    "key" => command.idempotency_key.push('x'),
                    "pool" => command.route.pool = Pool::PumpAmm,
                    "mint" => command.route.mint = Address::from_bytes([91; 32]),
                    "quote" => command.route.quote_mint = Address::from_bytes([91; 32]),
                    "pool_id" => command.route.pool_id = Some(Address::from_bytes([91; 32])),
                    "hop" => command.route.sol_hop_pool = Some(Pool::RaydiumCpmm),
                    "hop_id" => command.route.sol_hop_pool_id = Some(Address::from_bytes([91; 32])),
                    "method" => command.execution.method = ExecutionMethod::Pumpapi,
                    "mode" => command.execution.mode = ExecutionMode::Simulate,
                    "aggregate" => command.aggregator_sol = true,
                    "amount" => {
                        command.amount = if matches!(side, TradeType::Buy) {
                            TradeAmount::BuySol {
                                lamports: 1_000_001,
                            }
                        } else {
                            TradeAmount::SellPercent { percent: 20 }
                        }
                    }
                    "asset" => {
                        command.amount = TradeAmount::BuyQuote {
                            mint: command.route.quote_mint,
                            units: 1_000_000,
                        }
                    }
                    "direction" => {
                        command.amount = if matches!(side, TradeType::Buy) {
                            TradeAmount::SellPercent { percent: 10 }
                        } else {
                            TradeAmount::BuySol {
                                lamports: 1_000_000,
                            }
                        }
                    }
                    "slippage" => command.execution.slippage_bps += 1,
                    "tip" => command.execution.tip_sol += 0.0001,
                    "use_mev" => command.execution.use_mev = Some(provider == SwqosType::Default),
                    "provider" => command.execution.mev_provider = Some(MevProvider::Jito),
                    _ => unreachable!(),
                }
                assert!(
                    check(&observation, &f.execution.with_command_for_test(command)).is_err(),
                    "{side:?}/{provider:?}/{field}"
                );
            }
            let mut equivalent = f.execution.command().clone();
            equivalent.route.pool_id = None;
            equivalent.execution.use_mev = None;
            equivalent.execution.mev_provider = Some(MevProvider::Auto);
            check(&observation, &f.execution.with_command_for_test(equivalent)).unwrap();
            let mut parent = f.record.clone();
            parent.wallet_pubkey = Address::from_bytes([91; 32]).to_string();
            let wrong_wallet =
                AdmittedTrade::unreserved_fixture(&parent, f.execution.command().clone());
            assert!(check(&observation, &wrong_wallet)
                .unwrap_err()
                .to_string()
                .contains("wallet identity changed"));
        }
    }
}

#[tokio::test]
async fn pump_intent_partial_sell_uses_original_checked_percentage_and_excludes_close_profile() {
    let (f, mut observation) = admitted(
        TradePersistenceMode::Durable,
        TradeType::Sell,
        SwqosType::Default,
    )
    .await;
    assert_eq!(
        observation
            .template
            .source
            .source
            .observation
            .probe
            .funding
            .user_base_units,
        Some(10_000)
    );
    check(&observation, &f.execution).unwrap();
    for percent in [0, 1, 99, 100, 101, u64::MAX] {
        let mut command = f.execution.command().clone();
        command.amount = TradeAmount::SellPercent { percent };
        assert!(check(&observation, &f.execution.with_command_for_test(command)).is_err());
    }
    for balance in [None, Some(0), Some(9_999), Some(u64::MAX)] {
        observation
            .template
            .source
            .source
            .observation
            .probe
            .funding
            .user_base_units = balance;
        assert!(check(&observation, &f.execution).is_err());
    }
    observation
        .template
        .source
        .source
        .observation
        .probe
        .funding
        .user_base_units = Some(10_000);
    observation.bind_intent(&f.execution).unwrap();
}

#[tokio::test]
async fn pump_intent_keeps_original_reservation_identity_freshness_and_fixed_unsigned_packet() {
    for mode in [TradePersistenceMode::Memory, TradePersistenceMode::Shadow] {
        let (f, observation) = admitted(mode, TradeType::Buy, SwqosType::Default).await;
        assert!(observation.bind_intent(&f.execution).is_err());
    }
    let (f, observation) = admitted(
        TradePersistenceMode::Durable,
        TradeType::Buy,
        SwqosType::Default,
    )
    .await;
    let other = f
        .execution
        .with_command_for_test(f.execution.command().clone());
    let mut bound = observation.bind_intent(&f.execution).unwrap();
    bound.execution = &other;
    assert!(bound.packet().is_err());
    bound.execution = &f.execution;
    bound.packet().unwrap();
    bound.observation.template.transaction.signatures[0] = Signature::from([1; 64]);
    assert!(bound.packet().is_err());
    bound.observation.template.transaction.signatures[0] = Signature::default();
    bound.packet().unwrap();
    bound
        .observation
        .template
        .source
        .source
        .observation
        .probe
        .funding
        .started = Instant::now() - MAX_BALANCE_AGE - Duration::from_secs(1);
    assert!(bound.packet().is_err());
}
