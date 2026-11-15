//! Synthetic V1 normalization and durable identity baseline; never loads .env or RPC.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    models::TradeRequest,
    persistence::request_hash,
    validation::{validate_trade, NormalizedTrade},
};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../tests/fixtures/contracts/trade_normalization_v1.json"
    ))
    .unwrap()
}

fn request(patch: &Value) -> TradeRequest {
    let mut value = json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111","idempotency_key":" literal-key "});
    value
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

fn snapshot(n: &NormalizedTrade) -> Value {
    json!({
        "mint":n.mint.to_string(), "quote_mint":n.quote_mint.to_string(),
        "pool_id":n.pool_id.map(|v|v.to_string()),"settlement_asset":n.settlement_asset,
        "sol_hop_pool":n.sol_hop_pool,"sol_hop_pool_id":n.sol_hop_pool_id.map(|v|v.to_string()),
        "amount_lamports":n.amount_lamports,"sell_percent":n.sell_percent,
        "slippage_bps":n.slippage_bps,"tip_sol_bits":n.tip_sol.to_bits(),
        "execution_method":n.execution_method,"use_mev":n.use_mev,"mev_provider":n.mev_provider,
        "simulate":n.simulate,"aggregator_sol":n.aggregator_sol,
    })
}

#[tokio::test]
async fn ordinary_trade_v1_normalization_and_hash_baseline() {
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.stonkfun_via_sol.enabled = true;
    config.default_tip_sol = 0.0001;
    config.pumpapi.default_priority_fee_sol = 0.0002;
    let mut observations = vec![];
    for case in fixture()["accepted"].as_array().unwrap() {
        let request = request(&case["patch"]);
        let normalized =
            validate_trade(&request, &config).unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        observations.push(json!({"name":case["name"],"wire":request,"normalized":snapshot(&normalized),"hash":request_hash("fixture-wallet",&request,&normalized).unwrap()}));
    }
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&observations).unwrap())
    );
    assert_eq!(
        fingerprint,
        "cf2a091b670a8e19841659134a122f1a42f6817371f96c7ae4759ff19b838322"
    );
}

#[tokio::test]
async fn ordinary_trade_v1_errors_and_validation_order_remain_stable() {
    let state = crate::http_contract_tests::state().await;
    for case in fixture()["rejected"].as_array().unwrap() {
        let error = validate_trade(&request(&case["patch"]), &state.config)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            case["error"].as_str().unwrap(),
            "{}",
            case["patch"]
        );
    }
}

#[tokio::test]
async fn internal_command_and_both_adapter_mappings_preserve_normalized_values() {
    use crate::{
        adapters::{pumpapi::pumpapi_trade, sdk::trade_command::sdk_trade},
        domain::trade::TradeAmount,
    };
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.stonkfun_via_sol.enabled = true;
    config.default_tip_sol = 0.0001;
    config.pumpapi.default_priority_fee_sol = 0.0002;
    let hash = solana_sdk::hash::Hash::new_from_array([42; 32]);
    for case in fixture()["accepted"].as_array().unwrap() {
        let request = request(&case["patch"]);
        let n = validate_trade(&request, &config).unwrap();
        let command = request
            .to_command("resolved-wallet".into(), &n, &config)
            .unwrap();
        assert_eq!(command.wallet_id, "resolved-wallet");
        assert_eq!(command.idempotency_key, " literal-key ");
        assert_eq!(command.amount.side(), request.side);
        assert_eq!(command.route.pool, request.pool);
        assert_eq!(command.route.mint.to_string(), request.mint);
        assert_eq!(
            command.route.pool_id.map(|v| v.to_string()),
            request.pool_id
        );
        assert_eq!(command.quote_mint().to_string(), n.quote_mint.to_string());
        assert_eq!(command.aggregator_sol, n.aggregator_sol);
        let expected_kind = match case["name"].as_str().unwrap() {
            "usdc_units" | "usdc_alias" | "usd1" | "custom_quote" => "quote",
            "via_sol_sell" | "pumpapi_tip" | "sell_dust_percent" => "sell",
            _ => "sol",
        };
        assert_eq!(
            match command.amount {
                TradeAmount::BuySol { .. } => "sol",
                TradeAmount::BuyQuote { mint, .. } => {
                    assert_eq!(mint.to_bytes(), n.quote_mint.to_bytes());
                    "quote"
                }
                TradeAmount::SellPercent { .. } => "sell",
            },
            expected_kind,
            "{}",
            case["name"]
        );
        for wait in [false, true] {
            let sdk = sdk_trade(
                &command,
                crate::domain::blockhash::Blockhash::from_bytes(hash.to_bytes()),
                wait,
                987,
            );
            assert_eq!(sdk.pool, request.pool);
            assert_eq!(sdk.mint.to_bytes(), n.mint.to_bytes());
            assert_eq!(sdk.quote_mint.to_bytes(), n.quote_mint.to_bytes());
            assert_eq!(
                sdk.pool_id.map(|v| v.to_bytes()),
                n.pool_id.map(|v| v.to_bytes())
            );
            assert_eq!(sdk.sol_hop_pool, n.sol_hop_pool);
            assert_eq!(
                sdk.sol_hop_pool_id.map(|v| v.to_bytes()),
                n.sol_hop_pool_id.map(|v| v.to_bytes())
            );
            assert_eq!(sdk.amount_lamports, n.amount_lamports);
            assert_eq!(sdk.sell_percent, n.sell_percent);
            assert_eq!(sdk.slippage_bps, n.slippage_bps);
            assert_eq!(sdk.tip_sol.to_bits(), n.tip_sol.to_bits());
            assert_eq!(sdk.recent_blockhash, hash);
            assert_eq!(sdk.wait_tx_confirmed, wait);
            assert_eq!(sdk.mev_submit_verify_ms, 987);
            assert_eq!(sdk.simulate, n.simulate.unwrap_or(config.enable_simulate));
            assert_eq!(sdk.use_mev, n.use_mev);
            assert_eq!(sdk.mev_provider, n.mev_provider);
            let pumpapi = pumpapi_trade(&command, config.pumpapi.clone(), wait);
            let projected = crate::adapters::pumpapi::pumpapi_request(
                crate::domain::trade::Address::from_bytes([42; 32]),
                &pumpapi,
            );
            assert_eq!(pumpapi.config().url, config.pumpapi.url);
            assert_eq!(projected.side, request.side);
            assert_eq!(projected.mint.to_bytes(), n.mint.to_bytes());
            assert_eq!(projected.quote_mint.to_bytes(), n.quote_mint.to_bytes());
            assert_eq!(
                projected.pool_id.map(|v| v.to_bytes()),
                n.pool_id.map(|v| v.to_bytes())
            );
            assert_eq!(projected.amount_units, n.amount_lamports);
            assert_eq!(projected.sell_percent, n.sell_percent);
            assert_eq!(projected.slippage_bps, n.slippage_bps);
            assert_eq!(
                projected.priority_fee_sol.to_bits(),
                request
                    .tip_sol
                    .unwrap_or(config.pumpapi.default_priority_fee_sol)
                    .to_bits()
            );
            assert_eq!(pumpapi.simulate(), sdk.simulate);
            assert_eq!(pumpapi.wait_tx_confirmed(), wait);
            assert_eq!(pumpapi.command().wallet_id, command.wallet_id);
            assert_eq!(pumpapi.command().idempotency_key, command.idempotency_key);
            assert_eq!(pumpapi.command().route.pool, command.route.pool);
        }
    }
}

#[tokio::test]
async fn resolved_mode_keeps_wire_tristate_and_durable_hashes_distinct() {
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    let mut hashes = vec![];
    for default in [false, true] {
        config.enable_simulate = default;
        for requested in [None, Some(false), Some(true)] {
            let mut request = request(&json!({}));
            request.simulate = requested;
            let n = validate_trade(&request, &config).unwrap();
            let command = request.to_command("wallet".into(), &n, &config).unwrap();
            assert_eq!(
                command.execution.mode.simulate(),
                requested.unwrap_or(default)
            );
            assert_eq!(n.simulate, requested);
            hashes.push(request_hash("wallet", &request, &n).unwrap());
        }
    }
    assert_eq!(hashes[..3], hashes[3..]);
    assert_ne!(hashes[0], hashes[1]);
    assert_ne!(hashes[0], hashes[2]);
    assert_ne!(hashes[1], hashes[2]);
}

#[tokio::test]
async fn sdk_tip_and_pumpapi_priority_fee_defaults_stay_independent() {
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.default_tip_sol = 0.0001;
    config.pumpapi.default_priority_fee_sol = 0.0002;
    for tip in [None, Some(0.0), Some(-0.0), Some(0.0003)] {
        let mut request = request(&json!({"execution_method":"pumpapi"}));
        request.tip_sol = tip;
        let n = validate_trade(&request, &config).unwrap();
        let command = request.to_command("wallet".into(), &n, &config).unwrap();
        assert_eq!(
            command.execution.tip_sol.to_bits(),
            tip.unwrap_or(0.0001).to_bits()
        );
        assert_eq!(
            command.pumpapi_priority_fee_sol.to_bits(),
            tip.unwrap_or(0.0002).to_bits()
        );
    }
}

#[tokio::test]
async fn metadata_and_amount_aliases_keep_existing_durable_identity() {
    let state = crate::http_contract_tests::state().await;
    let hash = |r: &TradeRequest| {
        request_hash("wallet", r, &validate_trade(r, &state.config).unwrap()).unwrap()
    };
    let first = request(&json!({"amount_sol":0.001}));
    let raw_units = request(&json!({"amount_lamports":1000000}));
    assert_eq!(hash(&first), hash(&raw_units));
    let mut metadata = first.clone();
    metadata.wallet_id = Some("unresolved-alias".into());
    metadata.source = Some("fixture".into());
    metadata.source_signature = Some("synthetic".into());
    metadata.idempotency_key = "different-lookup-key".into();
    assert_eq!(hash(&first), hash(&metadata));
    assert_ne!(
        hash(&first),
        request_hash(
            "other-wallet",
            &first,
            &validate_trade(&first, &state.config).unwrap()
        )
        .unwrap()
    );
    let mut negative_zero = first.clone();
    negative_zero.tip_sol = Some(-0.0);
    assert_ne!(hash(&first), hash(&negative_zero));
}

#[tokio::test]
async fn nonfinite_amount_slippage_and_tip_are_rejected_before_command_mapping() {
    let state = crate::http_contract_tests::state().await;
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for field in ["amount_sol", "amount_quote", "slippage_percent", "tip_sol"] {
            let mut request = request(&json!({}));
            match field {
                "amount_sol" => request.amount_sol = Some(value),
                "amount_quote" => request.amount_quote = Some(value),
                "slippage_percent" => request.slippage_percent = Some(value),
                "tip_sol" => request.tip_sol = Some(value),
                _ => unreachable!(),
            }
            assert!(validate_trade(&request, &state.config).is_err(), "{field}");
        }
    }
}

#[tokio::test]
async fn command_mapping_rejects_missing_normalized_amount_without_panicking() {
    let state = crate::http_contract_tests::state().await;
    for patch in [json!({}), json!({"side":"sell","sell_percent":100})] {
        let request = request(&patch);
        let mut n = validate_trade(&request, &state.config).unwrap();
        n.amount_lamports = None;
        n.sell_percent = None;
        assert!(request
            .to_command("wallet".into(), &n, &state.config)
            .is_err());
    }
}

#[tokio::test]
async fn project_validated_addresses_stay_distinct_through_command_and_sdk_mapping() {
    use crate::{
        application::validated_trade::ValidatedTrade,
        domain::{blockhash::Blockhash, trade::Address},
    };
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.stonkfun_via_sol.enabled = true;
    for seed in 1u8..=32 {
        let mint = Address::from_bytes([seed; 32]);
        let quote = Address::from_bytes([seed + 1; 32]);
        let pool = Address::from_bytes([seed + 2; 32]);
        let hop = Address::from_bytes([seed + 3; 32]);
        let request = request(
            &json!({"pool":"stonkfun","mint":mint.to_string(),"quote_mint":quote.to_string(),
            "pool_id":pool.to_string(),"settlement_asset":"sol","sol_hop_pool":"raydium-cpmm",
            "sol_hop_pool_id":hop.to_string(),"amount_lamports":1_000_000}),
        );
        let normalized: ValidatedTrade = validate_trade(&request, &config).unwrap();
        let _: Address = normalized.mint;
        assert_eq!(
            (
                normalized.mint,
                normalized.quote_mint,
                normalized.pool_id,
                normalized.sol_hop_pool_id
            ),
            (mint, quote, Some(pool), Some(hop))
        );
        let command = request
            .to_command("wallet".into(), &normalized, &config)
            .unwrap();
        assert_eq!(
            (
                command.route.mint,
                command.route.quote_mint,
                command.route.pool_id,
                command.route.sol_hop_pool_id
            ),
            (mint, quote, Some(pool), Some(hop))
        );
        let sdk = crate::adapters::sdk::trade_command::sdk_trade(
            &command,
            Blockhash::from_bytes([42; 32]),
            false,
            0,
        );
        assert_eq!(sdk.mint.to_bytes(), mint.to_bytes());
        assert_eq!(sdk.quote_mint.to_bytes(), quote.to_bytes());
        assert_eq!(sdk.pool_id.unwrap().to_bytes(), pool.to_bytes());
        assert_eq!(sdk.sol_hop_pool_id.unwrap().to_bytes(), hop.to_bytes());
    }
}

#[tokio::test]
async fn old_wire_pubkey_parser_and_failure_order_precede_project_address_conversion() {
    use crate::domain::trade::Address;
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.stonkfun_via_sol.enabled = true;
    let base = json!({"pool":"stonkfun","mint":Address::from_bytes([1;32]).to_string(),
        "quote_mint":Address::from_bytes([2;32]).to_string(),"pool_id":Address::from_bytes([3;32]).to_string(),
        "settlement_asset":"sol","sol_hop_pool":"raydium-cpmm","sol_hop_pool_id":Address::from_bytes([4;32]).to_string(),"amount_lamports":1_000_000});
    for (patch, message) in [
        (
            json!({"mint":"invalid","pool_id":"invalid","quote_mint":"invalid","sol_hop_pool_id":"invalid"}),
            "mint must be a valid pubkey",
        ),
        (
            json!({"pool_id":"invalid","quote_mint":"invalid","sol_hop_pool_id":"invalid"}),
            "pool_id must be a valid pubkey",
        ),
        (
            json!({"quote_mint":"invalid","sol_hop_pool_id":"invalid"}),
            "quote_mint must be a valid pubkey",
        ),
        (
            json!({"sol_hop_pool_id":"invalid"}),
            "sol_hop_pool_id must be a valid pubkey",
        ),
        (
            json!({"mint":"1".repeat(1000)}),
            "mint must be a valid pubkey",
        ),
        (json!({"mint":" "}), "mint is required"),
    ] {
        let mut value = base.clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert_eq!(
            validate_trade(&request(&value), &config)
                .err()
                .unwrap()
                .to_string(),
            message
        );
    }
}

#[tokio::test]
async fn project_validated_result_preserves_simulate_tristate_and_exact_quote_units() {
    use crate::domain::trade::{TradeAmount, USDC_MINT};
    let state = crate::http_contract_tests::state().await;
    for simulate in [None, Some(false), Some(true)] {
        let request = request(
            &json!({"pool":"raydium-cpmm","pool_id":"11111111111111111111111111111111",
            "quote_mint":USDC_MINT,"amount_quote_units":u64::MAX,"simulate":simulate}),
        );
        let normalized = validate_trade(&request, &state.config).unwrap();
        assert_eq!(normalized.simulate, simulate);
        assert_eq!(normalized.amount_lamports, Some(u64::MAX));
        assert_eq!(normalized.quote_mint.to_string(), USDC_MINT);
        let command = request
            .to_command("wallet".into(), &normalized, &state.config)
            .unwrap();
        assert!(
            matches!(command.amount, TradeAmount::BuyQuote {mint, units} if mint == normalized.quote_mint && units == u64::MAX)
        );
        assert_eq!(
            command.execution.mode.simulate(),
            simulate.unwrap_or(state.config.enable_simulate)
        );
    }
}

#[tokio::test]
async fn legacy_hash_omits_sol_aggregator_vs_direct_quote_input_kind() {
    use crate::domain::trade::{TradeAmount, USDC_MINT};
    let state = crate::http_contract_tests::state().await;
    let sol = request(&json!({"pool":"pump-amm","quote_mint":USDC_MINT,"amount_sol":0.001}));
    let quote = request(&json!({"pool":"pump-amm","quote_mint":USDC_MINT,"amount_usdc":1.0}));
    let sol_n = validate_trade(&sol, &state.config).unwrap();
    let quote_n = validate_trade(&quote, &state.config).unwrap();
    assert!(sol_n.aggregator_sol);
    assert!(!quote_n.aggregator_sol);
    assert!(matches!(
        sol.to_command("wallet".into(), &sol_n, &state.config)
            .unwrap()
            .amount,
        TradeAmount::BuySol {
            lamports: 1_000_000
        }
    ));
    assert!(matches!(
        quote
            .to_command("wallet".into(), &quote_n, &state.config)
            .unwrap()
            .amount,
        TradeAmount::BuyQuote {
            units: 1_000_000,
            ..
        }
    ));
    // Historical hash behavior only, not an assertion that these trades are equivalent.
    assert_eq!(
        request_hash("wallet", &sol, &sol_n).unwrap(),
        request_hash("wallet", &quote, &quote_n).unwrap()
    );
}
