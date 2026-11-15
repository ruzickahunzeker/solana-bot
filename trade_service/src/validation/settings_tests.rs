use super::*;
use serde_json::json;

fn settings() -> TradeValidationSettings {
    TradeValidationSettings {
        default_buy_sol: 0.123456789,
        default_slippage_percent: 6.78,
        default_tip_sol: 0.000321,
        via_sol_enabled: true,
        via_sol_max_buy_sol: 0.2,
    }
}
fn request(patch: serde_json::Value) -> TradeRequest {
    let mut value = json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111","idempotency_key":"settings-only"});
    value
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

#[test]
fn narrow_settings_preserve_default_values_and_explicit_overrides_without_full_config() {
    let defaults = settings();
    let result = validate_with_settings(&request(json!({})), &defaults).unwrap();
    assert_eq!(result.amount_lamports, Some(123_456_789));
    assert_eq!(result.slippage_bps, 678);
    assert_eq!(result.tip_sol.to_bits(), defaults.default_tip_sol.to_bits());
    assert_eq!(result.simulate, None);
    let result = validate_with_settings(
        &request(json!({"amount_lamports":17,"slippage_bps":901,"tip_sol":0.000654})),
        &defaults,
    )
    .unwrap();
    assert_eq!(result.amount_lamports, Some(17));
    assert_eq!(result.slippage_bps, 901);
    assert_eq!(result.tip_sol.to_bits(), 0.000654f64.to_bits());
}

#[test]
fn via_sol_feature_gate_precedes_amount_limit_with_original_errors_and_exact_limit() {
    let request = request(
        json!({"pool":"stonkfun","pool_id":"11111111111111111111111111111111",
        "quote_mint":"11111111111111111111111111111111","settlement_asset":"sol",
        "sol_hop_pool":"raydium-cpmm","sol_hop_pool_id":"11111111111111111111111111111111","amount_sol":0.03}),
    );
    let mut defaults = settings();
    defaults.via_sol_enabled = false;
    defaults.via_sol_max_buy_sol = 0.02;
    assert_eq!(
        validate_with_settings(&request, &defaults)
            .err()
            .unwrap()
            .to_string(),
        "stonkfun Via-SOL is disabled by STONKFUN_VIA_SOL_ENABLED"
    );
    defaults.via_sol_enabled = true;
    assert_eq!(
        validate_with_settings(&request, &defaults)
            .err()
            .unwrap()
            .to_string(),
        "stonkfun Via-SOL buy exceeds STONKFUN_VIA_SOL_MAX_BUY_SOL"
    );
    defaults.via_sol_max_buy_sol = 0.03;
    assert_eq!(
        validate_with_settings(&request, &defaults)
            .unwrap()
            .amount_lamports,
        Some(30_000_000)
    );
}

#[test]
fn unused_invalid_defaults_are_not_eagerly_rejected_by_the_new_settings_boundary() {
    let defaults = TradeValidationSettings {
        default_buy_sol: f64::NAN,
        default_slippage_percent: f64::NAN,
        default_tip_sol: f64::NAN,
        via_sol_enabled: false,
        via_sol_max_buy_sol: f64::NAN,
    };
    let explicit = request(json!({"amount_lamports":100,"slippage_bps":500,"tip_sol":0.0}));
    let normalized = validate_with_settings(&explicit, &defaults).unwrap();
    assert_eq!(normalized.amount_lamports, Some(100));
    assert_eq!(normalized.slippage_bps, 500);
    let mut needs_tip = explicit.clone();
    needs_tip.tip_sol = None;
    assert_eq!(
        validate_with_settings(&needs_tip, &defaults)
            .err()
            .unwrap()
            .to_string(),
        "tip_sol must be greater than or equal to 0"
    );
    let mut needs_slippage = explicit;
    needs_slippage.slippage_bps = None;
    assert_eq!(
        validate_with_settings(&needs_slippage, &defaults)
            .err()
            .unwrap()
            .to_string(),
        "DEFAULT_SLIPPAGE_PERCENT must be between 0 and 100"
    );
}

#[tokio::test]
async fn validation_settings_projection_copies_exact_values_without_eager_resource_or_value_checks()
{
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.default_buy_sol = 0.123456789;
    config.default_slippage_percent = 6.78;
    config.default_tip_sol = -0.0;
    config.stonkfun_via_sol.enabled = false;
    config.stonkfun_via_sol.max_buy_sol = f64::NAN;
    config.rpc_urls.clear();
    config.wallets.clear();
    let snapshot = config.trade_validation_settings();
    config.default_buy_sol = 42.0;
    config.default_slippage_percent = 42.0;
    config.default_tip_sol = 42.0;
    config.stonkfun_via_sol.enabled = true;
    config.stonkfun_via_sol.max_buy_sol = 42.0;
    assert_eq!(snapshot.default_buy_sol.to_bits(), 0.123456789f64.to_bits());
    assert_eq!(
        snapshot.default_slippage_percent.to_bits(),
        6.78f64.to_bits()
    );
    assert_eq!(snapshot.default_tip_sol.to_bits(), (-0.0f64).to_bits());
    assert!(!snapshot.via_sol_enabled);
    assert_eq!(snapshot.via_sol_max_buy_sol.to_bits(), f64::NAN.to_bits());
    let normalized = validate_with_settings(&request(json!({})), &snapshot).unwrap();
    assert_eq!(normalized.amount_lamports, Some(123456789));
    assert_eq!(normalized.slippage_bps, 678);
    assert_eq!(normalized.tip_sol.to_bits(), (-0.0f64).to_bits());
}
