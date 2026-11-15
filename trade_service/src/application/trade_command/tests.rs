use super::*;
use crate::{
    application::trade_execution::ensure_execution_allowed,
    domain::execution_policy::{BroadcastPolicy, BROADCAST_DISABLED_ERROR},
};

fn validated() -> ValidatedTrade {
    ValidatedTrade {
        mint: Address::from_bytes([1; 32]),
        quote_mint: Address::from_bytes([2; 32]),
        pool_id: Some(Address::from_bytes([3; 32])),
        settlement_asset: SettlementAsset::Quote,
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(123),
        sell_percent: Some(100),
        slippage_bps: 1500,
        tip_sol: 0.0001,
        execution_method: ExecutionMethod::Sdk,
        use_mev: Some(true),
        mev_provider: Some(MevProvider::Temporal),
        simulate: None,
        aggregator_sol: false,
    }
}
fn context(side: Side, default_simulate: bool, priority_fee: f64) -> CommandContext<'static> {
    CommandContext {
        wallet_id: "resolved-wallet".into(),
        idempotency_key: " literal-key ",
        side,
        pool: Pool::RaydiumCpmm,
        default_simulate,
        pumpapi_priority_fee_sol: priority_fee,
    }
}

#[test]
fn internal_builder_keeps_quote_sol_and_sell_amount_kinds_without_wire_or_config() {
    for branch in ["quote", "settlement-sol", "aggregator", "wsol"] {
        let mut value = validated();
        match branch {
            "settlement-sol" => value.settlement_asset = SettlementAsset::Sol,
            "aggregator" => value.aggregator_sol = true,
            "wsol" => {
                value.quote_mint = Address::from_bytes(
                    bs58::decode(WSOL_MINT)
                        .into_vec()
                        .unwrap()
                        .try_into()
                        .unwrap(),
                )
            }
            _ => {}
        }
        let command =
            TradeCommand::from_validated(context(Side::Buy, false, 0.0002), &value).unwrap();
        if branch == "quote" {
            assert!(
                matches!(command.amount, TradeAmount::BuyQuote {mint, units:123} if mint == value.quote_mint)
            );
        } else {
            assert!(matches!(
                command.amount,
                TradeAmount::BuySol { lamports: 123 }
            ));
        }
        assert_eq!(command.route.mint, value.mint);
        assert_eq!(command.route.pool_id, value.pool_id);
        assert_eq!(command.wallet_id, "resolved-wallet");
        assert_eq!(command.idempotency_key, " literal-key ");
    }
    let command =
        TradeCommand::from_validated(context(Side::Sell, false, 0.0002), &validated()).unwrap();
    assert!(matches!(
        command.amount,
        TradeAmount::SellPercent { percent: 100 }
    ));
}

#[test]
fn internal_builder_preserves_mode_tristate_resolution_and_separate_backend_fees() {
    for simulate in [None, Some(false), Some(true)] {
        for default in [false, true] {
            for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
                let mut value = validated();
                value.simulate = simulate;
                value.execution_method = method;
                let command =
                    TradeCommand::from_validated(context(Side::Buy, default, 0.0009), &value)
                        .unwrap();
                assert_eq!(
                    command.execution.mode.simulate(),
                    simulate.unwrap_or(default)
                );
                assert_eq!(command.execution.method, method);
                assert_eq!(command.execution.tip_sol.to_bits(), 0.0001f64.to_bits());
                assert_eq!(
                    command.pumpapi_priority_fee_sol.to_bits(),
                    0.0009f64.to_bits()
                );
                assert_eq!(command.execution.slippage_bps, 1500);
                assert_eq!(command.execution.mev_provider, Some(MevProvider::Temporal));
            }
        }
    }
}

#[test]
fn internal_builder_keeps_missing_buy_and_sell_amount_errors() {
    let mut value = validated();
    value.amount_lamports = None;
    value.sell_percent = None;
    assert_eq!(
        TradeCommand::from_validated(context(Side::Buy, false, 0.0), &value)
            .unwrap_err()
            .to_string(),
        "validated buy amount is missing"
    );
    assert_eq!(
        TradeCommand::from_validated(context(Side::Sell, false, 0.0), &value)
            .unwrap_err()
            .to_string(),
        "validated sell percent is missing"
    );
}

#[test]
fn project_execution_policy_keeps_sdk_simulate_and_other_branch_broadcast_rules() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for aggregator in [false, true] {
            for simulate in [None, Some(false), Some(true)] {
                for default in [false, true] {
                    for policy in [BroadcastPolicy::Enabled, BroadcastPolicy::Disabled] {
                        let mut value = validated();
                        value.execution_method = method;
                        value.aggregator_sol = aggregator;
                        value.simulate = simulate;
                        let allowed = policy == BroadcastPolicy::Enabled
                            || (simulate.unwrap_or(default)
                                && (method == ExecutionMethod::Pumpapi || aggregator));
                        let result = ensure_execution_allowed(&value, default, policy);
                        assert_eq!(result.is_ok(), allowed);
                        if !allowed {
                            assert_eq!(result.unwrap_err().to_string(), BROADCAST_DISABLED_ERROR);
                        }
                    }
                }
            }
        }
    }
}
