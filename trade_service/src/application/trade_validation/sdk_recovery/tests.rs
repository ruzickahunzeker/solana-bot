use super::*;
use crate::{
    adapters::{
        persistence::trade_snapshot::request_hash,
        sdk::{
            address_parser::SdkAddressParser,
            prepared_candidate::{self, test_support::buy},
        },
    },
    domain::sdk_buy_candidate::DammBuyCandidateRecord,
};

fn original_settings() -> TradeValidationSettings {
    TradeValidationSettings {
        default_buy_sol: 0.0000001,
        default_slippage_percent: 15.0,
        default_tip_sol: 0.0001,
        via_sol_enabled: false,
        via_sol_max_buy_sol: 0.1,
    }
}

#[tokio::test]
async fn sdk_buy_replay_preserves_original_v1_hash_for_all_existing_explicit_amount_units() {
    let validator = TradeValidator::new(&SdkAddressParser);
    for native in [false, true] {
        let f = buy(native).await;
        let choices = if native {
            vec!["lamports", "sol", "quote", "raw"]
        } else {
            vec!["usdc", "quote", "raw"]
        };
        for field in choices {
            let mut request = f.request.clone();
            request.amount_lamports = None;
            request.amount_quote_units = None;
            match field {
                "lamports" => request.amount_lamports = Some(100),
                "sol" => request.amount_sol = Some(0.0000001),
                "quote" => request.amount_quote = Some(if native { 0.0000001 } else { 0.0001 }),
                "usdc" => request.amount_usdc = Some(0.0001),
                "raw" => request.amount_quote_units = Some(100),
                _ => unreachable!(),
            }
            let current = validator
                .validate(&request.validation_input(), &original_settings())
                .unwrap();
            let historical = validator
                .replay_sdk_candidate(&request.validation_input(), &f.record)
                .unwrap();
            assert_eq!(historical.amount_lamports, Some(100));
            assert_eq!(
                request_hash("fixture-wallet", &request, &current).unwrap(),
                request_hash("fixture-wallet", &request, &historical).unwrap(),
                "{native}/{field}"
            );
        }
    }
}

#[tokio::test]
async fn sdk_buy_replay_uses_original_integer_and_fee_defaults_without_current_settings() {
    let validator = TradeValidator::new(&SdkAddressParser);
    let f = buy(true).await;
    let mut request = f.request.clone();
    request.amount_lamports = None;
    request.tip_sol = None;
    request.slippage_bps = None;
    let original = validator
        .validate(&request.validation_input(), &original_settings())
        .unwrap();
    let historical = validator
        .replay_sdk_candidate(&request.validation_input(), &f.record)
        .unwrap();
    assert_eq!(
        request_hash("fixture-wallet", &request, &original).unwrap(),
        request_hash("fixture-wallet", &request, &historical).unwrap()
    );
    let mut changed = original_settings();
    changed.default_buy_sol = f64::NAN;
    changed.default_tip_sol = f64::NAN;
    changed.default_slippage_percent = f64::NAN;
    assert!(validator
        .validate(&request.validation_input(), &changed)
        .is_err());
    assert_eq!(
        validator
            .replay_sdk_candidate(&request.validation_input(), &f.record)
            .unwrap()
            .amount_lamports,
        Some(100)
    );
    request.slippage_percent = Some(15.0);
    request.slippage_bps = Some(1); // V1 percent precedence is retained.
    assert_eq!(
        validator
            .replay_sdk_candidate(&request.validation_input(), &f.record)
            .unwrap()
            .slippage_bps,
        1500
    );
}

#[tokio::test]
async fn sdk_buy_replay_rejects_payment_direction_scope_and_original_amount_fee_mutations() {
    let validator = TradeValidator::new(&SdkAddressParser);
    for native in [false, true] {
        let f = buy(native).await;
        if !native {
            let mut missing = f.request.clone();
            missing.amount_quote_units = None;
            assert!(validator
                .replay_sdk_candidate(&missing.validation_input(), &f.record)
                .is_err());
        }
        for field in [
            "side",
            "pool",
            "method",
            "simulate",
            "settlement",
            "hop",
            "hop_id",
            "mint",
            "quote",
            "pool_id",
            "amount",
            "mixed_amounts",
            "slippage",
            "tip",
            "provider",
            "sell_percent",
            "aggregate",
        ] {
            let mut r = f.request.clone();
            match field {
                "side" => r.side = Side::Sell,
                "pool" => r.pool = Pool::PumpAmm,
                "method" => r.execution_method = Some(ExecutionMethod::Pumpapi),
                "simulate" => r.simulate = Some(true),
                "settlement" => r.settlement_asset = Some(SettlementAsset::Sol),
                "hop" => r.sol_hop_pool = Some(Pool::RaydiumCpmm),
                "hop_id" => r.sol_hop_pool_id = Some(Address::from_bytes([91; 32]).to_string()),
                "mint" => r.mint = Address::from_bytes([91; 32]).to_string(),
                "quote" => r.quote_mint = Some(Address::from_bytes([91; 32]).to_string()),
                "pool_id" => r.pool_id = None,
                "amount" => {
                    if native {
                        r.amount_lamports = Some(101);
                    } else {
                        r.amount_quote_units = Some(101);
                    }
                }
                "mixed_amounts" => r.amount_sol = Some(0.001),
                "slippage" => r.slippage_bps = Some(1),
                "tip" => r.tip_sol = Some(0.002),
                "provider" => r.mev_provider = Some(MevProvider::Jito),
                "sell_percent" => r.sell_percent = Some(100),
                "aggregate" => {
                    r.amount_lamports = None;
                    r.amount_quote_units = None;
                    r.amount_sol = Some(0.0000001);
                    r.quote_mint = Some(USDC_MINT.into());
                }
                _ => unreachable!(),
            }
            assert!(
                validator
                    .replay_sdk_candidate(&r.validation_input(), &f.record)
                    .is_err(),
                "{native}/{field}"
            );
        }
    }
}

#[tokio::test]
async fn sdk_buy_replay_preserves_raw_units_above_float_precision_but_does_not_authorize_a_message()
{
    let validator = TradeValidator::new(&SdkAddressParser);
    for native in [false, true] {
        let f = buy(native).await;
        let mut r = f.request.clone();
        let mut d = f.record.buy().unwrap().details().clone();
        let units = 9_007_199_254_740_993;
        d.input_units = units;
        if native {
            d.native.deposit_lamports = units;
            d.native.before_lamports = units
                + d.native.rent_lamports
                + d.native.base_fee_lamports
                + d.native.remaining_lamports;
            r.amount_lamports = None;
        } else {
            r.amount_quote_units = Some(units);
        }
        let record =
            SdkPreparedRecord::DammBuy(DammBuyCandidateRecord::new(d, &f.message).unwrap());
        assert_eq!(
            validator
                .replay_sdk_candidate(&r.validation_input(), &record)
                .unwrap()
                .amount_lamports,
            Some(units)
        );
        assert!(prepared_candidate::validate_candidate(&record, &f.message).is_err());
    }
}
