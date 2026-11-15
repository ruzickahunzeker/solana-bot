use super::*;
use std::cell::RefCell;

#[derive(Default)]
struct FixtureParser {
    calls: RefCell<Vec<String>>,
}

impl TradeAddressParser for FixtureParser {
    fn parse(&self, value: &str) -> anyhow::Result<Address> {
        self.calls.borrow_mut().push(value.to_owned());
        if [SOL_MINT, WSOL_MINT, USDC_MINT, USD1_MINT, CUSTOM].contains(&value) {
            Ok(address(value))
        } else {
            anyhow::bail!("fixture syntax failure");
        }
    }
}

const CUSTOM: &str = "11111111111111111111111111111111";

fn address(value: &str) -> Address {
    Address::from_bytes(bs58::decode(value).into_vec().unwrap().try_into().unwrap())
}

fn request() -> TradeInput<'static> {
    TradeInput {
        side: Side::Buy,
        pool: Pool::Pump,
        mint: WSOL_MINT,
        pool_id: None,
        quote_mint: None,
        settlement_asset: None,
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_sol: None,
        amount_lamports: None,
        amount_quote: None,
        amount_usdc: None,
        amount_quote_units: None,
        sell_percent: None,
        slippage_percent: None,
        slippage_bps: None,
        tip_sol: None,
        execution_method: None,
        use_mev: None,
        mev_provider: None,
        simulate: None,
        idempotency_key: "fixture",
    }
}

fn settings() -> TradeValidationSettings {
    TradeValidationSettings {
        default_buy_sol: 0.001,
        default_slippage_percent: 15.0,
        default_tip_sol: 0.0001,
        via_sol_enabled: false,
        via_sol_max_buy_sol: 0.1,
    }
}

#[test]
fn amount_units_and_slippage_keep_existing_rounding_rules() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    assert_eq!(
        validator.token_to_units("amount_sol", 0.001, 9).unwrap(),
        1_000_000
    );
    assert_eq!(
        validator
            .token_to_units("amount_quote", 0.000001, 6)
            .unwrap(),
        1
    );
    assert_eq!(
        validator
            .token_to_units("amount_quote", 0.0000004, 6)
            .unwrap(),
        0
    );
    assert_eq!(validator.percent_to_bps("slippage", 15.004).unwrap(), 1_500);
    assert_eq!(validator.percent_to_bps("slippage", 15.006).unwrap(), 1_501);
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        assert!(validator.token_to_units("amount", value, 9).is_err());
        assert!(validator.percent_to_bps("slippage", value).is_err());
    }
    assert!(validator.token_to_units("amount", 1e30, 9).is_err());
    assert!(validator.percent_to_bps("slippage", 100.01).is_err());
    assert!(parser.calls.borrow().is_empty());
}

#[test]
fn buy_amount_fields_are_exclusive_and_cannot_leak_into_sell() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut request = request();
    request.amount_sol = Some(0.001);
    assert!(validator.validate_amount_fields(&request).is_ok());
    request.amount_quote_units = Some(1);
    assert!(validator.validate_amount_fields(&request).is_err());
    request.amount_quote_units = None;
    request.sell_percent = Some(100);
    assert!(validator.validate_amount_fields(&request).is_err());
    request.side = Side::Sell;
    assert!(validator.validate_amount_fields(&request).is_err());
    request.amount_sol = None;
    assert!(validator.validate_amount_fields(&request).is_ok());
}

#[test]
fn quote_alias_does_not_rewrite_custom_mint() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    assert_eq!(
        validator.normalize_quote_mint(None).unwrap(),
        address(WSOL_MINT)
    );
    assert_eq!(
        validator.normalize_quote_mint(Some(SOL_MINT)).unwrap(),
        address(WSOL_MINT)
    );
    let custom = address(CUSTOM);
    assert_eq!(
        validator.normalize_quote_mint(Some(CUSTOM)).unwrap(),
        custom
    );
    assert!(validator
        .validate_quote_mint_for_pool(Pool::RaydiumCpmm, &custom)
        .is_ok());
    assert!(validator
        .validate_quote_mint_for_pool(Pool::PumpAmm, &custom)
        .is_ok());
    assert!(validator
        .validate_quote_mint_for_pool(Pool::MeteoraDbc, &custom)
        .is_err());
    assert!(validator
        .validate_quote_mint_for_pool(Pool::Bonk, &address(USD1_MINT))
        .is_ok());
    assert!(validator
        .validate_quote_mint_for_pool(Pool::Bonk, &address(USDC_MINT))
        .is_err());
}

#[test]
fn pool_id_validation_retains_backend_specific_rules() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let pool_id = Some(address(CUSTOM));
    assert!(validator
        .validate_pool_id_rule(Pool::Pump, pool_id, ExecutionMethod::Sdk)
        .is_err());
    assert!(validator
        .validate_pool_id_rule(Pool::PumpAmm, None, ExecutionMethod::Sdk)
        .is_ok());
    for pool in [
        Pool::MeteoraDbc,
        Pool::MeteoraDammV2,
        Pool::RaydiumCpmm,
        Pool::StonkFun,
    ] {
        assert!(validator
            .validate_pool_id_rule(pool, None, ExecutionMethod::Sdk)
            .is_err());
        assert!(validator
            .validate_pool_id_rule(pool, pool_id, ExecutionMethod::Sdk)
            .is_ok());
        assert!(validator
            .validate_pool_id_rule(pool, None, ExecutionMethod::Pumpapi)
            .is_ok());
    }
}

#[test]
fn mev_overrides_and_pumpapi_constraints_remain_explicit() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut request = request();
    request.mev_provider = Some(MevProvider::Temporal);
    assert_eq!(
        validator.normalize_mev_override(&request).unwrap(),
        (Some(true), Some(MevProvider::Temporal))
    );
    request.use_mev = Some(false);
    assert!(validator.normalize_mev_override(&request).is_err());
    request.use_mev = None;
    request.mev_provider = Some(MevProvider::Default);
    assert_eq!(
        validator.normalize_mev_override(&request).unwrap(),
        (Some(false), Some(MevProvider::Default))
    );
    assert!(validator
        .validate_execution_method(&request, ExecutionMethod::Pumpapi, Some(true), None)
        .is_err());
    request.side = Side::Sell;
    request.sell_percent = Some(50);
    assert!(validator
        .validate_execution_method(
            &request,
            ExecutionMethod::Pumpapi,
            Some(false),
            Some(MevProvider::Default)
        )
        .is_err());
    request.sell_percent = Some(100);
    assert!(validator
        .validate_execution_method(
            &request,
            ExecutionMethod::Pumpapi,
            Some(false),
            Some(MevProvider::Default)
        )
        .is_ok());
}

#[test]
fn key_and_amount_shape_errors_precede_all_address_parser_calls() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = request();
    input.idempotency_key = " \t";
    input.mint = "not-an-address";
    input.amount_sol = Some(0.001);
    input.amount_quote_units = Some(1);
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "idempotency_key is required"
    );
    assert!(parser.calls.borrow().is_empty());
    input.idempotency_key = "key";
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "buy request must provide at most one amount field"
    );
    assert!(parser.calls.borrow().is_empty());
    input.side = Side::Sell;
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "sell request must not provide buy amount fields"
    );
    assert!(parser.calls.borrow().is_empty());
    input.side = Side::Buy;
    input.amount_quote_units = None;
    input.sell_percent = Some(100);
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "buy request must not provide sell_percent"
    );
    assert!(parser.calls.borrow().is_empty());
    input.sell_percent = None;
    assert_eq!(
        format!(
            "{:#}",
            validator.validate(&input, &settings()).err().unwrap()
        ),
        "mint must be a valid pubkey: fixture syntax failure"
    );
    assert_eq!(&*parser.calls.borrow(), &["not-an-address"]);
}

#[test]
fn parser_keeps_raw_input_and_mint_pool_quote_hop_error_order() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = request();
    input.pool = Pool::RaydiumCpmm;
    input.mint = " ";
    input.pool_id = Some("bad-pool");
    input.quote_mint = Some("bad-quote");
    input.sol_hop_pool_id = Some("bad-hop");
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "mint is required"
    );
    assert!(parser.calls.borrow().is_empty());
    input.mint = " 11111111111111111111111111111111";
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "mint must be a valid pubkey"
    );
    assert_eq!(parser.calls.borrow()[0], input.mint);
    parser.calls.borrow_mut().clear();
    input.mint = CUSTOM;
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "pool_id must be a valid pubkey"
    );
    assert_eq!(&*parser.calls.borrow(), &[CUSTOM, "bad-pool"]);
    parser.calls.borrow_mut().clear();
    input.pool_id = Some(CUSTOM);
    input.quote_mint = Some("");
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "quote_mint must be a valid pubkey"
    );
    assert_eq!(&*parser.calls.borrow(), &[CUSTOM, CUSTOM, ""]);
    input.quote_mint = Some(WSOL_MINT);
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "sol_hop_pool_id must be a valid pubkey"
    );
}

#[test]
fn hop_parse_stays_after_quote_slippage_tip_and_mev_rules() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = request();
    input.pool = Pool::Bonk;
    input.quote_mint = Some(USDC_MINT);
    input.slippage_bps = Some(10_001);
    input.tip_sol = Some(-1.0);
    input.use_mev = Some(false);
    input.mev_provider = Some(MevProvider::Temporal);
    input.sol_hop_pool_id = Some("bad-hop");
    for (expected, stage) in [
        ("bonk quote_mint must be WSOL or USD1", 0),
        ("slippage must be between 0% and 100%", 1),
        ("tip_sol must be greater than or equal to 0", 2),
        ("mev_provider requires use_mev=true or use_mev omitted", 3),
        ("sol_hop_pool_id must be a valid pubkey", 4),
    ] {
        if stage == 1 {
            input.quote_mint = Some(WSOL_MINT);
        }
        if stage == 2 {
            input.slippage_bps = Some(1_500);
        }
        if stage == 3 {
            input.tip_sol = Some(0.0001);
        }
        if stage == 4 {
            input.use_mev = Some(true);
        }
        parser.calls.borrow_mut().clear();
        assert_eq!(
            validator
                .validate(&input, &settings())
                .err()
                .unwrap()
                .to_string(),
            expected
        );
        assert_eq!(
            parser.calls.borrow().iter().any(|value| value == "bad-hop"),
            stage == 4
        );
    }
}

#[test]
fn replay_does_not_apply_current_validation_defaults_or_parse_unused_addresses() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = request();
    input.idempotency_key = "";
    input.mint = "invalid-unused-mint";
    input.pool_id = Some("invalid-unused-pool");
    input.sol_hop_pool_id = Some("invalid-unused-hop");
    input.pool = Pool::Bonk;
    input.quote_mint = Some(CUSTOM);
    input.amount_quote_units = Some(10);
    input.slippage_percent = Some(f64::NAN);
    input.tip_sol = Some(f64::NAN);
    input.use_mev = Some(false);
    input.mev_provider = Some(MevProvider::Temporal);
    assert!(!validator.replay_uses_sol_aggregator(&input).unwrap());
    assert!(!parser
        .calls
        .borrow()
        .iter()
        .any(|value| value.contains("unused")));
    input.amount_quote_units = None;
    input.amount_sol = Some(0.001);
    assert!(validator.replay_uses_sol_aggregator(&input).unwrap());
    input.amount_sol = None;
    input.amount_lamports = Some(1_000_000);
    assert!(!validator.replay_uses_sol_aggregator(&input).unwrap());
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "idempotency_key is required"
    );
}

#[test]
fn replay_amount_shape_precedes_quote_parse_but_ignores_current_pool_policy() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = request();
    input.quote_mint = Some("bad-quote");
    input.amount_sol = Some(0.001);
    input.amount_quote_units = Some(5);
    assert_eq!(
        validator
            .replay_uses_sol_aggregator(&input)
            .err()
            .unwrap()
            .to_string(),
        "buy request must provide at most one amount field"
    );
    assert!(parser.calls.borrow().is_empty());
    input.amount_sol = None;
    assert_eq!(
        validator
            .replay_uses_sol_aggregator(&input)
            .err()
            .unwrap()
            .to_string(),
        "quote_mint must be a valid pubkey"
    );
    input.quote_mint = Some(CUSTOM);
    input.pool = Pool::Bonk;
    assert!(!validator.replay_uses_sol_aggregator(&input).unwrap());
    assert_eq!(
        validator
            .validate(&input, &settings())
            .err()
            .unwrap()
            .to_string(),
        "bonk quote_mint must be WSOL or USD1"
    );
}

#[test]
fn native_input_preserves_quote_units_aliases_and_simulate_tristate() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = request();
    input.pool = Pool::RaydiumCpmm;
    input.pool_id = Some(CUSTOM);
    input.mint = CUSTOM;
    input.quote_mint = Some(USDC_MINT);
    input.amount_quote_units = Some(u64::MAX);
    for simulate in [None, Some(false), Some(true)] {
        input.simulate = simulate;
        let result = validator.validate(&input, &settings()).unwrap();
        assert_eq!(result.simulate, simulate);
        assert_eq!(result.mint, address(CUSTOM));
        assert_eq!(result.pool_id, Some(address(CUSTOM)));
        assert_eq!(result.quote_mint, address(USDC_MINT));
        assert_eq!(result.amount_lamports, Some(u64::MAX));
        assert!(!result.aggregator_sol);
    }
    input.quote_mint = Some(SOL_MINT);
    let result = validator.validate(&input, &settings()).unwrap();
    assert_eq!(result.quote_mint, address(WSOL_MINT));
    assert_eq!(result.amount_lamports, Some(u64::MAX));
}

#[test]
fn warmup_native_input_preserves_distinct_project_addresses_and_only_normalizes_quote() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = WarmupInput {
        pool: Pool::RaydiumCpmm,
        mint: CUSTOM,
        pool_id: Some(USDC_MINT),
        quote_mint: Some(USD1_MINT),
    };
    let result = validator.validate_warmup(&input).unwrap();
    assert_eq!(result.mint, address(CUSTOM));
    assert_eq!(result.pool_id, Some(address(USDC_MINT)));
    assert_eq!(result.quote_mint, address(USD1_MINT));
    input.mint = SOL_MINT;
    input.quote_mint = Some(SOL_MINT);
    let result = validator.validate_warmup(&input).unwrap();
    assert_eq!(result.mint, address(SOL_MINT));
    assert_eq!(result.pool_id, Some(address(USDC_MINT)));
    assert_eq!(result.quote_mint, address(WSOL_MINT));
    input.quote_mint = None;
    assert_eq!(
        validator.validate_warmup(&input).unwrap().quote_mint,
        address(WSOL_MINT)
    );
}

#[test]
fn warmup_keeps_address_order_and_quote_policy_before_required_pool_id() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    let mut input = WarmupInput {
        pool: Pool::MeteoraDbc,
        mint: " ",
        pool_id: Some("bad-pool"),
        quote_mint: Some("bad-quote"),
    };
    assert_eq!(
        validator.validate_warmup(&input).err().unwrap().to_string(),
        "mint is required"
    );
    assert!(parser.calls.borrow().is_empty());
    input.mint = CUSTOM;
    assert_eq!(
        validator.validate_warmup(&input).err().unwrap().to_string(),
        "pool_id must be a valid pubkey"
    );
    assert_eq!(&*parser.calls.borrow(), &[CUSTOM, "bad-pool"]);
    parser.calls.borrow_mut().clear();
    input.pool_id = Some(" ");
    assert_eq!(
        validator.validate_warmup(&input).err().unwrap().to_string(),
        "pool_id is required"
    );
    assert_eq!(&*parser.calls.borrow(), &[CUSTOM]);
    parser.calls.borrow_mut().clear();
    input.pool_id = None;
    assert_eq!(
        validator.validate_warmup(&input).err().unwrap().to_string(),
        "quote_mint must be a valid pubkey"
    );
    assert_eq!(&*parser.calls.borrow(), &[CUSTOM, "bad-quote"]);
    input.quote_mint = Some(USD1_MINT);
    assert_eq!(
        validator.validate_warmup(&input).err().unwrap().to_string(),
        format!("MeteoraDbc quote_mint must be WSOL or USDC; received {USD1_MINT}")
    );
    input.quote_mint = Some(WSOL_MINT);
    assert_eq!(
        validator.validate_warmup(&input).err().unwrap().to_string(),
        "pool_id is required for MeteoraDbc"
    );
}

#[test]
fn query_parser_preserves_raw_syntax_without_warmup_required_rule_or_quote_alias() {
    let parser = FixtureParser::default();
    let validator = TradeValidator::new(&parser);
    for field in ["mint", "quote_mint"] {
        for value in ["", " ", "bad-address"] {
            parser.calls.borrow_mut().clear();
            let error = validator.parse_query_address(field, value).unwrap_err();
            assert_eq!(
                format!("{error:#}"),
                format!("{field} must be a valid pubkey: fixture syntax failure")
            );
            assert_eq!(&*parser.calls.borrow(), &[value]);
        }
        assert_eq!(
            validator.parse_query_address(field, SOL_MINT).unwrap(),
            address(SOL_MINT)
        );
        assert_eq!(
            validator.parse_query_address(field, WSOL_MINT).unwrap(),
            address(WSOL_MINT)
        );
    }
}
