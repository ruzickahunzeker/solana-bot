use super::*;
use crate::domain::trade::{Address, TradeAmount, USDC_MINT};
use serde_json::json;

fn fixture() -> (TradeRequest, NormalizedTrade) {
    let mint = Address::from_bytes([2; 32]);
    let quote = Address::from_bytes(
        bs58::decode(USDC_MINT)
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let request = serde_json::from_value(json!({
        "wallet_id":"wire-wallet","side":"buy","pool":"raydium-cpmm",
        "mint":mint.to_string(),"quote_mint":USDC_MINT,
        "amount_quote_units":u64::MAX,"idempotency_key":" literal key "
    }))
    .unwrap();
    let normalized = NormalizedTrade {
        mint,
        quote_mint: quote,
        pool_id: None,
        settlement_asset: SettlementAsset::Quote,
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(u64::MAX),
        sell_percent: None,
        slippage_bps: 1500,
        tip_sol: 0.0001,
        execution_method: ExecutionMethod::Sdk,
        use_mev: None,
        mev_provider: None,
        simulate: None,
        aggregator_sol: false,
    };
    (request, normalized)
}

#[test]
fn command_defaults_resolve_all_modes_without_full_config_or_changing_wire_units() {
    let (mut request, mut value) = fixture();
    for default in [false, true] {
        for explicit in [None, Some(false), Some(true)] {
            request.simulate = explicit;
            value.simulate = explicit;
            let wire = serde_json::to_vec(&request).unwrap();
            let command = request
                .to_command(
                    "resolved-wallet".into(),
                    &value,
                    TradeCommandDefaults {
                        default_simulate: default,
                        pumpapi_priority_fee_sol: 0.0002,
                    },
                )
                .unwrap();
            assert_eq!(
                command.execution.mode.simulate(),
                explicit.unwrap_or(default)
            );
            assert_eq!(
                command.amount,
                TradeAmount::BuyQuote {
                    mint: value.quote_mint,
                    units: u64::MAX
                }
            );
            assert_eq!(command.wallet_id, "resolved-wallet");
            assert_eq!(command.idempotency_key, " literal key ");
            assert_eq!(serde_json::to_vec(&request).unwrap(), wire);
        }
    }
}

#[test]
fn command_defaults_keep_separate_fees_and_explicit_zero_bits_without_eager_validation() {
    let (mut request, value) = fixture();
    for default in [0.0002, f64::NAN, f64::INFINITY, -0.0] {
        for explicit in [None, Some(0.0), Some(-0.0), Some(0.0003)] {
            request.tip_sol = explicit;
            let command = request
                .to_command(
                    "resolved-wallet".into(),
                    &value,
                    TradeCommandDefaults {
                        default_simulate: false,
                        pumpapi_priority_fee_sol: default,
                    },
                )
                .unwrap();
            assert_eq!(
                command.pumpapi_priority_fee_sol.to_bits(),
                explicit.unwrap_or(default).to_bits()
            );
            assert_eq!(command.execution.tip_sol.to_bits(), value.tip_sol.to_bits());
        }
    }
}

#[test]
fn command_defaults_do_not_hide_missing_normalized_amount_errors() {
    let (mut request, mut value) = fixture();
    let defaults = TradeCommandDefaults {
        default_simulate: false,
        pumpapi_priority_fee_sol: f64::NAN,
    };
    value.amount_lamports = None;
    assert_eq!(
        request
            .to_command("wallet".into(), &value, defaults)
            .unwrap_err()
            .to_string(),
        "validated buy amount is missing"
    );
    request.side = Side::Sell;
    assert_eq!(
        request
            .to_command("wallet".into(), &value, defaults)
            .unwrap_err()
            .to_string(),
        "validated sell percent is missing"
    );
}
