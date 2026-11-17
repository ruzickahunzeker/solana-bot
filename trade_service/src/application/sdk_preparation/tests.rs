use super::*;
use crate::{
    adapters::sdk::prepared_candidate::test_support::buy,
    application::{
        ordinary_claims::test_support::{admit_request_for_pubkey, fixture},
        ports::ordinary_claims::OrdinaryExecutionClaimer,
    },
    config::TradePersistenceMode,
    domain::{
        sdk_buy_candidate::DammBuyCandidateRecord,
        trade::{ExecutionMode, MevProvider},
    },
};
use solana_sdk::signature::Signer;

#[tokio::test]
async fn sdk_buy_intent_binds_both_integer_payment_assets_but_does_not_grant_an_owner() {
    for native in [false, true] {
        let f = buy(native).await;
        let state = fixture(TradePersistenceMode::Durable).await.state;
        let admitted =
            admit_request_for_pubkey(state, f.request.clone(), f.key.pubkey().to_string()).await;
        validate_candidate_intent(&admitted.execution, &f.record, &f.message).unwrap();
        validate_origin(&admitted.execution, &f.record, &f.message).unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM ordinary_execution_claims WHERE request_id=?")
                .bind(&admitted.record.request_id)
                .fetch_one(&admitted.state.persistence.pool_for_tests())
                .await
                .unwrap();
        assert_eq!(count, 0);
        assert!(admitted
            .claims()
            .claim(&admitted.execution)
            .await
            .unwrap()
            .is_acquired());
    }
}

#[tokio::test]
async fn sdk_buy_intent_rejects_changed_original_command_amount_asset_mode_pool_and_fee_fields() {
    for native in [false, true] {
        let f = buy(native).await;
        let state = fixture(TradePersistenceMode::Durable).await.state;
        let admitted =
            admit_request_for_pubkey(state, f.request.clone(), f.key.pubkey().to_string()).await;
        for field in [
            "mint",
            "quote",
            "pool_id",
            "pool",
            "hop",
            "hop_id",
            "amount",
            "asset",
            "direction",
            "slippage",
            "tip",
            "use_mev",
            "provider",
            "mode",
            "method",
            "aggregate",
        ] {
            let mut command = admitted.execution.command().clone();
            match field {
                "mint" => command.route.mint = Address::from_bytes([91; 32]),
                "quote" => command.route.quote_mint = Address::from_bytes([91; 32]),
                "pool_id" => command.route.pool_id = None,
                "pool" => command.route.pool = Pool::PumpAmm,
                "hop" => command.route.sol_hop_pool = Some(Pool::RaydiumCpmm),
                "hop_id" => command.route.sol_hop_pool_id = Some(Address::from_bytes([91; 32])),
                "amount" => {
                    command.amount = if native {
                        TradeAmount::BuySol { lamports: 101 }
                    } else {
                        TradeAmount::BuyQuote {
                            mint: command.route.quote_mint,
                            units: 101,
                        }
                    }
                }
                "asset" => {
                    command.amount = if native {
                        TradeAmount::BuyQuote {
                            mint: command.route.quote_mint,
                            units: 100,
                        }
                    } else {
                        TradeAmount::BuySol { lamports: 100 }
                    }
                }
                "direction" => command.amount = TradeAmount::SellPercent { percent: 100 },
                "slippage" => command.execution.slippage_bps += 1,
                "tip" => command.execution.tip_sol += 0.0001,
                "use_mev" => command.execution.use_mev = Some(true),
                "provider" => command.execution.mev_provider = Some(MevProvider::Temporal),
                "mode" => command.execution.mode = ExecutionMode::Simulate,
                "method" => command.execution.method = ExecutionMethod::Pumpapi,
                "aggregate" => command.aggregator_sol = true,
                _ => unreachable!(),
            }
            assert!(
                validate_candidate_intent(
                    &admitted.execution.with_command_for_test(command),
                    &f.record,
                    &f.message
                )
                .is_err(),
                "{native}/{field}"
            );
        }
    }
}

#[tokio::test]
async fn sdk_buy_intent_rejects_self_consistent_wrong_wallet_or_runtime_policy_record() {
    let f = buy(true).await;
    let state = fixture(TradePersistenceMode::Durable).await.state;
    let admitted =
        admit_request_for_pubkey(state, f.request.clone(), f.key.pubkey().to_string()).await;
    for field in ["wallet", "simulate", "slippage", "funding", "provider"] {
        let mut details = f.record.buy().unwrap().details().clone();
        match field {
            "wallet" => details.wallet = [91; 32],
            "simulate" => details.simulate = true,
            "slippage" => {
                details.slippage_bps = 0;
                details.minimum_output_units = details.quoted_net_output_units;
            }
            "funding" => {
                details.funding = BuyFunding::ExistingQuote;
                details.native.remaining_lamports += details.native.deposit_lamports;
                details.native.deposit_lamports = 0;
            }
            "provider" => details.requested_mev_provider = Some(MevProvider::Jito),
            _ => unreachable!(),
        }
        let record =
            SdkPreparedRecord::DammBuy(DammBuyCandidateRecord::new(details, &f.message).unwrap());
        assert!(
            validate_candidate_intent(&admitted.execution, &record, &f.message).is_err(),
            "{field}"
        );
    }
}
